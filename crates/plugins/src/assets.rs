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
                let Some(host) = replacement(host, &settings, &stored) else {
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
                host_matches && permitted(url, &grants)
            });
        if !matching {
            continue;
        }
        for (name, value) in rule["headers"].as_object().into_iter().flatten() {
            if let Some(value) = value
                .as_str()
                .and_then(|s| replacement(s, &settings, &stored))
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

fn artwork_base(plugin: &str, value: &str) -> Option<Url> {
    if value.len() > 2048 {
        return None;
    }
    let value = value
        .replace("{size}", "600")
        .replace("%7Bsize%7D", "600")
        .replace("%7bsize%7d", "600");
    let url = Url::parse(&value).ok()?;
    if url.scheme() == "file" {
        return crate::plugin_files::owned_file(plugin, &url).map(|_| url);
    }
    if crate::network::for_plugin(plugin)
        .ok()
        .is_some_and(|grants| crate::network::explicit(&url, &grants))
    {
        Some(url)
    } else {
        Url::parse(&metadata::artwork_url(&value)?).ok()
    }
}

pub(crate) fn artwork(plugin: &str, value: &str) -> Option<String> {
    let mut url = artwork_base(plugin, value)?;
    if url.scheme() == "file"
        || crate::network::for_plugin(plugin)
            .ok()
            .is_some_and(|grants| crate::network::explicit(&url, &grants))
        || headers(plugin, &url, "image").is_ok_and(|headers| !headers.is_empty())
    {
        url.set_fragment(Some(&format!("musicx-plugin={plugin}")));
    }
    Some(url.into())
}

pub fn image(plugin: &str, value: &str) -> Result<Vec<u8>> {
    let mut url = artwork_base(plugin, value).context(tr!("Invalid artwork URL"))?;
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
    let headers = headers(plugin, &url, "image")?;
    let host = url.host_str().unwrap_or_default().to_owned();
    let origin = url.origin();
    let grants = crate::network::for_plugin(plugin)?;
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
