//! Read-only provider browsing and atomic local imports.
use crate::{mygo, protocol::*};
use anyhow::{Context, Result, ensure};
use musicx_core::local_library;
use musicx_core::protocol::tr;
use serde_json::{Value, json};
use std::collections::HashSet;

fn source_id(source: &str) -> Result<&str> {
    let id = source
        .strip_prefix("mygo:")
        .context(tr!("Choose a plugin source"))?;
    ensure!(mygo::valid_id(id), tr!("Invalid source identifier"));
    Ok(id)
}

fn bounded_id(id: &str) -> Result<&str> {
    ensure!(
        !id.is_empty() && id.len() <= 1024,
        tr!("Invalid source item identifier")
    );
    Ok(id)
}

fn tracks(plugin: &str, rows: &[Value]) -> Result<Vec<Track>> {
    ensure!(
        rows.len() <= MAX_QUEUE_TRACKS,
        tr!("Source exceeds 10,000 tracks")
    );
    rows.iter()
        .map(|v| {
            mygo::catalog_track(plugin, v).context(tr!("Source track information is incomplete"))
        })
        .collect()
}

fn more(v: &Value) -> bool {
    v["more"]
        .as_bool()
        .or_else(|| v["hasMore"].as_bool())
        .unwrap_or(false)
}

pub fn import(source: &str, liked: bool) -> Result<Value> {
    let plugin = source_id(source)?;
    let export = if liked { "liked" } else { "recent" };
    let mut all = Vec::new();
    let mut seen = HashSet::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    // Fetch the complete snapshot before touching the local library. A broken
    // pagination contract must not silently save the first page as complete.
    loop {
        ensure!(
            std::time::Instant::now() < deadline,
            tr!("Source import timed out; no incomplete copy was saved")
        );
        let value = mygo::invoke(plugin, export, json!({"offset":all.len()}))?;
        ensure!(
            std::time::Instant::now() < deadline,
            tr!("Source import timed out; no incomplete copy was saved")
        );
        let rows = if liked {
            value["tracks"].as_array()
        } else {
            value.as_array().or_else(|| value["tracks"].as_array())
        }
        .context(tr!("The source returned no track list"))?;
        let page = tracks(plugin, rows)?;
        ensure!(
            all.len() + page.len() <= MAX_QUEUE_TRACKS,
            tr!("Import exceeds 10,000 tracks; no incomplete copy was saved")
        );
        let continuing = liked && more(&value);
        if continuing {
            ensure!(
                !page.is_empty(),
                tr!("Source pagination did not advance; nothing was imported")
            );
            let fingerprint = serde_json::to_string(rows)?;
            ensure!(
                seen.insert(fingerprint),
                tr!("The source repeated the same page; nothing was imported")
            );
        }
        all.extend(page);
        if !continuing {
            break;
        }
    }
    let label = if liked {
        tr!("Cloud likes")
    } else {
        tr!("Cloud history")
    };
    local_library::import(
        PlaylistOrigin {
            provider: source.into(),
            id: format!("collection:{export}"),
            name: format!("{} · {label}", mygo::source_name(plugin)),
        },
        all,
    )
}

fn collection(plugin: &str, kind: &str, v: &Value) -> Option<SourceCollection> {
    Some(SourceCollection {
        kind: kind.into(),
        id: v["id"].as_str()?.into(),
        name: v["name"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(512)
            .collect(),
        description: v["description"]
            .as_str()
            .or_else(|| v["publisher"].as_str())
            .unwrap_or_default()
            .chars()
            .take(4000)
            .collect(),
        cover_url: v["imageUrl"]
            .as_str()
            .and_then(|url| crate::assets::artwork(plugin, url)),
    })
}

/// Flatten provider sections into the shared native browser without exposing helpers.
pub fn page(source: &str, method: &str, params: Value) -> Result<SourcePage> {
    ensure!(
        matches!(
            method,
            "home" | "browse" | "stations" | "stationNext" | "playlist"
        ),
        tr!("This is not a public source capability")
    );
    let plugin = source_id(source)?;
    let value = if method == "playlist" {
        let id = params["id"]
            .as_str()
            .context(tr!("Invalid source item identifier"))?;
        bounded_id(id)?;
        let value = mygo::invoke(plugin, method, json!({"id":id}))?;
        return Ok(SourcePage {
            id: id.into(),
            title: value["name"]
                .as_str()
                .unwrap_or(id)
                .chars()
                .take(512)
                .collect(),
            tracks: tracks(
                plugin,
                value["tracks"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            )?,
            ..Default::default()
        });
    } else {
        crate::capabilities::invoke(source, method, params.clone())?
    };
    let title = value["title"]
        .as_str()
        .or_else(|| value["name"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| match method {
            "stations" => tr!("Radio").into(),
            "stationNext" => params["name"]
                .as_str()
                .or_else(|| params["id"].as_str())
                .unwrap_or_default()
                .into(),
            _ => tr!("Recommendations").into(),
        });
    let mut result = SourcePage {
        id: params["id"]
            .as_str()
            .or_else(|| params["page"].as_str())
            .unwrap_or(method)
            .into(),
        title: title.chars().take(512).collect(),
        description: value["description"]
            .as_str()
            .or_else(|| value["subtitle"].as_str())
            .unwrap_or_default()
            .chars()
            .take(4000)
            .collect(),
        ..Default::default()
    };
    if method == "stationNext" {
        result.tracks = serde_json::from_value(
            value
                .as_array()
                .cloned()
                .map(Value::Array)
                .unwrap_or(value["tracks"].clone()),
        )?;
        result.station = Some(SourceStation {
            source: source.into(),
            id: result.id.clone(),
            name: result.title.clone(),
        });
    } else if method == "stations" {
        result.collections = value
            .as_array()
            .or_else(|| value["stations"].as_array())
            .into_iter()
            .flatten()
            .take(1000)
            .filter_map(|v| collection(plugin, "station", v))
            .collect();
    } else {
        fn section(plugin: &str, value: &Value, page: &mut SourcePage, depth: usize) -> Result<()> {
            ensure!(depth < 32, tr!("Source response is nested too deeply"));
            if let Some(rows) = value["tracks"].as_array() {
                for row in rows {
                    page.tracks.push(serde_json::from_value(row.clone())?);
                }
            }
            if let Some(links) = value["links"].as_array() {
                for link in links {
                    if let Some(id) = link["page"]
                        .as_str()
                        .filter(|s| !s.is_empty() && s.len() <= 1024)
                    {
                        page.collections.push(SourceCollection {
                            kind: "page".into(),
                            id: id.into(),
                            name: link["title"]
                                .as_str()
                                .unwrap_or(id)
                                .chars()
                                .take(512)
                                .collect(),
                            description: link["subtitle"]
                                .as_str()
                                .unwrap_or_default()
                                .chars()
                                .take(4000)
                                .collect(),
                            cover_url: link["imageUrl"]
                                .as_str()
                                .and_then(|url| crate::assets::artwork(plugin, url)),
                        });
                    }
                }
            }
            if let Some(id) = value["morePage"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 1024)
            {
                page.collections.push(SourceCollection {
                    kind: "page".into(),
                    id: id.into(),
                    name: tr!("More {}", value["title"].as_str().unwrap_or_default()),
                    ..Default::default()
                });
            }
            for (key, kind) in [
                ("playlists", "playlist"),
                ("albums", "album"),
                ("artists", "artist"),
                ("shows", "show"),
                ("stations", "station"),
            ] {
                if let Some(rows) = value[key].as_array() {
                    page.collections
                        .extend(rows.iter().filter_map(|v| collection(plugin, kind, v)));
                }
            }
            if let Some(rows) = value["sections"].as_array() {
                for row in rows {
                    section(plugin, row, page, depth + 1)?;
                }
            }
            ensure!(
                page.tracks.len() <= MAX_QUEUE_TRACKS && page.collections.len() <= 1000,
                tr!("Source exceeds 10,000 tracks")
            );
            Ok(())
        }
        section(plugin, &value, &mut result, 0)?;
    }
    ensure!(
        result.tracks.len() <= MAX_QUEUE_TRACKS,
        tr!("Source exceeds 10,000 tracks")
    );
    Ok(result)
}

pub fn browse(source: &str, kind: &str, id: &str, offset: u64) -> Result<SourcePage> {
    let plugin = source_id(source)?;
    ensure!(offset <= MAX_QUEUE_TRACKS as u64, tr!("Page out of range"));
    if kind != "shows" {
        bounded_id(id)?;
    }
    let value = mygo::invoke(plugin, kind, json!({"id":id,"offset":offset}))?;
    if kind == "shows" {
        let rows = value
            .as_array()
            .or_else(|| value["shows"].as_array())
            .context(tr!("The source returned no podcast list"))?;
        ensure!(rows.len() <= 1000, tr!("Too many source podcasts"));
        return Ok(SourcePage {
            id: "shows".into(),
            title: tr!("Podcasts").into(),
            collections: rows
                .iter()
                .filter_map(|v| collection(plugin, "show", v))
                .collect(),
            ..Default::default()
        });
    }
    let descriptor = if value[kind].is_object() {
        &value[kind]
    } else {
        &value
    };
    let rows = if kind == "show" {
        value["episodes"].as_array()
    } else {
        value["tracks"].as_array()
    };
    let rows = rows.map(Vec::as_slice).unwrap_or_default();
    ensure!(
        !more(&value) || !rows.is_empty(),
        tr!("Source pagination did not advance")
    );
    Ok(SourcePage {
        id: id.into(),
        title: descriptor["name"]
            .as_str()
            .unwrap_or(id)
            .chars()
            .take(512)
            .collect(),
        description: descriptor["description"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(4000)
            .collect(),
        cover_url: descriptor["imageUrl"]
            .as_str()
            .and_then(|url| crate::assets::artwork(plugin, url)),
        tracks: tracks(plugin, rows)?,
        collections: value["albums"]
            .as_array()
            .into_iter()
            .flatten()
            .take(1000)
            .filter_map(|v| collection(plugin, "album", v))
            .collect(),
        more: more(&value),
        next_offset: offset + rows.len() as u64,
        station: None,
    })
}
