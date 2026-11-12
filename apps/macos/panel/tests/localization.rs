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
use std::time::Duration;

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

fn exercise() {
    let directory = protocol::data_dir().unwrap();
    std::fs::write(directory.join("language"), "zh-CN\n").unwrap();
    protocol::i18n::refresh();
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
    app.press("English").unwrap();
    app.wait(Duration::from_millis(100)).unwrap();
    drop(app);
    let app = start();
    assert!(
        app.shows("Language"),
        "reopening settings must use the saved language"
    );
    println!("localization: live English/Chinese switching, Model preservation and reopen passed");
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
