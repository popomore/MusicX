//! Upgrade compatibility only. New code and documentation use MusicX names.
use std::ffi::OsString;

pub(super) const DATA_DIR_NAME: &str = "Mini Cloud Music";

pub(super) fn env_var(name: &str) -> Option<OsString> {
    let legacy = match name {
        "MUSICX_DATA_DIR" => "MINI_CLOUDMUSIC_DATA_DIR",
        "MUSICX_PORT" => "MINI_CLOUDMUSIC_PORT",
        "MUSICX_WEB_AUTH" => "MINI_CLOUDMUSIC_WEB_AUTH",
        _ => return None,
    };
    std::env::var_os(legacy)
}
