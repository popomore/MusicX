//! MyGo compatibility and explicit plugin source registration.
mod catalog;
mod http;
pub mod music_sources;
pub mod mygo;
mod quality;
mod repository;
mod source_catalog;
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
