//! Wire types shared by the independent background and NSH panel builds.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

mod update_channel;
pub use update_channel::UpdateChannel;

/// Maximum size of a playback queue that can be persisted and restored.
pub const MAX_QUEUE_TRACKS: usize = 10_000;

/// Build-time identity keeps separately packaged QA apps independent of MusicX.
pub const APP_NAME: &str = match option_env!("MUSICX_BUILD_APP_NAME") {
    Some(name) => name,
    None => "MusicX",
};
/// Base namespace for this build's native application components.
pub const APP_ID: &str = match option_env!("MUSICX_BUILD_APP_ID") {
    Some(id) => id,
    None => "local.musicx",
};

/// Scope component bundle IDs and notification names to the same build.
pub fn component_app_id(component: &str) -> String {
    format!("{APP_ID}.{component}")
}

/// PID-scoped native presentation request; it never carries playback data.
#[cfg(target_os = "macos")]
pub fn panel_reopen_notification() -> String {
    component_app_id("panel.reopen")
}

#[cfg(target_os = "macos")]
pub fn check_updates_notification() -> String {
    component_app_id("updates.check")
}

#[cfg(target_os = "macos")]
pub fn update_channel_notification() -> String {
    component_app_id("updates.channel")
}

#[cfg(not(target_os = "ios"))]
pub const ADDRESS: &str = match option_env!("MUSICX_BUILD_ADDRESS") {
    Some(address) => address,
    None => "127.0.0.1:45967",
};
#[cfg(target_os = "ios")]
pub const ADDRESS: &str = "127.0.0.1:45968";

/// Allow isolated local instances without sharing the live listener or history.
pub fn address() -> std::net::SocketAddr {
    let mut address: std::net::SocketAddr = ADDRESS.parse().expect("loopback address");
    if let Ok(port) = std::env::var("MUSICX_PORT")
        && let Ok(port) = port.parse::<u16>()
        && port != 0
    {
        address.set_port(port);
    }
    address
}

const DATA_DIR_ENV: &str = "MUSICX_DATA_DIR";

pub fn data_dir() -> Option<std::path::PathBuf> {
    std::env::var_os(DATA_DIR_ENV)
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::home_dir().map(|home| home.join("Library/Application Support").join(APP_NAME))
        })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Album {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Source {
    pub plugin: String,
    pub id: String,
}

#[derive(Clone, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Track {
    pub id: u64,
    pub title: String,
    pub artist: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artists: Vec<Artist>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<Episode>,
    pub duration: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<Album>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Artist {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub cover_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Episode {
    pub show_id: String,
    pub description: String,
    pub published: String,
}

/// Preserve identities of persisted legacy NetEase favorites and history.
/// This compatibility key never selects, installs or invokes a source plugin.
pub fn track_key(track: &Track) -> String {
    track.source.as_ref().map_or_else(
        || format!("netease:{}", track.id),
        |s| {
            if s.plugin == "netease" && s.id.parse::<u64>().is_ok_and(|id| id > 0) {
                format!("netease:{}", s.id.parse::<u64>().expect("validated ID"))
            } else {
                format!("mygo:{}:{}", s.plugin, s.id)
            }
        },
    )
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    #[default]
    Idle,
    Loading,
    Playing,
    Paused,
    Ended,
    Error,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    #[default]
    Standard,
    High,
    Lossless,
    Hires,
}
impl Quality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "标准",
            Self::High => "高音质",
            Self::Lossless => "无损",
            Self::Hires => "Hi-Res",
        }
    }
    pub fn plugin_quality(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::High => "high",
            Self::Lossless | Self::Hires => "lossless",
        }
    }
    pub fn for_plugin(self) -> Self {
        if self == Self::Hires {
            Self::Lossless
        } else {
            self
        }
    }
    pub fn choices(plugin: bool) -> &'static [Self] {
        if plugin {
            &[Self::Standard, Self::High, Self::Lossless]
        } else {
            &[Self::Standard, Self::High, Self::Lossless, Self::Hires]
        }
    }
}
impl std::str::FromStr for Quality {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "standard" => Ok(Self::Standard),
            "high" => Ok(Self::High),
            "lossless" => Ok(Self::Lossless),
            "hires" => Ok(Self::Hires),
            _ => Err("音质应为standard、high、lossless或hires"),
        }
    }
}

/// Source-reported stream properties, separate from the listener's preference.
#[derive(Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct AudioQuality {
    pub requested: Quality,
    pub level: Option<String>,
    pub format: Option<String>,
    pub bitrate_bps: Option<u64>,
    pub sample_rate_hz: Option<u32>,
    pub bit_depth: Option<u16>,
}
impl AudioQuality {
    pub fn level_label(&self) -> Option<&'static str> {
        let level = self.level.as_deref()?;
        if matches!(level, "lossless" | "hires")
            && self.format.as_deref().is_some_and(|format| {
                ["MP3", "AAC", "OPUS"]
                    .iter()
                    .any(|lossy| format.eq_ignore_ascii_case(lossy))
            })
        {
            return None;
        }
        match level {
            "standard" => Some("标准"),
            "higher" => Some("较高"),
            "high" | "exhigh" => Some("高音质"),
            "lossless" => Some("无损"),
            "hires" => Some("Hi-Res"),
            "jyeffect" => Some("高清臻音"),
            "sky" | "vivid" => Some("全景声"),
            "jymaster" => Some("超清母带"),
            _ => None,
        }
    }
    pub fn short_label(&self) -> String {
        self.level_label()
            .map(str::to_owned)
            .or_else(|| self.format.clone())
            .unwrap_or_else(|| "音质未知".into())
    }
    pub fn summary(&self) -> String {
        let mut parts = Vec::with_capacity(5);
        if let Some(label) = self.level_label() {
            parts.push(label.into());
        }
        if let Some(format) = &self.format {
            parts.push(format.clone());
        }
        if let Some(br) = self.bitrate_bps {
            parts.push(format!("{} kbps", br / 1000));
        }
        if let Some(sr) = self.sample_rate_hz {
            parts.push(format!("{} kHz", sr as f64 / 1000.0));
        }
        if let Some(depth) = self.bit_depth {
            parts.push(format!("{depth} bit"));
        }
        if parts.is_empty() {
            "来源未提供音质参数".into()
        } else {
            parts.join(" · ")
        }
    }
}

#[derive(Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Status {
    pub state: State,
    pub track: Option<Track>,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub index: Option<usize>,
    pub queue: Arc<Vec<Track>>,
    pub queue_length: usize,
    #[serde(default)]
    pub queue_revision: u64,
    #[serde(default)]
    pub library_revision: u64,
    #[serde(default)]
    pub queue_context: Option<QueueContext>,
    #[serde(default)]
    pub preferred_quality: Quality,
    #[serde(default)]
    pub normalization_enabled: bool,
    #[serde(default)]
    pub audio_quality: Option<AudioQuality>,
    pub error: Option<String>,
    pub switching: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UiAction {
    PlayTrack {
        track: Track,
    },
    PlayList {
        tracks: Vec<Track>,
        index: usize,
        context: QueueContext,
    },
    PlayLocalPlaylist {
        id: String,
        index: usize,
    },
    PlayQueue {
        index: usize,
        revision: u64,
    },
    Quality {
        quality: Quality,
    },
    Normalization {
        enabled: bool,
    },
    Pause,
    Resume,
    Toggle,
    Previous,
    Next,
    Seek {
        seconds: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track_key: Option<String>,
    },
    Volume {
        value: f32,
    },
    Quit,
}

/// The active listening session is a snapshot, separate from saved playlists
/// and the history log. Advancing or recording history never rebuilds it.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueContext {
    Album {
        id: String,
        name: String,
    },
    Playlist {
        id: u64,
        name: String,
    },
    LocalPlaylist {
        id: String,
        name: String,
    },
    History,
    SourceCollection {
        source: String,
        id: String,
        name: String,
    },
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    #[serde(flatten)]
    pub track: Track,
    pub played_at_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<HistoryProgress>,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HistoryProgress {
    pub position: f64,
    pub duration: f64,
    pub completed: bool,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Lyrics {
    pub text: String,
    pub translation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<LyricLine>,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LyricLine {
    pub start: u64,
    #[serde(default)]
    pub end: Option<u64>,
    pub text: String,
    #[serde(default)]
    pub translation: String,
    #[serde(default)]
    pub words: Vec<LyricWord>,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LyricWord {
    pub start: u64,
    pub end: u64,
    pub text: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceLink {
    pub label: String,
    pub url: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub track: Track,
    pub source_name: String,
    pub source_link: Option<SourceLink>,
    pub lyrics: Lyrics,
    pub lyrics_error: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumPage {
    pub album: Album,
    pub tracks: Vec<Track>,
}

/// Local library contains metadata and source identities, never resolved audio URLs.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalPlaylist {
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub pinned: bool,
    pub id: String,
    pub name: String,
    pub revision: u64,
    pub tracks: Vec<Track>,
    pub imported_from: Option<PlaylistOrigin>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistOrigin {
    pub provider: String,
    pub id: String,
    pub name: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalPlaylistSummary {
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub pinned: bool,
    pub id: String,
    pub name: String,
    pub revision: u64,
    pub track_count: usize,
    pub imported_from: Option<PlaylistOrigin>,
    #[serde(default)]
    pub cover_url: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LocalLibraryAction {
    SetFavorite {
        track: Box<Track>,
        liked: bool,
    },
    Create {
        name: String,
    },
    Rename {
        id: String,
        name: String,
    },
    SetPinned {
        id: String,
        pinned: bool,
    },
    Add {
        id: String,
        tracks: Vec<Track>,
    },
    Remove {
        id: String,
        index: usize,
        revision: u64,
    },
    Delete {
        id: String,
    },
    ImportSource {
        source: String,
        playlist_id: String,
    },
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FavoriteState {
    pub key: String,
    pub liked: bool,
    pub playlist_id: Option<String>,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceSettingDefinition {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub options: Vec<SourceSettingOption>,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceSettingOption {
    pub value: String,
    pub label: String,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceSetting {
    #[serde(flatten)]
    pub definition: SourceSettingDefinition,
    pub configured: bool,
    pub secret: bool,
    pub saved_value: Option<String>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MusicSource {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    pub repository: Option<String>,
    pub installed: bool,
    pub can_install: bool,
    pub limitation: Option<String>,
    pub capabilities: Vec<String>,
    pub settings: Vec<SourceSetting>,
    pub qr_sign_in: bool,
    #[serde(default)]
    pub web_sign_in: bool,
    #[serde(default)]
    pub oauth_sign_in: bool,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MusicSourceCatalog {
    pub sources: Vec<MusicSource>,
    pub refreshing: bool,
    pub warning: Option<String>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceAccount {
    pub connected: bool,
    pub name: Option<String>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceLogin {
    pub job: String,
    pub source: String,
    pub status: String,
    pub text: String,
    pub svg: Option<String>,
    #[serde(default)]
    pub qr_url: Option<String>,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SourcePlaylist {
    pub id: String,
    pub name: String,
    pub track_count: usize,
    #[serde(default)]
    pub cover_url: Option<String>,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceCollection {
    pub kind: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub cover_url: Option<String>,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourcePage {
    pub id: String,
    pub title: String,
    pub description: String,
    pub cover_url: Option<String>,
    pub tracks: Vec<Track>,
    pub collections: Vec<SourceCollection>,
    pub more: bool,
    pub next_offset: u64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MusicSourceAction {
    List,
    Catalog,
    CatalogView,
    RefreshCatalog,
    Get {
        source: String,
    },
    Install {
        repository: String,
    },
    CheckUpdate {
        source: String,
    },
    Update {
        source: String,
    },
    Uninstall {
        source: String,
    },
    Configure {
        source: String,
        values: std::collections::BTreeMap<String, String>,
    },
    Account {
        source: String,
    },
    SignIn {
        source: String,
        #[serde(default)]
        job: Option<String>,
        #[serde(default)]
        method: SignInMethod,
    },
    Poll {
        job: String,
    },
    Cancel {
        job: String,
    },
    SignOut {
        source: String,
    },
    Playlists {
        source: String,
    },
    ImportLiked {
        source: String,
    },
    ImportRecent {
        source: String,
    },
    Artist {
        source: String,
        id: String,
    },
    Album {
        source: String,
        id: String,
    },
    Shows {
        source: String,
    },
    Artwork {
        source: String,
        url: String,
    },
    Show {
        source: String,
        id: String,
        #[serde(default)]
        offset: u64,
    },
    ImportPlaylist {
        source: String,
        playlist_id: String,
    },
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignInMethod {
    #[default]
    Qr,
    Web,
    Oauth,
}
