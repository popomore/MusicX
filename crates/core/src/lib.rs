//! Shared Rust playback, catalog, history and MCP core for macOS and iPhone.
mod account_ui;
pub mod audio_quality;
pub mod catalog;
#[cfg(target_os = "macos")]
pub mod desktop;
pub mod history;
pub mod local_library;
pub mod mcp;
#[cfg(target_os = "ios")]
pub mod mobile;
pub mod music_sources;
pub mod mygo;
pub mod netease;
pub mod player;
pub use musicx_protocol as protocol;
mod system_media;
mod web_auth;

pub fn quit() {
    #[cfg(target_os = "macos")]
    desktop::quit();
    #[cfg(target_os = "ios")]
    player::submit(player::Action::Pause, None);
}
