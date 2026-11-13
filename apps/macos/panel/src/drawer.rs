//! Details beside the player, with one shared geometry for AppKit and NSH.
use musicx_protocol::tr;
use nsh_base::theme;
use nsh_core::prelude::*;

pub const PLAYER_WIDTH: f32 = 320.0;
pub const DETAILS_WIDTH: f32 = 296.0;
pub const DETAILS_GAP: f32 = 12.0;
pub const EXPANDED_WIDTH: f32 = PLAYER_WIDTH + DETAILS_GAP + DETAILS_WIDTH;

#[derive(Debug, Clone, PartialEq, Props)]
pub struct DetailsHintProps {
    pub left: Prop<bool>,
}

#[derive(Debug, Clone, PartialEq, Event)]
pub enum DetailsHintEvent {
    Press,
}

#[component]
/// A quiet edge target, revealed by hover or keyboard focus without resizing.
pub fn DetailsHint(props: DetailsHintProps, cx: Scope<DetailsHintEvent>) -> impl IntoElement {
    let label = move || {
        if props.left.get() {
            tr!("Open details on the left")
        } else {
            tr!("Open details on the right")
        }
    };
    view()
        .absolute()
        .top(px(60.0))
        .bottom(px(40.0))
        .left(move || px(if props.left.get() { 0.0 } else { PLAYER_WIDTH - 24.0 }))
        .w(px(24.0))
        .child(
            nsh_base::button()
                .accessibility_label(move || Some(label().into()))
                .on_press(move || cx.emit(DetailsHintEvent::Press))
                .w_full()
                .h_full()
                .items_center()
                .justify_center()
                .opacity(0.0)
                .hover(|s| s.opacity(1.0).cursor_pointer())
                .focus(|s| s.opacity(1.0))
                .active(|s| s.opacity(0.65))
                .child(
                    view()
                        .w(px(20.0))
                        .h(px(76.0))
                        .rounded(px(10.0))
                        .bg(move || theme().muted.opacity(0.82))
                        .items_center()
                        .justify_center()
                        .child(
                            nsh_core::element::svg(move || nsh_core::element::ImageSrc::svg_markup(
                                if props.left.get() {
                                    r#"<svg viewBox="0 0 16 16"><path d="m10 3-5 5 5 5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>"#
                                } else {
                                    r#"<svg viewBox="0 0 16 16"><path d="m6 3 5 5-5 5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>"#
                                }
                            ))
                            .w(px(14.0))
                            .h(px(14.0))
                            .text_color(move || theme().foreground.opacity(0.8)),
                        ),
                ),
        )
}

#[derive(Debug, Clone, PartialEq, Props)]
/// Controlled details content, positioned beside the player.
pub struct DetailsDrawerProps {
    /// The host is expanded and details are requested.
    pub open: Prop<bool>,
    /// Keep the chosen side for the lifetime of an opening.
    pub left: Prop<bool>,
    /// Keep content mounted until the exit completes.
    #[prop(lazy)]
    pub children: Prop<Children>,
}

#[derive(Debug, Clone, PartialEq, Event)]
/// Dismiss requests and completed animation milestones.
pub enum DetailsDrawerEvent {
    #[args(open)]
    OpenChange(bool),
    Open,
    Close,
}

#[component]
/// Reveal details from the player side, clipped away from its controls and gap.
pub fn DetailsDrawer(props: DetailsDrawerProps, cx: Scope<DetailsDrawerEvent>) -> impl IntoElement {
    let shown = signal(false);
    let reported_open = stored(false);
    let generation = stored(0_u64);
    let completed = stored(0_u64);
    let progress = animated_value(0.0);
    let offset = animated_value(DETAILS_WIDTH);
    let translation = progress.interpolate(&[0.0, 1.0], &[1.0, 0.0]).mul(&offset);
    effect(move || {
        let open = props.open.get();
        offset.set(if props.left.get() {
            DETAILS_WIDTH
        } else {
            -DETAILS_WIDTH
        });
        let reduced = reduce_motion() || window().width.0 < EXPANDED_WIDTH - 1.0;
        if open {
            shown.set(true);
        } else if !shown.get_untracked() {
            return;
        }
        let token = generation.get().wrapping_add(1);
        generation.replace(token);
        let target = if open { 1.0 } else { 0.0 };
        let finish = move || {
            if generation.get() != token || completed.get() == token {
                return;
            }
            completed.replace(token);
            progress.set(target);
            if open {
                if !reported_open.replace(true) {
                    cx.emit(DetailsDrawerEvent::Open);
                }
            } else {
                reported_open.replace(false);
                shown.set(false);
                cx.emit(DetailsDrawerEvent::Close);
            }
        };
        if reduced {
            finish();
        } else {
            let duration = if open { 220.0 } else { 165.0 };
            let end = progress.timing(target, duration, Easing::Bezier(0.22, 1.0, 0.36, 1.0));
            spawn(async move {
                if end.await {
                    finish();
                }
            });
            // A hidden surface can stop native display ticks. Settle that
            // transition too, without letting an older exit close a new one.
            set_timeout(
                std::time::Duration::from_millis(duration as u64 + 34),
                finish,
            );
        }
    });
    when(shown, move || {
        view()
            .child(
                overlay()
                    .top_0()
                    .bottom_0()
                    .left(move || px(if props.left.get() { 0.0 } else { PLAYER_WIDTH }))
                    .right(move || px(if props.left.get() { PLAYER_WIDTH } else { 0.0 }))
                    .label(SharedString::from(tr!("Close details pane")))
                    .focusable(false)
                    .on_press(move || cx.emit(DetailsDrawerEvent::OpenChange(false))),
            )
            .child(
                overlay()
                    .top_0()
                    .bottom_0()
                    .left(move || {
                        px(if props.left.get() {
                            0.0
                        } else {
                            PLAYER_WIDTH + DETAILS_GAP
                        })
                    })
                    .w(px(DETAILS_WIDTH))
                    .overflow_hidden()
                    .on_cancel(move || cx.emit(DetailsDrawerEvent::OpenChange(false)))
                    .child(
                        view()
                            .w(px(DETAILS_WIDTH))
                            .h_full()
                            .bind_translate_x(&translation)
                            .bg(move || theme().background)
                            .rounded(px(16.0))
                            .overflow_hidden()
                            .child(props.children),
                    ),
            )
    })
}
