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
