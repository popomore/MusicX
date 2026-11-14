//! Sources adapt a bounded subset of the MyGo contract; local playlists stay ours.
use crate::{mygo, protocol::*, repository, source_catalog};
use anyhow::{Context, Result, ensure};
use musicx_core::protocol::tr;
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

pub(crate) static SETTINGS_WRITER: Mutex<()> = Mutex::new(());
struct LoginRegistry {
    active: Option<Arc<LoginJob>>,
    cancelled: std::collections::VecDeque<String>,
    completed: std::collections::VecDeque<SourceLogin>,
}
impl LoginRegistry {
    fn activate(&mut self, job: Arc<LoginJob>) {
        if let Some(previous) = self.active.replace(job)
            && !previous.running.load(Ordering::SeqCst)
            && let Ok(mut state) = previous.snapshot()
            && matches!(state.status.as_str(), "success" | "error" | "cancelled")
        {
            // Keep only bounded terminal snapshots, never the sign-in VM or QR prompt.
            state.svg = None;
            state.qr_url = None;
            self.completed.push_back(state);
            if self.completed.len() > 32 {
                self.completed.pop_front();
            }
        }
    }
    fn state(&self, id: &str) -> Result<SourceLogin> {
        if let Some(job) = self
            .active
            .as_ref()
            .filter(|job| job.snapshot().is_ok_and(|state| state.job == id))
        {
            return job.snapshot();
        }
        self.completed
            .iter()
            .find(|state| state.job == id)
            .cloned()
            .context(tr!("Connection changed; please reopen"))
    }
}
static LOGIN: Mutex<LoginRegistry> = Mutex::new(LoginRegistry {
    active: None,
    cancelled: std::collections::VecDeque::new(),
    completed: std::collections::VecDeque::new(),
});

fn plugin(source: &str) -> Result<&str> {
    let id = source
        .strip_prefix("mygo:")
        .context(tr!("Choose a plugin source"))?;
    ensure!(mygo::valid_id(id), tr!("Invalid source identifier"));
    Ok(id)
}

fn script_words(source: &str) -> Vec<String> {
    // Template text is inert, but every ${...} expression is JavaScript again.
    // Keep those contexts on a heap stack rather than recursively scanning input.
    enum Context {
        Code(Option<usize>),
        Template,
    }
    let bytes = source.as_bytes();
    let mut contexts = vec![Context::Code(None)];
    let mut at = 0;
    let mut words = Vec::new();
    while at < bytes.len() {
        let c = bytes[at];
        if matches!(contexts.last(), Some(Context::Template)) {
            match c {
                b'\\' => at = (at + 2).min(bytes.len()),
                b'`' => {
                    contexts.pop();
                    at += 1;
                }
                b'$' if bytes.get(at + 1) == Some(&b'{') => {
                    contexts.push(Context::Code(Some(1)));
                    words.push("{".into());
                    at += 2;
                }
                _ => at += 1,
            }
            continue;
        }
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
        if c == b'`' {
            words.push("<literal>".into());
            contexts.push(Context::Template);
            at += 1;
            continue;
        }
        if b"'\"".contains(&c) {
            words.push("<literal>".into());
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
        // Skip regular-expression contents where an expression can begin.
        let expression_start = words.last().is_none_or(|word: &String| {
            matches!(
                word.as_str(),
                "(" | "["
                    | "{"
                    | "="
                    | ":"
                    | ","
                    | ";"
                    | "!"
                    | "?"
                    | "&"
                    | "|"
                    | "return"
                    | "throw"
                    | "case"
            )
        });
        if c == b'/' && expression_start {
            words.push("<literal>".into());
            at += 1;
            let mut class = false;
            while at < bytes.len() {
                match bytes[at] {
                    b'\\' => {
                        at = (at + 2).min(bytes.len());
                        continue;
                    }
                    b'[' => class = true,
                    b']' => class = false,
                    b'/' if !class => {
                        at += 1;
                        break;
                    }
                    _ => {}
                }
                at += 1;
            }
            while at < bytes.len() && bytes[at].is_ascii_alphabetic() {
                at += 1;
            }
            continue;
        }
        if c.is_ascii_digit() {
            words.push("<literal>".into());
            at += 1;
            while at < bytes.len()
                && (bytes[at].is_ascii_alphanumeric() || b"._".contains(&bytes[at]))
            {
                at += 1;
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
            if b"{}[]()*,;.:=!?&|".contains(&c) {
                words.push((c as char).to_string());
            }
            if let Some(Context::Code(Some(depth))) = contexts.last_mut() {
                if c == b'{' {
                    *depth += 1;
                }
                if c == b'}' {
                    *depth -= 1;
                    if *depth == 0 {
                        contexts.pop();
                    }
                }
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
            if words
                .get(i)
                .is_some_and(|v| matches!(v.as_str(), "const" | "let" | "var"))
            {
                i += 1;
                let mut nesting = 0usize;
                let mut binding = true;
                while let Some(word) = words.get(i) {
                    if nesting == 0
                        && matches!(word.as_str(), ";" | "export" | "const" | "let" | "var")
                    {
                        break;
                    }
                    if nesting == 0 && binding {
                        if word
                            .as_bytes()
                            .first()
                            .is_some_and(|c| c.is_ascii_alphabetic() || b"_$".contains(c))
                        {
                            out.push(word.clone());
                        }
                        binding = false;
                    }
                    match word.as_str() {
                        "{" | "[" | "(" => nesting += 1,
                        "}" | "]" | ")" => nesting = nesting.saturating_sub(1),
                        "," if nesting == 0 => binding = true,
                        _ => {}
                    }
                    i += 1;
                }
            } else if words
                .get(i)
                .is_some_and(|v| matches!(v.as_str(), "function" | "class"))
            {
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
            if words.get(at.wrapping_sub(1)).is_some_and(|v| v == ".")
                || words
                    .get(at + 1)
                    .is_some_and(|v| matches!(v.as_str(), ":" | "."))
            {
                return false;
            }
            return true;
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

pub(crate) fn check_single_file_script(source: &str) -> Result<()> {
    check_script_syntax(source)?;
    ensure!(
        !uses_module_imports(source),
        tr!("This plugin needs multiple modules; only single-file entries are supported")
    );
    Ok(())
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
            .map_err(|error| {
                anyhow::anyhow!(tr!("Could not compile the plugin entry: {error}"; error = error))
            })
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
                        "quality" => tr!("Quality preference"),
                        "autoVip" => tr!("Daily free VIP"),
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
            ensure!(
                bytes.len() <= 65536,
                tr!("Source configuration file is too large")
            );
            Ok(serde_json::from_slice(&bytes)?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e.into()),
    }
}
pub(crate) fn limitation(m: &mygo::Manifest, entry_error: bool, imports: bool) -> Option<String> {
    if let Some(reason) = mygo::minimum_version_limitation(m) {
        Some(reason)
    } else if m.playback != "stream" {
        Some(tr!("This plugin controls a remote player; only audio sources are supported").into())
    } else if !mygo::compatible(m) {
        Some(tr!("This plugin requires an external program that is not supported yet").into())
    } else if !m.main.ends_with(".js") && !m.main.ends_with(".ts") {
        Some(tr!("Only single-file JavaScript or TypeScript is supported").into())
    } else if entry_error {
        Some(
            tr!("The repository has no readable entry; a plugin release package is required")
                .into(),
        )
    } else if imports {
        Some(
            tr!("This plugin needs multiple modules; only single-file entries are supported")
                .into(),
        )
    } else if m.permissions.network.iter().any(|v| v == "*") {
        Some(
            tr!(
                "This plugin requires dynamic server network permissions that are not supported yet"
            )
            .into(),
        )
    } else {
        None
    }
}
fn source_descriptor(
    mut m: mygo::Manifest,
    installed: bool,
    exports: Vec<String>,
    mut fields: Vec<SourceSettingDefinition>,
    limit: Option<String>,
    qr: bool,
) -> Result<MusicSource> {
    if installed {
        let translations = crate::localization::translations(&m.id);
        if let Some(name) = translations.get(&m.name) {
            m.name.clone_from(name);
        }
        for field in &mut fields {
            if let Some(label) = translations.get(&field.label) {
                field.label.clone_from(label);
            }
            if let Some(help) = translations.get(&field.help) {
                field.help.clone_from(help);
            }
        }
    }
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
        oauth_sign_in: false,
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
    result.oauth_sign_in = cfg!(target_os = "macos")
        && code.contains("host.waitForRedirect")
        && code.contains("host.openURL");
    Ok(result)
}
pub fn list() -> Result<Vec<MusicSource>> {
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
                        tr!("Compatibility will be checked when adding the source")
                    } else {
                        tr!("No readable plugin manifest in the repository")
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
            limit = Some(
                tr!("This is a UI or lyrics extension without a music source interface").into(),
            );
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
    ensure!(
        bytes.len() <= 65536,
        tr!("Source configuration exceeds 64 KiB")
    );
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
    ensure!(
        !auth_active(id),
        tr!("Complete or cancel account connection for this source first")
    );
    let info = get_source(source)?;
    ensure!(changes.len() <= 64, tr!("Too many settings"));
    let guard = SETTINGS_WRITER
        .lock()
        .map_err(|_| anyhow::anyhow!(tr!("Settings cannot be saved right now")))?;
    let mut values = settings_value(id)?;
    for (key, value) in &changes {
        ensure!(
            info.settings.iter().any(|f| &f.definition.key == key),
            tr!("Undeclared setting")
        );
        ensure!(value.len() <= 16384, tr!("Setting value is too long"));
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
                warning = Some(tr!(
                    "Settings saved, but the source could not refresh. Check its configuration."
                ));
                break;
            }
        }
    }
    Ok(json!({"source":get_source(source)?,"warning":warning}))
}
fn ensure_installable(row: &source_catalog::Entry) -> Result<()> {
    let info = catalog_from(vec![row.clone()])?
        .pop()
        .context(tr!("Source not found in the catalog"))?;
    ensure!(
        info.can_install,
        "{}",
        info.limitation
            .unwrap_or_else(|| tr!("A source with this identifier has already been added").into())
    );
    Ok(())
}

pub fn install(repository: &str) -> Result<MusicSource> {
    install_package(repository, None)
}
fn install_package(repository: &str, replacing: Option<&str>) -> Result<MusicSource> {
    install_package_from(repository, replacing, repository::API, None)
}
fn install_package_from(
    repository: &str,
    replacing: Option<&str>,
    api: &str,
    expected: Option<&crate::installation::Snapshot>,
) -> Result<MusicSource> {
    let repo = repository::parse_repository(repository)?;
    // Discovery is optional. Only an already-cached explicit snapshot pins a version.
    let mut row = source_catalog::cached_entry(&repo).unwrap_or_else(|| source_catalog::Entry {
        repo: repo.clone(),
        discovered: true,
        ..Default::default()
    });
    if replacing.is_none() {
        ensure_installable(&row)?;
    }
    let package = if !row.discovered {
        repository::pinned(&repo, &row.commit)?
    } else {
        repository::download_from(&repo, api)?
    };
    let manifest = package.manifest;
    let script = package.script;
    use sha2::{Digest, Sha256};
    let mut m: mygo::Manifest = serde_json::from_slice(&manifest)?;
    if let Some(id) = replacing {
        ensure!(
            m.id == id,
            tr!("Update source identifier does not match the installed source")
        );
    }
    if let Some(expected) = expected {
        let installed: mygo::Manifest = serde_json::from_slice(&expected.manifest)?;
        if let (Ok(current), Ok(next)) = (
            semver::Version::parse(installed.version.trim_start_matches('v')),
            semver::Version::parse(m.version.trim_start_matches('v')),
        ) {
            ensure!(
                next >= current,
                tr!("Automatic updates skipped an older plugin version")
            );
        }
    }
    if !row.discovered {
        ensure!(
            format!("{:x}", Sha256::digest(&manifest)) == row.manifest_sha256,
            tr!("Source manifest verification failed")
        );
        ensure!(
            m.id == row.manifest["id"].as_str().unwrap_or_default()
                && m.version == row.manifest["version"].as_str().unwrap_or_default(),
            tr!("Source manifest version does not match the catalog")
        );
        if m.settings.is_empty() {
            m.settings =
                serde_json::from_value(row.manifest["settings"].clone()).unwrap_or_default();
        }
    }
    ensure!(
        mygo::valid_id(&m.id) && limitation(&m, false, false).is_none(),
        tr!("This source is not supported by the current host")
    );
    let original = std::str::from_utf8(&script).context(tr!("Plugin entry is not valid UTF-8"))?;
    let compiled = crate::script::javascript(&m.main, original)?;
    let code = compiled.as_str();
    check_single_file_script(code)?;
    if row.discovered {
        row.manifest = serde_json::from_slice(&manifest)?;
        row.exports = declared_exports(code);
        if replacing.is_none() {
            ensure_installable(&row)?;
        }
    } else {
        ensure!(
            format!("{:x}", Sha256::digest(&script)) == row.entry_sha256,
            tr!("Source entry verification failed")
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
    m.metadata.remove("sourceCommit");
    if let Some(commit) = package.commit {
        m.metadata.insert("sourceCommit".into(), json!(commit));
    }
    let staging = mygo::plugin_root()?.join(format!(".source-{}", storage::random_hex::<8>()));
    fs::create_dir(&staging)?;
    let result = (|| -> Result<String> {
        m.main = "index.js".into();
        fs::write(staging.join("index.js"), compiled)?;
        fs::write(staging.join("plugin.json"), serde_json::to_vec(&m)?)?;
        if !package.locales.is_empty() {
            fs::create_dir(staging.join("locales"))?;
            for (language, bytes) in package.locales {
                fs::write(
                    staging.join("locales").join(format!("{language}.json")),
                    bytes,
                )?;
            }
        }
        mygo::install_repository(&staging, replacing.is_some(), expected)
    })();
    let _ = fs::remove_dir_all(staging);
    let id = result?;
    get_source(&format!("mygo:{id}"))
}

pub fn update(source: &str) -> Result<MusicSource> {
    let id = plugin(source)?;
    let info = get_source(source)?;
    let repository = info.repository.context(tr!(
        "No repository recorded for this source; update from a local plugin directory"
    ))?;
    install_package(&repository, Some(id))
}
pub fn check_update(source: &str) -> Result<Value> {
    let id = plugin(source)?;
    let m = mygo::manifest(&mygo::plugin_root()?.join(id))?;
    let repo = repository::parse_repository(
        m.repository
            .as_deref()
            .context(tr!("No repository recorded for this source"))?,
    )?;
    let latest = latest_source_version(&repo, repository::API)?;
    let current = source_version(&m);
    Ok(json!({"available":current != latest,"installed_version":current,"latest_version":latest}))
}

fn latest_source_version(repo: &str, api: &str) -> Result<String> {
    Ok(
        if let Some(row) = source_catalog::cached_entry(repo).filter(|r| !r.discovered) {
            row.commit
        } else {
            repository::latest_version(repo, api)?
        },
    )
}

pub(crate) fn repository_version(m: &mygo::Manifest) -> Option<&str> {
    m.metadata
        .get("sourceVersion")
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .or_else(|| {
            m.metadata
                .get("sourceCommit")
                .and_then(Value::as_str)
                .filter(|sha| sha.len() == 40 && sha.bytes().all(|c| c.is_ascii_hexdigit()))
        })
}

pub(crate) fn source_version(m: &mygo::Manifest) -> &str {
    repository_version(m).unwrap_or(&m.version)
}

pub(crate) fn auto_update(
    id: &str,
    api: &str,
    expected: &crate::installation::Snapshot,
) -> Result<String> {
    let m: mygo::Manifest = serde_json::from_slice(&expected.manifest)?;
    let repository = m
        .repository
        .as_deref()
        .context(tr!("No repository recorded for this source"))?;
    let repo = repository::parse_repository(repository)?;
    if source_version(&m) != latest_source_version(&repo, api)? {
        install_package_from(repository, Some(id), api, Some(expected))?;
    }
    let installed = mygo::manifest(&mygo::plugin_root()?.join(id))?;
    Ok(source_version(&installed).to_owned())
}

pub fn account(source: &str) -> Result<SourceAccount> {
    let id = plugin(source)?;
    let info = get_source(source)?;
    if !info.capabilities.iter().any(|v| v == "account") {
        return Ok(SourceAccount::default());
    }
    let v = mygo::invoke(id, "account", json!({})).map_err(|_| {
        anyhow::anyhow!(tr!(
            "Could not read account status; check the source settings"
        ))
    })?;
    Ok(SourceAccount {
        connected: !v.is_null() && v["id"].is_string(),
        name: v["name"].as_str().map(|v| v.chars().take(100).collect()),
    })
}
pub fn playlists(source: &str) -> Result<Vec<SourcePlaylist>> {
    let id = plugin(source)?;
    let v = mygo::invoke(id, "playlists", json!({})).map_err(|_| {
        anyhow::anyhow!(tr!(
            "Could not read source playlists; connect an account or check settings"
        ))
    })?;
    let rows = v
        .as_array()
        .or_else(|| v["playlists"].as_array())
        .context(tr!("The source returned no playlist list"))?;
    ensure!(rows.len() <= 1000, tr!("Too many source playlists"));
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
                name: v["name"]
                    .as_str()
                    .unwrap_or(tr!("Untitled playlist"))
                    .into(),
                track_count: v["trackCount"].as_u64().unwrap_or(0) as usize,
            })
        })
        .collect())
}
pub fn playlist_snapshot(source: &str, id: &str) -> Result<(PlaylistOrigin, Vec<Track>)> {
    let id = id.trim();
    ensure!(
        !id.is_empty() && id.len() <= 1024,
        tr!("Invalid playlist identifier")
    );
    let plugin = plugin(source)?;
    let value = mygo::invoke(plugin, "playlist", json!({"id":id})).map_err(|_| {
        anyhow::anyhow!(tr!(
            "Could not load the source playlist; check your account and settings"
        ))
    })?;
    let page = if value["playlist"].is_object() {
        &value["playlist"]
    } else {
        &value
    };
    let rows = page["tracks"]
        .as_array()
        .or_else(|| value["tracks"].as_array())
        .context(tr!("The source returned no playlist tracks"))?;
    ensure!(rows.len() <= 10000, tr!("Playlist exceeds 10,000 tracks"));
    let tracks: Vec<_> = rows
        .iter()
        .filter_map(|v| mygo::catalog_track(plugin, v))
        .collect();
    ensure!(
        tracks.len() == rows.len(),
        tr!("Source track information is incomplete; nothing was imported")
    );
    ensure!(
        page["trackCount"]
            .as_u64()
            .is_none_or(|n| n as usize <= tracks.len()),
        tr!("The source returned an incomplete playlist; nothing was imported")
    );
    Ok((
        PlaylistOrigin {
            provider: source.into(),
            id: id.into(),
            name: page["name"]
                .as_str()
                .unwrap_or(tr!("Import playlist"))
                .into(),
        },
        tracks,
    ))
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
        ensure!(!self.cancelled(), tr!("Connection canceled"));
        let value: Value = serde_json::from_str(text)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!(tr!("Connection status unavailable")))?;
        if let Some(text) = value["text"].as_str() {
            state.text = text.chars().take(512).collect();
        }
        if let Some(qr) = value["qr"].as_str() {
            ensure!(qr.len() <= 8192, tr!("QR code is too large"));
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
            .map_err(|_| anyhow::anyhow!(tr!("Connection status unavailable")))?
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
        match method {
            SignInMethod::Web => info.web_sign_in,
            SignInMethod::Qr => info.qr_sign_in,
            SignInMethod::Oauth => info.oauth_sign_in,
        },
        tr!("This connection method is not supported; check the source settings")
    );
    let request = requested.unwrap_or_else(storage::random_hex::<12>);
    ensure!(valid_job(&request), tr!("Invalid connection task"));
    let mut registry = LOGIN
        .lock()
        .map_err(|_| anyhow::anyhow!(tr!("Connection status unavailable")))?;
    ensure!(
        !registry.cancelled.contains(&request),
        tr!("Connection canceled")
    );
    ensure!(
        !registry
            .active
            .as_ref()
            .is_some_and(|j| j.running.load(Ordering::SeqCst)),
        tr!("Complete or cancel the current source connection first")
    );
    let job = Arc::new(LoginJob {
        state: Mutex::new(SourceLogin {
            job: request,
            source: info.id.clone(),
            status: "waiting".into(),
            text: if method == SignInMethod::Web {
                tr!("Complete sign-in in the web sign-in window")
            } else if method == SignInMethod::Oauth {
                tr!("Complete source sign-in in your browser")
            } else {
                tr!("Getting sign-in QR code…")
            }
            .into(),
            svg: None,
            qr_url: None,
        }),
        cancel: AtomicBool::new(false),
        running: AtomicBool::new(true),
    });
    registry.activate(job.clone());
    let initial = job.snapshot()?;
    drop(registry);
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(|| mygo::invoke_sign_in(&id, job.clone(), method))
            .unwrap_or_else(|_| Err(anyhow::anyhow!(tr!("Connection task ended"))));
        if let Ok(mut state) = job.state.lock() {
            state.svg = None;
            state.qr_url = None;
            if job.cancelled() {
                state.status = "cancelled".into();
                state.text = tr!("Source account connection canceled").into();
            } else if result.is_ok_and(|v| !v["id"].is_null()) {
                state.status = "success".into();
                state.text = tr!("Account connected").into();
            } else {
                state.status = "error".into();
                state.text = if method == SignInMethod::Web {
                    tr!("Web sign-in incomplete; reopen the window or use QR sign-in")
                } else {
                    tr!("Connection incomplete; generate a new QR code or check source settings")
                }
                .into();
            }
        }
        job.running.store(false, Ordering::SeqCst);
    });
    Ok(initial)
}
pub fn login_state(id: &str, cancel: bool) -> Result<SourceLogin> {
    ensure!(valid_job(id), tr!("Invalid connection task"));
    let mut registry = LOGIN
        .lock()
        .map_err(|_| anyhow::anyhow!(tr!("Connection status unavailable")))?;
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
                    state.text = tr!("Canceling connection…").into();
                    state.svg = None;
                    state.qr_url = None;
                }
            }
            return job.snapshot();
        }
        return Ok(SourceLogin {
            job: id.into(),
            status: "cancelled".into(),
            text: tr!("Connection canceled").into(),
            ..Default::default()
        });
    }
    registry.state(id)
}

pub fn execute(input: MusicSourceAction) -> Result<Value> {
    Ok(match input {
        MusicSourceAction::List => serde_json::to_value(list()?)?,
        MusicSourceAction::AutoUpdateSettings => {
            serde_json::to_value(crate::plugin_updates::settings()?)?
        }
        MusicSourceAction::SetAutoUpdate { enabled } => {
            serde_json::to_value(crate::plugin_updates::set_enabled(enabled)?)?
        }
        MusicSourceAction::Catalog => serde_json::to_value(catalog()?)?,
        MusicSourceAction::CatalogView => serde_json::to_value(catalog_view(false)?)?,
        MusicSourceAction::RefreshCatalog => serde_json::to_value(catalog_view(true)?)?,
        MusicSourceAction::Get { source } => serde_json::to_value(get_source(&source)?)?,
        MusicSourceAction::Install { repository } => serde_json::to_value(install(&repository)?)?,
        MusicSourceAction::CheckUpdate { source } => check_update(&source)?,
        MusicSourceAction::Update { source } => serde_json::to_value(update(&source)?)?,
        MusicSourceAction::Uninstall { source } => {
            mygo::uninstall(plugin(&source)?)?;
            json!({"ok":true})
        }
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
                .map_err(|_| anyhow::anyhow!(tr!("Could not sign out of the source")))?;
            json!({"ok":true})
        }
        MusicSourceAction::Playlists { source } => serde_json::to_value(playlists(&source)?)?,
        MusicSourceAction::ImportLiked { source } => crate::discovery::import(&source, true)?,
        MusicSourceAction::ImportRecent { source } => crate::discovery::import(&source, false)?,
        MusicSourceAction::Artist { source, id } => {
            serde_json::to_value(crate::discovery::browse(&source, "artist", &id, 0)?)?
        }
        MusicSourceAction::Album { source, id } => {
            serde_json::to_value(crate::discovery::browse(&source, "album", &id, 0)?)?
        }
        MusicSourceAction::Shows { source } => {
            serde_json::to_value(crate::discovery::browse(&source, "shows", "", 0)?)?
        }
        MusicSourceAction::Artwork { source, url } => {
            use base64::Engine;
            json!({"data":base64::engine::general_purpose::STANDARD.encode(crate::assets::image(plugin(&source)?, &url)?)})
        }
        MusicSourceAction::Show { source, id, offset } => {
            serde_json::to_value(crate::discovery::browse(&source, "show", &id, offset)?)?
        }
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
    fn completed_login_remains_pollable_after_another_browser_starts() {
        let job = |id: &str, status: &str| {
            Arc::new(LoginJob {
                state: Mutex::new(SourceLogin {
                    job: id.into(),
                    source: "mygo:qa".into(),
                    status: status.into(),
                    ..Default::default()
                }),
                cancel: AtomicBool::new(false),
                running: AtomicBool::new(status == "waiting"),
            })
        };
        let mut registry = LoginRegistry {
            active: None,
            cancelled: Default::default(),
            completed: Default::default(),
        };
        registry.activate(job("settings-A", "success"));
        registry.activate(job("player-B", "waiting"));
        assert_eq!(registry.state("settings-A").unwrap().status, "success");
        assert_eq!(registry.state("player-B").unwrap().status, "waiting");
        for index in 0..40 {
            registry.activate(job(&format!("completed-{index}"), "success"));
        }
        assert!(registry.completed.len() <= 32);
        assert!(registry.state("settings-A").is_err());
        assert_eq!(registry.state("completed-38").unwrap().status, "success");
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
    fn imports_in_template_interpolations_are_rejected() {
        for code in [
            "export const stream = () => `${import('./helper.js')}`;",
            "export const stream = () => `outer ${`inner ${import('./helper.js')}`}`;",
            "export const stream = () => `${({ text: '}', module: import('./helper.js') })}`;",
        ] {
            assert!(check_script_syntax(code).is_ok(), "{code}");
            assert!(uses_module_imports(code), "{code}");
        }
        assert!(!uses_module_imports(
            "const text = `import('./helper.js') \\${ignored}`; export function stream() {}"
        ));
    }

    #[test]
    fn import_property_names_do_not_require_other_modules() {
        for code in [
            "const options = { import: false }; export function stream() {}",
            "const options = { import: () => false }; export function stream() { return options.import(); }",
            "export function stream() { return import.meta.url; }",
        ] {
            assert!(check_script_syntax(code).is_ok(), "{code}");
            assert!(!uses_module_imports(code), "{code}");
        }
    }

    #[test]
    fn exported_variable_declarations_include_later_bindings() {
        for declaration in ["const", "let", "var"] {
            let code = format!(
                "export {declaration} version = '1', options = {{a: [1,2]}}, stream = () => ({{url:'https://example.com/a.mp3'}}), search = () => [];"
            );
            assert!(check_script_syntax(&code).is_ok());
            assert_eq!(
                declared_exports(&code),
                ["options", "search", "stream", "version"]
            );
        }
        assert_eq!(
            declared_exports("export const ratio = 4 / 2, stream = () => ({});"),
            ["ratio", "stream"]
        );
    }

    #[test]
    fn installation_validation_rejects_dependencies_without_catalog_import_metadata() {
        let code = "export function stream() { return import('./helper.js'); }";
        assert!(check_single_file_script(code).is_err());
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
