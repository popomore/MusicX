//! Native iPhone player: its own audio engine, queue, history and loopback MCP.
use nsh_core::prelude::*;
use mini_cloudmusic::protocol;
pub use ui::App;

#[cfg(target_os = "ios")]
mod library;
#[path = "../../macos/panel/src/model.rs"]
mod model;
#[path = "../../macos/panel/src/remote.rs"]
mod remote;
#[path = "../../macos/panel/src/ui.rs"]
mod ui;

#[native_module("app/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

#[cfg(target_os = "ios")]
fn initialize() -> anyhow::Result<()> {
    nsh_core::image::budget().set_limit(4 * 1024 * 1024);
    mini_cloudmusic::mobile::start().and_then(|_| remote::start())
}

#[cfg(target_os = "macos")]
fn on_disconnected() {}
