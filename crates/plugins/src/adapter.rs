//! MyGo implements the MusicX source contract; script names stay in this adapter.
use crate::{MyGo, assets, discovery, music_sources, mygo, plugin_updates};
use anyhow::{Context, Result, ensure};
use music_sources::{get_source, plugin};
use musicx_core::{metadata, protocol::*};
use musicx_plugin_api::PluginSource;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn account(source: &str) -> Result<SourceAccount> {
    let id = plugin(source)?;
    let info = get_source(source)?;
    if !info.adapter_exports.iter().any(|v| v == "account") {
        return Ok(SourceAccount::default());
    }
    let v = mygo::invoke(id, "account", json!({})).map_err(|_| {
        anyhow::anyhow!(tr!(
            "Could not read account status; check the source settings"
        ))
    })?;
    Ok(SourceAccount {
        connected: !v.is_null() && v["id"].is_string(),
        name: v["name"].as_str().map(|v| v.chars().take(100).collect()),
    })
}
fn playlists(source: &str) -> Result<Vec<SourcePlaylist>> {
    let id = plugin(source)?;
    let v = mygo::invoke(id, "playlists", json!({})).map_err(|_| {
        anyhow::anyhow!(tr!(
            "Could not read source playlists; connect an account or check settings"
        ))
    })?;
    let rows = v
        .as_array()
        .or_else(|| v["playlists"].as_array())
        .context(tr!("The source returned no playlist list"))?;
    ensure!(rows.len() <= 1000, tr!("Too many source playlists"));
    Ok(rows
        .iter()
        .filter_map(|v| {
            let id = v["id"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| v["id"].as_u64().map(|v| v.to_string()))?;
            Some(SourcePlaylist {
                cover_url: v["imageUrl"].as_str().and_then(metadata::artwork_url),
                id,
                name: v["name"]
                    .as_str()
                    .unwrap_or(tr!("Untitled playlist"))
                    .into(),
                track_count: v["trackCount"].as_u64().unwrap_or(0) as usize,
            })
        })
        .collect())
}
fn playlist_snapshot(source: &str, id: &str) -> Result<(PlaylistOrigin, Vec<Track>)> {
    let id = id.trim();
    ensure!(
        !id.is_empty() && id.len() <= 1024,
        tr!("Invalid playlist identifier")
    );
    let plugin = plugin(source)?;
    let value = mygo::invoke(plugin, "playlist", json!({"id":id})).map_err(|_| {
        anyhow::anyhow!(tr!(
            "Could not load the source playlist; check your account and settings"
        ))
    })?;
    let page = if value["playlist"].is_object() {
        &value["playlist"]
    } else {
        &value
    };
    let (rows, completion) = page["tracks"]
        .as_array()
        .map(|rows| (rows, page))
        .or_else(|| value["tracks"].as_array().map(|rows| (rows, &value)))
        .context(tr!("The source returned no playlist tracks"))?;
    ensure!(rows.len() <= 10000, tr!("Playlist exceeds 10,000 tracks"));
    let catalog = mygo::Catalog::new(plugin);
    let tracks: Vec<_> = rows.iter().filter_map(|v| catalog.track(v)).collect();
    ensure!(
        tracks.len() == rows.len(),
        tr!("Source track information is incomplete; nothing was imported")
    );
    ensure!(
        completion["trackCount"]
            .as_u64()
            .is_none_or(|n| n as usize <= tracks.len()),
        tr!("The source returned an incomplete playlist; nothing was imported")
    );
    Ok((
        PlaylistOrigin {
            provider: source.into(),
            id: id.into(),
            name: page["name"]
                .as_str()
                .unwrap_or(tr!("Import playlist"))
                .into(),
        },
        tracks,
    ))
}

impl PluginSource for MyGo {
    fn search(&self, source: &str, request: SourceSearchRequest) -> Result<SourceSearchPage> {
        let id = source.strip_prefix("mygo:").unwrap_or(source);
        ensure!(mygo::valid_id(id), tr!("Invalid plugin ID"));
        let key = match request.kind {
            SourceSearchKind::Tracks => "tracks",
            SourceSearchKind::Albums => "albums",
            SourceSearchKind::Artists => "artists",
            SourceSearchKind::Playlists => "playlists",
            SourceSearchKind::Shows => "shows",
        };
        let value = mygo::search_kind(id, &request.query, key, request.offset)?;
        let rows = value[key]
            .as_array()
            .context(tr!("Source search information is incomplete"))?;
        ensure!(
            rows.len() <= MAX_QUEUE_TRACKS,
            tr!("Source search exceeds 10,000 items")
        );
        let mut result = SourceSearchPage {
            source: format!("mygo:{id}"),
            more: value["more"]
                .as_bool()
                .or_else(|| value["hasMore"].as_bool())
                .unwrap_or(false),
            ..Default::default()
        };
        if request.kind == SourceSearchKind::Tracks {
            result.tracks = rows
                .iter()
                .map(|r| serde_json::from_value(r.clone()))
                .collect::<std::result::Result<_, _>>()?;
        } else {
            let artwork = assets::Artwork::new(id);
            let entities = rows
                .iter()
                .map(|r| search_entity(&artwork, r))
                .collect::<Result<Vec<_>>>()?;
            match request.kind {
                SourceSearchKind::Albums => result.albums = entities,
                SourceSearchKind::Artists => result.artists = entities,
                SourceSearchKind::Playlists => result.playlists = entities,
                SourceSearchKind::Shows => result.shows = entities,
                SourceSearchKind::Tracks => unreachable!(),
            }
        }
        Ok(result)
    }
    fn sources(&self) -> Result<Vec<MusicSource>> {
        music_sources::list()
    }
    fn installed_plugins(&self) -> Result<Vec<InstalledPlugin>> {
        Ok(serde_json::from_value(mygo::list()?)?)
    }
    fn source(&self, source: &str) -> Result<MusicSource> {
        get_source(source)
    }
    fn catalog(&self, refresh: bool) -> Result<MusicSourceCatalog> {
        music_sources::catalog_view(refresh)
    }
    fn install(&self, repository: &str) -> Result<MusicSource> {
        music_sources::install(repository)
    }
    fn check_update(&self, source: &str) -> Result<SourceUpdateCheck> {
        Ok(serde_json::from_value(music_sources::check_update(
            source,
        )?)?)
    }
    fn update(&self, source: &str) -> Result<MusicSource> {
        music_sources::update(source)
    }
    fn uninstall(&self, source: &str) -> Result<()> {
        mygo::uninstall(plugin(source)?)
    }
    fn configure(
        &self,
        source: &str,
        values: BTreeMap<String, String>,
    ) -> Result<SourceConfiguration> {
        Ok(serde_json::from_value(music_sources::configure(
            source, values,
        )?)?)
    }
    fn account(&self, source: &str) -> Result<SourceAccount> {
        account(source)
    }
    fn sign_in(
        &self,
        source: &str,
        job: Option<String>,
        method: SignInMethod,
    ) -> Result<SourceLogin> {
        music_sources::begin_sign_in_method(source, job, method)
    }
    fn login_state(&self, job: &str, cancel: bool) -> Result<SourceLogin> {
        music_sources::login_state(job, cancel)
    }
    fn sign_out(&self, source: &str) -> Result<()> {
        mygo::invoke(plugin(source)?, "signOut", json!({}))
            .map_err(|_| anyhow::anyhow!(tr!("Could not sign out of the source")))?;
        Ok(())
    }
    fn playlists(&self, source: &str) -> Result<Vec<SourcePlaylist>> {
        playlists(source)
    }
    fn playlist(&self, source: &str, id: &str) -> Result<SourcePlaylistSnapshot> {
        let (origin, tracks) = playlist_snapshot(source, id)?;
        Ok(SourcePlaylistSnapshot { origin, tracks })
    }
    fn personal_collection(
        &self,
        source: &str,
        collection: SourcePersonalCollection,
    ) -> Result<SourcePlaylistSnapshot> {
        discovery::personal_collection(source, collection)
    }
    fn page(&self, source: &str, request: SourcePageRequest) -> Result<SourcePage> {
        let mut page = discovery::page(source, request)?;
        page.playable = get_source(source)?.supports(SourceCapability::Playback);
        Ok(page)
    }
    fn source_artwork(&self, source: &str, url: &str) -> Result<Vec<u8>> {
        assets::image(plugin(source)?, url)
    }
    fn process_permission(
        &self,
        source: &str,
        enabled: bool,
        fingerprint: Option<&str>,
    ) -> Result<Option<MusicSource>> {
        let guard = crate::permissions::lock()?;
        crate::process::set(plugin(source)?, enabled, fingerprint, &guard)?;
        drop(guard);
        if enabled {
            get_source(source).map(Some)
        } else {
            Ok(get_source(source).ok())
        }
    }
    fn network_permission(
        &self,
        source: &str,
        enabled: bool,
        fingerprint: Option<&str>,
    ) -> Result<Option<MusicSource>> {
        let guard = crate::permissions::lock()?;
        crate::network::set(plugin(source)?, enabled, fingerprint, &guard)?;
        drop(guard);
        if enabled {
            get_source(source).map(Some)
        } else {
            Ok(get_source(source).ok())
        }
    }
    fn update_settings(&self, enabled: Option<bool>) -> Result<PluginUpdateSettings> {
        if let Some(enabled) = enabled {
            plugin_updates::set_enabled(enabled)
        } else {
            plugin_updates::settings()
        }
    }
    fn start_updates(&self) -> Result<()> {
        plugin_updates::start()
    }
}

fn search_entity(artwork: &assets::Artwork<'_>, value: &Value) -> Result<SourceSearchEntity> {
    let id = value["id"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| value["id"].as_u64().map(|id| id.to_string()))
        .context(tr!("Source search information is incomplete"))?;
    ensure!(
        !id.is_empty() && id.len() <= 1024,
        tr!("Source search information is incomplete")
    );
    Ok(SourceSearchEntity {
        name: value["name"]
            .as_str()
            .unwrap_or(&id)
            .chars()
            .take(512)
            .collect(),
        id,
        image_url: value["imageUrl"].as_str().and_then(|url| artwork.url(url)),
        artists: value["artists"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| {
                Some(Artist {
                    id: a["id"]
                        .as_str()
                        .map(str::to_owned)
                        .or_else(|| a["id"].as_u64().map(|id| id.to_string()))
                        .unwrap_or_default(),
                    name: a["name"].as_str()?.chars().take(512).collect(),
                    cover_url: a["imageUrl"].as_str().and_then(|url| artwork.url(url)),
                })
            })
            .collect(),
        description: value["description"]
            .as_str()
            .map(|s| s.chars().take(4000).collect()),
        publisher: value["publisher"]
            .as_str()
            .map(|s| s.chars().take(512).collect()),
        track_count: value["trackCount"].as_u64(),
        release_date: value["releaseDate"]
            .as_str()
            .map(|s| s.chars().take(64).collect()),
    })
}
