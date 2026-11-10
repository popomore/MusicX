//! Download the configured catalog into the application's data directory on demand.
use anyhow::{Context, Result, ensure};
use musicx_core::storage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

const URL_ENV: &str = "MUSICX_PLUGIN_CATALOG_URL";
// Same query used by egoist/mygo-music's website/src/plugins.ts.
const DEFAULT_URL: &str = "https://api.github.com/search/repositories?q=topic%3Amygo-music-plugin%20archived%3Afalse&sort=stars&per_page=100";
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const CACHE_FILE: &str = "mygo_source_catalog.json";
const CACHE_TTL: u64 = 60 * 60;

#[derive(Deserialize, Serialize)]
struct CachedCatalog {
    url: String,
    fetched_at: u64,
    catalog: Value,
    #[serde(default)]
    etag: Option<String>,
}

#[derive(Clone, Default)]
struct RefreshState {
    running: bool,
    warning: Option<String>,
    retry_at: u64,
}
type CatalogKey = (PathBuf, String);
static REFRESHES: Mutex<BTreeMap<CatalogKey, RefreshState>> = Mutex::new(BTreeMap::new());

pub(crate) struct Catalog {
    pub entries: Vec<Entry>,
    pub refreshing: bool,
    pub warning: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
pub(crate) struct Entry {
    pub repo: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(skip)]
    pub discovered: bool,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub manifest: Value,
    #[serde(default)]
    pub exports: Vec<String>,
    #[serde(default)]
    pub imports: Vec<String>,
    #[serde(default)]
    pub host_api: Vec<String>,
    #[serde(default)]
    pub entry_error: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub manifest_sha256: String,
    #[serde(default)]
    pub entry_sha256: String,
}

fn configured_url() -> Result<reqwest::Url> {
    let url = match std::env::var(URL_ENV) {
        Ok(url) => Some(url),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error).context("插件目录 URL 配置无效"),
    };
    catalog_url(url.as_deref())
}

fn catalog_url(url: Option<&str>) -> Result<reqwest::Url> {
    let url = url.unwrap_or(DEFAULT_URL);
    let url = reqwest::Url::parse(url.trim()).context("插件目录 URL 无效")?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "插件目录 URL 必须使用 HTTP 或 HTTPS"
    );
    Ok(url)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn begin_refresh(key: &CatalogKey, force: bool) -> bool {
    let mut registry = REFRESHES.lock().unwrap_or_else(|e| e.into_inner());
    let state = registry.entry(key.clone()).or_default();
    if state.running || (!force && state.retry_at > now()) {
        return false;
    }
    state.running = true;
    state.warning = None;
    true
}

fn finish_refresh(key: &CatalogKey, failed: bool) {
    let mut registry = REFRESHES.lock().unwrap_or_else(|e| e.into_inner());
    let state = registry.entry(key.clone()).or_default();
    state.running = false;
    state.warning = failed.then(|| "目录更新失败，正在显示已保存的目录".into());
    state.retry_at = if failed { now() + 60 } else { 0 };
}

fn state(key: &CatalogKey) -> RefreshState {
    REFRESHES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn load() -> Result<Catalog> {
    view_from(configured_url()?, &storage::state_dir()?)
}

pub(crate) fn refresh() -> Result<Catalog> {
    view_from_mode(configured_url()?, &storage::state_dir()?, true)
}

pub(crate) fn cached_entry(repo: &str) -> Option<Entry> {
    let url = configured_url().ok()?;
    let directory = storage::state_dir().ok()?;
    parse(read_cache(&directory, url.as_str())?.catalog)
        .ok()?
        .into_iter()
        .find(|row| row.repo.eq_ignore_ascii_case(repo))
}

fn view_from(url: reqwest::Url, directory: &Path) -> Result<Catalog> {
    view_from_mode(url, directory, false)
}

fn view_from_mode(url: reqwest::Url, directory: &Path, force: bool) -> Result<Catalog> {
    let key = (directory.to_path_buf(), url.to_string());
    if let Some(cached) = read_cache(directory, url.as_str()) {
        let fresh = now()
            .checked_sub(cached.fetched_at)
            .is_some_and(|age| age < CACHE_TTL);
        let started = (force || !fresh) && begin_refresh(&key, force);
        if started {
            let job = key.clone();
            std::thread::spawn(move || {
                let result = download(&url, &job.0);
                finish_refresh(&job, result.is_err());
            });
        }
        let state = state(&key);
        return Ok(Catalog {
            entries: parse(cached.catalog)?,
            // Keep polling once even if the download finished before this response.
            refreshing: started || state.running,
            warning: state.warning,
        });
    }
    if !begin_refresh(&key, force) {
        ensure!(!state(&key).running, "插件目录正在下载，请稍后重试");
        anyhow::bail!("目录暂不可用，请稍后刷新，也可直接输入仓库添加");
    }
    let result = download(&url, directory);
    finish_refresh(&key, result.is_err());
    Ok(Catalog {
        entries: result?,
        refreshing: false,
        warning: None,
    })
}

fn download(url: &reqwest::Url, directory: &Path) -> Result<Vec<Entry>> {
    let previous = read_cache(directory, url.as_str());
    let response = crate::http::request(
        url.as_str(),
        previous.as_ref().and_then(|cache| cache.etag.as_deref()),
        None,
    )
    .context("无法读取远程插件目录")?;
    let mut cached = if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        let mut cached = previous.context("插件目录没有可复用的缓存")?;
        cached.fetched_at = now();
        cached
    } else {
        let response = response
            .error_for_status()
            .context("无法读取远程插件目录")?;
        let etag = response
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = crate::http::read(response, MAX_BYTES).context("无法读取远程插件目录")?;
        CachedCatalog {
            url: url.to_string(),
            fetched_at: now(),
            etag,
            catalog: serde_json::from_slice(&bytes).context("远程插件目录 JSON 格式无效")?,
        }
    };
    cached.catalog = lightweight_catalog(cached.catalog);
    let entries = parse(cached.catalog.clone())?;
    write_cache(directory, &cached).context("无法保存插件目录缓存")?;
    Ok(entries)
}

fn lightweight_catalog(mut catalog: Value) -> Value {
    if let Some(items) = catalog.get_mut("items").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(repo) = item.as_object_mut() {
                repo.retain(|key, _| matches!(key.as_str(), "full_name" | "name" | "description"));
            }
        }
    }
    catalog
}

#[cfg(test)]
fn load_from(url: Option<&str>, directory: &Path) -> Result<Vec<Entry>> {
    Ok(view_from(catalog_url(url)?, directory)?.entries)
}

fn read_cache(directory: &Path, url: &str) -> Option<CachedCatalog> {
    let mut bytes = Vec::new();
    File::open(directory.join(CACHE_FILE))
        .ok()?
        .take(MAX_BYTES * 2 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_BYTES * 2 {
        return None;
    }
    let cached: CachedCatalog = serde_json::from_slice(&bytes).ok()?;
    if cached.url != url {
        return None;
    }
    parse(cached.catalog.clone()).ok()?;
    Some(cached)
}

fn write_cache(directory: &Path, cached: &CachedCatalog) -> Result<()> {
    let temporary = directory.join(format!(".source-catalog-{}", storage::random_hex::<8>()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(cached)?)?;
        fs::rename(&temporary, directory.join(CACHE_FILE))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub(crate) fn valid_repository(repo: &str) -> bool {
    let parts: Vec<_> = repo.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && !matches!(*part, "." | "..")
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
}

fn parse(catalog: Value) -> Result<Vec<Entry>> {
    #[derive(Deserialize)]
    struct Repository {
        full_name: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        description: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Catalog {
        Snapshot(Vec<Entry>),
        GitHub { items: Vec<Repository> },
    }
    let catalog: Catalog = serde_json::from_value(catalog).context("远程插件目录 JSON 格式无效")?;
    let entries = match catalog {
        Catalog::Snapshot(entries) => entries,
        Catalog::GitHub { items } => items
            .into_iter()
            .map(|repo| Entry {
                repo: repo.full_name,
                name: repo.name,
                description: repo.description.unwrap_or_default(),
                discovered: true,
                ..Default::default()
            })
            .collect(),
    };
    ensure!(
        entries.iter().all(|row| valid_repository(&row.repo)),
        "插件目录包含无效的仓库路径"
    );
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        thread::{self, JoinHandle},
        time::Duration,
    };

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("musicx-catalog-{}", storage::random_hex::<8>()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn serve(status: &str, body: Vec<u8>) -> (String, JoinHandle<()>) {
        serve_headers(status, "", body, None)
    }
    fn serve_headers(
        status: &str,
        headers: &str,
        body: Vec<u8>,
        expected_etag: Option<&str>,
    ) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/catalog.json", listener.local_addr().unwrap());
        let status = status.to_owned();
        let headers = headers.to_owned();
        let expected_etag = expected_etag.map(str::to_owned);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            assert!(request.starts_with(b"GET /catalog.json HTTP/1.1\r\n"));
            if let Some(etag) = expected_etag {
                assert!(
                    String::from_utf8_lossy(&request)
                        .to_ascii_lowercase()
                        .contains(&format!("if-none-match: {etag}\r\n"))
                );
            }
            write!(
                stream,
                "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        (url, server)
    }

    fn wait_for_refresh(directory: &Path, url: &str) {
        let key = (directory.to_path_buf(), url.to_owned());
        for _ in 0..200 {
            if !state(&key).running {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("catalog refresh did not complete");
    }

    #[test]
    fn downloaded_catalog_is_saved_and_reused_without_a_server() {
        let directory = Directory::new();
        let body = br#"[{"repo":"example/plugin","commit":"custom","manifest":{"id":"custom"},"exports":["stream"]}]"#;
        let (url, server) = serve("200 OK", body.to_vec());
        let catalog = load_from(Some(&url), &directory.0).unwrap();
        server.join().unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].repo, "example/plugin");
        assert_eq!(catalog[0].manifest["id"], "custom");
        assert_eq!(catalog[0].exports, ["stream"]);
        let path = directory.0.join(CACHE_FILE);
        let cache: CachedCatalog = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(cache.url, url);
        assert_eq!(cache.catalog[0]["repo"], "example/plugin");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            load_from(Some(&url), &directory.0).unwrap()[0].repo,
            "example/plugin"
        );
    }

    #[test]
    fn remote_failures_without_a_valid_cache_are_reported() {
        for (status, body, message) in [
            ("404 Not Found", b"[]".to_vec(), "无法读取远程插件目录"),
            ("200 OK", b"not json".to_vec(), "远程插件目录 JSON 格式无效"),
            ("200 OK", b"{}".to_vec(), "远程插件目录 JSON 格式无效"),
            (
                "200 OK",
                vec![b' '; MAX_BYTES as usize + 1],
                "无法读取远程插件目录",
            ),
        ] {
            let directory = Directory::new();
            let (url, server) = serve(status, body);
            let error = load_from(Some(&url), &directory.0)
                .err()
                .expect("invalid catalog");
            server.join().unwrap();
            assert_eq!(error.to_string(), message);
            assert!(!directory.0.join(CACHE_FILE).exists());
        }
    }

    #[test]
    fn invalid_or_non_http_urls_are_rejected() {
        let directory = Directory::new();
        for url in [
            "",
            "catalog.json",
            "file:///tmp/catalog.json",
            "ftp://example.com/catalog.json",
        ] {
            assert!(load_from(Some(url), &directory.0).is_err(), "{url}");
        }
    }

    #[test]
    fn github_discovery_needs_no_bundled_metadata() {
        let entries = parse(serde_json::json!({"items":[{"full_name":"example/fixture-source","name":"Fixture","description":"测试来源"},{"full_name":"example/new-plugin"}]})).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].repo, "example/fixture-source");
        assert!(entries[0].discovered);
        assert_eq!(entries[0].name, "Fixture");
        assert_eq!(entries[0].description, "测试来源");
        assert!(entries[1].discovered);
        assert_eq!(entries[1].repo, "example/new-plugin");
    }

    #[test]
    fn repository_paths_cannot_escape_the_github_repository() {
        for repo in [
            "../plugin",
            "owner/..",
            "owner/repo/extra",
            "owner/repo?x=1",
            "owner/",
            "/repo",
        ] {
            let body = serde_json::json!({"items":[{"full_name":repo}]});
            assert!(parse(body).is_err(), "{repo}");
        }
    }

    #[test]
    fn expired_cache_is_refreshed_from_the_configured_url() {
        let directory = Directory::new();
        let body = br#"{"items":[{"full_name":"example/new"}]}"#;
        let (url, server) = serve("200 OK", body.to_vec());
        write_cache(
            &directory.0,
            &CachedCatalog {
                url: url.clone(),
                fetched_at: 0,
                etag: None,
                catalog: serde_json::json!({"items":[{"full_name":"example/old"}]}),
            },
        )
        .unwrap();
        let view = view_from(catalog_url(Some(&url)).unwrap(), &directory.0).unwrap();
        assert_eq!(view.entries[0].repo, "example/old");
        assert!(view.refreshing);
        server.join().unwrap();
        wait_for_refresh(&directory.0, &url);
        assert_eq!(
            load_from(Some(&url), &directory.0).unwrap()[0].repo,
            "example/new"
        );
    }

    #[test]
    fn failed_refresh_keeps_and_uses_the_last_valid_download() {
        let directory = Directory::new();
        let (url, server) = serve("200 OK", b"invalid json".to_vec());
        write_cache(
            &directory.0,
            &CachedCatalog {
                url: url.clone(),
                fetched_at: 0,
                etag: None,
                catalog: serde_json::json!({"items":[{"full_name":"example/cached"}]}),
            },
        )
        .unwrap();
        let before = fs::read(directory.0.join(CACHE_FILE)).unwrap();
        let entries = load_from(Some(&url), &directory.0).unwrap();
        server.join().unwrap();
        wait_for_refresh(&directory.0, &url);
        assert_eq!(entries[0].repo, "example/cached");
        assert_eq!(fs::read(directory.0.join(CACHE_FILE)).unwrap(), before);
        let view = view_from(catalog_url(Some(&url)).unwrap(), &directory.0).unwrap();
        assert!(!view.refreshing);
        assert!(view.warning.is_some());
    }

    #[test]
    fn changing_urls_does_not_reuse_another_catalogs_cache() {
        let directory = Directory::new();
        let (url, server) = serve("404 Not Found", b"[]".to_vec());
        write_cache(
            &directory.0,
            &CachedCatalog {
                url: "https://example.com/previous.json".into(),
                fetched_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                etag: None,
                catalog: serde_json::json!({"items":[{"full_name":"example/old"}]}),
            },
        )
        .unwrap();
        assert!(load_from(Some(&url), &directory.0).is_err());
        server.join().unwrap();
    }

    #[test]
    fn corrupt_cache_is_replaced_with_a_valid_download() {
        let directory = Directory::new();
        fs::write(directory.0.join(CACHE_FILE), b"invalid json").unwrap();
        let (url, server) = serve(
            "200 OK",
            br#"{"items":[{"full_name":"example/restored"}]}"#.to_vec(),
        );
        let entries = load_from(Some(&url), &directory.0).unwrap();
        server.join().unwrap();
        assert_eq!(entries[0].repo, "example/restored");
        assert!(read_cache(&directory.0, &url).is_some());
    }

    #[test]
    fn manual_refresh_revalidates_a_fresh_cache_with_etag() {
        let directory = Directory::new();
        let (url, server) = serve_headers("304 Not Modified", "", Vec::new(), Some("\"v1\""));
        write_cache(
            &directory.0,
            &CachedCatalog {
                url: url.clone(),
                fetched_at: now(),
                etag: Some("\"v1\"".into()),
                catalog: serde_json::json!({"items":[{"full_name":"example/unchanged"}]}),
            },
        )
        .unwrap();
        let view = view_from_mode(catalog_url(Some(&url)).unwrap(), &directory.0, true).unwrap();
        assert!(view.refreshing);
        assert_eq!(view.entries[0].repo, "example/unchanged");
        server.join().unwrap();
        wait_for_refresh(&directory.0, &url);
        let cached = read_cache(&directory.0, &url).unwrap();
        assert!(state(&(directory.0.clone(), url.clone())).warning.is_none());
        assert_eq!(cached.etag.as_deref(), Some("\"v1\""));
        assert_eq!(parse(cached.catalog).unwrap()[0].repo, "example/unchanged");
    }

    #[test]
    fn etag_is_saved_from_a_successful_download() {
        let directory = Directory::new();
        let (url, server) = serve_headers("200 OK", "ETag: \"v2\"\r\n", b"[]".to_vec(), None);
        load_from(Some(&url), &directory.0).unwrap();
        server.join().unwrap();
        assert_eq!(
            read_cache(&directory.0, &url).unwrap().etag.as_deref(),
            Some("\"v2\"")
        );
    }

    #[test]
    fn automatic_cold_cache_retries_back_off_but_manual_refresh_can_retry() {
        let directory = Directory::new();
        let (url, server) = serve("200 OK", b"[]".to_vec());
        finish_refresh(&(directory.0.clone(), url.clone()), true);
        assert!(view_from(catalog_url(Some(&url)).unwrap(), &directory.0).is_err());
        let manual = view_from_mode(catalog_url(Some(&url)).unwrap(), &directory.0, true).unwrap();
        assert!(manual.entries.is_empty());
        assert!(!manual.refreshing);
        server.join().unwrap();
    }

    #[test]
    fn downloaded_github_cache_keeps_only_discovery_metadata() {
        let directory = Directory::new();
        let body = br#"{"items":[{"full_name":"example/plugin","name":"Plugin","description":"Music source","owner":{"login":"example"},"stargazers_count":5}]}"#;
        let (url, server) = serve("200 OK", body.to_vec());
        let rows = load_from(Some(&url), &directory.0).unwrap();
        server.join().unwrap();
        assert_eq!(rows[0].description, "Music source");
        let cached = read_cache(&directory.0, &url).unwrap();
        assert_eq!(cached.catalog["items"][0].as_object().unwrap().len(), 3);
        assert_eq!(cached.catalog["items"][0]["full_name"], "example/plugin");
    }

    #[test]
    fn stale_cache_returns_before_download_finishes_and_only_starts_one_job() {
        use std::sync::mpsc;
        let directory = Directory::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/catalog.json", listener.local_addr().unwrap());
        let (arrived, receive) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buffer = [0; 2048];
            let mut request = Vec::new();
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            arrived.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(3)).unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]")
                .unwrap();
            listener.set_nonblocking(true).unwrap();
            assert!(listener.accept().is_err());
        });
        write_cache(
            &directory.0,
            &CachedCatalog {
                url: url.clone(),
                fetched_at: 0,
                etag: None,
                catalog: serde_json::json!({"items":[{"full_name":"example/stale"}]}),
            },
        )
        .unwrap();
        let view = view_from(catalog_url(Some(&url)).unwrap(), &directory.0).unwrap();
        assert_eq!(view.entries[0].repo, "example/stale");
        receive.recv_timeout(Duration::from_secs(3)).unwrap();
        let repeated = view_from(catalog_url(Some(&url)).unwrap(), &directory.0).unwrap();
        assert!(repeated.refreshing);
        assert_eq!(repeated.entries[0].repo, "example/stale");
        release.send(()).unwrap();
        server.join().unwrap();
        wait_for_refresh(&directory.0, &url);
        assert!(load_from(Some(&url), &directory.0).unwrap().is_empty());
    }

    #[test]
    #[ignore = "queries the live MyGo GitHub catalog"]
    fn live_default_catalog_uses_the_official_topic_search() {
        let directory = Directory::new();
        let entries = load_from(None, &directory.0).unwrap();
        assert!(!entries.is_empty());
        assert!(
            entries
                .iter()
                .all(|row| row.discovered && valid_repository(&row.repo))
        );
        assert!(directory.0.join(CACHE_FILE).is_file());
        println!("official catalog: {} repositories", entries.len());
    }
}
