//! Typed data-source contract shared by the player and its source adapters.
use anyhow::Result;
use musicx_protocol::{AlbumPage, AudioQuality, Quality, Track, TrackInfo};
use musicx_protocol::{
    InstalledPlugin, MusicSource, MusicSourceCatalog, PluginUpdateSettings, SignInMethod,
    SourceAccount, SourceConfiguration, SourceLogin, SourcePage, SourcePageRequest,
    SourcePersonalCollection, SourcePlaylist, SourcePlaylistSnapshot, SourceSearchPage,
    SourceSearchRequest, SourceUpdateCheck, tr,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use url::Url;

/// A source error independent of its script runtime or adapter implementation.
#[derive(Debug)]
pub struct SourceError {
    pub code: String,
    pub message: String,
}
impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for SourceError {}
pub fn error_code(error: &anyhow::Error) -> Option<&str> {
    error
        .downcast_ref::<SourceError>()
        .map(|e| e.code.as_str())
        .filter(|c| !c.is_empty())
}
fn unsupported<T>() -> Result<T> {
    Err(SourceError {
        code: "unsupported".into(),
        message: tr!("This source does not support this capability").into(),
    }
    .into())
}

/// Product capabilities implemented by source adapters. Host/runtime injection is
/// separate; transport layers choose which of these operations they expose.
pub trait PluginSource: DataSource {
    fn search(&self, _source: &str, _request: SourceSearchRequest) -> Result<SourceSearchPage> {
        unsupported()
    }
    fn sources(&self) -> Result<Vec<MusicSource>> {
        unsupported()
    }
    fn installed_plugins(&self) -> Result<Vec<InstalledPlugin>> {
        unsupported()
    }
    fn source(&self, _source: &str) -> Result<MusicSource> {
        unsupported()
    }
    fn catalog(&self, _refresh: bool) -> Result<MusicSourceCatalog> {
        unsupported()
    }
    fn install(&self, _repository: &str) -> Result<MusicSource> {
        unsupported()
    }
    fn check_update(&self, _source: &str) -> Result<SourceUpdateCheck> {
        unsupported()
    }
    fn update(&self, _source: &str) -> Result<MusicSource> {
        unsupported()
    }
    fn uninstall(&self, _source: &str) -> Result<()> {
        unsupported()
    }
    fn configure(
        &self,
        _source: &str,
        _values: BTreeMap<String, String>,
    ) -> Result<SourceConfiguration> {
        unsupported()
    }
    fn account(&self, _source: &str) -> Result<SourceAccount> {
        unsupported()
    }
    fn sign_in(
        &self,
        _source: &str,
        _job: Option<String>,
        _method: SignInMethod,
    ) -> Result<SourceLogin> {
        unsupported()
    }
    fn login_state(&self, _job: &str, _cancel: bool) -> Result<SourceLogin> {
        unsupported()
    }
    fn sign_out(&self, _source: &str) -> Result<()> {
        unsupported()
    }
    fn playlists(&self, _source: &str) -> Result<Vec<SourcePlaylist>> {
        unsupported()
    }
    fn playlist(&self, _source: &str, _id: &str) -> Result<SourcePlaylistSnapshot> {
        unsupported()
    }
    fn personal_collection(
        &self,
        _source: &str,
        _collection: SourcePersonalCollection,
    ) -> Result<SourcePlaylistSnapshot> {
        unsupported()
    }
    fn page(&self, _source: &str, _request: SourcePageRequest) -> Result<SourcePage> {
        unsupported()
    }
    fn source_artwork(&self, _source: &str, _url: &str) -> Result<Vec<u8>> {
        unsupported()
    }
    fn process_permission(
        &self,
        _source: &str,
        _enabled: bool,
        _fingerprint: Option<&str>,
    ) -> Result<MusicSource> {
        unsupported()
    }
    fn network_permission(
        &self,
        _source: &str,
        _enabled: bool,
        _fingerprint: Option<&str>,
    ) -> Result<MusicSource> {
        unsupported()
    }
    fn update_settings(&self, _enabled: Option<bool>) -> Result<PluginUpdateSettings> {
        unsupported()
    }
    fn start_updates(&self) -> Result<()> {
        Ok(())
    }
}

/// Audio acquired by an adapter for the MusicX player. Adapters never own audio
/// output or player controls: the platform player reads this URL and owns playback.
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
