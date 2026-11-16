//! Application source operations consume only the injected Rust contract.
use crate::{local_library, protocol::*};
use anyhow::Result;
use musicx_plugin::PluginSource;
use serde_json::{Value, json};

pub fn change_library(sources: &dyn PluginSource, action: LocalLibraryAction) -> Result<Value> {
    if let LocalLibraryAction::ImportSource {
        source,
        playlist_id,
    } = action
    {
        let snapshot = sources.playlist(&source, &playlist_id)?;
        local_library::import(snapshot.origin, snapshot.tracks)
    } else {
        local_library::change(action)
    }
}

pub fn execute(sources: &dyn PluginSource, action: MusicSourceAction) -> Result<Value> {
    use MusicSourceAction::*;
    Ok(match action {
        List => serde_json::to_value(sources.sources()?)?,
        AutoUpdateSettings => serde_json::to_value(sources.update_settings(None)?)?,
        SetAutoUpdate { enabled } => serde_json::to_value(sources.update_settings(Some(enabled))?)?,
        Catalog => serde_json::to_value(sources.catalog(false)?.sources)?,
        CatalogView => serde_json::to_value(sources.catalog(false)?)?,
        RefreshCatalog => serde_json::to_value(sources.catalog(true)?)?,
        Get { source } => serde_json::to_value(sources.source(&source)?)?,
        Install { repository } => serde_json::to_value(sources.install(&repository)?)?,
        CheckUpdate { source } => serde_json::to_value(sources.check_update(&source)?)?,
        Update { source } => serde_json::to_value(sources.update(&source)?)?,
        Uninstall { source } => {
            sources.uninstall(&source)?;
            json!({"ok":true})
        }
        Configure { source, values } => serde_json::to_value(sources.configure(&source, values)?)?,
        Account { source } => serde_json::to_value(sources.account(&source)?)?,
        SignIn {
            source,
            job,
            method,
        } => serde_json::to_value(sources.sign_in(&source, job, method)?)?,
        Poll { job } => serde_json::to_value(sources.login_state(&job, false)?)?,
        Cancel { job } => serde_json::to_value(sources.login_state(&job, true)?)?,
        SignOut { source } => {
            sources.sign_out(&source)?;
            json!({"ok":true})
        }
        Playlists { source } => serde_json::to_value(sources.playlists(&source)?)?,
        ImportLiked { source } => {
            import_personal(sources, &source, SourcePersonalCollection::LikedTracks)?
        }
        ImportRecent { source } => {
            import_personal(sources, &source, SourcePersonalCollection::RecentTracks)?
        }
        ImportPlaylist {
            source,
            playlist_id,
        } => change_library(
            sources,
            LocalLibraryAction::ImportSource {
                source,
                playlist_id,
            },
        )?,
        Page { source, request } => serde_json::to_value(sources.page(&source, request)?)?,
        Artist { source, id } => {
            serde_json::to_value(sources.page(&source, SourcePageRequest::Artist { id })?)?
        }
        Album { source, id } => {
            serde_json::to_value(sources.page(&source, SourcePageRequest::Album { id })?)?
        }
        Shows { source } => {
            serde_json::to_value(sources.page(&source, SourcePageRequest::Podcasts {})?)?
        }
        Show { source, id, offset } => serde_json::to_value(
            sources.page(&source, SourcePageRequest::PodcastEpisodes { id, offset })?,
        )?,
        Artwork { source, url } => {
            use base64::Engine;
            json!({"data":base64::engine::general_purpose::STANDARD.encode(sources.source_artwork(&source,&url)?)})
        }
        SetProcessPermission {
            source,
            enabled,
            expected_fingerprint,
        } => serde_json::to_value(sources.process_permission(
            &source,
            enabled,
            expected_fingerprint.as_deref(),
        )?)?,
        SetNetworkPermission {
            source,
            enabled,
            expected_fingerprint,
        } => serde_json::to_value(sources.network_permission(
            &source,
            enabled,
            expected_fingerprint.as_deref(),
        )?)?,
    })
}

fn import_personal(
    sources: &dyn PluginSource,
    source: &str,
    collection: SourcePersonalCollection,
) -> Result<Value> {
    let snapshot = sources.personal_collection(source, collection)?;
    local_library::import(snapshot.origin, snapshot.tracks)
}

pub fn search(
    sources: &dyn PluginSource,
    source: &str,
    mut request: SourceSearchRequest,
) -> Result<SourceSearchPage> {
    anyhow::ensure!(
        !source.is_empty() && source.len() <= 512,
        tr!("Invalid source identifier")
    );
    request.query = request.query.trim().into();
    anyhow::ensure!(
        !request.query.is_empty() && request.query.chars().count() <= 120,
        tr!("Enter 1 to 120 characters")
    );
    sources.search(source, request)
}
