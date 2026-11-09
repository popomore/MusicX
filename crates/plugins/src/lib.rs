//! MyGo compatibility, source registration and legacy NetEase entry points.
mod catalog;
pub mod music_sources;
pub mod mygo;
pub mod netease;
mod quality;
mod web_auth;

use anyhow::Result;
use musicx_core::{audio_quality, local_library, protocol};
use musicx_plugin_api::{DataSource, Media};
use protocol::{AlbumPage, LocalLibraryAction, Quality, Track, TrackInfo};
use serde_json::Value;

pub use web_auth::APP_NAME as WEB_AUTH_APP_NAME;

pub struct MyGo;
pub static MYGO: MyGo = MyGo;

impl DataSource for MyGo {
    fn search(&self, query: String, limit: u64) -> Result<Vec<Track>> {
        netease::search(query, limit)
    }

    fn resolve(&self, track: Track, quality: Quality) -> Result<Media> {
        catalog::resolve_with_quality(track, quality)
    }

    fn resolve_selection(&self, track: Track, quality: Quality) -> Result<Media> {
        if track.source.is_some() {
            self.resolve(track, quality)
        } else {
            netease::resolve_with_quality(track.id, quality)
        }
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
        LocalLibraryAction::ImportNetease { id } => {
            music_sources::playlist_snapshot("netease", &id.to_string())?
        }
        LocalLibraryAction::ImportSource {
            source,
            playlist_id,
        } => music_sources::playlist_snapshot(&source, &playlist_id)?,
        edit => return local_library::change(edit),
    };
    local_library::import(origin, tracks)
}
