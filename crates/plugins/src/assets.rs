//! Credentials stay in the helper. Only public source URLs enter Track metadata.
use crate::mygo;
use anyhow::{Context, Result, ensure};
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
    ensure!(mygo::valid_id(plugin), "无效的数据源标识");
    let m = mygo::manifest(&mygo::plugin_root()?.join(plugin))?;
    let read = |path| -> Value {
        fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(json!({}))
    };
    let mut settings = read(mygo::plugin_root()?.join(plugin).join("settings.json"));
    for field in &m.settings {
        if let Some(value) = &field.value {
            settings
                .as_object_mut()
                .context("来源设置格式无效")?
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
                let host = Url::parse(&host)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .unwrap_or(host);
                host != "*"
                    && mygo::permitted(url, &[host])
                    && mygo::permitted(url, &m.permissions.network)
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
                    "来源请求头不受支持"
                );
                reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .context("来源请求头名称无效")?;
                reqwest::header::HeaderValue::from_str(&value).context("来源请求头内容无效")?;
                result.insert(name.to_ascii_lowercase(), value);
            }
        }
    }
    Ok(result)
}

pub(crate) fn artwork(plugin: &str, value: &str) -> Option<String> {
    let mut url = Url::parse(&metadata::artwork_url(value)?).ok()?;
    if headers(plugin, &url, "image").is_ok_and(|headers| !headers.is_empty()) {
        url.set_fragment(Some(&format!("musicx-plugin={plugin}")));
    }
    Some(url.into())
}

pub fn image(plugin: &str, value: &str) -> Result<Vec<u8>> {
    let mut url = Url::parse(&metadata::artwork_url(value).context("无效的封面地址")?)?;
    url.set_fragment(None);
    let headers = headers(plugin, &url, "image")?;
    let host = url.host_str().unwrap_or_default().to_owned();
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(Policy::custom(move |r| {
            if r.previous().len() < 4
                && r.url().host_str() == Some(&host)
                && valid_redirect_target(r.url())
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
        .context("封面下载失败")?
        .error_for_status()
        .context("来源拒绝封面请求")?;
    let mut bytes = Vec::new();
    response.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 4 * 1024 * 1024, "封面超过4MiB");
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
