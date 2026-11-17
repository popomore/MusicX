use super::{MAX_BYTES, merge::Snapshot};
use crate::protocol::{
    self, HistoryEntry, LocalPlaylist, PlaylistOrigin, Quality, Track, track_key,
};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

const SETTINGS: [&str; 6] = [
    "language",
    "audio-quality.json",
    "normalization.json",
    "panel-preferences.json",
    "update-channel",
    "plugin-auto-update.json",
];
fn encode(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(value)
}

fn portable_track(track: &Track) -> Track {
    let mut track = track.clone();
    if track
        .cover_url
        .as_deref()
        .is_some_and(|value| url::Url::parse(value).is_ok_and(|url| url.scheme() == "file"))
    {
        track.cover_url = None;
    }
    track
}

pub fn capture(
    playlists: &[LocalPlaylist],
    history: &BTreeMap<String, HistoryEntry>,
    dir: &Path,
) -> Result<Snapshot> {
    let mut result = Snapshot::new();
    let favorite = playlists
        .iter()
        .position(|p| p.favorite || p.id == "liked")
        .or_else(|| playlists.iter().position(|p| p.name == "我喜欢"));
    for (index, list) in playlists.iter().enumerate() {
        let id = if favorite == Some(index) {
            "liked"
        } else {
            &list.id
        };
        let prefix = format!("p/{}/", encode(id));
        result.insert(
            format!("{prefix}meta"),
            json!({"id":id,"favorite":id == "liked","origin":list.imported_from}),
        );
        result.insert(format!("{prefix}name"), json!(list.name));
        result.insert(format!("{prefix}pin"), json!(list.pinned && id != "liked"));
        result.insert(
            format!("{prefix}order"),
            json!(list.tracks.iter().map(track_key).collect::<Vec<_>>()),
        );
        for track in &list.tracks {
            result.insert(
                format!("{prefix}t/{}", encode(&track_key(track))),
                serde_json::to_value(portable_track(track))?,
            );
        }
    }
    for (id, entry) in history {
        let mut entry = entry.clone();
        entry.track = portable_track(&entry.track);
        result.insert(format!("h/{id}"), serde_json::to_value(entry)?);
    }
    for name in SETTINGS {
        let path = dir.join(name);
        let value = match fs::read_to_string(&path) {
            Ok(text) => {
                ensure!(text.len() < 4096, "Invalid application preference");
                if name == "language" || name == "update-channel" {
                    json!(text.trim())
                } else {
                    serde_json::from_str(&text).context("Could not read application preference")?
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if name == "plugin-auto-update.json" {
                    if let Some(legacy) =
                        super::read_json::<Value>(&dir.join("plugin-updates.json"))?
                    {
                        let enabled = legacy
                            .get("enabled")
                            .and_then(Value::as_bool)
                            .context("Invalid plugin update preference")?;
                        result.insert(format!("s/{name}"), json!(enabled));
                    }
                }
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        result.insert(format!("s/{name}"), value);
    }
    validate_settings(&result)?;
    Ok(result)
}

pub fn playlists(snapshot: &Snapshot, previous: &[LocalPlaylist]) -> Result<Vec<LocalPlaylist>> {
    let mut result = Vec::new();
    for (key, meta) in snapshot
        .iter()
        .filter(|(key, _)| key.starts_with("p/") && key.ends_with("/meta"))
    {
        let prefix = key.strip_suffix("meta").unwrap();
        let id = meta["id"]
            .as_str()
            .context("Invalid synced playlist identifier")?;
        ensure!(
            prefix == format!("p/{}/", encode(id)) && !id.is_empty() && id.len() <= 256,
            "Invalid synced playlist identifier"
        );
        let favorite = meta["favorite"]
            .as_bool()
            .context("Invalid synced favorite flag")?;
        ensure!(
            favorite == (id == "liked"),
            "Invalid synced favorite playlist"
        );
        let name = snapshot
            .get(&format!("{prefix}name"))
            .and_then(Value::as_str)
            .context("Missing synced playlist name")?
            .to_owned();
        let pinned = snapshot
            .get(&format!("{prefix}pin"))
            .and_then(Value::as_bool)
            .context("Missing synced playlist pin")?;
        let imported_from: Option<PlaylistOrigin> = serde_json::from_value(meta["origin"].clone())?;
        let order: Vec<String> = serde_json::from_value(
            snapshot
                .get(&format!("{prefix}order"))
                .context("Missing synced playlist order")?
                .clone(),
        )?;
        let mut tracks: BTreeMap<String, Track> = BTreeMap::new();
        let track_prefix = format!("{prefix}t/");
        for (key, value) in snapshot
            .range(track_prefix.clone()..)
            .take_while(|(key, _)| key.starts_with(&track_prefix))
        {
            let track: Track = portable_track(&serde_json::from_value(value.clone())?);
            let identity = track_key(&track);
            ensure!(
                *key == format!("{track_prefix}{}", encode(&identity)),
                "Invalid synced track identifier"
            );
            tracks.insert(identity, track);
        }
        let mut ordered = Vec::new();
        for id in order {
            if let Some(track) = tracks.remove(&id) {
                ordered.push(track);
            }
        }
        // Concurrent additions omitted by the winning order are retained.
        ordered.extend(tracks.into_values());
        let mut list = LocalPlaylist {
            id: id.into(),
            name,
            favorite,
            pinned: pinned && !favorite,
            revision: 0,
            tracks: ordered,
            imported_from,
        };
        if let Some(old) = previous
            .iter()
            .find(|p| p.id == id || (favorite && p.favorite))
        {
            // Keep this device's own usable artwork while omitting its path
            // from the portable replica sent to other devices.
            for track in &mut list.tracks {
                if track.cover_url.is_none()
                    && let Some(cover) = old
                        .tracks
                        .iter()
                        .find(|old| track_key(old) == track_key(track))
                        .and_then(|old| old.cover_url.as_ref())
                    && url::Url::parse(cover)
                        .ok()
                        .filter(|url| url.scheme() == "file")
                        .and_then(|url| url.to_file_path().ok())
                        .is_some_and(|path| path.is_file())
                {
                    track.cover_url = Some(cover.clone());
                }
            }
            list.revision = old.revision;
            let mut comparable = old.clone();
            comparable.id = id.into();
            if list != comparable {
                list.revision = list.revision.wrapping_add(1);
            }
        } else {
            list.revision = 1;
        }
        result.push(list);
    }
    // Keep local collection order; deterministic IDs order newly received lists.
    result.sort_by_key(|list| {
        previous
            .iter()
            .position(|p| p.id == list.id || (p.favorite && list.favorite))
            .unwrap_or(usize::MAX)
    });
    Ok(result)
}

pub fn history(snapshot: &Snapshot) -> Result<BTreeMap<String, HistoryEntry>> {
    let mut result = BTreeMap::new();
    for (key, value) in snapshot.iter().filter(|(key, _)| key.starts_with("h/")) {
        let id = &key[2..];
        let (device, offset) = id
            .split_once('/')
            .context("Invalid synced history identifier")?;
        ensure!(
            super::merge::valid_device(device) && offset.parse::<u64>().is_ok(),
            "Invalid synced history identifier"
        );
        let mut entry: HistoryEntry = serde_json::from_value(value.clone())?;
        entry.track = portable_track(&entry.track);
        crate::local_library::validate_track(&entry.track)?;
        if let Some(progress) = entry.progress {
            ensure!(
                progress.position.is_finite()
                    && progress.duration.is_finite()
                    && progress.position >= 0.0
                    && progress.duration >= 0.0,
                "Invalid synced listening progress"
            );
        }
        result.insert(id.to_owned(), entry);
    }
    ensure!(
        serde_json::to_vec(&result)?.len() as u64 <= MAX_BYTES,
        "Synced history is too large"
    );
    Ok(result)
}

pub fn validate_settings(snapshot: &Snapshot) -> Result<()> {
    for (key, value) in snapshot.iter().filter(|(key, _)| key.starts_with("s/")) {
        let valid = match &key[2..] {
            "language" => matches!(value.as_str(), Some("system" | "en" | "zh-CN")),
            "update-channel" => matches!(value.as_str(), Some("stable" | "all")),
            "audio-quality.json" => serde_json::from_value::<Quality>(value.clone()).is_ok(),
            "normalization.json" | "panel-preferences.json" | "plugin-auto-update.json" => {
                value.is_boolean()
            }
            _ => false,
        };
        ensure!(valid, "Invalid synced application preference");
    }
    Ok(())
}

pub fn apply_settings(snapshot: &Snapshot, dir: &Path) -> Result<()> {
    validate_settings(snapshot)?;
    for name in SETTINGS {
        let Some(value) = snapshot.get(&format!("s/{name}")) else {
            match fs::remove_file(dir.join(name)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            continue;
        };
        let bytes = if name == "language" || name == "update-channel" {
            format!("{}\n", value.as_str().unwrap()).into_bytes()
        } else {
            serde_json::to_vec(value)?
        };
        if fs::read(dir.join(name)).ok().as_deref() != Some(bytes.as_slice()) {
            protocol::preferences::write_locked(dir, name, &bytes)?;
        }
    }
    Ok(())
}

pub fn local_view(snapshot: &Snapshot, device: &str) -> Snapshot {
    let own = format!("h/{device}/");
    snapshot
        .iter()
        .filter(|(key, _)| !key.starts_with("h/") || key.starts_with(&own))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
