//! Fetch authenticated plugin artwork through the helper, without exposing headers.
use base64::Engine;
use nsh_core::image::{
    Fetched,
    loader::{Get, Transport, redirect, set_transport},
};
use reqwest::{Url, blocking::Client, redirect::Policy};
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

struct Images;
pub fn install() {
    set_transport(Arc::new(Images));
}
impl Transport for Images {
    fn get(&self, get: Get<'_>) -> Result<Fetched, String> {
        let mut run = || -> anyhow::Result<Fetched> {
            let mut url = Url::parse(get.url)?;
            if let Some(plugin) = url
                .fragment()
                .and_then(|f| f.strip_prefix("musicx-plugin="))
                .map(str::to_owned)
            {
                url.set_fragment(None);
                let result: Value = crate::remote::post_json(
                    "/ui/music-sources",
                    &json!({"action":"artwork","source":format!("mygo:{plugin}"),"url":url.as_str()}),
                )?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(result["data"].as_str().unwrap_or_default())?;
                anyhow::ensure!(bytes.len() <= 4 * 1024 * 1024, "封面过大");
                return Ok(Fetched::new(200, Default::default(), bytes));
            }
            let client = Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(Policy::none())
                .build()?;
            for _ in 0..5 {
                anyhow::ensure!(!get.cancelled.load(Ordering::Relaxed), "封面请求已取消");
                let mut request = client.get(url.clone());
                for (name, value) in get.headers {
                    request = request.header(*name, value);
                }
                let response = request.send()?;
                if response.status().is_redirection() && response.status().as_u16() != 304 {
                    let next = response
                        .headers()
                        .get("location")
                        .and_then(|v| v.to_str().ok())
                        .ok_or_else(|| anyhow::anyhow!("封面跳转缺少地址"))?;
                    url = Url::parse(
                        &redirect(get.access, url.as_str(), next).map_err(anyhow::Error::msg)?,
                    )?;
                    continue;
                }
                let status = response.status().as_u16();
                let mut bytes = Vec::new();
                response.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                anyhow::ensure!(bytes.len() <= 4 * 1024 * 1024, "封面过大");
                (get.progress)(bytes.len() as u64, bytes.len() as u64);
                return Ok(Fetched::new(status, Default::default(), bytes));
            }
            anyhow::bail!("封面跳转过多")
        };
        run().map_err(|_| "封面下载失败".into())
    }
}
