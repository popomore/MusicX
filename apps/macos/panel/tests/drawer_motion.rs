//! Replay the actual component's native animation bindings without a window.
#[path = "../src/drawer.rs"]
mod drawer;

use nsh_base::button;
use nsh_core::{
    MountItem, Size,
    animation::{AnimationProp, Graph},
    element::Root,
    prelude::*,
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

#[component]
/// Exercise the production drawer without AppKit or a playback connection.
fn Screen() -> impl IntoElement {
    let open = signal(false);
    let left = signal(false);
    let opened = signal(0);
    let closed = signal(0);
    view()
        .child(text(move || {
            format!("opened:{} closed:{}", opened.get(), closed.get())
        }))
        .child(
            button()
                .on_press(move || {
                    left.set(true);
                    open.set(true);
                })
                .child(text("Left")),
        )
        .child(
            button()
                .on_press(move || {
                    left.set(false);
                    open.set(true);
                })
                .child(text("Right")),
        )
        .child(
            button()
                .on_press(move || {
                    open.set(true);
                    set_timeout(Duration::from_millis(50), move || open.set(false));
                })
                .child(text("Cancel entrance")),
        )
        .child(
            drawer::details_drawer()
                .open(move || open.get() && window().width.0 >= drawer::EXPANDED_WIDTH - 1.0)
                .left(left)
                .on_open_change(move |next: &bool| open.set(*next))
                .on_open(move || opened.update(|n| *n += 1))
                .on_close(move || closed.update(|n| *n += 1))
                .child(
                    view()
                        .child(
                            button()
                                .on_press(move || open.set(false))
                                .child(text("Close details")),
                        )
                        .child(
                            button()
                                .on_press(move || {
                                    open.set(false);
                                    set_timeout(Duration::from_millis(40), move || open.set(true));
                                })
                                .child(text("Reopen during close")),
                        )
                        .child(
                            button()
                                .on_press(move || {
                                    open.set(false);
                                    set_timeout(Duration::from_millis(40), move || open.set(true));
                                    set_timeout(Duration::from_millis(80), move || open.set(false));
                                })
                                .child(text("Cycle")),
                        ),
                ),
        )
}

/// Capture the first presented translation from the real native animation graph.
fn start(reduced_motion: bool) -> (Headless, Arc<Mutex<Vec<f32>>>) {
    let mut config = EngineConfig {
        viewport: Size::new(px(drawer::EXPANDED_WIDTH), px(512.0)),
        ..EngineConfig::new(Root::of::<Screen>())
    };
    config.environment.window = config.viewport;
    config.environment.reduce_motion = reduced_motion;
    let mut app = Headless::start(config).unwrap();
    let positions = Arc::new(Mutex::new(Vec::new()));
    let observed = positions.clone();
    let mut graph = Graph::default();
    let mut bindings = HashMap::new();
    let mut finished = Vec::new();
    app.observe(move |_, batch| {
        let mut frames = Vec::new();
        for item in &batch.items {
            match item {
                MountItem::Animation(op) => graph.apply(op.clone(), &mut finished),
                MountItem::AnimationBindings {
                    id,
                    bindings: values,
                } => {
                    for (prop, node) in values {
                        if *prop == AnimationProp::Transform(0) {
                            bindings.insert(*id, *node);
                        }
                    }
                }
                MountItem::Frame { id, frame }
                    if (frame.size.width.0 - drawer::DETAILS_WIDTH).abs() < 0.1 =>
                {
                    frames.push(*id)
                }
                _ => {}
            }
        }
        for id in frames {
            if let Some(node) = bindings.get(&id) {
                observed.lock().unwrap().push(graph.eval(*node).number());
            }
        }
    });
    app.first_render().unwrap();
    (app, positions)
}

#[test]
fn details_open_outward_on_both_sides() {
    for (side, sign) in [("Left", 1.0), ("Right", -1.0)] {
        let (mut app, positions) = start(false);
        app.press(side).unwrap();
        app.wait(Duration::from_millis(350)).unwrap();
        let positions = positions.lock().unwrap();
        let first = positions.first().expect("native panel translation");
        assert!(
            (first - sign * drawer::DETAILS_WIDTH).abs() < 0.1,
            "{side} details must start behind the player and move outward; native translations: {positions:?}"
        );
    }
}

#[test]
fn closing_and_reopening_finishes_each_lifecycle_once() {
    let (mut app, _) = start(false);
    for (index, side) in ["Left", "Right", "Left"].into_iter().enumerate() {
        app.press(side).unwrap();
        app.wait_for(&format!("text:opened:{} closed:{}", index + 1, index))
            .unwrap();
        app.press("Close details").unwrap();
        app.wait_for(&format!("text:opened:{} closed:{}", index + 1, index + 1))
            .unwrap();
        assert!(!app.shows("Close details"));
    }
}

#[test]
fn reduced_motion_opens_at_rest_on_either_side() {
    for side in ["Left", "Right"] {
        let (mut app, positions) = start(true);
        app.press(side).unwrap();
        app.wait_for("text:opened:1 closed:0").unwrap();
        let translations = positions.lock().unwrap();
        assert!(!translations.is_empty(), "native panel translation");
        assert!(translations.iter().all(|x| x.abs() < 0.1));
        drop(translations);
        app.press("Close details").unwrap();
        app.wait_for("text:opened:1 closed:1").unwrap();
    }
}

#[test]
fn an_interrupted_entrance_closes_without_a_spurious_open() {
    let (mut app, _) = start(false);
    app.press("Cancel entrance").unwrap();
    app.wait_for("text:opened:0 closed:1").unwrap();
    assert!(!app.shows("Close details"));
}

#[test]
fn reopening_during_exit_keeps_the_new_drawer_open() {
    let (mut app, _) = start(false);
    app.press("Left").unwrap();
    app.wait_for("text:opened:1 closed:0").unwrap();
    app.press("Reopen during close").unwrap();
    app.wait(Duration::from_millis(350)).unwrap();
    assert!(app.shows("opened:1 closed:0"));
    assert!(app.shows("Close details"));
    app.press("Close details").unwrap();
    app.wait_for("text:opened:1 closed:1").unwrap();
}

#[test]
fn an_older_exit_cannot_finish_a_newer_exit_twice() {
    let (mut app, _) = start(false);
    app.press("Right").unwrap();
    app.wait_for("text:opened:1 closed:0").unwrap();
    app.press("Cycle").unwrap();
    app.wait_for("text:opened:1 closed:1").unwrap();
    app.wait(Duration::from_millis(350)).unwrap();
    assert!(app.shows("opened:1 closed:1"));
    assert!(!app.shows("Close details"));
}

#[test]
fn escape_and_the_gap_close_on_both_sides() {
    for side in ["Left", "Right"] {
        let (mut app, _) = start(false);
        app.press(side).unwrap();
        app.wait_for("text:opened:1 closed:0").unwrap();
        app.press_key("Escape").unwrap();
        app.wait_for("text:opened:1 closed:1").unwrap();
        app.press(side).unwrap();
        app.wait_for("text:opened:2 closed:1").unwrap();
        app.press_at(
            &format!("label:{}", musicx_protocol::tr!("收起详情侧栏")),
            point(px(if side == "Left" { 302.0 } else { 6.0 }), px(256.0)),
        )
        .unwrap();
        app.wait_for("text:opened:2 closed:2").unwrap();
    }
}

#[test]
fn compacting_the_host_removes_the_drawer() {
    let (mut app, _) = start(false);
    app.press("Left").unwrap();
    app.wait_for("text:opened:1 closed:0").unwrap();
    app.set_viewport(Size::new(px(drawer::PLAYER_WIDTH), px(512.0)))
        .unwrap();
    app.wait_for("text:opened:1 closed:1").unwrap();
    assert!(!app.shows("Close details"));
}

#[test]
fn changing_reduce_motion_does_not_repeat_open_callbacks() {
    let (mut app, _) = start(false);
    app.press("Right").unwrap();
    app.wait_for("text:opened:1 closed:0").unwrap();
    app.set_environment("Reduce Motion", |env| env.reduce_motion = true)
        .unwrap();
    app.wait_for("text:opened:1 closed:0").unwrap();
    app.press("Close details").unwrap();
    app.wait_for("text:opened:1 closed:1").unwrap();
}
