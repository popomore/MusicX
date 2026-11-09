use anyhow::{Context, Result, anyhow, ensure};
use md5::{Digest, Md5};
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
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::netease::{Media, Track, state_dir};

pub const NETEASE_REPOSITORY: &str = "sogud/mygo-music-netease";
const NETEASE_SCRIPT_SHA: &str = "e887ec38987cbbcc66735314019d54a5a0b8cda6544234d232df16f259d8f8b0";
pub(crate) fn is_pinned_netease(script: &[u8]) -> bool {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(script)) == NETEASE_SCRIPT_SHA
}

/// Install the catalog's pinned plugin once, without importing legacy credentials.
pub fn ensure_netease() -> Result<()> {
    static INSTALL: Mutex<()> = Mutex::new(());
    let _guard = INSTALL.lock().map_err(|_| anyhow!("插件安装锁不可用"))?;
    let dir = plugin_root()?.join("netease");
    if !dir.exists() {
        crate::music_sources::install(NETEASE_REPOSITORY)?;
    }
    let m = manifest(&dir)?;
    ensure!(m.id == "netease" && compatible(&m), "网易云插件类型不兼容");
    Ok(())
}

fn aes_ecb(key: &str, text: &str) -> Result<String> {
    use aes::{
        Aes128,
        cipher::{Block, BlockEncrypt, KeyInit},
    };
    let cipher =
        Aes128::new_from_slice(key.as_bytes()).map_err(|_| anyhow!("AES密钥必须为16字节"))?;
    ensure!(text.len() <= MAX_BODY as usize, "加密输入过大");
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
        duration: value["duration"].as_f64().unwrap_or(0.0) / 1000.0,
        cover_url: [&album["imageUrl"], &value["imageUrl"], &value["coverUrl"]]
            .into_iter()
            .filter_map(Value::as_str)
            .find_map(crate::netease::artwork_url),
        album: album["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(|id| crate::protocol::Album {
                id: id.chars().take(512).collect(),
                name: album["name"]
                    .as_str()
                    .unwrap_or("专辑")
                    .chars()
                    .take(512)
                    .collect(),
            }),
        source_url: value["url"].as_str().and_then(crate::netease::public_url),
        source: Some(Source {
            plugin: plugin.to_owned(),
            id,
        }),
    })
}

/// Only enrich the same source identity. Older queue and library entries may
/// have title and artist but lack all three catalog fields.
pub(crate) fn merge_metadata(track: &mut Track, metadata: &Track) -> bool {
    if track.source != metadata.source || (track.source.is_none() && track.id != metadata.id) {
        return false;
    }
    let mut changed = false;
    if track.cover_url.is_none() && metadata.cover_url.is_some() {
        track.cover_url.clone_from(&metadata.cover_url);
        changed = true;
    }
    if track.album.is_none() && metadata.album.is_some() {
        track.album.clone_from(&metadata.album);
        changed = true;
    }
    if track.source_url.is_none() && metadata.source_url.is_some() {
        track.source_url.clone_from(&metadata.source_url);
        changed = true;
    }
    changed
}

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
    plugin_root()
        .and_then(|root| manifest(&root.join(plugin)))
        .map_or_else(|_| plugin.to_owned(), |manifest| manifest.name)
}

pub fn album(track: &Track) -> Result<crate::protocol::AlbumPage> {
    let source = track.source.as_ref().context("缺少来源")?;
    let mut album = track.album.clone().context("当前来源没有专辑信息")?;
    let result = invoke(&source.plugin, "album", json!({"id":album.id}))?;
    if let Some(name) = result["name"].as_str().filter(|v| !v.is_empty()) {
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
    let source = track.source.as_ref().context("缺少来源")?;
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
            ctx.eval::<(),_>(r#"globalThis.__storage='{}';globalThis.__settings='{}';globalThis.__platform='darwin';globalThis.__arch='arm64';globalThis.__fetch=()=>JSON.stringify({status:200,body:'{}',headers:[['set-cookie','a=1; Expires=Thu, 01 Jan 2037 00:00:00 GMT'],['set-cookie','b=2']]});"#).unwrap();
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
    pub name: String,
    pub version: String,
    pub main: String,
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
    let value: Manifest = serde_json::from_str(&fs::read_to_string(dir.join("plugin.json"))?)?;
    ensure!(valid_id(&value.id), "无效的 MyGo 插件 ID");
    Ok(value)
}

pub(crate) fn entry(dir: &Path, manifest: &Manifest) -> Result<PathBuf> {
    let path = dir.join(&manifest.main).canonicalize()?;
    ensure!(
        path.starts_with(dir.canonicalize()?) && path.extension().is_some_and(|ext| ext == "js"),
        "目前只支持插件目录内的 JavaScript 入口"
    );
    ensure!(fs::metadata(&path)?.len() <= MAX_BODY, "插件入口超过 2 MiB");
    Ok(path)
}

pub fn install(source: &Path) -> Result<String> {
    let manifest = manifest(source)?;
    let script = entry(source, &manifest)?;
    let root = plugin_root()?;
    let target = root.join(&manifest.id);
    ensure!(!target.exists(), "插件 {} 已存在", manifest.id);
    let pending = root.join(format!(".install-{}", crate::netease::random_hex::<8>()));
    fs::create_dir(&pending)?;
    let result = (|| -> Result<()> {
        let mut installed = manifest;
        installed.main = "index.js".into();
        fs::copy(script, pending.join("index.js"))?;
        fs::write(pending.join("plugin.json"), serde_json::to_vec(&installed)?)?;
        fs::rename(&pending, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(pending);
    }
    result?;
    Ok(target.file_name().unwrap().to_string_lossy().into_owned())
}

pub fn list() -> Result<Value> {
    let mut found = Vec::new();
    for item in fs::read_dir(plugin_root()?)? {
        let item = item?;
        if item.file_type()?.is_dir() && valid_id(&item.file_name().to_string_lossy()) {
            let manifest = manifest(&item.path())?;
            found.push(json!({
                "id": manifest.id, "name": manifest.name, "version": manifest.version,
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
            Value::Array(commands) => commands.is_empty(),
            _ => false,
        }
}

pub(crate) fn permitted(url: &Url, hosts: &[String]) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.domain().is_some_and(|host| {
            host.contains('.')
                && !host.ends_with(".local")
                && !host.ends_with(".localhost")
                && hosts.iter().any(|grant| {
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
    ensure!(permitted(&url, grants), "插件网络权限未覆盖目标");
    let remaining = deadline.saturating_duration_since(Instant::now());
    ensure!(!remaining.is_zero(), "插件请求超时");
    let method = Method::from_bytes(input.method.as_deref().unwrap_or("GET").as_bytes())?;
    ensure!(
        matches!(method, Method::GET | Method::POST | Method::HEAD),
        "不支持此 HTTP 方法"
    );
    let mut request = client
        .request(method, url)
        .timeout(remaining.min(Duration::from_secs(10)));
    for (name, value) in input.headers {
        ensure!(
            !["host", "connection", "proxy-authorization"]
                .iter()
                .any(|header| name.eq_ignore_ascii_case(header)),
            "插件请求含有禁止的 HTTP 头"
        );
        request = request.header(name, value);
    }
    if let Some(body) = input.body {
        request = request.body(body);
    }
    let response = request.send().map_err(|_| anyhow!("插件网络请求失败"))?;
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
    ensure!(bytes.len() as u64 <= MAX_BODY, "插件响应超过 2 MiB");
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
    let _guard = PLUGIN_CALL
        .lock()
        .map_err(|_| anyhow!("插件运行锁不可用"))?;
    ensure!(
        !crate::music_sources::auth_active(id),
        "此数据源正在关联账号，请先完成或取消"
    );
    invoke_inner(id, export, args, None, Duration::from_secs(18))
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
            .map_err(|_| anyhow!("插件运行锁不可用"))?,
    );
    let pinned_web = if id == "netease" && method == crate::protocol::SignInMethod::Web {
        let dir = plugin_root()?.join(id);
        let m = manifest(&dir)?;
        is_pinned_netease(&fs::read(entry(&dir, &m)?)?)
    } else {
        false
    };
    let export = if pinned_web {
        "__mini_web_signin"
    } else {
        "signIn"
    };
    invoke_inner(
        id,
        export,
        json!({"method":method}),
        Some(job),
        Duration::from_secs(300),
    )
}

fn invoke_inner(
    id: &str,
    export: &str,
    args: Value,
    login: Option<std::sync::Arc<crate::music_sources::LoginJob>>,
    timeout: Duration,
) -> Result<Value> {
    ensure!(valid_id(id), "无效的插件 ID");
    let dir = plugin_root()?.join(id);
    let manifest = manifest(&dir).context("未安装此 MyGo 插件")?;
    ensure!(
        id == manifest.id && compatible(&manifest),
        "此插件需要外部进程或远程播放"
    );
    let mut script = fs::read(entry(&dir, &manifest)?)?;
    if export == "__mini_web_signin" {
        ensure!(
            id == "netease" && is_pinned_netease(&script),
            "此网易云插件版本不支持网页登录适配"
        );
        script.extend_from_slice(r#"
export async function __mini_web_signin(){
  const result=await host.webSignIn({url:"https://music.163.com/#/login",cookiesOf:"https://music.163.com/",until:["MUSIC_U"]});
  const cookies={};for(const part of result.cookie.split(/;\s*/)){const at=part.indexOf("=");if(at>0){const key=part.slice(0,at);if(COOKIE_NAMES.includes(key))cookies[key]=part.slice(at+1);}}
  const pending={cookies,profile:null};pending.profile=await profileFor(pending);
  if(!pending.profile)throw new Error("网页登录未取得有效账号，请重新登录");
  session=pending;profileCheckedAt=Date.now();persistSession();return pending.profile;
}
"#.as_bytes());
    }
    if export == "__mini_track" {
        use sha2::{Digest, Sha256};
        ensure!(
            id == "netease" && format!("{:x}", Sha256::digest(&script)) == NETEASE_SCRIPT_SHA,
            "当前网易云插件不提供ID单曲兼容，请通过搜索结果选歌"
        );
        // The MyGo contract passes full Track objects to playback. Existing MCP
        // ID-only calls still need metadata; reuse this pinned plugin's own
        // converter and detail request, without another NetEase API client.
        script.extend_from_slice(b"\nexport async function __mini_track({id}) { return (await songDetails([id])).map(toTrack)[0] || null; }\n");
    }
    let storage_name = format!("mygo-{id}-storage.json");
    let storage = optional_json(&state_dir()?.join(&storage_name))?;
    let settings = optional_json(&dir.join("settings.json"))?;
    let mut settings: Value = serde_json::from_str(&settings)?;
    for field in &manifest.settings {
        if let Some(default) = &field.value {
            settings
                .as_object_mut()
                .context("插件设置格式错误")?
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
        let web_job=login.clone();let web_grants=grants.clone();let web_name=manifest.name.clone();
        global.set("__webSignIn",Function::new(ctx.clone(),move|text:String|{
            let result=(||->Result<Value>{
                let job=web_job.clone().context("请先发起来源登录")?;
                crate::web_auth::sign_in(serde_json::from_str(&text)?,&web_grants,&web_name,job,deadline)
            })();
            result.unwrap_or_else(|_|json!({"error":"网页登录未完成，请重试或使用扫码登录"})).to_string()
        })?)?;
        global.set("__aesEcb", Function::new(ctx.clone(), |key: String, text: String| {
            match aes_ecb(&key, &text) {
                Ok(hex) => json!({"hex":hex}),
                Err(error) => json!({"error":error.to_string()}),
            }.to_string()
        })?)?;
        global.set("__url", Function::new(ctx.clone(), |value: String| {
            match Url::parse(&value) {
                Ok(url) => json!({"href":url.as_str(),"protocol":format!("{}:",url.scheme()),"hostname":url.host_str().unwrap_or_default()}),
                Err(_) => json!({"error":"Invalid URL"}),
            }.to_string()
        })?)?;
        global.set("__storage", storage)?;
        global.set("__settings", settings)?;
        let prompt = login.clone();
        global.set(
            "__signInPrompt",
            Function::new(ctx.clone(), move |value: String| {
                prompt
                    .as_ref()
                    .is_some_and(|job| job.prompt(&value).is_ok())
            })?,
        )?;
        global.set("__arch", std::env::consts::ARCH.replace("aarch64", "arm64"))?;
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
                    && crate::netease::save_private(&storage_name, &text).is_ok()
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
            let pending: MaybePromise = function.call((args,))?;
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
        .map_err(|error| anyhow!("插件 {id} 的 {export} 失败：{error}"))?;
        Ok(serde_json::from_str(
            &result.unwrap_or_else(|| "null".into()),
        )?)
    })
}

pub fn resolve(track: Track) -> Result<Media> {
    resolve_with_quality(track, crate::audio_quality::preferred())
}
pub fn resolve_with_quality(mut track: Track, quality: crate::protocol::Quality) -> Result<Media> {
    let source = track.source.as_ref().context("缺少插件曲目标识")?;
    let stream = invoke(
        &source.plugin,
        "stream",
        json!({"id":source.id,"quality":quality.plugin_quality()}),
    )?;
    let mut url = Url::parse(stream["url"].as_str().context("插件未返回音频地址")?)?;
    if url.scheme() == "http" {
        url.set_scheme("https")
            .map_err(|_| anyhow!("无效的音频地址"))?;
    }
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443)
            && url.domain().is_some_and(|host| host.contains('.')
                && !host.ends_with(".local")
                && !host.ends_with(".localhost")),
        "插件返回了不支持的音频地址"
    );
    let mut user_agent = None;
    if let Some(headers) = stream["headers"].as_object() {
        for (key, value) in headers {
            // NetEase's public CDN works without this non-authentication hint.
            // AVPlayer exposes a public User-Agent option, but no Referer API.
            if source.plugin == "netease"
                && key.eq_ignore_ascii_case("Referer")
                && value.as_str() == Some("https://music.163.com/")
            {
                continue;
            }
            ensure!(
                key.eq_ignore_ascii_case("User-Agent"),
                "此音频需要当前播放器不支持的 HTTP 头"
            );
            user_agent = value.as_str().map(str::to_owned);
        }
    }
    fill_artwork(&mut track);
    Ok(Media {
        track,
        url,
        user_agent,
        quality: crate::audio_quality::metadata(quality.for_plugin(), &stream, true),
    })
}
