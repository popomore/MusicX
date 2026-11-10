mod controls;
mod desktop;
mod drawer;
mod library;
mod model;
use musicx_protocol as protocol;
mod remote;
mod source_browser;
mod sources_ui;
mod ui;
fn on_disconnected() {
    dispatch2::DispatchQueue::main().exec_async(desktop::stop_panel);
}
fn main() -> std::process::ExitCode {
    if let Err(error) = remote::start() {
        eprintln!("{error:#}");
        return std::process::ExitCode::FAILURE;
    }
    desktop::run_panel()
}
