use crate::protocol::Track;
use url::Url;

pub fn public_url(value: &str) -> Option<String> {
    if value.len() > 2048 {
        return None;
    }
    let mut url = Url::parse(value).ok()?;
    if url.scheme() == "http" {
        url.set_scheme("https").ok()?;
    }
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || !url.domain().is_some_and(|host| {
            host.contains('.') && !host.ends_with(".local") && !host.ends_with(".localhost")
        })
    {
        return None;
    }
    Some(url.to_string())
}

pub fn artwork_url(value: &str) -> Option<String> {
    let value = value
        .replace("{size}", "600")
        .replace("%7Bsize%7D", "600")
        .replace("%7bsize%7d", "600");
    let mut url = Url::parse(&public_url(&value)?).ok()?;
    if url.domain().is_some_and(|host| host.ends_with(".126.net"))
        && !url.query_pairs().any(|(k, _)| k == "param")
    {
        url.query_pairs_mut().append_pair("param", "512y512");
    }
    Some(url.to_string())
}

pub fn merge_metadata(track: &mut Track, metadata: &Track) -> bool {
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
