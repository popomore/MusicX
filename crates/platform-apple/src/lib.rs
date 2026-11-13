//! Apple audio, native media controls and the macOS background application.
#[cfg(target_os = "macos")]
pub mod desktop;
#[cfg(target_os = "ios")]
pub mod mobile;
pub mod player;
pub mod spectrum;
mod system_media;
#[cfg(target_os = "macos")]
mod updater;
