//! Explicit public MyGo actions. Internal helper exports are never RPC endpoints.
use crate::{music_sources, mygo};
use anyhow::{Context, Result, ensure};
use musicx_core::protocol::tr;
use serde_json::{Value, json};

pub(crate) const ACTIONS: &[&str] = &[
    "home",
    "browse",
    "suggest",
    "isLiked",
    "like",
    "likeEpisode",
    "played",
    "savedAlbums",
    "saveAlbum",
    "followArtist",
    "savePlaylist",
    "subscribeShow",
    "stations",
    "stationNext",
    "comments",
    "likeComment",
    "postComment",
    "createPlaylist",
    "renamePlaylist",
    "deletePlaylist",
    "addToPlaylist",
    "removeFromPlaylist",
    "movePlaylistTrack",
    "remoteDevices",
    "remoteUse",
];

pub(crate) fn supported(exports: &[String]) -> Vec<String> {
    let mut result: Vec<_> = exports
        .iter()
        .filter(|name| {
            ACTIONS.contains(&name.as_str())
                || matches!(
                    name.as_str(),
                    "search"
                        | "stream"
                        | "lyrics"
                        | "album"
                        | "artist"
                        | "account"
                        | "signIn"
                        | "signOut"
                        | "setSetting"
                        | "playlists"
                        | "playlist"
                        | "liked"
                        | "recent"
                        | "shows"
                        | "show"
                        | "remotePlay"
                        | "remotePause"
                        | "remoteResume"
                        | "remoteSeek"
                        | "remoteVolume"
                        | "remoteState"
                )
        })
        .cloned()
        .collect();
    result.sort();
    result.dedup();
    result
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
fn list(params: &Value, key: &str) -> Result<()> {
    let rows = params[key]
        .as_array()
        .context(tr!("Invalid source capability parameters"))?;
    ensure!(
        !rows.is_empty()
            && rows.len() <= 1000
            && rows
                .iter()
                .all(|v| v.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 512)),
        tr!("Invalid source capability parameters")
    );
    Ok(())
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
        "remoteUse" => {
            text(params, "deviceId", 512)?;
        }
        "suggest" => {
            text(params, "query", 120)?;
        }
        "isLiked" => list(params, "ids")?,
        "like" | "likeEpisode" | "saveAlbum" | "followArtist" | "savePlaylist"
        | "subscribeShow" => {
            text(params, "id", 512)?;
            ensure!(
                params["on"].is_boolean(),
                tr!("Invalid source capability parameters")
            );
        }
        "stationNext" | "played" | "deletePlaylist" => {
            text(params, "id", 512)?;
        }
        "createPlaylist" => {
            text(params, "name", 512)?;
        }
        "renamePlaylist" => {
            text(params, "id", 512)?;
            text(params, "name", 512)?;
        }
        "addToPlaylist" | "removeFromPlaylist" => {
            text(params, "id", 512)?;
            list(params, "trackIds")?;
        }
        "movePlaylistTrack" => {
            text(params, "id", 512)?;
            for key in ["from", "to"] {
                ensure!(
                    params[key].as_u64().is_some_and(|v| v < 10000),
                    tr!("Invalid source capability parameters")
                );
            }
        }
        "comments" => {
            text(params, "trackId", 512)?;
        }
        "likeComment" => {
            text(params, "trackId", 512)?;
            text(params, "commentId", 512)?;
            ensure!(
                params["on"].is_boolean(),
                tr!("Invalid source capability parameters")
            );
        }
        "postComment" => {
            text(params, "trackId", 512)?;
            text(params, "text", 20000)?;
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

fn track(plugin: &str, value: &Value) -> Result<Value> {
    let mut result = serde_json::to_value(
        mygo::catalog_track(plugin, value)
            .context(tr!("Source track information is incomplete"))?,
    )?;
    if let Some(unavailable) = value["unavailable"].as_bool() {
        result["unavailable"] = json!(unavailable);
    }
    Ok(result)
}
fn adapt(plugin: &str, value: &mut Value, depth: usize) -> Result<()> {
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
                        *value = track(plugin, value)?;
                    }
                } else {
                    adapt(plugin, value, depth + 1)?;
                }
            }
        }
        Value::Array(rows) => {
            for row in rows {
                adapt(plugin, row, depth + 1)?;
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
        info.capabilities.iter().any(|c| c == method),
        tr!("This source does not support this capability")
    );
    let params = if params.is_null() { json!({}) } else { params };
    validate(method, &params)?;
    let plugin = source
        .strip_prefix("mygo:")
        .context(tr!("Choose a plugin source"))?;
    let mut result = mygo::invoke(plugin, method, params)?;
    mygo::remember_artwork(plugin, &result);
    if method == "stationNext" {
        if let Some(rows) = result.as_array_mut() {
            ensure!(
                rows.len() <= musicx_core::protocol::MAX_QUEUE_TRACKS,
                tr!("Source exceeds 10,000 tracks")
            );
            for row in rows {
                *row = track(plugin, row)?;
            }
        }
    } else {
        adapt(plugin, &mut result, 0)?;
    }
    Ok(result)
}
