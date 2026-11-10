//! Native iPhone player: its own audio engine, queue, history and loopback MCP.
use musicx_core::protocol;
use nsh_core::prelude::*;
pub use ui::App;

#[cfg(target_os = "ios")]
#[path = "../../macos/panel/src/controls.rs"]
mod controls;
#[cfg(target_os = "ios")]
#[path = "../../macos/panel/src/library.rs"]
mod library;
#[path = "../../macos/panel/src/model.rs"]
mod model;
#[path = "../../macos/panel/src/remote.rs"]
mod remote;
#[cfg(target_os = "ios")]
#[path = "../../macos/panel/src/source_browser.rs"]
mod source_browser;
#[cfg(target_os = "ios")]
#[path = "../../macos/panel/src/sources_ui.rs"]
mod sources_ui;
#[path = "../../macos/panel/src/ui.rs"]
mod ui;

#[native_module("app/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

#[cfg(target_os = "ios")]
fn initialize() -> anyhow::Result<()> {
    nsh_core::image::budget().set_limit(4 * 1024 * 1024);
    use musicx_platform_apple::{mobile, player};
    static STARTED: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    STARTED
        .get_or_init(|| {
            let start = || -> anyhow::Result<()> {
                player::configure(&musicx_plugins::MYGO)?;
                musicx_mcp::start(musicx_mcp::Services {
                    submit: player::submit,
                    quit: || player::submit(player::Action::Pause, None),
                    sources: &musicx_plugins::MYGO,
                })?;
                mobile::start()
            };
            start().map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!(error.clone()))?;
    remote::start()
}

#[cfg(target_os = "macos")]
fn on_disconnected() {}
