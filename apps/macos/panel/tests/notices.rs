//! Exercise the real notice popover without pointer actions dismissing it by accident.
#[path = "../src/controls.rs"]
#[allow(dead_code)]
mod controls;
use musicx_protocol as protocol;
use nsh_core::{Size, element::Root, prelude::*};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;
use std::{process::Command, time::Duration};

#[component]
fn NoticeScreen() -> impl IntoElement {
    view().flex_1().p(px(16.0)).child(controls::notice_badge(
        // Environment changes drive the same reactive inputs as incoming playback snapshots.
        || {
            if window().width.0 < 330.0 {
                "first-track"
            } else {
                "second-track"
            }
            .into()
        },
        || {
            let height = window().height.0;
            if height < 520.0 {
                vec![
                    controls::Notice::error("First error"),
                    controls::Notice::error("Second error"),
                ]
            } else if height < 530.0 {
                vec![controls::Notice::error("First error")]
            } else if height < 540.0 {
                vec![
                    controls::Notice::error("First error"),
                    controls::Notice::error("Third error"),
                ]
            } else if height < 580.0 {
                vec![controls::Notice::warning("Unencrypted fixture audio")]
            } else {
                Vec::new()
            }
        },
    ))
}

fn screen(height: f32) -> Headless {
    let mut config = EngineConfig::new(Root::of::<NoticeScreen>());
    config.viewport = Size::new(px(320.0), px(height));
    config.environment.window = config.viewport;
    let mut app = Headless::start(config).unwrap();
    app.first_render().unwrap();
    app.wait(Duration::from_millis(50)).unwrap();
    app
}

fn error_changes() {
    let mut app = screen(512.0);
    assert!(
        app.shows("First error") && app.shows("Second error"),
        "new errors must open automatically"
    );
    app.press("label:View playback errors").unwrap();
    assert!(!app.shows("First error"));
    app.set_viewport(Size::new(px(320.0), px(520.0))).unwrap();
    assert!(
        !app.shows("First error"),
        "clearing another error must preserve dismissal of the remaining error"
    );
    app.set_viewport(Size::new(px(320.0), px(530.0))).unwrap();
    assert!(
        app.shows("Third error"),
        "a newly added error must still open automatically"
    );
    app.set_viewport(Size::new(px(320.0), px(550.0))).unwrap();
    assert!(!app.shows("First error") && !app.shows("Third error"));
}

fn track_changes() {
    let mut app = screen(550.0);
    app.press("label:View playback warnings").unwrap();
    assert!(app.shows("Unencrypted fixture audio"));
    // Change only the context, without a click outside the popover or an empty notice list.
    app.set_viewport(Size::new(px(340.0), px(550.0))).unwrap();
    assert!(
        !app.shows("Unencrypted fixture audio"),
        "another track with the same warning must close the previous popover"
    );
    app.press("label:View playback warnings").unwrap();
    assert!(app.shows("Unencrypted fixture audio"));
    app.set_viewport(Size::new(px(340.0), px(600.0))).unwrap();
    assert!(!app.shows("Unencrypted fixture audio"));
    assert!(
        app.press("label:View playback warnings").is_err(),
        "no notices must hide the entry"
    );
}

fn main() {
    if std::env::var_os("MUSICX_NOTICE_CHILD").is_some() {
        match std::env::var("MUSICX_NOTICE_CASE").as_deref() {
            Ok("errors") => error_changes(),
            Ok("tracks") => track_changes(),
            _ => {
                error_changes();
                track_changes();
            }
        }
        println!(
            "notice popovers: error dismissal, new errors, track identity and clearing passed"
        );
        return;
    }
    let directory = std::env::temp_dir().join(format!("musicx-notices-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let result = Command::new(std::env::current_exe().unwrap())
        .env("MUSICX_NOTICE_CHILD", "1")
        .env("MUSICX_DATA_DIR", &directory)
        .env("MUSICX_LANGUAGE", "en")
        .status()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(result.success(), "isolated notice check failed: {result}");
}
