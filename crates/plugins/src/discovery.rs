//! Read-only provider browsing and atomic local imports.
use crate::{mygo, protocol::*};
use anyhow::{Context, Result, ensure};
use musicx_core::local_library;
use serde_json::{Value, json};
use std::collections::HashSet;

fn source_id(source: &str) -> Result<&str> {
    let id = source.strip_prefix("mygo:").context("请选择插件数据源")?;
    ensure!(mygo::valid_id(id), "无效的数据源标识");
    Ok(id)
}

fn bounded_id(id: &str) -> Result<&str> {
    ensure!(!id.is_empty() && id.len() <= 1024, "无效的来源项目标识");
    Ok(id)
}

fn tracks(plugin: &str, rows: &[Value]) -> Result<Vec<Track>> {
    ensure!(rows.len() <= MAX_QUEUE_TRACKS, "来源曲目超过10000首");
    rows.iter()
        .map(|v| mygo::catalog_track(plugin, v).context("来源曲目信息不完整"))
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
    // Fetch the complete snapshot before touching the local library. A broken
    // pagination contract must not silently save the first page as complete.
    loop {
        let value = mygo::invoke(plugin, export, json!({"offset":all.len()}))?;
        let rows = if liked {
            value["tracks"].as_array()
        } else {
            value.as_array().or_else(|| value["tracks"].as_array())
        }
        .context("来源未返回曲目列表")?;
        let page = tracks(plugin, rows)?;
        ensure!(
            all.len() + page.len() <= MAX_QUEUE_TRACKS,
            "导入超过10000首，未保存不完整副本"
        );
        let continuing = liked && more(&value);
        if continuing {
            ensure!(!page.is_empty(), "来源分页没有前进，未导入");
            let fingerprint = serde_json::to_string(rows)?;
            ensure!(seen.insert(fingerprint), "来源重复返回同一页，未导入");
        }
        all.extend(page);
        if !continuing {
            break;
        }
    }
    let label = if liked {
        "云端喜欢"
    } else {
        "云端播放记录"
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

pub fn browse(source: &str, kind: &str, id: &str, offset: u64) -> Result<SourcePage> {
    let plugin = source_id(source)?;
    ensure!(offset <= MAX_QUEUE_TRACKS as u64, "分页超出范围");
    if kind != "shows" {
        bounded_id(id)?;
    }
    let value = mygo::invoke(plugin, kind, json!({"id":id,"offset":offset}))?;
    if kind == "shows" {
        let rows = value
            .as_array()
            .or_else(|| value["shows"].as_array())
            .context("来源未返回播客列表")?;
        ensure!(rows.len() <= 1000, "来源播客过多");
        return Ok(SourcePage {
            id: "shows".into(),
            title: "播客".into(),
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
    ensure!(!more(&value) || !rows.is_empty(), "来源分页没有前进");
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
    })
}
