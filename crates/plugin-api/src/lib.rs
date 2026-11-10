//! Typed data-source contract shared by the player and its source adapters.
use anyhow::Result;
use musicx_protocol::{AlbumPage, AudioQuality, Quality, Track, TrackInfo};
use url::Url;

#[derive(Clone)]
pub struct Media {
    pub track: Track,
    pub url: Url,
    pub user_agent: Option<String>,
    pub quality: AudioQuality,
}

/// Supplies playable media and metadata for explicitly identified source tracks. Implementations
/// can block on the network and are called outside the platform's UI thread.
pub trait DataSource: Sync {
    fn resolve(&self, track: Track, quality: Quality) -> Result<Media>;
    fn resolve_selection(&self, track: Track, quality: Quality) -> Result<Media> {
        self.resolve(track, quality)
    }
    fn info(&self, track: Track) -> TrackInfo;
    fn album(&self, track: Track) -> Result<AlbumPage>;
}
