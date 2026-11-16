//! MusicX source contracts. Adapter export names are implementation details.
use crate::{Artist, MusicSource, PlaylistOrigin, SourceVerification, Track};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceCollectionKind {
    #[default]
    #[serde(rename = "playlist")]
    Playlist,
    #[serde(rename = "album")]
    Album,
    #[serde(rename = "artist")]
    Artist,
    #[serde(rename = "show")]
    Podcast,
    #[serde(rename = "station")]
    Station,
    #[serde(rename = "page")]
    BrowsePage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SourceCapability {
    #[serde(rename = "search")]
    Search,
    #[serde(rename = "stream")]
    Playback,
    #[serde(rename = "lyrics")]
    Lyrics,
    #[serde(rename = "album")]
    Album,
    #[serde(rename = "artist")]
    Artist,
    #[serde(rename = "account")]
    Account,
    #[serde(rename = "signIn")]
    SignIn,
    #[serde(rename = "signOut")]
    SignOut,
    #[serde(rename = "setSetting")]
    Settings,
    #[serde(rename = "playlists")]
    Playlists,
    #[serde(rename = "playlist")]
    Playlist,
    #[serde(rename = "liked")]
    LikedTracks,
    #[serde(rename = "recent")]
    RecentTracks,
    #[serde(rename = "shows")]
    Podcasts,
    #[serde(rename = "show")]
    PodcastEpisodes,
    #[serde(rename = "home")]
    Recommendations,
    #[serde(rename = "browse")]
    Browse,
    #[serde(rename = "stations")]
    Stations,
    #[serde(rename = "stationNext")]
    StationContinuation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SourceSearchKind {
    #[default]
    Tracks,
    Albums,
    Artists,
    Playlists,
    Shows,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSearchRequest {
    pub query: String,
    pub kind: SourceSearchKind,
    pub offset: u64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceSearchTrack {
    #[serde(flatten)]
    pub track: Track,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<bool>,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceSearchEntity {
    pub id: String,
    pub name: String,
    #[serde(default, rename = "imageUrl", skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artists: Vec<Artist>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(
        default,
        rename = "trackCount",
        skip_serializing_if = "Option::is_none"
    )]
    pub track_count: Option<u64>,
    #[serde(
        default,
        rename = "releaseDate",
        skip_serializing_if = "Option::is_none"
    )]
    pub release_date: Option<String>,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceSearchPage {
    pub source: String,
    pub tracks: Vec<SourceSearchTrack>,
    pub albums: Vec<SourceSearchEntity>,
    pub artists: Vec<SourceSearchEntity>,
    pub playlists: Vec<SourceSearchEntity>,
    pub shows: Vec<SourceSearchEntity>,
    pub more: bool,
}

/// Legacy JSON method/params keys remain stable; callers select Rust operations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "method", content = "params")]
pub enum SourcePageRequest {
    #[serde(rename = "home")]
    Recommendations {},
    #[serde(rename = "browse")]
    Browse { page: String },
    #[serde(rename = "stations")]
    Stations {},
    #[serde(rename = "stationNext")]
    StationTracks {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    #[serde(rename = "playlist")]
    Playlist { id: String },
    #[serde(rename = "artist")]
    Artist { id: String },
    #[serde(rename = "album")]
    Album { id: String },
    #[serde(rename = "shows")]
    Podcasts {},
    #[serde(rename = "show")]
    PodcastEpisodes {
        id: String,
        #[serde(default)]
        offset: u64,
    },
}

// Older clients may omit params or send null for parameterless pages. Decode
// that transport shape into closed, typed operations before reaching an adapter.
impl<'de> Deserialize<'de> for SourcePageRequest {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        enum Method {
            Home,
            Browse,
            Stations,
            StationNext,
            Playlist,
            Artist,
            Album,
            Shows,
            Show,
        }
        #[derive(Default, Deserialize)]
        #[serde(default)]
        struct Params {
            id: Option<String>,
            page: Option<String>,
            name: Option<String>,
            offset: Option<u64>,
        }
        #[derive(Deserialize)]
        struct Wire {
            method: Method,
            #[serde(default)]
            params: Option<Params>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let params = wire.params.unwrap_or_default();
        let required =
            |value: Option<String>, key| value.ok_or_else(|| D::Error::missing_field(key));
        Ok(match wire.method {
            Method::Home => Self::Recommendations {},
            Method::Browse => Self::Browse {
                page: required(params.page, "page")?,
            },
            Method::Stations => Self::Stations {},
            Method::StationNext => Self::StationTracks {
                id: required(params.id, "id")?,
                name: params.name,
            },
            Method::Playlist => Self::Playlist {
                id: required(params.id, "id")?,
            },
            Method::Artist => Self::Artist {
                id: required(params.id, "id")?,
            },
            Method::Album => Self::Album {
                id: required(params.id, "id")?,
            },
            Method::Shows => Self::Podcasts {},
            Method::Show => Self::PodcastEpisodes {
                id: required(params.id, "id")?,
                offset: params.offset.unwrap_or(0),
            },
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourcePersonalCollection {
    LikedTracks,
    RecentTracks,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SourcePlaylistSnapshot {
    pub origin: PlaylistOrigin,
    pub tracks: Vec<Track>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceConfiguration {
    pub source: MusicSource,
    pub warning: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceUpdateCheck {
    pub available: bool,
    pub installed_version: String,
    pub latest_version: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub repository: Option<String>,
    pub network: Vec<String>,
    pub catalog_supported: bool,
    pub verification: Option<SourceVerification>,
}
