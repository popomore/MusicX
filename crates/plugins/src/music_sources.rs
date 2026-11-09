//! Sources adapt a bounded subset of the MyGo contract; local playlists stay ours.
use crate::{mygo, protocol::*, repository, source_catalog};
use anyhow::{Context, Result, ensure};
use musicx_core::{metadata, storage};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

static SETTINGS_WRITER: Mutex<()> = Mutex::new(());
struct LoginRegistry {
    active: Option<Arc<LoginJob>>,
    cancelled: std::collections::VecDeque<String>,
}
static LOGIN: Mutex<LoginRegistry> = Mutex::new(LoginRegistry {
    active: None,
    cancelled: std::collections::VecDeque::new(),
});

fn plugin(source: &str) -> Result<&str> {
    if source == "netease" {
        return Ok("netease");
    }
    let id = source.strip_prefix("mygo:").context("请选择插件数据源")?;
    ensure!(mygo::valid_id(id), "无效的数据源标识");
    Ok(id)
}

fn script_words(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut words = Vec::new();
    while at < bytes.len() {
        let c = bytes[at];
        if c == b'/' && bytes.get(at + 1) == Some(&b'/') {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        if c == b'/' && bytes.get(at + 1) == Some(&b'*') {
            at += 2;
            while at + 1 < bytes.len() && &bytes[at..at + 2] != b"*/" {
                at += 1;
            }
            at = (at + 2).min(bytes.len());
            continue;
        }
        if b"'\"`".contains(&c) {
            at += 1;
            while at < bytes.len() {
                if bytes[at] == b'\\' {
                    at = (at + 2).min(bytes.len());
                } else if bytes[at] == c {
                    at += 1;
                    break;
                } else {
                    at += 1;
                }
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' || c == b'$' {
            let start = at;
            at += 1;
            while at < bytes.len()
                && (bytes[at].is_ascii_alphanumeric() || b"_$".contains(&bytes[at]))
            {
                at += 1;
            }
            words.push(String::from_utf8_lossy(&bytes[start..at]).into_owned());
        } else {
            if b"{}*,;.(".contains(&c) {
                words.push((c as char).to_string());
            }
            at += 1;
        }
    }
    words
}

/// Lexical declarations, not plugin execution. Comments/strings cannot invent capabilities.
pub(crate) fn declared_exports(source: &str) -> Vec<String> {
    let words = script_words(source);
    let mut out = Vec::new();
    let mut depth = 0;
    let mut at = 0;
    while at < words.len() {
        if depth == 0 && words[at] == "export" {
            let mut i = at + 1;
            if words.get(i).is_some_and(|v| v == "async") {
                i += 1;
            }
            if words.get(i).is_some_and(|v| {
                matches!(v.as_str(), "function" | "const" | "let" | "class" | "var")
            }) {
                i += 1;
                if words.get(i).is_some_and(|v| v == "*") {
                    i += 1;
                }
                if let Some(name) = words.get(i) {
                    out.push(name.clone());
                }
            } else if words.get(i).is_some_and(|v| v == "{") {
                i += 1;
                while i < words.len() && words[i] != "}" {
                    let mut name = words[i].clone();
                    i += 1;
                    if words.get(i).is_some_and(|v| v == "as") {
                        i += 1;
                        if let Some(alias) = words.get(i) {
                            name = alias.clone();
                            i += 1;
                        }
                    }
                    if name != "," {
                        out.push(name);
                    }
                    if words.get(i).is_some_and(|v| v == ",") {
                        i += 1;
                    }
                }
            }
        }
        match words[at].as_str() {
            "{" => depth += 1,
            "}" => depth -= 1,
            _ => {}
        }
        at += 1;
    }
    out.sort();
    out.dedup();
    out
}

fn uses_module_imports(source: &str) -> bool {
    let words = script_words(source);
    words.iter().enumerate().any(|(at, word)| {
        if word == "import" {
            return at == 0 || words[at - 1] != ".";
        }
        if word == "export" {
            if words.get(at + 1).is_some_and(|v| v == "*") {
                return true;
            }
            if words.get(at + 1).is_some_and(|v| v == "{") {
                return words[at + 2..]
                    .iter()
                    .position(|v| v == "}")
                    .and_then(|end| words.get(at + 3 + end))
                    .is_some_and(|v| v == "from");
            }
        }
        false
    })
}

fn check_script_syntax(source: &str) -> Result<()> {
    use rquickjs::CatchResultExt;
    let runtime = rquickjs::Runtime::new()?;
    runtime.set_memory_limit(32 * 1024 * 1024);
    runtime.set_max_stack_size(256 * 1024);
    let context = rquickjs::Context::full(&runtime)?;
    context.with(|ctx| {
        rquickjs::Module::declare(ctx.clone(), "plugin-install", source)
            .catch(&ctx)
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("插件入口无法编译：{error}"))
    })
}
fn legacy_fields(source: &str) -> Vec<SourceSettingDefinition> {
    let mut fields = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find("host.setting(") {
        rest = &rest[at + 13..];
        let value = rest.trim_start();
        let Some(quote) = value.chars().next() else {
            break;
        };
        if !['\'', '"'].contains(&quote) {
            continue;
        }
        if let Some(end) = value[1..].find(quote) {
            let key = &value[1..end + 1];
            if key.len() <= 100
                && !fields
                    .iter()
                    .any(|f: &SourceSettingDefinition| f.key == key)
            {
                fields.push(SourceSettingDefinition {
                    key: key.into(),
                    label: match key {
                        "quality" => "音质偏好",
                        "autoVip" => "每日免费 VIP",
                        _ => key,
                    }
                    .into(),
                    ..Default::default()
                });
            }
        }
    }
    fields
}
fn secret(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    [
        "password",
        "token",
        "secret",
        "cookie",
        "apikey",
        "credential",
        "smscode",
        "otp",
        "accesscode",
        "wrapperkey",
        "spdc",
        "sp_dc",
        "authorization",
    ]
    .iter()
    .any(|part| k.contains(part))
}
fn settings_value(id: &str) -> Result<BTreeMap<String, String>> {
    let path = mygo::plugin_root()?.join(id).join("settings.json");
    match fs::read(&path) {
        Ok(bytes) => {
            ensure!(bytes.len() <= 65536, "数据源配置文件过大");
            Ok(serde_json::from_slice(&bytes)?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e.into()),
    }
}
fn limitation(m: &mygo::Manifest, entry_error: bool, imports: bool) -> Option<String> {
    if m.playback != "stream" {
        Some("此插件控制远程播放器，当前只接入音频来源".into())
    } else if !mygo::compatible(m) {
        Some("此插件需要外部程序，当前宿主尚未接入".into())
    } else if !m.main.ends_with(".js") {
        Some("此入口需要 TypeScript 转译，当前支持单文件 JavaScript".into())
    } else if entry_error {
        Some("仓库未提供可直接读取的入口，需要插件发布包".into())
    } else if imports {
        Some("此插件需要多个模块，当前支持单文件入口".into())
    } else if m.permissions.network.iter().any(|v| v == "*") {
        Some("此插件需要动态服务器网络权限，当前尚未接入".into())
    } else {
        None
    }
}
fn source_descriptor(
    m: mygo::Manifest,
    installed: bool,
    exports: Vec<String>,
    fields: Vec<SourceSettingDefinition>,
    limit: Option<String>,
    qr: bool,
) -> Result<MusicSource> {
    let values = if installed {
        settings_value(&m.id)?
    } else {
        BTreeMap::new()
    };
    let settings = fields
        .into_iter()
        .take(64)
        .map(|mut field| {
            field.link = field.link.and_then(|url| metadata::public_url(&url));
            let private = secret(&field.key);
            let value = values.get(&field.key).cloned();
            let configured = value.as_ref().is_some_and(|v| !v.is_empty());
            if private {
                field.value = None;
            }
            SourceSetting {
                definition: field,
                configured,
                secret: private,
                saved_value: if private { None } else { value },
            }
        })
        .collect();
    Ok(MusicSource {
        description: m
            .metadata
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        id: format!("mygo:{}", m.id),
        name: m.name,
        version: m.version,
        repository: m.repository,
        installed,
        can_install: !installed && limit.is_none(),
        limitation: limit,
        capabilities: exports,
        settings,
        qr_sign_in: qr,
        web_sign_in: false,
    })
}
pub fn get_source(source: &str) -> Result<MusicSource> {
    let id = plugin(source)?;
    let dir = mygo::plugin_root()?.join(id);
    let m = mygo::manifest(&dir)?;
    let code = fs::read_to_string(mygo::entry(&dir, &m)?)?;
    let exports = declared_exports(&code);
    let mut fields = m.settings.clone();
    if fields.is_empty() {
        fields = legacy_fields(&code);
    }
    let limit = limitation(&m, false, false);
    let qr = code.contains("host.signInPrompt") && exports.iter().any(|v| v == "signIn");
    let web = code.contains("host.webSignIn");
    let mut result = source_descriptor(m, true, exports, fields, limit, qr)?;
    result.web_sign_in = cfg!(target_os = "macos") && web;
    Ok(result)
}
pub fn list() -> Result<Vec<MusicSource>> {
    mygo::ensure_netease()?;
    let mut sources = Vec::new();
    for row in mygo::list()?.as_array().into_iter().flatten() {
        if let Some(id) = row["id"].as_str() {
            sources.push(get_source(&format!("mygo:{id}"))?);
        }
    }
    Ok(sources)
}
pub fn catalog() -> Result<Vec<MusicSource>> {
    catalog_from(source_catalog::load()?.entries)
}

pub fn catalog_view(refresh: bool) -> Result<MusicSourceCatalog> {
    let catalog = if refresh {
        source_catalog::refresh()?
    } else {
        source_catalog::load()?
    };
    Ok(MusicSourceCatalog {
        sources: catalog_from(catalog.entries)?,
        refreshing: catalog.refreshing,
        warning: catalog.warning,
    })
}

fn catalog_from(entries: Vec<source_catalog::Entry>) -> Result<Vec<MusicSource>> {
    let installed = mygo::list()?;
    let ids: Vec<_> = installed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["id"].as_str())
        .collect();
    let mut result = Vec::new();
    for row in entries {
        let Ok(mut m) = serde_json::from_value::<mygo::Manifest>(row.manifest.clone()) else {
            let repository = format!("https://github.com/{}", row.repo);
            let existing = installed.as_array().into_iter().flatten().find(|p| {
                p["repository"]
                    .as_str()
                    .is_some_and(|url| url.eq_ignore_ascii_case(&repository))
            });
            if let Some(id) = existing.and_then(|p| p["id"].as_str()) {
                result.push(get_source(&format!("mygo:{id}"))?);
                continue;
            }
            result.push(MusicSource {
                id: format!("catalog:{}", row.repo),
                name: if row.name.is_empty() {
                    row.repo.clone()
                } else {
                    row.name.clone()
                },
                description: row.description,
                repository: Some(repository),
                can_install: row.discovered,
                limitation: Some(
                    if row.discovered {
                        "添加时检查来源兼容性"
                    } else {
                        "仓库没有可读取的插件清单"
                    }
                    .into(),
                ),
                ..Default::default()
            });
            continue;
        };
        m.repository = Some(format!("https://github.com/{}", row.repo));
        let has_source = row
            .exports
            .iter()
            .any(|v| v == "search" || v == "stream" || v == "remotePlay");
        let mut limit = limitation(
            &m,
            row.entry_error.is_some() || row.error.is_some(),
            !row.imports.is_empty(),
        );
        if !has_source && row.entry_error.is_none() {
            limit = Some("这是界面或歌词扩展，未提供音乐数据源接口".into());
        }
        let exists = ids.contains(&m.id.as_str());
        let fields = m.settings.clone();
        let qr = row.host_api.iter().any(|v| v == "signInPrompt");
        result.push(source_descriptor(
            m,
            exists,
            row.exports,
            fields,
            limit,
            qr,
        )?);
    }
    Ok(result)
}
fn save_settings(id: &str, values: &BTreeMap<String, String>) -> Result<()> {
    let bytes = serde_json::to_vec(values)?;
    ensure!(bytes.len() <= 65536, "数据源配置超过64KiB");
    let dir = mygo::plugin_root()?.join(id);
    let temp = dir.join(format!(".settings-{}", storage::random_hex::<8>()));
    let result = (|| -> Result<()> {
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        f.write_all(&bytes)?;
        fs::rename(&temp, dir.join("settings.json"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub fn configure(source: &str, changes: BTreeMap<String, String>) -> Result<Value> {
    let id = plugin(source)?;
    ensure!(!auth_active(id), "请先完成或取消此来源的账号关联");
    let info = get_source(source)?;
    ensure!(changes.len() <= 64, "配置项过多");
    let guard = SETTINGS_WRITER
        .lock()
        .map_err(|_| anyhow::anyhow!("配置写入暂不可用"))?;
    let mut values = settings_value(id)?;
    for (key, value) in &changes {
        ensure!(
            info.settings.iter().any(|f| &f.definition.key == key),
            "未声明的配置项"
        );
        ensure!(value.len() <= 16384, "单个配置项过长");
        if value.is_empty() {
            values.remove(key);
        } else {
            values.insert(key.clone(), value.clone());
        }
    }
    save_settings(id, &values)?;
    drop(guard);
    let mut warning = None;
    if info.capabilities.iter().any(|v| v == "setSetting") {
        for (key, value) in changes {
            if mygo::invoke(id, "setSetting", json!({"key":key,"value":value})).is_err() {
                warning = Some("配置已保存，但来源刷新未完成；请检查来源配置");
                break;
            }
        }
    }
    Ok(json!({"source":get_source(source)?,"warning":warning}))
}
fn ensure_installable(row: &source_catalog::Entry) -> Result<()> {
    let info = catalog_from(vec![row.clone()])?
        .pop()
        .context("来源不在目录中")?;
    ensure!(
        info.can_install,
        "{}",
        info.limitation
            .unwrap_or_else(|| "相同标识的数据源已经添加".into())
    );
    Ok(())
}

pub fn install(repository: &str) -> Result<MusicSource> {
    let repo = repository::parse_repository(repository)?;
    // Discovery is optional. Only an already-cached explicit snapshot pins a version.
    let mut row = source_catalog::cached_entry(&repo).unwrap_or_else(|| source_catalog::Entry {
        repo: repo.clone(),
        discovered: true,
        ..Default::default()
    });
    ensure_installable(&row)?;
    let package = if !row.discovered {
        repository::pinned(&repo, &row.commit)?
    } else {
        repository::download(&repo)?
    };
    let manifest = package.manifest;
    let script = package.script;
    use sha2::{Digest, Sha256};
    let mut m: mygo::Manifest = serde_json::from_slice(&manifest)?;
    if !row.discovered {
        ensure!(
            format!("{:x}", Sha256::digest(&manifest)) == row.manifest_sha256,
            "来源清单校验失败"
        );
        ensure!(
            m.id == row.manifest["id"].as_str().unwrap_or_default()
                && m.version == row.manifest["version"].as_str().unwrap_or_default(),
            "来源清单与目录版本不符"
        );
        if m.settings.is_empty() {
            m.settings =
                serde_json::from_value(row.manifest["settings"].clone()).unwrap_or_default();
        }
    }
    ensure!(
        mygo::valid_id(&m.id) && limitation(&m, false, false).is_none(),
        "来源超出当前宿主支持范围"
    );
    let code = std::str::from_utf8(&script).context("插件入口不是有效的 UTF-8")?;
    check_script_syntax(code)?;
    if row.discovered {
        row.manifest = serde_json::from_slice(&manifest)?;
        row.exports = declared_exports(code);
        ensure!(
            !uses_module_imports(code),
            "此插件需要多个模块，当前支持单文件入口"
        );
        ensure_installable(&row)?;
    } else {
        ensure!(
            format!("{:x}", Sha256::digest(&script)) == row.entry_sha256,
            "来源入口校验失败"
        );
    }
    m.repository = Some(format!("https://github.com/{repo}"));
    if !row.description.is_empty() {
        m.metadata
            .entry("description".into())
            .or_insert_with(|| json!(row.description));
    }
    m.metadata
        .insert("sourceVersion".into(), json!(package.version));
    if let Some(commit) = package.commit {
        m.metadata.insert("sourceCommit".into(), json!(commit));
    }
    let staging = mygo::plugin_root()?.join(format!(".source-{}", storage::random_hex::<8>()));
    fs::create_dir(&staging)?;
    let result = (|| -> Result<String> {
        m.main = "index.js".into();
        fs::write(staging.join("index.js"), script)?;
        fs::write(staging.join("plugin.json"), serde_json::to_vec(&m)?)?;
        mygo::install(&staging)
    })();
    let _ = fs::remove_dir_all(staging);
    let id = result?;
    get_source(&format!("mygo:{id}"))
}

pub fn account(source: &str) -> Result<SourceAccount> {
    let id = plugin(source)?;
    let info = get_source(source)?;
    if !info.capabilities.iter().any(|v| v == "account") {
        return Ok(SourceAccount::default());
    }
    let v = mygo::invoke(id, "account", json!({}))
        .map_err(|_| anyhow::anyhow!("账号状态读取失败，请检查数据源配置"))?;
    Ok(SourceAccount {
        connected: !v.is_null() && v["id"].is_string(),
        name: v["name"].as_str().map(|v| v.chars().take(100).collect()),
    })
}
pub fn playlists(source: &str) -> Result<Vec<SourcePlaylist>> {
    let id = plugin(source)?;
    let v = mygo::invoke(id, "playlists", json!({}))
        .map_err(|_| anyhow::anyhow!("来源歌单读取失败，请先关联账号或检查配置"))?;
    let rows = v
        .as_array()
        .or_else(|| v["playlists"].as_array())
        .context("来源未返回歌单列表")?;
    ensure!(rows.len() <= 1000, "来源歌单过多");
    Ok(rows
        .iter()
        .filter_map(|v| {
            let id = v["id"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| v["id"].as_u64().map(|v| v.to_string()))?;
            Some(SourcePlaylist {
                cover_url: v["imageUrl"].as_str().and_then(metadata::artwork_url),
                id,
                name: v["name"].as_str().unwrap_or("未命名歌单").into(),
                track_count: v["trackCount"].as_u64().unwrap_or(0) as usize,
            })
        })
        .collect())
}
pub fn playlist_snapshot(source: &str, id: &str) -> Result<(PlaylistOrigin, Vec<Track>)> {
    let normalized = normalize_playlist_id(source, id)?;
    let id = normalized.as_str();
    ensure!(!id.is_empty() && id.len() <= 1024, "无效的歌单标识");
    let plugin = plugin(source)?;
    let value = mygo::invoke(plugin, "playlist", json!({"id":id}))
        .map_err(|_| anyhow::anyhow!("来源歌单加载失败，请检查账号和配置"))?;
    let page = if value["playlist"].is_object() {
        &value["playlist"]
    } else {
        &value
    };
    let rows = page["tracks"]
        .as_array()
        .or_else(|| value["tracks"].as_array())
        .context("来源没有返回歌单曲目")?;
    ensure!(rows.len() <= 10000, "歌单超过10000首");
    let tracks: Vec<_> = rows
        .iter()
        .filter_map(|v| mygo::catalog_track(plugin, v))
        .collect();
    ensure!(tracks.len() == rows.len(), "来源曲目信息不完整，未导入");
    ensure!(
        page["trackCount"]
            .as_u64()
            .is_none_or(|n| n as usize <= tracks.len()),
        "来源未返回完整歌单，未导入"
    );
    Ok((
        PlaylistOrigin {
            provider: source.into(),
            id: id.into(),
            name: page["name"].as_str().unwrap_or("导入歌单").into(),
        },
        tracks,
    ))
}

fn normalize_playlist_id(source: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if !matches!(source, "netease" | "mygo:netease") || !value.contains("://") {
        return Ok(value.to_owned());
    }
    let url = reqwest::Url::parse(&value.replace("/#/playlist", "/playlist"))?;
    ensure!(
        matches!(url.host_str(), Some("music.163.com" | "y.music.163.com"))
            && url.path() == "/playlist",
        "请输入网易云歌单链接或ID"
    );
    let id = url
        .query_pairs()
        .find(|(k, _)| k == "id")
        .context("链接缺少歌单ID")?
        .1
        .into_owned();
    ensure!(id.parse::<u64>().is_ok_and(|n| n > 0), "无效歌单ID");
    Ok(id)
}

pub(crate) struct LoginJob {
    state: Mutex<SourceLogin>,
    cancel: AtomicBool,
    running: AtomicBool,
}
impl LoginJob {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
    pub fn prompt(&self, text: &str) -> Result<()> {
        ensure!(!self.cancelled(), "关联已取消");
        let value: Value = serde_json::from_str(text)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("关联状态不可用"))?;
        if let Some(text) = value["text"].as_str() {
            state.text = text.chars().take(512).collect();
        }
        if let Some(qr) = value["qr"].as_str() {
            ensure!(qr.len() <= 8192, "二维码过大");
            state.qr_url = Some(qr.to_owned());
            state.svg = Some(
                qrcode::QrCode::new(qr.as_bytes())?
                    .render::<qrcode::render::svg::Color>()
                    .min_dimensions(220, 220)
                    .build(),
            );
        }
        Ok(())
    }
    fn snapshot(&self) -> Result<SourceLogin> {
        Ok(self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("关联状态不可用"))?
            .clone())
    }
}
pub(crate) fn auth_active(id: &str) -> bool {
    LOGIN
        .lock()
        .ok()
        .and_then(|registry| registry.active.clone())
        .is_some_and(|job| {
            job.running.load(Ordering::SeqCst)
                && job
                    .state
                    .lock()
                    .is_ok_and(|s| s.source == format!("mygo:{id}"))
        })
}
fn valid_job(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
}
pub fn begin_sign_in(source: &str, requested: Option<String>) -> Result<SourceLogin> {
    begin_sign_in_method(source, requested, SignInMethod::Qr)
}
pub fn begin_sign_in_method(
    source: &str,
    requested: Option<String>,
    method: SignInMethod,
) -> Result<SourceLogin> {
    let id = plugin(source)?.to_owned();
    let info = get_source(source)?;
    ensure!(
        if method == SignInMethod::Web {
            info.web_sign_in
        } else {
            info.qr_sign_in
        },
        "此来源的关联方式暂未接入，请检查来源配置项"
    );
    let request = requested.unwrap_or_else(storage::random_hex::<12>);
    ensure!(valid_job(&request), "无效的关联任务");
    let mut registry = LOGIN
        .lock()
        .map_err(|_| anyhow::anyhow!("关联状态不可用"))?;
    ensure!(!registry.cancelled.contains(&request), "关联已取消");
    ensure!(
        !registry
            .active
            .as_ref()
            .is_some_and(|j| j.running.load(Ordering::SeqCst)),
        "请先完成或取消当前的数据源关联"
    );
    let job = Arc::new(LoginJob {
        state: Mutex::new(SourceLogin {
            job: request,
            source: info.id.clone(),
            status: "waiting".into(),
            text: if method == SignInMethod::Web {
                "请在网页登录窗口中完成登录"
            } else {
                "正在获取登录二维码…"
            }
            .into(),
            svg: None,
            qr_url: None,
        }),
        cancel: AtomicBool::new(false),
        running: AtomicBool::new(true),
    });
    registry.active = Some(job.clone());
    let initial = job.snapshot()?;
    drop(registry);
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(|| mygo::invoke_sign_in(&id, job.clone(), method))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("关联任务已结束")));
        if let Ok(mut state) = job.state.lock() {
            state.svg = None;
            state.qr_url = None;
            if job.cancelled() {
                state.status = "cancelled".into();
                state.text = "账号关联已取消".into();
            } else if result.is_ok_and(|v| !v["id"].is_null()) {
                state.status = "success".into();
                state.text = "账号已关联".into();
            } else {
                state.status = "error".into();
                state.text = if method == SignInMethod::Web {
                    "网页登录未完成，请重新打开窗口或使用扫码登录"
                } else {
                    "关联未完成，请重新生成二维码或检查数据源配置"
                }
                .into();
            }
        }
        job.running.store(false, Ordering::SeqCst);
    });
    Ok(initial)
}
pub fn login_state(id: &str, cancel: bool) -> Result<SourceLogin> {
    ensure!(valid_job(id), "无效的关联任务");
    let mut registry = LOGIN
        .lock()
        .map_err(|_| anyhow::anyhow!("关联状态不可用"))?;
    let current = registry
        .active
        .clone()
        .filter(|job| job.snapshot().is_ok_and(|s| s.job == id));
    if cancel {
        if !registry.cancelled.iter().any(|key| key == id) {
            registry.cancelled.push_back(id.into());
            if registry.cancelled.len() > 32 {
                registry.cancelled.pop_front();
            }
        }
        if let Some(job) = current {
            if job.running.load(Ordering::SeqCst) {
                job.cancel.store(true, Ordering::SeqCst);
                if let Ok(mut state) = job.state.lock() {
                    state.status = "cancelling".into();
                    state.text = "正在取消关联…".into();
                    state.svg = None;
                    state.qr_url = None;
                }
            }
            return job.snapshot();
        }
        return Ok(SourceLogin {
            job: id.into(),
            status: "cancelled".into(),
            text: "关联已取消".into(),
            ..Default::default()
        });
    }
    current.context("关联已更新，请重新打开")?.snapshot()
}

pub fn execute(input: MusicSourceAction) -> Result<Value> {
    Ok(match input {
        MusicSourceAction::List => serde_json::to_value(list()?)?,
        MusicSourceAction::Catalog => serde_json::to_value(catalog()?)?,
        MusicSourceAction::CatalogView => serde_json::to_value(catalog_view(false)?)?,
        MusicSourceAction::RefreshCatalog => serde_json::to_value(catalog_view(true)?)?,
        MusicSourceAction::Get { source } => serde_json::to_value(get_source(&source)?)?,
        MusicSourceAction::Install { repository } => serde_json::to_value(install(&repository)?)?,
        MusicSourceAction::Configure { source, values } => configure(&source, values)?,
        MusicSourceAction::Account { source } => serde_json::to_value(account(&source)?)?,
        MusicSourceAction::SignIn {
            source,
            job,
            method,
        } => serde_json::to_value(begin_sign_in_method(&source, job, method)?)?,
        MusicSourceAction::Poll { job } => serde_json::to_value(login_state(&job, false)?)?,
        MusicSourceAction::Cancel { job } => serde_json::to_value(login_state(&job, true)?)?,
        MusicSourceAction::SignOut { source } => {
            let id = plugin(&source)?;
            mygo::invoke(id, "signOut", json!({}))
                .map_err(|_| anyhow::anyhow!("来源退出登录失败"))?;
            json!({"ok":true})
        }
        MusicSourceAction::Playlists { source } => serde_json::to_value(playlists(&source)?)?,
        MusicSourceAction::ImportPlaylist {
            source,
            playlist_id,
        } => crate::change_library(LocalLibraryAction::ImportSource {
            source,
            playlist_id,
        })?,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_and_plugin_playlist_links_share_the_same_identity() {
        for source in ["netease", "mygo:netease"] {
            assert_eq!(
                normalize_playlist_id(source, "https://music.163.com/#/playlist?id=123").unwrap(),
                "123"
            );
            assert!(normalize_playlist_id(source, "https://example.com/playlist?id=123").is_err());
            assert!(normalize_playlist_id(source, "https://music.163.com/song?id=123").is_err());
        }
    }
    #[test]
    fn exports_skip_comments_strings_and_support_aliases() {
        assert_eq!(
            declared_exports(
                "// export function fake(){}\nconst x='export function bad(){}'; export async function search(){}; function s(){} export { s as stream };"
            ),
            ["search", "stream"]
        );
    }
    #[test]
    fn single_file_checks_detect_imports_and_reexports_without_matching_strings() {
        for code in [
            "import './other.js';",
            "const result = import('./other.js');",
            "export * from './other.js';",
            "export { search } from './other.js';",
        ] {
            assert!(uses_module_imports(code), "{code}");
        }
        assert!(!uses_module_imports(
            "// import 'fake';\nconst text = 'export * from fake'; export function search() { return Array.from([]); }"
        ));
        assert!(!uses_module_imports("const from = 1; export { from };"));
    }

    #[test]
    fn installation_compiles_without_running_plugin_code() {
        assert!(
            check_script_syntax(
                "throw new Error('must not run'); export function stream() { return {}; }"
            )
            .is_ok()
        );
        assert!(check_script_syntax("export function stream( {").is_err());
    }
    #[test]
    fn legacy_settings_are_discoverable_without_running_code() {
        let f = legacy_fields(
            "host.setting(\"quality\");host.setting('autoVip');host.setting('quality');",
        );
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].key, "quality");
    }
    #[test]
    fn credentials_are_recognized_for_redaction() {
        for key in [
            "password",
            "apiKey",
            "oauthToken",
            "spDcCookie",
            "credential",
            "smsCode",
        ] {
            assert!(secret(key));
        }
        assert!(!secret("server"));
    }

    #[test]
    fn secret_manifest_defaults_never_escape_the_source_descriptor() {
        let manifest:mygo::Manifest=serde_json::from_value(json!({"id":"example","name":"Example","version":"1","main":"index.js","playback":"stream"})).unwrap();
        let field = SourceSettingDefinition {
            key: "apiKey".into(),
            label: "API Key".into(),
            value: Some("do-not-return-this".into()),
            ..Default::default()
        };
        let source = source_descriptor(
            manifest,
            false,
            vec!["stream".into()],
            vec![field],
            None,
            false,
        )
        .unwrap();
        assert!(source.settings[0].secret);
        assert!(
            !serde_json::to_string(&source)
                .unwrap()
                .contains("do-not-return-this")
        );
    }
}
