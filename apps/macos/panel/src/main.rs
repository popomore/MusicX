mod artwork;
mod controls;
mod desktop;
mod discovery_ui;
mod drawer;
mod library;
mod model;
use musicx_protocol as protocol;
mod remote;
mod source_browser;
mod sources_ui;
mod ui;
mod updates;
fn on_disconnected() {
    dispatch2::DispatchQueue::main().exec_async(desktop::stop_panel);
}
fn main() -> std::process::ExitCode {
    artwork::install();
    if let Err(error) = remote::start() {
        eprintln!("{error:#}");
        return std::process::ExitCode::FAILURE;
    }
    desktop::run_panel()
}
