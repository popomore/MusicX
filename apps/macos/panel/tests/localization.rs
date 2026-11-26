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
fn MiniPreviewRaceScreen() -> impl IntoElement {
    let model = model::Model::new();
    let mirror = model::Model::new();
    model.refresh_mini_lyrics();
    mirror.refresh_mini_lyrics();
    view()
        .child(text(move || {
            format!(
                "mini:{} mirror:{}",
                model.show_mini_lyrics.get(),
                mirror.show_mini_lyrics.get()
            )
        }))
        .child(controls::action(
            "Toggle preview",
            || false,
            move || model.toggle_mini_lyrics(),
        ))
        .child(controls::action(
            "Refresh previews",
            || false,
            move || {
                model.refresh_mini_lyrics();
                mirror.refresh_mini_lyrics();
            },
        ))
}

fn mini_preview_race() {
    let dir = protocol::data_dir().unwrap();
    std::fs::write(dir.join("panel-preferences.json"), "true").unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(".preferences.lock"))
        .unwrap();
    lock.lock().unwrap();
    let mut app = Headless::start(EngineConfig::new(Root::of::<MiniPreviewRaceScreen>())).unwrap();
    app.first_render().unwrap();
    app.press("Toggle preview").unwrap();
    app.press("Refresh previews").unwrap();
    assert!(
        app.shows("mini:false mirror:false"),
        "A delayed local write must keep the selected preview value across models"
    );
    app.press("Toggle preview").unwrap();
    app.press("Refresh previews").unwrap();
    assert!(app.shows("mini:true mirror:true"));
    drop(lock);
    app.wait(Duration::from_millis(200)).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("panel-preferences.json")).unwrap(),
        "true"
    );
}

#[component]
fn HistoryRaceScreen() -> impl IntoElement {
    let model = model::Model::new();
    model.page.set(model::Page::History);
    model.load();
    view()
        .child(text(move || {
            model.history.with(|rows| {
                rows.first()
                    .map(|(_, entry)| entry.track.title.clone())
                    .unwrap_or_else(|| "Loading history".into())
            })
        }))
        .child(controls::action(
            "History changed",
            || false,
            move || {
                model.update(protocol::Status {
                    history_revision: 1,
                    ..Default::default()
                })
            },
        ))
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
        config.environment.os = nsh_core::platform::Os::MacOs;
        let mut app = Headless::start(config).unwrap();
        app.first_render().unwrap();
        let label = protocol::tr!("Trusted version {}", "0.2.4");
        assert!(
            !app.shows(&label),
            "trust uses an icon instead of a text badge"
        );
        assert!(app.screen().matches(&format!("label:{label}")).unwrap());
        app.hover(&format!("label:{label}")).unwrap();
        app.wait_for(&format!("text:{label}")).unwrap();
        assert!(app.shows(protocol::tr!("Trust covers the listed functional checks, not a security audit or platform certification")));
        assert!(app.shows(protocol::tr!(
            "The tested track returned a 60-second preview"
        )));
        assert!(app.shows(protocol::tr!(
            "Full-track playback and account access still depend on the service"
        )));
    }
}

static SOURCE_PLAYABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[component]
fn SourcePageScreen() -> impl IntoElement {
    discovery_ui::preview_page(model::Model::new(), protocol::SourcePage {
        playable: SOURCE_PLAYABLE.load(std::sync::atomic::Ordering::Relaxed),
        tracks: vec![serde_json::from_value(serde_json::json!({"id":1,"title":"Fixture track","artist":"Fixture artist","duration":42})).unwrap()],
        ..Default::default()
    })
}
fn source_playback_capability() {
    for playable in [false, true] {
        SOURCE_PLAYABLE.store(playable, std::sync::atomic::Ordering::Relaxed);
        let mut config = EngineConfig::new(Root::of::<SourcePageScreen>());
        config.viewport = Size::new(px(440.0), px(600.0));
        config.environment.window = config.viewport;
        let mut app = Headless::start(config).unwrap();
        app.first_render().unwrap();
        let play = protocol::tr!("Play {}", "Fixture track");
        assert_eq!(
            app.screen().matches(&format!("label:{play}")).unwrap(),
            playable
        );
        if !playable {
            assert!(app.screen().matches("label:Fixture track").unwrap());
            assert!(
                app.press("label:Fixture track").is_err(),
                "metadata-only track must be disabled"
            );
        }
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
fn CloudSyncScreen() -> impl IntoElement {
    use protocol::{CloudSyncPhase as Phase, CloudSyncStatus};
    let status = signal(Some(CloudSyncStatus {
        enabled: true,
        phase: Phase::AccountChanged,
        ..Default::default()
    }));
    view().flex_1().child(sources_ui::cloud_sync_content(
        status,
        signal(false),
        signal(String::new()),
        move |action| {
            status.set(Some(match action {
                protocol::CloudSyncAction::SetEnabled { enabled } => CloudSyncStatus {
                    enabled,
                    phase: if enabled {
                        Phase::Waiting
                    } else {
                        Phase::Disabled
                    },
                    ..Default::default()
                },
                protocol::CloudSyncAction::SyncNow => CloudSyncStatus {
                    enabled: true,
                    phase: Phase::Syncing,
                    ..Default::default()
                },
            }));
        },
    ))
}

fn cloud_sync_controls() {
    for language in [
        protocol::i18n::Language::English,
        protocol::i18n::Language::SimplifiedChinese,
    ] {
        language.save().unwrap();
        let mut config = EngineConfig::new(Root::of::<CloudSyncScreen>());
        config.viewport = Size::new(px(440.0), px(600.0));
        config.environment.window = config.viewport;
        let mut app = Headless::start(config).unwrap();
        app.first_render().unwrap();
        assert!(app.shows(protocol::tr!("iCloud account changed")));
        assert!(app.shows(protocol::tr!(
            "Turn sync off and on to use this iCloud account. Your local data will be kept."
        )));
        app.press(protocol::tr!("Sync with iCloud")).unwrap();
        assert!(app.shows(protocol::tr!("Stored on this device")));
        assert!(
            app.press(protocol::tr!("Sync now")).is_err(),
            "Disabled sync cannot be requested"
        );
        app.press(protocol::tr!("Sync with iCloud")).unwrap();
        assert!(app.shows(protocol::tr!("Waiting for iCloud")));
        app.press(protocol::tr!("Sync now")).unwrap();
        assert!(app.shows(protocol::tr!("Syncing…")));
        assert!(
            app.press(protocol::tr!("Sync now")).is_err(),
            "An active exchange must not be duplicated"
        );
    }
    protocol::i18n::Language::English.save().unwrap();
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
            status.index = Some(0);
            status.queue_length = 1;
            status.switching = height >= 570.0;
            status.queue_context = (height >= 560.0).then(|| protocol::QueueContext::Station {
                source: "mygo:fixture".into(),
                id: "fm".into(),
                name: "Fixture radio".into(),
            });
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

    for (height, enabled) in [(550.0, false), (560.0, true), (570.0, false)] {
        app.set_viewport(Size::new(px(628.0), px(height))).unwrap();
        let selector = format!("label:{}", protocol::tr!("Next track"));
        assert_eq!(
            app.screen().press_target(&selector).unwrap().is_some(),
            enabled,
            "one-track station queues must allow Next, finite or switching queues must not"
        );
    }
}

#[component]
fn RadioScreen() -> impl IntoElement {
    let model = model::Model::new();
    view()
        .flex_1()
        .child(library::library_view(model))
        .child(controls::action(
            "Simulate library revision",
            || false,
            move || {
                model.status.update(|status| status.library_revision += 1);
            },
        ))
}

#[derive(Default)]
struct SettingsFixture {
    plugin_writes: Mutex<Vec<bool>>,
    cloud_enabled: std::sync::atomic::AtomicBool,
    cloud_actions: std::sync::atomic::AtomicUsize,
    cloud_reads: std::sync::atomic::AtomicUsize,
}

const CLOUD_SAVE_ERROR: &str = "fixture could not save iCloud settings";

fn cloud_action_error_polling(fixture: &SettingsFixture) {
    use std::sync::atomic::Ordering;
    fixture.cloud_enabled.store(false, Ordering::SeqCst);
    fixture.cloud_actions.store(0, Ordering::SeqCst);
    fixture.cloud_reads.store(0, Ordering::SeqCst);
    protocol::i18n::Language::English.save().unwrap();
    sources_ui::select_settings_group(sources_ui::SettingsGroup::Cloud);
    let mut app = start();
    app.wait_for("Stored on this device").unwrap();
    app.press("label:Sync with iCloud").unwrap();
    app.wait_for(CLOUD_SAVE_ERROR).unwrap();
    let reads = fixture.cloud_reads.load(Ordering::SeqCst);
    for _ in 0..200 {
        if fixture.cloud_reads.load(Ordering::SeqCst) > reads {
            break;
        }
        app.wait(Duration::from_millis(20)).unwrap();
    }
    assert!(fixture.cloud_reads.load(Ordering::SeqCst) > reads);
    app.wait(Duration::from_millis(100)).unwrap();
    assert!(
        app.shows(CLOUD_SAVE_ERROR),
        "A successful status poll must not erase an unsuccessful setting change"
    );
    assert!(!fixture.cloud_enabled.load(Ordering::SeqCst));
    app.press("label:Sync with iCloud").unwrap();
    app.wait_for("Waiting for iCloud").unwrap();
    assert!(!app.shows(CLOUD_SAVE_ERROR));
    assert!(fixture.cloud_enabled.load(Ordering::SeqCst));
    assert_eq!(fixture.cloud_actions.load(Ordering::SeqCst), 2);
}

fn plugin_toggle(app: &Headless) -> Option<nsh_core::Checked> {
    app.screen()
        .accessibility_state("label:Automatically update plugins")
        .unwrap()
        .checked
}

fn plugin_preference_polling(fixture: &SettingsFixture) {
    use nsh_core::Checked;
    let directory = protocol::data_dir().unwrap();
    let preference = directory.join("plugin-auto-update.json");
    let legacy = directory.join("plugin-updates.json");
    protocol::i18n::Language::English.save().unwrap();
    for fallback in [None, Some(false)] {
        std::fs::write(&preference, "false").unwrap();
        if let Some(enabled) = fallback {
            std::fs::write(&legacy, serde_json::json!({"enabled":enabled}).to_string()).unwrap();
        } else {
            let _ = std::fs::remove_file(&legacy);
        }
        sources_ui::select_settings_group(sources_ui::SettingsGroup::Sources);
        let mut app = start();
        app.wait_for("Automatically update plugins").unwrap();
        for _ in 0..100 {
            if app
                .screen()
                .enabled("label:Automatically update plugins")
                .unwrap()
            {
                break;
            }
            app.wait(Duration::from_millis(20)).unwrap();
        }
        assert_eq!(plugin_toggle(&app), Some(Checked::False));
        std::fs::remove_file(&preference).unwrap();
        let effective = fallback.unwrap_or(true);
        for _ in 0..150 {
            if plugin_toggle(&app)
                == Some(if effective {
                    Checked::True
                } else {
                    Checked::False
                })
            {
                break;
            }
            app.wait(Duration::from_millis(20)).unwrap();
        }
        // Give an already matching fallback a full poll to reject hardcoded defaults.
        app.wait(Duration::from_millis(1100)).unwrap();
        assert_eq!(
            plugin_toggle(&app),
            Some(if effective {
                Checked::True
            } else {
                Checked::False
            }),
            "A deleted standalone preference must display the service's effective value"
        );
        fixture.plugin_writes.lock().unwrap().clear();
        app.press("label:Automatically update plugins").unwrap();
        for _ in 0..100 {
            if !fixture.plugin_writes.lock().unwrap().is_empty() {
                break;
            }
            app.wait(Duration::from_millis(10)).unwrap();
        }
        assert_eq!(*fixture.plugin_writes.lock().unwrap(), [!effective]);
        app.wait(Duration::from_millis(50)).unwrap();
    }
    for unreadable in [false, true] {
        std::fs::write(&preference, "false").unwrap();
        sources_ui::select_settings_group(sources_ui::SettingsGroup::Sources);
        let mut app = start();
        app.wait_for("Automatically update plugins").unwrap();
        app.wait(Duration::from_millis(100)).unwrap();
        if unreadable {
            std::fs::remove_file(&preference).unwrap();
            std::fs::create_dir(&preference).unwrap();
        } else {
            std::fs::write(&preference, "invalid JSON").unwrap();
        }
        app.wait(Duration::from_millis(1200)).unwrap();
        assert_eq!(plugin_toggle(&app), Some(Checked::False));
        drop(app);
        if unreadable {
            std::fs::remove_dir(&preference).unwrap();
        }
    }
    let _ = std::fs::remove_file(preference);
    let _ = std::fs::remove_file(legacy);
}

fn radio_flow() {
    use std::io::{Read, Write};
    use std::sync::{
        Condvar,
        atomic::{AtomicUsize, Ordering},
    };
    let requests = Arc::new(AtomicUsize::new(0));
    let plays = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let history_gate = Arc::new((Mutex::new(false), Condvar::new()));
    let history_requests = Arc::new(AtomicUsize::new(0));
    let settings = Arc::new(SettingsFixture::default());
    let listener = std::net::TcpListener::bind(protocol::address()).unwrap();
    std::fs::write(
        protocol::data_dir().unwrap().join("mcp-token"),
        "radio-fixture",
    )
    .unwrap();
    let server_requests = requests.clone();
    let server_plays = plays.clone();
    let server_gate = gate.clone();
    let server_history_gate = history_gate.clone();
    let server_history_requests = history_requests.clone();
    let server_settings = settings.clone();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let mut socket = connection.unwrap();
            let requests = server_requests.clone();
            let plays = server_plays.clone();
            let gate = server_gate.clone();
            let history_gate = server_history_gate.clone();
            let history_requests = server_history_requests.clone();
            let settings = server_settings.clone();
            std::thread::spawn(move || {
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let (header_end, length) = loop {
                    let mut buffer = [0u8; 2048];
                    let count = socket.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..at]);
                        assert!(
                            headers
                                .to_ascii_lowercase()
                                .contains("authorization: bearer radio-fixture")
                        );
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        break (at + 4, length);
                    }
                };
                while bytes.len() < header_end + length {
                    let mut buffer = [0u8; 2048];
                    let count = socket.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let first = String::from_utf8_lossy(&bytes[..header_end]);
                let body = &bytes[header_end..header_end + length];
                let mut status = "200 OK";
                let cloud_status = || {
                    let enabled = settings.cloud_enabled.load(Ordering::SeqCst);
                    serde_json::to_value(protocol::CloudSyncStatus {
                        enabled,
                        phase: if enabled {
                            protocol::CloudSyncPhase::Waiting
                        } else {
                            protocol::CloudSyncPhase::Disabled
                        },
                        ..Default::default()
                    })
                    .unwrap()
                };
                let playlist = serde_json::json!({"id":"radio","name":"Radio","revision":1,"tracks":[{"id":1,"title":"Personal fixture song","artist":"Fixture artist","duration":180,"source":{"plugin":"fixture","id":"personal-1"}}],"imported_from":null});
                let response = if first.starts_with("GET /ui/icloud") {
                    settings.cloud_reads.fetch_add(1, Ordering::SeqCst);
                    cloud_status()
                } else if first.starts_with("POST /ui/icloud") {
                    let protocol::CloudSyncAction::SetEnabled { enabled } =
                        serde_json::from_slice(body).unwrap()
                    else {
                        panic!("unexpected cloud action");
                    };
                    if settings.cloud_actions.fetch_add(1, Ordering::SeqCst) == 0 {
                        status = "500 Internal Server Error";
                        serde_json::json!({"error":CLOUD_SAVE_ERROR})
                    } else {
                        settings.cloud_enabled.store(enabled, Ordering::SeqCst);
                        cloud_status()
                    }
                } else if first.starts_with("GET /ui/music-sources") {
                    serde_json::json!([])
                } else if first.starts_with("POST /ui/music-sources") {
                    match serde_json::from_slice::<protocol::MusicSourceAction>(body).unwrap() {
                        protocol::MusicSourceAction::AutoUpdateSettings => {
                            let directory = protocol::data_dir().unwrap();
                            let enabled =
                                match std::fs::read(directory.join("plugin-auto-update.json")) {
                                    Ok(bytes) => serde_json::from_slice::<bool>(&bytes).ok(),
                                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                        Some(
                                            std::fs::read(directory.join("plugin-updates.json"))
                                                .ok()
                                                .and_then(|bytes| {
                                                    serde_json::from_slice::<serde_json::Value>(
                                                        &bytes,
                                                    )
                                                    .ok()
                                                })
                                                .and_then(|value| value["enabled"].as_bool())
                                                .unwrap_or(true),
                                        )
                                    }
                                    Err(_) => None,
                                };
                            if let Some(enabled) = enabled {
                                serde_json::json!({"enabled":enabled})
                            } else {
                                status = "500 Internal Server Error";
                                serde_json::json!({"error":"invalid plugin preference"})
                            }
                        }
                        protocol::MusicSourceAction::SetAutoUpdate { enabled } => {
                            settings.plugin_writes.lock().unwrap().push(enabled);
                            std::fs::write(
                                protocol::data_dir()
                                    .unwrap()
                                    .join("plugin-auto-update.json"),
                                enabled.to_string(),
                            )
                            .unwrap();
                            serde_json::json!({"enabled":enabled})
                        }
                        _ => panic!("unexpected plugin settings action"),
                    }
                } else if first.starts_with("GET /ui/history") {
                    let at = history_requests.fetch_add(1, Ordering::SeqCst);
                    if at == 0 {
                        let (lock, ready) = &*history_gate;
                        drop(
                            ready
                                .wait_while(lock.lock().unwrap(), |released| !*released)
                                .unwrap(),
                        );
                    }
                    serde_json::json!([{"id":1,"title":if at==0 {"Stale history"} else {"Fresh history"},"artist":"Fixture","duration":90,"played_at_unix":1}])
                } else if first.starts_with("GET /ui/local-playlists?id=radio") {
                    playlist
                } else if first.starts_with("GET /ui/local-playlists") {
                    serde_json::json!([])
                } else if first.starts_with("POST /ui/local-playlists") {
                    let value: serde_json::Value = serde_json::from_slice(body).unwrap();
                    assert_eq!(value["action"], "refresh_radio");
                    let at = requests.fetch_add(1, Ordering::SeqCst) % 3;
                    if at == 0 {
                        let (lock, ready) = &*gate;
                        drop(
                            ready
                                .wait_while(lock.lock().unwrap(), |released| !*released)
                                .unwrap(),
                        );
                    }
                    if at == 2 {
                        status = "503 Service Unavailable";
                        serde_json::json!({"error":"fixture connection failed"})
                    } else {
                        serde_json::json!({"playlist":playlist,"added":if at==0 {1}else{0},"sources":[{"source":"mygo:fixture","name":"Fixture","tracks":1,"error":null},{"source":"mygo:broken","name":"Broken source","tracks":0,"error":if at==0{Some("fixture provider failed")}else{None}}]})
                    }
                } else if first.starts_with("POST /ui/action") {
                    let value: serde_json::Value = serde_json::from_slice(body).unwrap();
                    assert_eq!(value["action"], "play_local_playlist");
                    assert_eq!(value["id"], "radio");
                    plays.fetch_add(1, Ordering::SeqCst);
                    serde_json::to_value(protocol::Status::default()).unwrap()
                } else {
                    panic!(
                        "unexpected Radio UI request: {}",
                        first.lines().next().unwrap()
                    );
                };
                let response = response.to_string();
                write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
            });
        }
    });
    remote::start().unwrap();
    if std::env::var("MUSICX_LOCALIZATION_CASE").as_deref() == Ok("plugin-sync") {
        plugin_preference_polling(&settings);
        return;
    }
    if std::env::var("MUSICX_LOCALIZATION_CASE").as_deref() == Ok("cloud-error") {
        cloud_action_error_polling(&settings);
        return;
    }
    for (language, iteration) in [("en", 0), ("zh-CN", 1)] {
        std::fs::write(protocol::data_dir().unwrap().join("language"), language).unwrap();
        protocol::i18n::refresh();
        *gate.0.lock().unwrap() = false;
        let mut config = EngineConfig::new(Root::of::<RadioScreen>());
        config.viewport = Size::new(px(272.0), px(430.0));
        config.environment.window = config.viewport;
        let mut app = Headless::start(config).unwrap();
        app.first_render().unwrap();
        app.wait_for(protocol::tr!("No playlists yet")).unwrap();
        assert!(app.shows(protocol::tr!("Radio")));
        assert_eq!(
            requests.load(Ordering::SeqCst),
            iteration * 3,
            "opening Radio must not fetch recommendations"
        );
        app.press(protocol::tr!("Add recommendations")).unwrap();
        app.wait_for(protocol::tr!("Getting recommendations…"))
            .unwrap();
        assert!(
            app.screen()
                .press_target(&format!("label:{}", protocol::tr!("Add recommendations")))
                .unwrap()
                .is_none()
        );
        assert_eq!(plays.load(Ordering::SeqCst), iteration);
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        app.wait_for("Personal fixture song").unwrap();
        assert!(app.shows(protocol::tr!("Radio")));
        assert!(app.shows("Broken source: fixture provider failed"));
        assert!(app.shows(&protocol::tr!(
            "Added {} to Radio.",
            protocol::i18n::tracks(1)
        )));
        app.press("Simulate library revision").unwrap();
        app.wait(Duration::from_millis(100)).unwrap();
        assert!(
            app.shows("Broken source: fixture provider failed"),
            "a library revision refresh must preserve per-source failures"
        );
        assert_eq!(
            plays.load(Ordering::SeqCst),
            iteration,
            "fetching must not start playback"
        );
        app.press(protocol::tr!("Add recommendations")).unwrap();
        app.wait_for(protocol::tr!("No new recommendations. Try again later."))
            .unwrap();
        assert!(!app.shows("fixture provider failed"));
        assert!(app.shows("Personal fixture song"));
        app.press(protocol::tr!("Add recommendations")).unwrap();
        app.wait_for("fixture connection failed").unwrap();
        assert!(
            app.shows("Personal fixture song"),
            "failed refresh must preserve the visible playlist"
        );
        app.press(protocol::tr!("Play playlist")).unwrap();
        for _ in 0..50 {
            if plays.load(Ordering::SeqCst) == iteration + 1 {
                break;
            }
            app.wait(Duration::from_millis(20)).unwrap();
        }
        assert_eq!(plays.load(Ordering::SeqCst), iteration + 1);
    }
    assert_eq!(
        requests.load(Ordering::SeqCst),
        6,
        "busy controls must not send duplicate requests"
    );
    let mut history = Headless::start(EngineConfig::new(Root::of::<HistoryRaceScreen>())).unwrap();
    history.first_render().unwrap();
    for _ in 0..50 {
        if history_requests.load(Ordering::SeqCst) == 1 {
            break;
        }
        history.wait(Duration::from_millis(10)).unwrap();
    }
    assert_eq!(history_requests.load(Ordering::SeqCst), 1);
    history.press("History changed").unwrap();
    *history_gate.0.lock().unwrap() = true;
    history_gate.1.notify_all();
    history.wait_for("Fresh history").unwrap();
    assert_eq!(history_requests.load(Ordering::SeqCst), 2);
    plugin_preference_polling(&settings);
    cloud_action_error_polling(&settings);
}

fn exercise() {
    if matches!(
        std::env::var("MUSICX_LOCALIZATION_CASE").as_deref(),
        Ok("history-race" | "plugin-sync" | "cloud-error")
    ) {
        radio_flow();
        return;
    }
    if std::env::var("MUSICX_LOCALIZATION_CASE").as_deref() == Ok("mini-race") {
        mini_preview_race();
        return;
    }
    mini_preview_race();
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
    source_playback_capability();
    settings_escape();
    setup_navigation();
    cloud_sync_controls();
    player_entries();
    radio_flow();
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
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    std::fs::create_dir_all(&directory).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .env("MUSICX_LOCALIZATION_CHILD", "1")
        .env("MUSICX_DATA_DIR", &directory)
        .env("MUSICX_PORT", port.to_string())
        .env_remove("MUSICX_LANGUAGE")
        .status()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        result.success(),
        "isolated localization UI check failed: {result}"
    );
}
