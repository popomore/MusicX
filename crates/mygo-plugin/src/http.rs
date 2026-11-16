use anyhow::{Result, ensure};
use musicx_core::protocol::tr;
use std::{io::Read, time::Duration};

pub(crate) fn request(
    url: &str,
    etag: Option<&str>,
    accept: Option<&str>,
) -> Result<reqwest::blocking::Response> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(25))
        .build()?;
    let mut request = client.get(url).header("User-Agent", "MusicX");
    if let Some(etag) = etag {
        request = request.header("If-None-Match", etag);
    }
    if let Some(accept) = accept {
        request = request.header("Accept", accept);
    }
    Ok(request.send()?)
}

pub(crate) fn read(response: reqwest::blocking::Response, limit: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    response.take(limit + 1).read_to_end(&mut out)?;
    ensure!(
        out.len() as u64 <= limit,
        tr!("Plugin file exceeds the size limit")
    );
    Ok(out)
}

pub(crate) fn fetch(url: &str, limit: u64) -> Result<Vec<u8>> {
    read(request(url, None, None)?.error_for_status()?, limit)
}
