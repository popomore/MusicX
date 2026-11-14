//! MyGo compatibility and explicit plugin source registration.
pub mod assets;
mod capabilities;
mod catalog;
mod discovery;
mod ffmpeg;
mod http;
mod installation;
mod localization;
pub mod music_sources;
pub mod mygo;
mod network;
mod oauth;
mod plugin_files;
pub mod plugin_updates;
mod process;
mod quality;
mod remote_playback;
mod repository;
mod requests;
mod script;
mod source_catalog;
mod stream_proxy;
mod trust;
mod web_api;
mod web_auth;

use anyhow::Result;
use musicx_core::{local_library, protocol};
use musicx_plugin_api::{DataSource, Media};
use protocol::{AlbumPage, LocalLibraryAction, Quality, Track, TrackInfo};
use serde_json::Value;

pub use web_auth::configure as configure_web_auth;

pub struct MyGo;
pub static MYGO: MyGo = MyGo;

impl DataSource for MyGo {
    fn station_next(&self, source: &str, id: &str) -> Result<Vec<Track>> {
        Ok(discovery::page(source, "stationNext", serde_json::json!({"id":id}))?.tracks)
    }
    fn remote(
        &self,
        track: &Track,
    ) -> Option<std::sync::Arc<dyn musicx_plugin_api::RemotePlayback>> {
        remote_playback::for_track(track)
    }
    fn artwork(&self, value: &str) -> Option<Vec<u8>> {
        let url = url::Url::parse(value).ok()?;
        let plugin = url.fragment()?.strip_prefix("musicx-plugin=")?;
        assets::image(plugin, value).ok()
    }
    fn resolve(&self, track: Track, quality: Quality) -> Result<Media> {
        catalog::resolve_with_quality(track, quality)
    }

    fn info(&self, track: Track) -> TrackInfo {
        catalog::info(track)
    }

    fn album(&self, track: Track) -> Result<AlbumPage> {
        catalog::album(track)
    }
}

/// Fetch imports before acquiring the local library's writer lock.
pub fn change_library(action: LocalLibraryAction) -> Result<Value> {
    let (origin, tracks) = match action {
        LocalLibraryAction::ImportSource {
            source,
            playlist_id,
        } => music_sources::playlist_snapshot(&source, &playlist_id)?,
        edit => return local_library::change(edit),
    };
    local_library::import(origin, tracks)
}
