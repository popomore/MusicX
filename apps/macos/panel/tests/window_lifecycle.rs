//! Opt-in native regression: run the real panel on AppKit's main thread.
// Shared source files include libtest imports that this custom harness doesn't use.
#![allow(unused_imports)]
#[path = "../src/controls.rs"]
mod controls;
#[path = "../src/desktop.rs"]
#[allow(dead_code)] // The custom harness doesn't run embedded geometry unit tests.
mod desktop;
#[path = "../src/discovery_ui.rs"]
mod discovery_ui;
#[path = "../src/drawer.rs"]
mod drawer;
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
#[path = "../src/updates.rs"]
mod updates;
use musicx_protocol as protocol;
use nsh_platform_macos::objc2_app_kit::*;
use objc2::{MainThreadMarker, rc::Retained};
use std::{sync::mpsc, time::Duration};

fn on_disconnected() {
    dispatch2::DispatchQueue::main().exec_async(desktop::stop_panel);
}

fn on_main<T: Send + 'static>(action: impl FnOnce() -> T + Send + 'static) -> T {
    let (send, receive) = mpsc::sync_channel(1);
    dispatch2::DispatchQueue::main().exec_async(move || {
        let _ = send.send(action());
    });
    receive
        .recv_timeout(Duration::from_secs(10))
        .expect("AppKit main thread timed out")
}

fn app_window() -> (Retained<NSApplication>, Retained<NSWindow>) {
    let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
    let window = app.windows().firstObject().expect("player window");
    (app, window)
}

fn wait_for(predicate: fn(&NSApplication, &NSWindow) -> bool) -> bool {
    for _ in 0..100 {
        if on_main(move || {
            let (app, window) = app_window();
            predicate(&app, &window)
        }) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn front(app: &NSApplication, window: &NSWindow) -> bool {
    app.isActive() && window.isVisible() && window.isKeyWindow() && !window.isMiniaturized()
}

fn reopen() {
    let (app, _) = app_window();
    assert!(
        !app.delegate()
            .unwrap()
            .applicationShouldHandleReopen_hasVisibleWindows(&app, false)
    );
}

fn reopen_from_helper() {
    // Exercise the real helper sender in another process, not an in-process
    // delegate call, so notification routing is part of the regression.
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--reveal-panel", &std::process::id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}

fn lifecycle_checks() -> anyhow::Result<Vec<&'static str>> {
    let mut passed = Vec::new();
    let mut check = |phase, predicate: fn(&NSApplication, &NSWindow) -> bool| {
        anyhow::ensure!(wait_for(predicate), "{phase} timed out");
        passed.push(phase);
        Ok::<_, anyhow::Error>(())
    };
    check("launch", front)?;
    on_main(|| app_window().1.miniaturize(None));
    // Confirm the Dock transition before requesting reopen; a delayed animation
    // must fail this precondition instead of yielding a false positive.
    check("confirmed minimized", |_, window| window.isMiniaturized())?;
    on_main(|| app_window().0.hide(None));
    check("inactive before minimized reopen", |app, _| !app.isActive())?;
    on_main(reopen);
    check("minimized reopen", front)?;
    on_main(|| {
        let (app, window) = app_window();
        window.orderOut(None);
        app.hide(None);
    });
    check("inactive hidden window", |app, window| {
        !app.isActive() && !window.isVisible()
    })?;
    on_main(reopen);
    check("hidden reopen", front)?;
    on_main(|| {
        let (app, window) = app_window();
        window.miniaturize(None);
        app.hide(None);
    });
    check("hidden minimized before helper reopen", |app, window| {
        !app.isActive() && window.isMiniaturized()
    })?;
    on_main(reopen_from_helper);
    check("helper minimized reopen", front)?;
    on_main(|| {
        let (app, window) = app_window();
        window.orderOut(None);
        app.hide(None);
    });
    check("hidden before helper reopen", |app, window| {
        !app.isActive() && !window.isVisible()
    })?;
    on_main(|| musicx_platform_apple::desktop::reveal_panel(0));
    std::thread::sleep(Duration::from_millis(200));
    anyhow::ensure!(
        on_main(|| !app_window().1.isVisible()),
        "another PID reopened this panel"
    );
    on_main(reopen_from_helper);
    check("helper hidden reopen", front)?;
    Ok(passed)
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--reveal-panel") {
        let process_id = std::env::args().nth(2).unwrap().parse().unwrap();
        musicx_platform_apple::desktop::reveal_panel(process_id);
        return;
    }
    if std::env::var("MUSICX_NATIVE_UI_TEST").as_deref() != Ok("1") {
        println!("Skipped: set MUSICX_NATIVE_UI_TEST=1 in an interactive macOS session.");
        return;
    }
    let worker = std::thread::spawn(|| {
        let result = lifecycle_checks();
        on_main(desktop::stop_panel);
        result
    });
    assert_eq!(desktop::run_panel(), std::process::ExitCode::SUCCESS);
    let passed = worker.join().unwrap().unwrap();
    assert_eq!(passed.len(), 10);
    for phase in passed {
        println!("{phase}: passed");
    }
}
