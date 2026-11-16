//! Credentials stay in the helper. Only public source URLs enter Track metadata.
use crate::mygo;
use anyhow::{Context, Result, ensure};
use musicx_core::protocol::tr;
use musicx_core::{metadata, storage};
use reqwest::{Url, blocking::Client, redirect::Policy};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Read, time::Duration};

pub(crate) fn valid_redirect_target(url: &Url) -> bool {
    url.scheme() == "https" && metadata::public_url(url.as_str()).is_some()
}

fn replacement(value: &str, settings: &Value, stored: &Value) -> Option<String> {
    let mut out = value.to_owned();
    for (prefix, values) in [("setting", settings), ("storage", stored)] {
        let marker = format!("{{{prefix}:");
        while let Some(start) = out.find(&marker) {
            let end = out[start..].find('}')? + start;
            let key = &out[start + marker.len()..end];
            let value = values[key].as_str().filter(|s| !s.is_empty())?;
            // Values are data, never another template to evaluate.
            if value.contains('{') || value.contains(['\r', '\n']) {
                return None;
            }
            out.replace_range(start..=end, value);
        }
    }
    Some(out)
}

pub fn headers(plugin: &str, url: &Url, kind: &str) -> Result<BTreeMap<String, String>> {
    RequestContext::new(plugin)?.headers(url, kind)
}

struct RequestContext {
    manifest: mygo::Manifest,
    settings: Value,
    stored: Value,
    grants: Vec<String>,
}
impl RequestContext {
    fn new(plugin: &str) -> Result<Self> {
        ensure!(mygo::valid_id(plugin), tr!("Invalid source identifier"));
        let m = mygo::manifest(&mygo::plugin_root()?.join(plugin))?;
        let read = |path| -> Value {
            fs::read(path)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(json!({}))
        };
        let mut settings = read(mygo::plugin_root()?.join(plugin).join("settings.json"));
        let grants = crate::network::for_manifest(&m, &settings);
        for field in &m.settings {
            if let Some(value) = &field.value {
                settings
                    .as_object_mut()
                    .context(tr!("Invalid source settings format"))?
                    .entry(&field.key)
                    .or_insert(json!(value));
            }
        }
        let stored = read(storage::state_dir()?.join(format!("mygo-{plugin}-storage.json")));
        Ok(Self {
            manifest: m,
            settings,
            stored,
            grants,
        })
    }
    fn headers(&self, url: &Url, kind: &str) -> Result<BTreeMap<String, String>> {
        let m = &self.manifest;
        let settings = &self.settings;
        let stored = &self.stored;
        let grants = &self.grants;
        let mut result = BTreeMap::new();
        for rule in m
            .metadata
            .get("requestHeaders")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if rule["kinds"]
                .as_array()
                .is_some_and(|k| !k.iter().any(|v| v.as_str() == Some(kind)))
            {
                continue;
            }
            let matching = rule["hosts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|host| {
                    let Some(host) = replacement(host, settings, stored) else {
                        return false;
                    };
                    let host_matches = match Url::parse(&host) {
                        Ok(rule_url) => {
                            rule_url.username().is_empty()
                                && rule_url.password().is_none()
                                && rule_url.origin() == url.origin()
                        }
                        Err(_) => {
                            // Existing host-only rules were HTTPS-only before HTTP audio support.
                            if url.scheme() != "https" {
                                return false;
                            }
                            mygo::permitted_audio(url, &[host])
                        }
                    };
                    let permitted = if kind == "audio" {
                        mygo::permitted_audio
                    } else {
                        mygo::permitted
                    };
                    host_matches && permitted(url, grants)
                });
            if !matching {
                continue;
            }
            for (name, value) in rule["headers"].as_object().into_iter().flatten() {
                if let Some(value) = value
                    .as_str()
                    .and_then(|s| replacement(s, settings, stored))
                {
                    ensure!(
                        !matches!(
                            name.to_ascii_lowercase().as_str(),
                            "host"
                                | "connection"
                                | "content-length"
                                | "transfer-encoding"
                                | "proxy-authorization"
                        ),
                        tr!("Unsupported source request headers")
                    );
                    reqwest::header::HeaderName::from_bytes(name.as_bytes())
                        .context(tr!("Invalid source request header name"))?;
                    reqwest::header::HeaderValue::from_str(&value)
                        .context(tr!("Invalid source request header value"))?;
                    result.insert(name.to_ascii_lowercase(), value);
                }
            }
        }
        Ok(result)
    }
}

/// A response owns this snapshot. Actual image requests validate current grants
/// again; nothing here caches authorization across source responses.
pub(crate) struct Artwork<'a> {
    plugin: &'a str,
    context: Option<RequestContext>,
}
impl<'a> Artwork<'a> {
    pub(crate) fn new(plugin: &'a str) -> Self {
        Self {
            plugin,
            context: RequestContext::new(plugin).ok(),
        }
    }
    fn explicit(&self, url: &Url) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| crate::network::explicit(url, &c.grants))
    }
    fn base(&self, value: &str) -> Option<Url> {
        if value.len() > 2048 {
            return None;
        }
        let value = value
            .replace("{size}", "600")
            .replace("%7Bsize%7D", "600")
            .replace("%7bsize%7d", "600");
        let url = Url::parse(&value).ok()?;
        if url.scheme() == "file" {
            return crate::plugin_files::owned_file(self.plugin, &url).map(|_| url);
        }
        if self.explicit(&url) {
            Some(url)
        } else {
            Url::parse(&metadata::artwork_url(&value)?).ok()
        }
    }

    pub(crate) fn url(&self, value: &str) -> Option<String> {
        let mut url = self.base(value)?;
        if url.scheme() == "file"
            || self.explicit(&url)
            || self
                .context
                .as_ref()
                .is_some_and(|c| c.headers(&url, "image").is_ok_and(|h| !h.is_empty()))
        {
            url.set_fragment(Some(&format!("musicx-plugin={}", self.plugin)));
        }
        Some(url.into())
    }
}

pub fn image(plugin: &str, value: &str) -> Result<Vec<u8>> {
    let artwork = Artwork::new(plugin);
    let mut url = artwork.base(value).context(tr!("Invalid artwork URL"))?;
    url.set_fragment(None);
    if url.scheme() == "file" {
        let path =
            crate::plugin_files::owned_file(plugin, &url).context(tr!("Invalid artwork URL"))?;
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 4 * 1024 * 1024, tr!("Artwork exceeds 4 MiB"));
        return Ok(bytes);
    }
    let context = artwork.context.context(tr!("Invalid artwork URL"))?;
    let headers = context.headers(&url, "image")?;
    let host = url.host_str().unwrap_or_default().to_owned();
    let origin = url.origin();
    let grants = context.grants;
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(Policy::custom(move |r| {
            if r.previous().len() < 4
                && r.url().host_str() == Some(&host)
                && r.url().origin() == origin
                && (valid_redirect_target(r.url()) || crate::network::explicit(r.url(), &grants))
            {
                r.follow()
            } else {
                r.stop()
            }
        }))
        .build()?;
    let mut request = client.get(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request
        .send()
        .context(tr!("Artwork download failed"))?
        .error_for_status()
        .context(tr!("The source rejected the artwork request"))?;
    let mut bytes = Vec::new();
    response.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 4 * 1024 * 1024, tr!("Artwork exceeds 4 MiB"));
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artwork_permission_checks_are_bounded_per_response() {
        use musicx_core::protocol::*;
        use musicx_plugin_api::PluginSource;
        use std::process::Command;
        if std::env::var_os("MUSICX_ARTWORK_SNAPSHOT_CHILD").is_none() {
            let root = std::env::temp_dir()
                .join(format!("musicx-artwork-snapshot-{}", std::process::id()));
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "assets::tests::artwork_permission_checks_are_bounded_per_response",
                    "--nocapture",
                ])
                .env("MUSICX_ARTWORK_SNAPSHOT_CHILD", "1")
                .env("MUSICX_DATA_DIR", &root)
                .output()
                .unwrap();
            if root.exists() {
                fs::remove_dir_all(root).unwrap();
            }
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let root = storage::state_dir().unwrap();
        let fixture = root.join("fixture");
        fs::create_dir_all(&fixture).unwrap();
        fs::write(
            fixture.join("plugin.json"),
            r#"{"id":"artwork.fixture","permissions":{"network":["*"]}}"#,
        )
        .unwrap();
        fs::write(fixture.join("asset.bin"), vec![0u8; 128 * 1024]).unwrap();
        fs::write(fixture.join("index.js"), r#"
const imageUrl='http://127.0.0.1:4533/cover';
const tracks=Array.from({length:100},(_,i)=>({id:''+i,name:'Track',duration:1000,imageUrl,artists:[{id:'artist',name:'Artist',imageUrl}]}));
export function search(){return {tracks}}
export function playlist(){return {tracks,name:'Playlist'}}
export function home(){return {sections:[{tracks,albums:tracks}]}}
"#).unwrap();
        mygo::install(&fixture).unwrap();
        let source = &crate::MYGO;
        let descriptor = source.source("mygo:artwork.fixture").unwrap();
        source
            .network_permission(
                "mygo:artwork.fixture",
                true,
                descriptor.permission_fingerprint.as_deref(),
            )
            .unwrap();
        for request in ["search", "playlist", "page"] {
            let before = crate::plugin_files::hash_count();
            let tracks = match request {
                "search" => source
                    .search(
                        "mygo:artwork.fixture",
                        SourceSearchRequest {
                            query: "fixture".into(),
                            kind: SourceSearchKind::Tracks,
                            offset: 0,
                        },
                    )
                    .unwrap()
                    .tracks
                    .into_iter()
                    .map(|t| t.track)
                    .collect::<Vec<_>>(),
                "playlist" => {
                    source
                        .playlist("mygo:artwork.fixture", "list")
                        .unwrap()
                        .tracks
                }
                _ => {
                    source
                        .page(
                            "mygo:artwork.fixture",
                            SourcePageRequest::Recommendations {},
                        )
                        .unwrap()
                        .tracks
                }
            };
            assert_eq!(tracks.len(), 100);
            assert!(
                tracks[0]
                    .cover_url
                    .as_ref()
                    .is_some_and(|s| s.contains("musicx-plugin=artwork.fixture"))
            );
            let checks = crate::plugin_files::hash_count() - before;
            assert!(
                checks <= 12,
                "{request} rehashed the payload {checks} times for one response"
            );
        }
        fs::write(
            root.join("mygo-plugins/artwork.fixture/asset.bin"),
            "changed",
        )
        .unwrap();
        let tracks = source
            .playlist("mygo:artwork.fixture", "list")
            .unwrap()
            .tracks;
        assert!(
            tracks[0].cover_url.is_none(),
            "a new response must revalidate changed payload permissions"
        );
    }
    #[test]
    fn private_requests_reject_https_downgrades() {
        assert!(!valid_redirect_target(
            &"http://example.com/audio".parse().unwrap()
        ));
        assert!(!valid_redirect_target(
            &"https://example.com:8443/audio".parse().unwrap()
        ));
        assert!(valid_redirect_target(
            &"https://example.com/audio".parse().unwrap()
        ));
    }
}
