//! Opt-in native regression: run the real panel on AppKit's main thread.
// Shared source files include libtest imports that this custom harness doesn't use.
#![allow(unused_imports)]
#[path = "../src/controls.rs"]
mod controls;
#[path = "../src/desktop.rs"]
#[allow(dead_code)] // The custom harness doesn't run embedded geometry unit tests.
mod desktop;
#[path = "../src/library.rs"]
mod library;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/remote.rs"]
#[allow(dead_code)] // Window lifecycle checks do not start a playback connection.
mod remote;
#[path = "../src/source_browser.rs"]
mod source_browser;
#[path = "../src/sources_ui.rs"]
mod sources_ui;
#[path = "../src/ui.rs"]
mod ui;
use musicx_protocol as protocol;
use nsh_platform_macos::objc2_app_kit::*;
use objc2::MainThreadMarker;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

fn on_disconnected() {
    dispatch2::DispatchQueue::main().exec_async(desktop::stop_panel);
}

fn main() {
    if std::env::var("MUSICX_NATIVE_UI_TEST").as_deref() != Ok("1") {
        println!("Skipped: set MUSICX_NATIVE_UI_TEST=1 in an interactive macOS session.");
        return;
    }
    let results = Arc::new(Mutex::new(Vec::new()));
    let observed = results.clone();
    let worker = std::thread::spawn(move || {
        for phase in ["launch", "minimized reopen", "hidden reopen"] {
            std::thread::sleep(Duration::from_secs(1));
            let observed = observed.clone();
            dispatch2::DispatchQueue::main().exec_async(move || {
                let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
                let window = app.windows().firstObject().expect("player window");
                let passed = app.isActive()
                    && window.isVisible()
                    && window.isKeyWindow()
                    && !window.isMiniaturized();
                observed.lock().unwrap().push((phase, passed));
                match phase {
                    "launch" => window.miniaturize(None),
                    "minimized reopen" => window.orderOut(None),
                    _ => {
                        desktop::stop_panel();
                        return;
                    }
                }
                let delegate = app.delegate().unwrap();
                assert!(!delegate.applicationShouldHandleReopen_hasVisibleWindows(&app, false));
            });
        }
    });
    assert_eq!(desktop::run_panel(), std::process::ExitCode::SUCCESS);
    worker.join().unwrap();
    let results = results.lock().unwrap();
    assert_eq!(results.len(), 3);
    for (phase, passed) in results.iter() {
        println!("{phase}: visible, active, key, not minimized = {passed}");
        assert!(passed, "{phase}");
    }
}
