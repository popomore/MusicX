//! Exercise the production settings language buttons in an isolated process.
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
    let mirror = model::Model::new();
    set_interval(Duration::from_millis(50), move || {
        mirror.refresh_mini_lyrics()
    });
    model.status.update(|status| status.volume = 0.42);
    view()
        .flex_1()
        .child(text(move || {
            model
                .status
                .with(|status| format!("volume:{}", status.volume))
        }))
        .child(text(move || {
            format!(
                "mini:{} mirror:{}",
                model.show_mini_lyrics.get(),
                mirror.show_mini_lyrics.get()
            )
        }))
        .child(dynamic(move || {
            model.language.get();
            untracked(|| sources_ui::settings_view(model))
        }))
}

fn start() -> Headless {
    let mut config = EngineConfig::new(Root::of::<LanguageScreen>());
    config.viewport = Size::new(px(704.0), px(520.0));
    config.environment.window = config.viewport;
    let mut app = Headless::start(config).unwrap();
    app.first_render().unwrap();
    app
}

#[component]
fn VerifiedSourceScreen() -> impl IntoElement {
    source_browser::sources_view(
        |_| {},
        || {},
        || {},
        Some(protocol::MusicSource {
            id: "mygo:qq-fixture".into(),
            name: "Fixture source".into(),
            version: "0.2.4".into(),
            can_install: true,
            verification: Some(protocol::SourceVerification {
                version: "0.2.4".into(),
                verified_at: "2026-11-15".into(),
                platform: "macos-aarch64".into(),
                checks: vec!["search".into(), "playback_sample".into()],
                limitations: vec!["preview_only".into()],
                installed_payload_matches: false,
            }),
            ..Default::default()
        }),
        source_browser::Start::Detail,
    )
}
fn verified_source_details() {
    for language in ["en", "zh-CN"] {
        std::fs::write(protocol::data_dir().unwrap().join("language"), language).unwrap();
        protocol::i18n::refresh();
        let mut config = EngineConfig::new(Root::of::<VerifiedSourceScreen>());
        config.viewport = Size::new(px(440.0), px(600.0));
        config.environment.window = config.viewport;
        let mut app = Headless::start(config).unwrap();
        app.first_render().unwrap();
        assert!(app.shows(&protocol::tr!("Trusted version {}", "0.2.4")));
        assert!(app.shows(protocol::tr!(
            "The tested track returned a 60-second preview"
        )));
        assert!(app.shows(protocol::tr!(
            "Full-track playback and account access still depend on the service"
        )));
    }
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
fn PlayerNavigationScreen() -> impl IntoElement {
    let model = model::Model::new();
    desktop::menus::connect(model);
    view().child(text(move || {
        match (model.page.get(), model.discovery.get()) {
            (model::Page::Discovery, Some(protocol::MusicSourceAction::Shows { source }))
                if model.drawer_open.get() =>
            {
                format!("Podcast source: {source}")
            }
            _ => "Player ready".into(),
        }
    }))
}

#[component]
fn SettingsNavigationScreen() -> impl IntoElement {
    let model = model::Model::new();
    view()
        .child(
            nsh_base::button()
                .on_press(|| {
                    desktop::menus::request_source(protocol::MusicSourceAction::Shows {
                        source: "qa-podcasts".into(),
                    })
                })
                .child(text("Browse podcasts")),
        )
        .child(text(move || {
            if model.page.get() == model::Page::Player && !model.drawer_open.get() {
                "Settings remains independent"
            } else {
                "Settings opened a player page"
            }
        }))
}

fn source_navigation() {
    let mut player =
        Headless::start(EngineConfig::new(Root::of::<PlayerNavigationScreen>())).unwrap();
    player.first_render().unwrap();
    let mut settings =
        Headless::start(EngineConfig::new(Root::of::<SettingsNavigationScreen>())).unwrap();
    settings.first_render().unwrap();
    settings.press("Browse podcasts").unwrap();
    player.wait_for("text:Podcast source: qa-podcasts").unwrap();
    assert!(settings.shows("Settings remains independent"));
}

fn settings_escape() {
    use nsh_core::{Command, desktop::DesktopCommand, platform::Os};
    let start = |root, width| {
        let mut config = EngineConfig::new(root);
        config.viewport = Size::new(px(width), px(520.0));
        config.environment.window = config.viewport;
        config.environment.os = Os::MacOs;
        let mut app = Headless::start(config).unwrap();
        app.first_render().unwrap();
        app
    };
    let player = start(Root::of::<ui::App>(), 320.0);
    let mut settings = start(Root::of::<ui::Settings>(), 704.0);
    let close = Command::Desktop(Box::new(DesktopCommand::Close));
    let close_count = |app: &Headless| {
        app.screen()
            .commands()
            .iter()
            .filter(|(_, command)| command == &close)
            .count()
    };
    for language in ["English", "简体中文"] {
        settings.press(language).unwrap();
        let before = close_count(&settings);
        settings.press_key("Escape").unwrap();
        assert_eq!(
            close_count(&settings),
            before + 1,
            "Escape must request closing the settings surface after selecting {language}"
        );
        assert!(
            !player.screen().sent(&close),
            "closing settings must leave the player surface open"
        );
    }
}

#[component]
fn SetupNavigationScreen() -> impl IntoElement {
    let model = model::Model::new();
    model.status_ready.set(true);
    view()
        .flex_1()
        .child(text(move || {
            if model.drawer_open.get() {
                "Details opened"
            } else {
                "Compact player"
            }
        }))
        .child(ui::desktop_ui::shell(model))
}

fn setup_navigation() {
    sources_ui::select_settings_group(sources_ui::SettingsGroup::Language);
    let mut config = EngineConfig::new(Root::of::<SetupNavigationScreen>());
    config.viewport = Size::new(px(320.0), px(540.0));
    config.environment.window = config.viewport;
    config.environment.os = nsh_core::platform::Os::MacOs;
    let mut player = Headless::start(config).unwrap();
    player.first_render().unwrap();
    player.press(protocol::tr!("Add music source")).unwrap();
    assert_eq!(
        sources_ui::selected_settings_group(),
        sources_ui::SettingsGroup::Sources,
        "adding a source must target standalone Settings, never the player drawer"
    );
    assert!(player.shows("Compact player"));
    player.press(protocol::tr!("Connect your agent")).unwrap();
    assert_eq!(
        sources_ui::selected_settings_group(),
        sources_ui::SettingsGroup::Agent
    );
    assert!(player.shows("Compact player"));
}

#[component]
fn PlayerEntryScreen() -> impl IntoElement {
    let model = model::Model::new();
    model.status_ready.set(true);
    effect(move || {
        let height = window().height.0;
        let mut track: protocol::Track = serde_json::from_value(serde_json::json!({
            "id": 1, "title": "Navigation fixture", "artist": "Alpha",
            "duration": 180.0, "source": {"plugin": "fixture", "id": "track-1"},
            "artists": [{"id": "alpha-id", "name": "Alpha"}]
        }))
        .unwrap();
        if (520.0..530.0).contains(&height) {
            track.artist = "Alpha / Beta".into();
            track.artists.push(protocol::Artist {
                id: "beta-id".into(),
                name: "Beta".into(),
                ..Default::default()
            });
        } else if (530.0..540.0).contains(&height) {
            track.artists.clear();
        } else if (540.0..550.0).contains(&height) {
            track.source = None;
        } else if height >= 550.0 {
            track.artists[0].id = " ".into();
        }
        model.status.update(|status| {
            status.track = Some(track);
            status.position = f64::from(height - 500.0);
        });
    });
    view()
        .flex_1()
        .child(ui::desktop_ui::shell(model))
        .child(text(move || {
            if let Some(protocol::MusicSourceAction::Artist { source, id }) = model.discovery.get()
                && model.page.get() == model::Page::Discovery
                && model.drawer_open.get()
            {
                format!("Artist destination: {source}/{id}")
            } else if model.page.get() == model::Page::Queue && model.drawer_open.get() {
                "Queue destination".into()
            } else {
                "Compact destination".into()
            }
        }))
}

fn player_entries() {
    for language in ["en", "zh-CN"] {
        std::fs::write(protocol::data_dir().unwrap().join("language"), language).unwrap();
        protocol::i18n::refresh();
        player_entry_checks();
    }
}

fn player_entry_checks() {
    let press_artist = |app: &mut Headless, name: &str| {
        app.press(&format!("label:{}", protocol::tr!("View artist {}", name)))
    };
    let mut config = EngineConfig::new(Root::of::<PlayerEntryScreen>());
    config.viewport = Size::new(px(628.0), px(512.0));
    config.environment.window = config.viewport;
    let mut app = Headless::start(config).unwrap();
    app.first_render().unwrap();
    app.press(&format!("label:{}", protocol::tr!("Show now playing")))
        .unwrap();
    assert!(
        app.shows("Queue destination"),
        "a track without an album still opens Now Playing"
    );
    press_artist(&mut app, "Alpha").unwrap();
    assert!(app.shows("Artist destination: mygo:fixture/alpha-id"));

    app.set_viewport(Size::new(px(628.0), px(520.0))).unwrap();
    press_artist(&mut app, "Alpha / Beta").unwrap();
    // A normal playback snapshot must preserve an open collaboration chooser.
    app.set_viewport(Size::new(px(628.0), px(521.0))).unwrap();
    press_artist(&mut app, "Beta").unwrap();
    assert!(app.shows("Artist destination: mygo:fixture/beta-id"));
    press_artist(&mut app, "Alpha / Beta").unwrap();
    press_artist(&mut app, "Alpha").unwrap();
    assert!(app.shows("Artist destination: mygo:fixture/alpha-id"));

    for height in [530.0, 540.0, 550.0] {
        app.set_viewport(Size::new(px(628.0), px(height))).unwrap();
        assert!(app.shows("Alpha"));
        assert!(
            press_artist(&mut app, "Alpha").is_err(),
            "missing artist/source identity must leave plain text"
        );
    }
    app.set_viewport(Size::new(px(628.0), px(512.0))).unwrap();
    press_artist(&mut app, "Alpha").unwrap();
    assert!(app.shows("Artist destination: mygo:fixture/alpha-id"));
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
    app.press("Playback").unwrap();
    assert!(app.shows("Volume normalization"));
    assert!(app.shows("Lyric preview"));
    assert!(
        !app.shows("English"),
        "only the selected settings group is shown"
    );
    app.press("Lyric preview").unwrap();
    app.wait_for("text:mini:false mirror:false").unwrap();
    app.press("Music sources").unwrap();
    assert!(app.shows("Automatically update plugins"));
    app.press("Agent").unwrap();
    assert!(app.shows("Copy MCP config"));
    assert!(app.shows("Codex"));
    assert!(app.shows("Claude Code"));
    assert!(!app.shows("Lyric preview"));
    app.press("Version & updates").unwrap();
    assert!(app.shows("Check for updates"));
    assert!(!app.shows("Copy MCP config"));
    sources_ui::select_settings_group(sources_ui::SettingsGroup::Agent);
    app.wait(Duration::from_millis(150)).unwrap();
    assert!(
        app.shows("Copy MCP config"),
        "contextual setup opens the MCP group"
    );
    app.press("Language").unwrap();
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
    drop(app);
    source_navigation();
    verified_source_details();
    settings_escape();
    setup_navigation();
    player_entries();
    println!(
        "localization: real App layout, settings sidebar, resizing, language refresh, system default, manual switching, Model preservation, reopen, scoped Escape closing and setup navigation passed"
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
