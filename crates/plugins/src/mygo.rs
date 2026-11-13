use anyhow::{Context, Result, anyhow, ensure};
use md5::{Digest, Md5};
use musicx_core::protocol::tr;
use reqwest::{Method, Url, blocking::Client, redirect::Policy};
use rquickjs::{
    CatchResultExt, Context as JsContext, Function, Module, Runtime, Value as JsValue,
    promise::MaybePromise,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::protocol::Track;
use musicx_core::{metadata::merge_metadata, storage::state_dir};
use musicx_plugin_api::Media;

fn aes_ecb(key: &str, text: &str) -> Result<String> {
    use aes::{
        Aes128,
        cipher::{Block, BlockEncrypt, KeyInit},
    };
    let cipher = Aes128::new_from_slice(key.as_bytes())
        .map_err(|_| anyhow!(tr!("AES key must be 16 bytes")))?;
    ensure!(
        text.len() <= MAX_BODY as usize,
        tr!("Encryption input is too large")
    );
    let padding = 16 - text.len() % 16;
    let mut bytes = text.as_bytes().to_vec();
    bytes.resize(bytes.len() + padding, padding as u8);
    for chunk in bytes.chunks_exact_mut(16) {
        let mut block = Block::<Aes128>::default();
        block.copy_from_slice(chunk);
        cipher.encrypt_block(&mut block);
        chunk.copy_from_slice(&block);
    }
    Ok(hex::encode(bytes))
}

const MAX_BODY: u64 = 2 * 1024 * 1024;
const METADATA_TIMEOUT: Duration = Duration::from_secs(3);
static PLUGIN_CALL: Mutex<()> = Mutex::new(());
static METADATA: Mutex<VecDeque<(String, String, Track)>> = Mutex::new(VecDeque::new());

/// Normalize source results at the adapter boundary so players receive one Track type.
pub fn search(plugin: &str, query: &str, offset: u64) -> Result<Value> {
    search_kind(plugin, query, "tracks", offset)
}

pub fn search_kind(plugin: &str, query: &str, kind: &str, offset: u64) -> Result<Value> {
    ensure!(
        matches!(
            kind,
            "tracks" | "albums" | "artists" | "playlists" | "shows"
        ),
        tr!("Unsupported search type")
    );
    let query = query.trim();
    ensure!(
        !query.is_empty() && query.chars().count() <= 120,
        tr!("Enter 1 to 120 characters")
    );
    let mut result = invoke(
        plugin,
        "search",
        json!({"query":query,"kind":kind,"offset":offset}),
    )?;
    if kind != "tracks" {
        return Ok(result);
    }
    let rows = result["tracks"]
        .as_array()
        .context(tr!("The source returned no song list"))?;
    let mut tracks = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(track) = catalog_track(plugin, row) else {
            continue;
        };
        let mut track = serde_json::to_value(track)?;
        if let Some(unavailable) = row["unavailable"].as_bool() {
            track["unavailable"] = Value::Bool(unavailable);
        }
        tracks.push(track);
    }
    remember_artwork(plugin, &result);
    result["tracks"] = Value::Array(tracks);
    Ok(result)
}

/// Keep a small metadata-only window of plugin search covers, never image bytes.
pub fn remember_artwork(plugin: &str, result: &Value) {
    let Some(tracks) = result["tracks"].as_array() else {
        return;
    };
    let mut cache = METADATA.lock().unwrap_or_else(|e| e.into_inner());
    for track in tracks.iter().take(32) {
        if let Some(track) = catalog_track(plugin, track) {
            let id = track.source.as_ref().expect("plugin track").id.clone();
            cache.retain(|(p, i, _)| p != plugin || i != &id);
            if cache.len() == 32 {
                cache.pop_front();
            }
            cache.push_back((plugin.to_owned(), id, track));
        }
    }
}

pub(crate) fn catalog_track(plugin: &str, value: &Value) -> Option<Track> {
    let id = value["id"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| value["id"].as_u64().map(|id| id.to_string()))?;
    if id.is_empty() || id.len() > 512 {
        return None;
    }
    let album = &value["album"];
    Some(Track {
        id: id.parse().unwrap_or(0),
        title: value["name"].as_str()?.chars().take(512).collect(),
        artist: value["artists"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["name"].as_str())
            .collect::<Vec<_>>()
            .join(" / ")
            .chars()
            .take(1024)
            .collect(),
        artists: value["artists"]
            .as_array()
            .into_iter()
            .flatten()
            .take(64)
            .filter_map(|a| {
                Some(crate::protocol::Artist {
                    id: a["id"].as_str()?.chars().take(512).collect(),
                    name: a["name"].as_str()?.chars().take(512).collect(),
                    cover_url: a["imageUrl"]
                        .as_str()
                        .and_then(|url| crate::assets::artwork(plugin, url)),
                })
            })
            .collect(),
        episode: value["showId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(|id| crate::protocol::Episode {
                show_id: id.chars().take(512).collect(),
                description: value["description"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(8000)
                    .collect(),
                published: value["published"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(100)
                    .collect(),
            }),
        duration: value["duration"].as_f64().unwrap_or(0.0) / 1000.0,
        cover_url: [&album["imageUrl"], &value["imageUrl"], &value["coverUrl"]]
            .into_iter()
            .filter_map(Value::as_str)
            .find_map(|url| crate::assets::artwork(plugin, url)),
        album: album["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(|id| crate::protocol::Album {
                id: id.chars().take(512).collect(),
                name: album["name"]
                    .as_str()
                    .unwrap_or(tr!("Album"))
                    .chars()
                    .take(512)
                    .collect(),
            }),
        source_url: value["url"]
            .as_str()
            .and_then(musicx_core::metadata::public_url),
        source: Some(Source {
            plugin: plugin.to_owned(),
            id,
        }),
    })
}

/// Only enrich the same source identity. Older queue and library entries may
/// have title and artist but lack all three catalog fields.
pub(crate) fn fill_artwork(track: &mut Track) {
    let Some(source) = track.source.clone() else {
        return;
    };
    {
        let cache = METADATA.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, _, metadata)) = cache
            .iter()
            .find(|(plugin, id, _)| plugin == &source.plugin && id == &source.id)
        {
            merge_metadata(track, metadata);
        }
    }
    if track.cover_url.is_some() {
        return;
    }
    let query: String = format!("{} {}", track.artist, track.title)
        .trim()
        .chars()
        .take(120)
        .collect();
    if query.is_empty() {
        return;
    }
    // A cover lookup is optional and must not queue behind another plugin call.
    let Ok(_guard) = PLUGIN_CALL.try_lock() else {
        return;
    };
    if crate::music_sources::auth_active(&source.plugin) {
        return;
    }
    let Ok(result) = invoke_inner(
        &source.plugin,
        "search",
        json!({"query":query,"kind":"tracks","offset":0}),
        None,
        None,
        METADATA_TIMEOUT,
    ) else {
        return;
    };
    if let Some(metadata) = result["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|value| {
            let id = &value["id"];
            let matches = id.as_str().is_some_and(|id| id == source.id)
                || id.as_u64().is_some_and(|id| id.to_string() == source.id);
            matches
                .then(|| catalog_track(&source.plugin, value))
                .flatten()
        })
    {
        merge_metadata(track, &metadata);
    }
}

pub fn source_name(plugin: &str) -> String {
    if !valid_id(plugin) {
        return tr!("No source connected").into();
    }
    plugin_root()
        .and_then(|root| manifest(&root.join(plugin)))
        .map_or_else(
            |_| plugin.to_owned(),
            |manifest| {
                crate::localization::translations(plugin)
                    .get(&manifest.name)
                    .cloned()
                    .unwrap_or(manifest.name)
            },
        )
}

pub(crate) fn source_repository(plugin: &str) -> Option<String> {
    if !valid_id(plugin) {
        return None;
    }
    let manifest = manifest(&plugin_root().ok()?.join(plugin)).ok()?;
    musicx_core::metadata::public_url(manifest.repository.as_deref()?)
}

pub fn album(track: &Track) -> Result<crate::protocol::AlbumPage> {
    let source = track.source.as_ref().context(tr!("Source missing"))?;
    let mut album = track
        .album
        .clone()
        .context(tr!("This source has no album information"))?;
    let result = invoke(&source.plugin, "album", json!({"id":album.id}))?;
    let descriptor = if result["album"].is_object() {
        &result["album"]
    } else {
        &result
    };
    if let Some(name) = descriptor["name"].as_str().filter(|v| !v.is_empty()) {
        album.name = name.chars().take(512).collect();
    }
    remember_artwork(&source.plugin, &result);
    let tracks = result["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| catalog_track(&source.plugin, value))
        .collect();
    Ok(crate::protocol::AlbumPage { album, tracks })
}

pub fn lyrics(track: &Track) -> Result<crate::protocol::Lyrics> {
    let source = track.source.as_ref().context(tr!("Source missing"))?;
    let result = invoke(&source.plugin, "lyrics", json!({"id":source.id}))?;
    Ok(parse_lyrics(&result))
}

fn parse_lyrics(result: &Value) -> crate::protocol::Lyrics {
    let lines = result["lines"].as_array();
    let original = lines
        .map(|lines| {
            lines
                .iter()
                .filter_map(|line| {
                    if let Some(text) = line["text"].as_str().filter(|s| !s.is_empty()) {
                        return Some(text.to_owned());
                    }
                    let text: String = line["words"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|word| word["text"].as_str())
                        .collect();
                    (!text.is_empty()).then_some(text)
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let translation = lines
        .map(|lines| {
            lines
                .iter()
                .filter_map(|line| line["translation"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    crate::protocol::Lyrics {
        lines: lines
            .into_iter()
            .flatten()
            .take(2048)
            .filter_map(|line| {
                let start = line["start"].as_u64()?;
                let words: Vec<_> = line["words"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(1024)
                    .filter_map(|word| {
                        let start = word["start"].as_u64()?;
                        let end = word["end"].as_u64()?;
                        (end >= start).then(|| crate::protocol::LyricWord {
                            start,
                            end,
                            text: word["text"]
                                .as_str()
                                .unwrap_or_default()
                                .chars()
                                .take(512)
                                .collect(),
                        })
                    })
                    .collect();
                Some(crate::protocol::LyricLine {
                    start,
                    end: line["end"].as_u64().filter(|end| *end >= start),
                    text: line["text"]
                        .as_str()
                        .map(|s| s.chars().take(4000).collect())
                        .unwrap_or_else(|| words.iter().map(|w| w.text.as_str()).collect()),
                    translation: line["translation"]
                        .as_str()
                        .unwrap_or_default()
                        .chars()
                        .take(4000)
                        .collect(),
                    words,
                })
            })
            .collect(),
        text: if original.is_empty() {
            result["lrc"].as_str().unwrap_or_default()
        } else {
            &original
        }
        .chars()
        .take(32000)
        .collect(),
        translation: if translation.is_empty() {
            result["translation"].as_str().unwrap_or_default()
        } else {
            &translation
        }
        .chars()
        .take(32000)
        .collect(),
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    #[test]
    fn aes_ecb_matches_openssl_pkcs7_vector_and_rejects_invalid_keys() {
        assert_eq!(
            aes_ecb("0123456789abcdef", "hello").unwrap(),
            "674c7ef38e78cabd9cec9c125823a639"
        );
        assert!(aes_ecb("bad", "hello").is_err());
        assert_eq!(aes_ecb("0123456789abcdef", "").unwrap().len(), 32);
    }
    #[test]
    fn fetch_keeps_cookie_fields_separate_from_expiry_commas() {
        let rt = Runtime::new().unwrap();
        let context = JsContext::full(&rt).unwrap();
        context.with(|ctx| {
            ctx.eval::<(),_>(r#"globalThis.__translations='{}';globalThis.__language='zh-CN';globalThis.__storage='{}';globalThis.__settings='{}';globalThis.__platform='darwin';globalThis.__arch='arm64';globalThis.__fetch=()=>JSON.stringify({status:200,body:'{}',headers:[['set-cookie','a=1; Expires=Thu, 01 Jan 2037 00:00:00 GMT'],['set-cookie','b=2']]});"#).unwrap();
            ctx.eval::<(),_>(include_str!("mygo_host.js")).unwrap();
            ctx.eval::<(),_>("globalThis.checked='';fetch('https://example.com').then(r=>checked=JSON.stringify(r.headers.getSetCookie()));").unwrap();
            while ctx.execute_pending_job() {}
            let result:String=ctx.eval("checked").unwrap();
            assert_eq!(serde_json::from_str::<Vec<String>>(&result).unwrap(),["a=1; Expires=Thu, 01 Jan 2037 00:00:00 GMT","b=2"]);
        });
    }
    #[test]
    fn timed_words_and_line_translation_take_precedence() {
        let lyrics = parse_lyrics(
            &json!({"lrc":"old", "translation":"old translation", "lines":[
                {"words":[{"text":"Hello"},{"text":" world"}],"translation":"你好"},
                {"text":"Second line"}
            ]}),
        );
        assert_eq!(lyrics.text, "Hello world\nSecond line");
        assert_eq!(lyrics.translation, "你好");
    }
    #[test]
    fn empty_timed_lines_fall_back_and_unicode_is_bounded() {
        let lyrics = parse_lyrics(&json!({"lrc":"海".repeat(40000),"lines":[{}]}));
        assert_eq!(lyrics.text.chars().count(), 32000);
        assert!(lyrics.translation.is_empty());
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default = "default_main")]
    pub main: String,
    #[serde(default = "default_playback")]
    pub playback: String,
    #[serde(default)]
    pub settings: Vec<crate::protocol::SourceSettingDefinition>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(flatten)]
    pub metadata: std::collections::BTreeMap<String, Value>,
    #[serde(default)]
    pub permissions: Permissions,
}

fn default_main() -> String {
    "index.js".into()
}
fn default_playback() -> String {
    "stream".into()
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Permissions {
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub process: Value,
    #[serde(flatten)]
    pub metadata: std::collections::BTreeMap<String, Value>,
}

pub use crate::protocol::Source;

#[derive(Deserialize)]
struct Fetch {
    url: String,
    method: Option<String>,
    #[serde(default)]
    headers: HashMap<String, String>,
    body: Option<String>,
}

pub(crate) fn plugin_root() -> Result<PathBuf> {
    let path = state_dir()?.join("mygo-plugins");
    fs::create_dir_all(&path)?;
    Ok(path)
}

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

pub(crate) fn manifest(dir: &Path) -> Result<Manifest> {
    let mut value: Manifest = serde_json::from_str(&fs::read_to_string(dir.join("plugin.json"))?)?;
    ensure!(valid_id(&value.id), tr!("Invalid MyGo plugin ID"));
    if value.name.is_empty() {
        value.name = value.id.clone();
    }
    Ok(value)
}

pub(crate) fn entry(dir: &Path, manifest: &Manifest) -> Result<PathBuf> {
    let path = dir.join(&manifest.main).canonicalize()?;
    ensure!(
        path.starts_with(dir.canonicalize()?)
            && path
                .extension()
                .is_some_and(|ext| ext == "js" || ext == "ts"),
        tr!("Only JavaScript or TypeScript entries inside the plugin directory are supported")
    );
    ensure!(
        fs::metadata(&path)?.len() <= MAX_BODY,
        tr!("Plugin entry exceeds 2 MiB")
    );
    Ok(path)
}

pub fn install(source: &Path) -> Result<String> {
    install_mode(source, false)
}
pub fn update_local(source: &Path) -> Result<String> {
    install_mode(source, true)
}
fn install_mode(source: &Path, replace: bool) -> Result<String> {
    let manifest = manifest(source)?;
    ensure!(
        crate::music_sources::limitation(&manifest, false, false).is_none(),
        tr!("This source is not supported by the current host")
    );
    let script = entry(source, &manifest)?;
    let root = plugin_root()?;
    let target = root.join(&manifest.id);
    ensure!(
        !replace || target.is_dir(),
        tr!("Install this source before updating")
    );
    let _settings = crate::music_sources::SETTINGS_WRITER
        .lock()
        .map_err(|_| anyhow!(tr!("Settings cannot be saved right now")))?;
    let _guard = PLUGIN_CALL
        .lock()
        .map_err(|_| anyhow!(tr!("Plugin runtime lock unavailable")))?;
    ensure!(
        !crate::music_sources::auth_active(&manifest.id),
        tr!("Complete or cancel account connection for this source first")
    );
    ensure!(
        replace || !target.exists(),
        tr!("Plugin {} already exists", manifest.id)
    );
    let pending = root.join(format!(
        ".install-{}",
        musicx_core::storage::random_hex::<8>()
    ));
    fs::create_dir(&pending)?;
    let result = (|| -> Result<()> {
        let mut installed = manifest;
        installed.main = "index.js".into();
        let code =
            crate::script::javascript(&script.to_string_lossy(), &fs::read_to_string(&script)?)?;
        crate::music_sources::check_single_file_script(&code)?;
        fs::write(pending.join("index.js"), code)?;
        fs::write(pending.join("plugin.json"), serde_json::to_vec(&installed)?)?;
        for language in ["en", "zh-CN"] {
            let path = source.join("locales").join(format!("{language}.json"));
            if path.exists() {
                ensure!(
                    path.canonicalize()?.starts_with(source.canonicalize()?),
                    tr!("Invalid plugin language file path")
                );
                ensure!(
                    fs::metadata(&path)?.len() <= 65536,
                    tr!("Plugin language file is too large")
                );
                let bytes = fs::read(path)?;
                serde_json::from_slice::<std::collections::BTreeMap<String, String>>(&bytes)?;
                fs::create_dir_all(pending.join("locales"))?;
                fs::write(
                    pending.join("locales").join(format!("{language}.json")),
                    bytes,
                )?;
            }
        }
        if replace && target.exists() {
            ensure!(
                !fs::symlink_metadata(&target)?.file_type().is_symlink(),
                tr!("Invalid plugin directory type")
            );
            if target.join("settings.json").is_file() {
                fs::copy(target.join("settings.json"), pending.join("settings.json"))?;
            }
            let backup = root.join(format!(
                ".backup-{}",
                musicx_core::storage::random_hex::<8>()
            ));
            fs::rename(&target, &backup)?;
            if let Err(error) = fs::rename(&pending, &target) {
                let _ = fs::rename(&backup, &target);
                return Err(error.into());
            }
            let _ = fs::remove_dir_all(backup);
        } else {
            fs::rename(&pending, &target)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(pending);
    }
    result?;
    Ok(target.file_name().unwrap().to_string_lossy().into_owned())
}

pub(crate) fn uninstall(id: &str) -> Result<()> {
    ensure!(valid_id(id), tr!("Invalid plugin ID"));
    let _settings = crate::music_sources::SETTINGS_WRITER
        .lock()
        .map_err(|_| anyhow!(tr!("Settings cannot be saved right now")))?;
    let _guard = PLUGIN_CALL
        .lock()
        .map_err(|_| anyhow!(tr!("Plugin runtime lock unavailable")))?;
    ensure!(
        !crate::music_sources::auth_active(id),
        tr!("Complete or cancel account connection for this source first")
    );
    let target = plugin_root()?.join(id);
    ensure!(
        target.is_dir() && !fs::symlink_metadata(&target)?.file_type().is_symlink(),
        tr!("Source not installed or invalid directory type")
    );
    fs::remove_dir_all(target)?;
    let account = state_dir()?.join(format!("mygo-{id}-storage.json"));
    match fs::remove_file(account) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

pub(crate) fn minimum_version_limitation(m: &Manifest) -> Option<String> {
    let version = m.metadata.get("minAppVersion")?;
    let Some(version) = version.as_str() else {
        return Some(tr!("Invalid plugin minimum version declaration").into());
    };
    if version.is_empty() {
        return None;
    }
    let parsed: Option<Vec<u64>> = version.split('.').map(|v| v.parse().ok()).collect();
    let Some(parsed) = parsed.filter(|v| v.len() == 3) else {
        return Some(tr!("Invalid plugin minimum version declaration").into());
    };
    (parsed.as_slice() > [0, 1, 22].as_slice())
        .then(|| tr!("This plugin requires MyGo {version}; the current compatibility baseline is 0.1.22"; version = version))
}

pub fn list() -> Result<Value> {
    let mut found = Vec::new();
    for item in fs::read_dir(plugin_root()?)? {
        let item = item?;
        if item.file_type()?.is_dir() && valid_id(&item.file_name().to_string_lossy()) {
            let manifest = manifest(&item.path())?;
            found.push(json!({
                "id": manifest.id, "name": manifest.name, "version": manifest.version,
                "repository": manifest.repository,
                "network": manifest.permissions.network, "catalog_supported": compatible(&manifest)
            }));
        }
    }
    found.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    Ok(json!(found))
}

pub(crate) fn compatible(manifest: &Manifest) -> bool {
    manifest.playback == "stream"
        && match &manifest.permissions.process {
            Value::Null | Value::Bool(false) => true,
            Value::Array(commands) => commands.is_empty() || crate::ffmpeg::supported(commands),
            _ => false,
        }
}

pub(crate) fn permitted(url: &Url, hosts: &[String]) -> bool {
    url.scheme() == "https" && permitted_audio(url, hosts)
}

pub(crate) fn valid_audio_url(url: &Url) -> bool {
    matches!(
        (url.scheme(), url.port_or_known_default()),
        ("https", Some(443)) | ("http", Some(80))
    ) && url.username().is_empty()
        && url.password().is_none()
        && url.domain().is_some_and(|host| {
            host.contains('.') && !host.ends_with(".local") && !host.ends_with(".localhost")
        })
}

pub(crate) fn permitted_audio(url: &Url, hosts: &[String]) -> bool {
    valid_audio_url(url)
        && url.domain().is_some_and(|host| {
            hosts.iter().any(|grant| {
                if let Some(suffix) = grant.strip_prefix("*.") {
                    host.strip_suffix(suffix)
                        .is_some_and(|prefix| prefix.ends_with('.'))
                } else {
                    host == grant
                }
            })
        })
}

fn fetch(client: &Client, input: &str, grants: &[String], deadline: Instant) -> Result<Value> {
    let input: Fetch = serde_json::from_str(input)?;
    let url = Url::parse(&input.url)?;
    ensure!(
        permitted(&url, grants),
        tr!("Plugin network permissions do not cover this address")
    );
    let remaining = deadline.saturating_duration_since(Instant::now());
    ensure!(!remaining.is_zero(), tr!("Plugin request timed out"));
    let method = Method::from_bytes(input.method.as_deref().unwrap_or("GET").as_bytes())?;
    ensure!(
        matches!(method, Method::GET | Method::POST | Method::HEAD),
        tr!("Unsupported HTTP method")
    );
    let mut request = client
        .request(method, url)
        .timeout(remaining.min(Duration::from_secs(10)));
    for (name, value) in input.headers {
        ensure!(
            !["host", "connection", "proxy-authorization"]
                .iter()
                .any(|header| name.eq_ignore_ascii_case(header)),
            tr!("Plugin request contains a prohibited HTTP header")
        );
        request = request.header(name, value);
    }
    if let Some(body) = input.body {
        request = request.body(body);
    }
    let response = request
        .send()
        .map_err(|_| anyhow!(tr!("Plugin network request failed")))?;
    let status = response.status().as_u16();
    // Preserve repeated Set-Cookie fields: joining them corrupts Expires dates.
    // These values stay inside the short-lived plugin VM, never in UI results.
    let headers: Vec<_> = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect();
    let mut bytes = Vec::new();
    response.take(MAX_BODY + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BODY,
        tr!("Plugin response exceeds 2 MiB")
    );
    Ok(json!({"status":status,"headers":headers,"body":String::from_utf8_lossy(&bytes)}))
}

fn optional_json(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok("{}".into()),
        Err(error) => Err(error.into()),
    }
}

pub fn invoke(id: &str, export: &str, args: Value) -> Result<Value> {
    invoke_with_playback(id, export, args, None)
}

fn invoke_with_playback(
    id: &str,
    export: &str,
    args: Value,
    playback: Option<Arc<Mutex<crate::ffmpeg::Playback>>>,
) -> Result<Value> {
    let _guard = PLUGIN_CALL
        .lock()
        .map_err(|_| anyhow!(tr!("Plugin runtime lock unavailable")))?;
    ensure!(
        !crate::music_sources::auth_active(id),
        tr!("This source is connecting an account; complete or cancel first")
    );
    invoke_inner(id, export, args, None, playback, Duration::from_secs(18))
}

pub(crate) fn invoke_sign_in(
    id: &str,
    job: std::sync::Arc<crate::music_sources::LoginJob>,
    method: crate::protocol::SignInMethod,
) -> Result<Value> {
    // Drain an in-flight ordinary call before using this source's storage.
    // Its live QR runtime must not hold the global normal-call lock for minutes.
    drop(
        PLUGIN_CALL
            .lock()
            .map_err(|_| anyhow!(tr!("Plugin runtime lock unavailable")))?,
    );
    invoke_inner(
        id,
        "signIn",
        json!({"method":method}),
        Some(job),
        None,
        Duration::from_secs(300),
    )
}

fn invoke_inner(
    id: &str,
    export: &str,
    args: Value,
    login: Option<std::sync::Arc<crate::music_sources::LoginJob>>,
    playback: Option<Arc<Mutex<crate::ffmpeg::Playback>>>,
    timeout: Duration,
) -> Result<Value> {
    ensure!(valid_id(id), tr!("Invalid plugin ID"));
    let dir = plugin_root()?.join(id);
    let manifest = manifest(&dir).context(tr!("This MyGo plugin is not installed"))?;
    if let Some(reason) = minimum_version_limitation(&manifest) {
        anyhow::bail!("{reason}");
    }
    ensure!(
        id == manifest.id && compatible(&manifest),
        tr!("This plugin requires an external process or remote playback")
    );
    let script = fs::read(entry(&dir, &manifest)?)?;
    let storage_name = format!("mygo-{id}-storage.json");
    let storage = optional_json(&state_dir()?.join(&storage_name))?;
    let settings = optional_json(&dir.join("settings.json"))?;
    let mut settings: Value = serde_json::from_str(&settings)?;
    for field in &manifest.settings {
        if let Some(default) = &field.value {
            settings
                .as_object_mut()
                .context(tr!("Invalid plugin settings format"))?
                .entry(&field.key)
                .or_insert(json!(default));
        }
    }
    if login.is_some()
        && settings.is_object()
        && matches!(args["method"].as_str(), Some("web" | "qr"))
    {
        settings["loginMethod"] = args["method"].clone();
    }
    let settings = settings.to_string();
    let grants = manifest.permissions.network;
    let process = manifest.permissions.process;
    let redirect_grants = grants.clone();
    let client = Client::builder()
        .redirect(Policy::custom(move |attempt| {
            if attempt.previous().len() < 5
                && permitted(attempt.url(), &redirect_grants)
                && attempt.previous().last().and_then(Url::host_str) == attempt.url().host_str()
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()?;
    let deadline = Instant::now() + timeout;
    let rt = Runtime::new()?;
    rt.set_memory_limit(24 * 1024 * 1024);
    rt.set_max_stack_size(512 * 1024);
    let interrupted = login.clone();
    rt.set_interrupt_handler(Some(Box::new(move || {
        Instant::now() >= deadline || interrupted.as_ref().is_some_and(|job| job.cancelled())
    })));
    let context = JsContext::full(&rt)?;
    context.with(|ctx| {
        let global = ctx.globals();
        let _oauth = crate::oauth::Bindings::install(&ctx, login.clone(), grants.clone(), deadline)?;
        let web_job=login.clone();let web_grants=grants.clone();let web_name=manifest.name.clone();
        global.set("__webSignIn",Function::new(ctx.clone(),move|text:String|{
            let result=(||->Result<Value>{
                let job=web_job.clone().context(tr!("Start source sign-in first"))?;
                crate::web_auth::sign_in(serde_json::from_str(&text)?,&web_grants,&web_name,job,deadline)
            })();
            result.unwrap_or_else(|_|json!({"error":tr!("Web sign-in incomplete; try again or use QR sign-in")})).to_string()
        })?)?;
        global.set("__aesEcb", Function::new(ctx.clone(), |key: String, text: String| {
            match aes_ecb(&key, &text) {
                Ok(hex) => json!({"hex":hex}),
                Err(error) => json!({"error":error.to_string()}),
            }.to_string()
        })?)?;
        global.set("__url", Function::new(ctx.clone(), |value: String| {
            match Url::parse(&value) {
                Ok(url) => json!({"href":url.as_str(),"protocol":format!("{}:",url.scheme()),"hostname":url.host_str().unwrap_or_default(),"host":&url[url::Position::BeforeHost..url::Position::AfterPort],"port":url.port().map(|port| port.to_string()).unwrap_or_default()}),
                Err(_) => json!({"error":"Invalid URL"}),
            }.to_string()
        })?)?;
        if let Some(playback) = playback.filter(|_| process.as_array().is_some_and(|commands| !commands.is_empty() && crate::ffmpeg::supported(commands))) {
            let decode_grants = grants.clone();
            global.set("__spawn", Function::new(ctx.clone(), move |text: String| {
                let result = (|| {
                playback.lock().map_err(|_| anyhow!(tr!("Could not start FFmpeg")))?
                    .spawn(&text, &process, &decode_grants, deadline)
                })();
                result.unwrap_or_else(|error: anyhow::Error| json!({"error":error.to_string()})).to_string()
            })?)?;
        }
        global.set("__storage", storage)?;
        global.set("__settings", settings)?;
        global.set("__language", crate::localization::language())?;
        global.set("__translations", serde_json::to_string(&crate::localization::translations(id))?)?;
        let prompt = login.clone();
        global.set(
            "__signInPrompt",
            Function::new(ctx.clone(), move |value: String| {
                prompt
                    .as_ref()
                    .is_some_and(|job| job.prompt(&value).is_ok())
            })?,
        )?;
        global.set("__arch", std::env::consts::ARCH.replace("aarch64", "arm64").replace("x86_64", "amd64"))?;
        global.set(
            "__platform",
            if cfg!(target_os = "ios") {
                "ios"
            } else {
                "darwin"
            },
        )?;
        let storage_job = login.clone();
        global.set(
            "__save",
            Function::new(ctx.clone(), move |text: String| {
                !storage_job.as_ref().is_some_and(|job| job.cancelled())
                    && text.len() <= 64 * 1024
                    && musicx_core::storage::save_private(&storage_name, &text).is_ok()
            })?,
        )?;
        global.set(
            "__md5",
            Function::new(ctx.clone(), |text: String| {
                format!("{:x}", Md5::digest(text))
            })?,
        )?;
        global.set(
            "__query",
            Function::new(ctx.clone(), |text: String| {
                let mut pairs = url::form_urlencoded::Serializer::new(String::new());
                for [key, value] in
                    serde_json::from_str::<Vec<[String; 2]>>(&text).unwrap_or_default()
                {
                    pairs.append_pair(&key, &value);
                }
                pairs.finish()
            })?,
        )?;
        let sleeper = login.clone();
        global.set(
            "__sleep",
            Function::new(ctx.clone(), move |ms: f64| {
                let duration = Duration::from_millis(ms.max(0.0) as u64);
                if duration > deadline.saturating_duration_since(Instant::now()) {
                    return false;
                }
                if let Some(job) = &sleeper {
                    let until = Instant::now() + duration;
                    while Instant::now() < until {
                        if job.cancelled() {
                            return false;
                        }
                        std::thread::sleep(
                            until
                                .saturating_duration_since(Instant::now())
                                .min(Duration::from_millis(50)),
                        );
                    }
                } else {
                    std::thread::sleep(duration);
                }
                true
            })?,
        )?;
        global.set(
            "__fetch",
            Function::new(ctx.clone(), move |text: String| {
                fetch(&client, &text, &grants, deadline)
                    .unwrap_or_else(|error| json!({"error":error.to_string()}))
                    .to_string()
            })?,
        )?;
        let result = (|| -> rquickjs::Result<_> {
            ctx.eval::<(), _>(include_str!("mygo_host.js"))?;
            let (module, loading) = Module::declare(ctx.clone(), "plugin.js", script)?.eval()?;
            loading.finish::<()>()?;
            let function: Function = module.namespace()?.get(export)?;
            let args = ctx.json_parse(args.to_string())?;
            let wrapper: Function = global.get("__invoke")?;
            let pending: MaybePromise = wrapper.call((function, args))?;
            let result: JsValue = loop {
                if let Some(value) = pending.result() {
                    break value?;
                }
                if !ctx.execute_pending_job() && !ctx.eval::<bool, _>("__nextTimer()")? {
                    return pending.finish::<JsValue>().and_then(|result| {
                        ctx.json_stringify(result)?
                            .map(|text| text.to_string())
                            .transpose()
                    });
                }
            };
            ctx.json_stringify(result)?
                .map(|text| text.to_string())
                .transpose()
        })()
        .catch(&ctx)
        .map_err(|error| anyhow!(tr!("Plugin {id} failed in {export}: {error}"; id = id, export = export, error = error)))?;
        let result: Value = serde_json::from_str(
            &result.unwrap_or_else(|| "null".into()),
        )?;
        if result["ok"].as_bool() == Some(true) { Ok(result["value"].clone()) }
        else {
            let code = result["code"].as_str().unwrap_or_default();
            let prompt = match code { "signIn" => tr!("Connect an account in Music Sources first"), "setup" => tr!("Check the source settings"), "unavailable" => tr!("This source cannot play this item right now"), "unsupported" => tr!("This source does not support this capability"), _ => tr!("Source request failed") };
            Err(anyhow!(PluginFailure { code:code.into(), message: format!("{prompt}：{}", result["message"].as_str().unwrap_or_default().chars().take(512).collect::<String>()) }))
        }
    })
}

#[derive(Debug)]
struct PluginFailure {
    code: String,
    message: String,
}
impl std::fmt::Display for PluginFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for PluginFailure {}
pub fn error_code(error: &anyhow::Error) -> Option<&str> {
    error
        .downcast_ref::<PluginFailure>()
        .map(|failure| failure.code.as_str())
        .filter(|code| !code.is_empty())
}

pub fn resolve_with_quality(mut track: Track, quality: crate::protocol::Quality) -> Result<Media> {
    let source = track
        .source
        .as_ref()
        .context(tr!("Plugin track identifier missing"))?;
    let playback = Arc::new(Mutex::new(crate::ffmpeg::Playback::default()));
    let stream = invoke_with_playback(
        &source.plugin,
        "stream",
        json!({"id":source.id,"quality":quality.plugin_quality()}),
        Some(playback.clone()),
    )?;
    let decoded = playback
        .lock()
        .map_err(|_| anyhow!(tr!("Could not start FFmpeg")))?
        .take(&stream);
    let mut url = Url::parse(
        stream["url"]
            .as_str()
            .context(tr!("The plugin returned no audio URL"))?,
    )?;
    ensure!(
        decoded.is_some() || valid_audio_url(&url),
        tr!("The plugin returned an unsupported audio URL")
    );
    let uses_http = decoded.is_none() && url.scheme() == "http";
    let mut request_headers = if decoded.is_some() {
        Default::default()
    } else {
        crate::assets::headers(&source.plugin, &url, "audio")?
    };
    if decoded.is_none()
        && let Some(headers) = stream["headers"].as_object()
    {
        for (key, value) in headers {
            ensure!(
                !matches!(
                    key.to_ascii_lowercase().as_str(),
                    "host"
                        | "connection"
                        | "content-length"
                        | "transfer-encoding"
                        | "proxy-authorization"
                ),
                tr!("Unsupported audio request headers")
            );
            let value = value
                .as_str()
                .context(tr!("Audio request header values must be strings"))?;
            reqwest::header::HeaderName::from_bytes(key.as_bytes())
                .context(tr!("Invalid audio request header name"))?;
            reqwest::header::HeaderValue::from_str(value)
                .context(tr!("Invalid audio request header value"))?;
            request_headers.insert(key.to_ascii_lowercase(), value.into());
        }
    }
    let mut user_agent = request_headers.get("user-agent").cloned();
    let mut lease = None;
    if let Some(decoded) = decoded {
        url = crate::ffmpeg::file_url(&decoded)?;
        lease = Some(decoded as std::sync::Arc<dyn Send + Sync>);
    } else if uses_http || request_headers.keys().any(|k| k != "user-agent") {
        let content_type = (stream["type"] == "m4a").then_some("audio/mp4");
        let registration = crate::stream_proxy::register_audio(url, request_headers, content_type)?;
        url = registration.0;
        lease = Some(registration.1 as std::sync::Arc<dyn Send + Sync>);
        user_agent = None;
    }
    fill_artwork(&mut track);
    Ok(Media {
        lease,
        track,
        url,
        uses_http,
        user_agent,
        quality: crate::quality::metadata(quality.for_plugin(), &stream, true),
        gain_db: stream["gain"]
            .as_f64()
            .filter(|n| n.is_finite() && (-60.0..=60.0).contains(n))
            .map(|n| n as f32),
        peak: stream["peak"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.0 && *n <= 32.0)
            .map(|n| n as f32),
    })
}
