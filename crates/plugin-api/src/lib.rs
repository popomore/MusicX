//! Typed data-source contract shared by the player and its source adapters.
use anyhow::Result;
use musicx_protocol::{AlbumPage, AudioQuality, Quality, Track, TrackInfo};
use std::sync::Arc;
use url::Url;

#[derive(Clone)]
pub struct Media {
    pub track: Track,
    pub url: Url,
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
