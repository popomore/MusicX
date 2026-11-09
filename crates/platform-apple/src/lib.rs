//! Apple audio, native media controls and the macOS background application.
#[cfg(target_os = "macos")]
pub mod desktop;
#[cfg(target_os = "ios")]
pub mod mobile;
pub mod player;
mod system_media;
