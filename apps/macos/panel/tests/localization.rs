//! Exercise the production settings language buttons in an isolated process.
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
#[allow(dead_code)] // Settings checks do not start a playback connection.
mod remote;
#[path = "../src/source_browser.rs"]
mod source_browser;
#[path = "../src/sources_ui.rs"]
mod sources_ui;
#[path = "../src/ui.rs"]
mod ui;
use ui::spectrum_ui;
#[path = "../src/updates.rs"]
mod updates;
use musicx_protocol as protocol;
use nsh_core::{Size, element::Root, prelude::*};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

fn on_disconnected() {}

#[component]
fn LanguageScreen() -> impl IntoElement {
    let model = model::Model::new();
    model.status.update(|status| status.volume = 0.42);
    view()
        .flex_1()
        .child(text(move || {
            model
                .status
                .with(|status| format!("volume:{}", status.volume))
        }))
        .child(dynamic(move || {
            model.language.get();
            untracked(|| sources_ui::settings_view(model))
        }))
}

fn start() -> Headless {
    let mut config = EngineConfig::new(Root::of::<LanguageScreen>());
    config.viewport = Size::new(px(296.0), px(512.0));
    config.environment.window = config.viewport;
    let mut app = Headless::start(config).unwrap();
    app.first_render().unwrap();
    app
}

fn player_layout() {
    let mut config = EngineConfig::new(Root::of::<ui::App>());
    config.viewport = Size::new(px(320.0), px(512.0));
    config.environment.window = config.viewport;
    let mut player = Headless::start(config).unwrap();
    let root = Arc::new(Mutex::new((None, None)));
    let observed_root = root.clone();
    player.observe(move |_, batch| {
        let mut root = observed_root.lock().unwrap();
        for item in &batch.items {
            match item {
                nsh_core::MountItem::Insert {
                    parent: nsh_core::ROOT_ID,
                    child,
                    ..
                } => {
                    *root = (Some(*child), None);
                }
                nsh_core::MountItem::Frame { id, frame } if root.0 == Some(*id) => {
                    root.1 = Some(frame.size);
                }
                _ => {}
            }
        }
    });
    player.first_render().unwrap();
    assert_eq!(
        root.lock().unwrap().1,
        Some(Size::new(px(320.0), px(512.0))),
        "the real application root must fill the window so the player body is not clipped"
    );
    for (language, width, height) in [("en", 628.0, 512.0), ("zh-CN", 320.0, 640.0)] {
        std::fs::write(protocol::data_dir().unwrap().join("language"), language).unwrap();
        protocol::i18n::refresh();
        let size = Size::new(px(width), px(height));
        player.set_viewport(size).unwrap();
        player.wait(Duration::from_millis(600)).unwrap();
        assert_eq!(
            root.lock().unwrap().1,
            Some(size),
            "localized root must follow the window size"
        );
        assert!(
            player.shows(protocol::tr!("Connecting to player…")),
            "the player body must stay visible after resizing and changing language"
        );
    }
}

#[component]
fn SpectrumScreen() -> impl IntoElement {
    use nsh_base::button;
    use protocol::spectrum::{Spectrum, SpectrumState};
    let model = model::Model::new();
    model.status.update(|s| {
        s.state = protocol::State::Playing;
        s.track = Some(
            serde_json::from_value(serde_json::json!({
                "id": 1, "title": "Spectrum fixture", "artist": "MusicX", "duration": 8.0
            }))
            .unwrap(),
        );
    });
    let frame = signal(Spectrum {
        state: SpectrumState::Live,
        track_key: "netease:1".into(),
        sample_rate: 48_000.0,
        bands: [-12.0; 32],
    });
    let disconnected = signal(false);
    view()
        .flex_1()
        .px(px(20.0))
        .child(
            button()
                .on_press(move || model.status.update(|s| s.state = protocol::State::Paused))
                .child(text("Pause fixture")),
        )
        .child(
            button()
                .on_press(move || {
                    model.status.update(|s| {
                        s.state = protocol::State::Playing;
                        s.track.as_mut().unwrap().id = 2;
                    })
                })
                .child(text("Switch fixture")),
        )
        .child(
            button()
                .on_press(move || {
                    frame.update(|s| {
                        s.track_key = "netease:2".into();
                        s.state = SpectrumState::Unavailable;
                    });
                })
                .child(text("Unsupported fixture")),
        )
        .child(
            button()
                .on_press(move || disconnected.set(true))
                .child(text("Disconnect fixture")),
        )
        .child(spectrum_ui::content(model, frame, disconnected))
}

fn spectrum_layout() {
    let mut config = EngineConfig::new(Root::of::<SpectrumScreen>());
    config.viewport = Size::new(px(296.0), px(512.0));
    config.environment.window = config.viewport;
    let mut app = Headless::start(config).unwrap();
    let rectangles = Arc::new(Mutex::new(0));
    let observed = rectangles.clone();
    app.observe(move |_, batch| {
        for item in &batch.items {
            if let nsh_core::MountItem::Canvas { drawing, .. } = item {
                *observed.lock().unwrap() = drawing
                    .ops
                    .iter()
                    .filter(|op| matches!(op, nsh_core::canvas::CanvasOp::FillRect(_)))
                    .count();
            }
        }
    });
    app.first_render().unwrap();
    assert!(app.shows("Live audio · before volume adjustment"));
    assert_eq!(
        *rectangles.lock().unwrap(),
        37,
        "five grid lines plus 32 actual spectrum bars"
    );
    app.press("Pause fixture").unwrap();
    assert!(app.shows("Paused"));
    assert_eq!(
        *rectangles.lock().unwrap(),
        5,
        "pause must clear all bars immediately"
    );
    app.press("Switch fixture").unwrap();
    assert!(app.shows("Waiting for audio…"));
    assert_eq!(
        *rectangles.lock().unwrap(),
        5,
        "an old track's frame must never leak into another track"
    );
    app.press("Unsupported fixture").unwrap();
    assert!(app.shows("Spectrum is unavailable for this stream"));
    app.press("Disconnect fixture").unwrap();
    assert!(app.shows("Spectrum connection lost"));
    println!(
        "spectrum UI: native canvas, live bars, pause, track identity, unsupported stream and disconnection passed"
    );
}

fn exercise() {
    let directory = protocol::data_dir().unwrap();
    assert_eq!(
        protocol::i18n::Language::load(),
        protocol::i18n::Language::System
    );
    std::fs::write(directory.join("language"), "zh-CN\n").unwrap();
    protocol::i18n::refresh();
    player_layout();
    let mut app = start();
    assert!(app.shows("语言"));
    app.press("English").unwrap();
    app.wait(Duration::from_millis(100)).unwrap();
    assert!(app.shows("Language"));
    assert!(app.shows("Version & updates"));
    assert!(!app.shows("版本与更新"));
    assert!(
        app.shows("volume:0.42"),
        "language changes must preserve Model state"
    );
    assert_eq!(
        std::fs::read_to_string(directory.join("language")).unwrap(),
        "en\n"
    );
    app.press("简体中文").unwrap();
    app.wait(Duration::from_millis(100)).unwrap();
    assert!(app.shows("版本与更新"));
    app.press(protocol::i18n::Language::System.label()).unwrap();
    app.wait(Duration::from_millis(100)).unwrap();
    assert_eq!(
        std::fs::read_to_string(directory.join("language")).unwrap(),
        "system\n"
    );
    assert_eq!(
        protocol::i18n::Language::load(),
        protocol::i18n::Language::System
    );
    app.press("English").unwrap();
    app.wait(Duration::from_millis(100)).unwrap();
    drop(app);
    let app = start();
    assert!(
        app.shows("Language"),
        "reopening settings must use the saved language"
    );
    println!(
        "localization: real App layout, resizing, language refresh, system default, manual switching, Model preservation and reopen passed"
    );
    spectrum_layout();
}

fn main() {
    if std::env::var_os("MUSICX_LOCALIZATION_CHILD").is_some() {
        exercise();
        return;
    }
    let directory = std::env::temp_dir().join(format!("musicx-ui-language-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .env("MUSICX_LOCALIZATION_CHILD", "1")
        .env("MUSICX_DATA_DIR", &directory)
        .env_remove("MUSICX_LANGUAGE")
        .status()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        result.success(),
        "isolated localization UI check failed: {result}"
    );
}
