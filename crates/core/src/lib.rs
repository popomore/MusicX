//! Playback state, local storage and library logic shared by platform adapters.
pub mod audio;
pub mod audio_quality;
pub mod history;
pub mod local_library;
pub mod metadata;
pub mod player;
mod session;
pub mod storage;

pub use musicx_protocol as protocol;
