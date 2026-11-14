//! Opt-in native regression: run the real panel on AppKit's main thread.
// Shared source files include libtest imports that this custom harness doesn't use.
#![allow(unused_imports)]
#[path = "../src/agent_setup.rs"]
#[allow(dead_code)] // This custom harness does not run embedded configuration unit tests.
mod agent_setup;
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
use objc2_foundation::{NSPoint, NSString};
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
    let window = app
        .windows()
        .iter()
        .find(|window| window.title().to_string() == protocol::APP_NAME)
        .expect("player window");
    (app, window)
}

fn wait_for(predicate: impl Fn(&NSApplication, &NSWindow) -> bool + Copy + Send + 'static) -> bool {
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
        anyhow::ensure!(
            wait_for(predicate),
            "{phase} timed out: {}",
            on_main(|| {
                let (app, window) = app_window();
                format!(
                    "active={} visible={} key={} minimized={} frame={:?}",
                    app.isActive(),
                    window.isVisible(),
                    window.isKeyWindow(),
                    window.isMiniaturized(),
                    window.frame()
                )
            })
        );
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
    passed.extend(menu_checks()?);
    Ok(passed)
}

fn menu_command(command: desktop::menus::Command) -> Retained<NSMenuItem> {
    app_window()
        .0
        .mainMenu()
        .unwrap()
        .itemArray()
        .iter()
        .filter_map(|item| item.submenu())
        .find_map(|menu| menu.itemWithTag(command as isize))
        .expect("native menu command")
}

fn shortcut(key: &str, key_code: u16) -> bool {
    let (app, window) = app_window();
    let key = NSString::from_str(key);
    let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown, NSPoint::new(0.0, 0.0), NSEventModifierFlags::Command,
        0.0, window.windowNumber(), None, &key, &key, false, key_code,
    ).unwrap();
    app.mainMenu().unwrap().performKeyEquivalent(&event)
}

fn menu_checks() -> anyhow::Result<Vec<&'static str>> {
    use desktop::menus::Command;
    direction_checks()?;
    anyhow::ensure!(
        wait_for(|_, _| menu_command(Command::Settings).isEnabled()),
        "menu bridge not ready"
    );
    anyhow::ensure!(
        on_main(|| {
            let app = app_window().0;
            let bar = app.mainMenu().unwrap();
            ["MusicX", "Edit", "Playback", "View", "Window", "Help"]
                .iter()
                .enumerate()
                .all(|(index, title)| {
                    bar.itemAtIndex(index as isize).unwrap().title().to_string()
                        == protocol::i18n::text(title)
                })
                && app.servicesMenu().is_some()
                && app.windowsMenu().is_some()
                && app.helpMenu().is_some()
        }),
        "standard menu registration"
    );
    anyhow::ensure!(
        on_main(|| {
            [
                Command::TogglePlayback,
                Command::Previous,
                Command::Next,
                Command::Lyrics,
            ]
            .iter()
            .all(|command| !menu_command(*command).isEnabled())
        }),
        "empty player enabled unavailable actions"
    );
    anyhow::ensure!(
        on_main(|| shortcut(",", 43)),
        "settings shortcut not handled"
    );
    anyhow::ensure!(
        wait_for(|_, window| {
            window.frame().size.width == f64::from(desktop::PLAYER_WIDTH)
                && settings_window().is_some_and(|settings| settings.isVisible())
        }),
        "settings shortcut did not open an independent panel: {}",
        on_main(|| app_window()
            .0
            .windows()
            .iter()
            .map(|window| format!(
                "title={:?} visible={} key={} frame={:?}",
                window.title().to_string(),
                window.isVisible(),
                window.isKeyWindow(),
                window.frame()
            ))
            .collect::<Vec<_>>()
            .join("; "))
    );
    let settings_id = on_main(|| settings_window().unwrap().windowNumber());
    on_main(|| desktop::menus::perform(Command::Settings as isize));
    anyhow::ensure!(
        on_main(move || settings_window().unwrap().windowNumber() == settings_id),
        "settings opened a duplicate window"
    );
    on_main(|| app_window().1.miniaturize(None));
    anyhow::ensure!(
        wait_for(|_, window| window.isMiniaturized()),
        "player did not minimize"
    );
    on_main(|| settings_window().unwrap().performClose(None));
    anyhow::ensure!(
        wait_for(|_, window| window.isMiniaturized() && settings_window().is_none()),
        "closing settings destroyed the minimized player"
    );
    on_main(|| desktop::menus::perform(Command::Settings as isize));
    anyhow::ensure!(
        wait_for(|_, window| window.isMiniaturized() && settings_window().is_some()),
        "settings did not reopen beside the minimized player"
    );
    on_main(reopen);
    anyhow::ensure!(
        wait_for(|_, window| !window.isMiniaturized() && window.isVisible()),
        "minimized player failed to reopen"
    );
    on_main(|| app_window().1.performClose(None));
    anyhow::ensure!(
        wait_for(|_, window| !window.isVisible() && settings_window().is_some()),
        "closing the player closed settings"
    );
    on_main(reopen);
    anyhow::ensure!(
        wait_for(|_, window| window.isVisible() && !window.isMiniaturized()),
        "player could not reopen while settings remained open"
    );
    on_main(|| settings_window().unwrap().performClose(None));
    anyhow::ensure!(
        wait_for(|_, window| settings_window().is_none() && window.isVisible()),
        "closing settings affected the player"
    );
    on_main(|| desktop::menus::perform(Command::Settings as isize));
    anyhow::ensure!(
        wait_for(|_, _| settings_window().is_some_and(|window| window.isVisible())),
        "settings failed to reopen"
    );
    on_main(|| {
        let item = menu_command(Command::History);
        // The item and its owning menu remain alive on AppKit's main thread.
        let menu = unsafe { item.menu() }.unwrap();
        menu.performActionForItemAtIndex(menu.indexOfItem(&item));
    });
    anyhow::ensure!(
        wait_for(|_, _| menu_command(Command::History).state() == NSControlStateValueOn),
        "history menu did not navigate"
    );
    let original = protocol::i18n::Language::load();
    protocol::i18n::Language::English.save()?;
    anyhow::ensure!(
        wait_for(|app, _| {
            app.mainMenu()
                .unwrap()
                .itemAtIndex(2)
                .unwrap()
                .title()
                .to_string()
                == "Playback"
                && menu_command(Command::Settings).title().to_string() == "Settings…"
        }),
        "English menu refresh"
    );
    protocol::i18n::Language::SimplifiedChinese.save()?;
    anyhow::ensure!(
        wait_for(|app, _| {
            app.mainMenu()
                .unwrap()
                .itemAtIndex(2)
                .unwrap()
                .title()
                .to_string()
                == "播放"
                && menu_command(Command::Settings).title().to_string() == "设置…"
        }),
        "Chinese menu refresh"
    );
    original.save()?;
    on_main(|| settings_window().unwrap().performClose(None));
    anyhow::ensure!(
        wait_for(|_, window| settings_window().is_none() && window.isVisible()),
        "settings did not finish closing"
    );
    Ok(vec![
        "closing settings preserves the minimized player",
        "hint direction follows native player moves",
        "standard menus",
        "empty playback actions disabled",
        "settings shortcut opens an independent reusable panel",
        "view menu navigates details",
        "English menu refresh",
        "Chinese menu refresh",
    ])
}

fn direction_checks() -> anyhow::Result<()> {
    let original = on_main(|| app_window().1.frame());
    for left in [false, true] {
        on_main(move || {
            let window = app_window().1;
            let visible = window.screen().unwrap().visibleFrame();
            let x = if left {
                visible.origin.x + visible.size.width - window.frame().size.width - 24.0
            } else {
                visible.origin.x + 24.0
            };
            window.setFrameOrigin(NSPoint::new(x, window.frame().origin.y));
        });
        anyhow::ensure!(
            wait_for(move |_, _| desktop::preferred_details_left() == left),
            "hint did not follow the native window move"
        );
        let chosen = desktop::request_details_left().blocking_recv()?;
        anyhow::ensure!(
            chosen == left,
            "hint disagreed with the native opening direction"
        );
    }
    on_main(move || app_window().1.setFrame_display(original, true));
    Ok(())
}

fn settings_window() -> Option<Retained<NSWindow>> {
    use protocol::i18n::{Locale, text_for};
    app_window().0.windows().iter().find(|window| {
        let title = window.title().to_string();
        window.isVisible()
            && [Locale::English, Locale::SimplifiedChinese]
                .into_iter()
                .any(|locale| title == text_for(locale, "Settings"))
    })
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
    if std::env::args().any(|argument| argument == "--preview") {
        assert_eq!(desktop::run_panel(), std::process::ExitCode::SUCCESS);
        return;
    }
    let menus_only = std::env::args().any(|argument| argument == "--menus-only");
    let worker = std::thread::spawn(move || {
        let result = if menus_only {
            menu_checks()
        } else {
            lifecycle_checks()
        };
        let result = result.and_then(|mut passed| {
            anyhow::ensure!(
                on_main(|| {
                    let window = app_window().1;
                    shortcut("w", 13) && !window.isVisible()
                }),
                "close window shortcut did not close the player"
            );
            passed.push("close window shortcut");
            Ok(passed)
        });
        if result.is_err() {
            on_main(desktop::stop_panel);
        }
        result
    });
    assert_eq!(desktop::run_panel(), std::process::ExitCode::SUCCESS);
    let passed = worker.join().unwrap().unwrap();
    assert_eq!(passed.len(), if menus_only { 9 } else { 19 });
    for phase in passed {
        println!("{phase}: passed");
    }
}
