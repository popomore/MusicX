//! MyGo runtime and adapter implementing the `musicx-plugin` contracts.
mod adapter;
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
mod permissions;
mod plugin_files;
pub mod plugin_updates;
mod process;
mod quality;
mod radio;
mod repository;
mod requests;
mod script;
mod source_catalog;
mod stream_proxy;
mod trust;
mod web_api;
mod web_auth;

use anyhow::Result;
use musicx_core::protocol;
use musicx_plugin::{DataSource, Media};
use protocol::{AlbumPage, LocalLibraryAction, Quality, Track, TrackInfo};
use serde_json::Value;

pub use web_auth::configure as configure_web_auth;

pub struct MyGo;
pub static MYGO: MyGo = MyGo;

impl DataSource for MyGo {
    fn station_next(&self, source: &str, id: &str) -> Result<Vec<Track>> {
        Ok(discovery::page(
            source,
            protocol::SourcePageRequest::StationTracks {
                id: id.into(),
                name: None,
            },
        )?
        .tracks)
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
    musicx_core::sources::change_library(&MYGO, action)
}
