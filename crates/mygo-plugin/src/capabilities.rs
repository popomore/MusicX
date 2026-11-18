//! Internal MyGo adapter operations for native MusicX pages, never raw RPC endpoints.
use crate::{music_sources, mygo};
use anyhow::{Context, Result, ensure};
use musicx_core::protocol::tr;
use serde_json::{Value, json};

const ACTIONS: &[&str] = &["home", "browse", "stations", "stationNext"];

pub(crate) fn supported(exports: &[String]) -> Vec<musicx_core::protocol::SourceCapability> {
    use musicx_core::protocol::SourceCapability;
    let mut capabilities: Vec<_> = exports
        .iter()
        .filter_map(|export| {
            Some(match export.as_str() {
                "search" => SourceCapability::Search,
                "stream" => SourceCapability::Playback,
                "lyrics" => SourceCapability::Lyrics,
                "album" => SourceCapability::Album,
                "artist" => SourceCapability::Artist,
                "account" => SourceCapability::Account,
                "signIn" => SourceCapability::SignIn,
                "signOut" => SourceCapability::SignOut,
                "setSetting" => SourceCapability::Settings,
                "playlists" => SourceCapability::Playlists,
                "playlist" => SourceCapability::Playlist,
                "liked" => SourceCapability::LikedTracks,
                "recent" => SourceCapability::RecentTracks,
                "shows" => SourceCapability::Podcasts,
                "show" => SourceCapability::PodcastEpisodes,
                "home" => SourceCapability::Recommendations,
                "browse" => SourceCapability::Browse,
                "stations" => SourceCapability::Stations,
                "stationNext" => SourceCapability::StationContinuation,
                _ => return None,
            })
        })
        .collect();
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

fn text<'a>(params: &'a Value, key: &str, limit: usize) -> Result<&'a str> {
    let value = params[key]
        .as_str()
        .context(tr!("Invalid source capability parameters"))?;
    ensure!(
        !value.trim().is_empty() && value.len() <= limit,
        tr!("Invalid source capability parameters")
    );
    Ok(value)
}
fn validate(method: &str, params: &Value) -> Result<()> {
    ensure!(
        params.is_object() && serde_json::to_vec(params)?.len() <= 256 * 1024,
        tr!("Invalid source capability parameters")
    );
    match method {
        "browse" => {
            text(params, "page", 1024)?;
        }
        "stationNext" => {
            text(params, "id", 512)?;
        }
        _ => {}
    }
    if let Some(offset) = params.get("offset") {
        ensure!(
            offset.as_u64().is_some_and(|v| v <= 1_000_000),
            tr!("Invalid source capability parameters")
        );
    }
    Ok(())
}

fn track(catalog: &mygo::Catalog<'_>, value: &Value) -> Result<Value> {
    let mut result = serde_json::to_value(
        catalog
            .track(value)
            .context(tr!("Source track information is incomplete"))?,
    )?;
    if let Some(unavailable) = value["unavailable"].as_bool() {
        result["unavailable"] = json!(unavailable);
    }
    Ok(result)
}
fn adapt(catalog: &mygo::Catalog<'_>, value: &mut Value, depth: usize) -> Result<()> {
    ensure!(depth < 64, tr!("Source response is nested too deeply"));
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "tracks" | "episodes") && value.is_array() {
                    ensure!(
                        value.as_array().unwrap().len() <= musicx_core::protocol::MAX_QUEUE_TRACKS,
                        tr!("Source exceeds 10,000 tracks")
                    );
                    for value in value.as_array_mut().unwrap() {
                        *value = track(catalog, value)?;
                    }
                } else {
                    adapt(catalog, value, depth + 1)?;
                }
            }
        }
        Value::Array(rows) => {
            for row in rows {
                adapt(catalog, row, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn invoke(source: &str, method: &str, params: Value) -> Result<Value> {
    ensure!(
        ACTIONS.contains(&method),
        tr!("This is not a public source capability")
    );
    let info = music_sources::get_source(source)?;
    ensure!(
        info.adapter_exports.iter().any(|c| c == method),
        tr!("This source does not support this capability")
    );
    let params = if params.is_null() { json!({}) } else { params };
    validate(method, &params)?;
    let plugin = source
        .strip_prefix("mygo:")
        .context(tr!("Choose a plugin source"))?;
    let mut result = mygo::invoke(plugin, method, params)?;
    let catalog = mygo::Catalog::new(plugin);
    catalog.remember(&result);
    if method == "stationNext" && result.is_array() {
        if let Some(rows) = result.as_array_mut() {
            ensure!(
                rows.len() <= musicx_core::protocol::MAX_QUEUE_TRACKS,
                tr!("Source exceeds 10,000 tracks")
            );
            for row in rows {
                *row = track(&catalog, row)?;
            }
        }
    } else {
        adapt(&catalog, &mut result, 0)?;
    }
    Ok(result)
}
