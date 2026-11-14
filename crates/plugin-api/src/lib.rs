//! Typed data-source contract shared by the player and its source adapters.
use anyhow::Result;
use musicx_protocol::{AlbumPage, AudioQuality, Quality, Track, TrackInfo};
use std::sync::Arc;
use url::Url;

/// A host-created marker; plugin stream URLs cannot opt into remote control.
pub const REMOTE_SCHEME: &str = "musicx-remote";

#[derive(Clone, Default)]
pub struct RemoteSnapshot {
    pub position: f64,
    pub duration: Option<f64>,
    pub playing: bool,
    pub waiting: bool,
    pub volume: f32,
    pub error: Option<String>,
}

/// Nonblocking controls and cached state for a source-owned remote device.
pub trait RemotePlayback: Send + Sync {
    fn start(&self, play: bool, volume: f32) -> Result<()>;
    fn snapshot(&self) -> RemoteSnapshot;
    fn play(&self);
    fn pause(&self);
    fn set_volume(&self, volume: f32);
    fn seek(&self, seconds: f64, complete: Box<dyn Fn(bool) + Send>);
}

#[derive(Clone)]
pub struct Media {
    pub track: Track,
    pub url: Url,
    /// The remote source uses HTTP, even when `url` is a local transport URL.
    pub uses_http: bool,
    pub user_agent: Option<String>,
    pub quality: AudioQuality,
    pub gain_db: Option<f32>,
    pub peak: Option<f32>,
    /// Resources required by this URL; retain while its native audio item is loaded.
    pub lease: Option<Arc<dyn Send + Sync>>,
}

/// Supplies playable media and metadata for explicitly identified source tracks. Implementations
/// can block on the network and are called outside the platform's UI thread.
pub trait DataSource: Sync {
    fn station_next(&self, _source: &str, _id: &str) -> Result<Vec<Track>> {
        anyhow::bail!(musicx_protocol::tr!(
            "This source does not support this capability"
        ))
    }
    fn remote(&self, _track: &Track) -> Option<Arc<dyn RemotePlayback>> {
        None
    }
    fn artwork(&self, _url: &str) -> Option<Vec<u8>> {
        None
    }
    fn resolve(&self, track: Track, quality: Quality) -> Result<Media>;
    fn resolve_selection(&self, track: Track, quality: Quality) -> Result<Media> {
        self.resolve(track, quality)
    }
    fn info(&self, track: Track) -> TrackInfo;
    fn album(&self, track: Track) -> Result<AlbumPage>;
}
