//! Quality selection is anchored to the playback page, independent of detail navigation.
use crate::protocol::tr;
use crate::{
    controls::section,
    model::Model,
    protocol::{Quality, State},
};
use nsh_base::{button, popover, theme};
use nsh_core::prelude::*;

fn actual(model: Model) -> String {
    model.status.with(|s| match &s.audio_quality {
        Some(quality) => quality.summary(),
        None if s.state == State::Paused => tr!("Available when playback resumes").into(),
        None => tr!("This source has no audio quality details").into(),
    })
}
pub(super) fn badge(model: Model) -> impl IntoElement {
    let open = signal(false);
    popover()
        .open(move || Some(open.get()))
        .on_open_change(move |event: &nsh_base::PopoverEvent| {
            if let nsh_base::PopoverEvent::OpenChange(value, _) = event {
                open.set(*value);
            }
        })
        .placement(Placement::Top)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(12.0))
        .popup_style(move |s| {
            s.w(px(252.0))
                .p(px(12.0))
                .bg(theme().background)
                .border_1()
                .border_color(theme().muted)
                .rounded(px(10.0))
        })
        .trigger(
            button()
                .accessibility_label(move || {
                    Some(SharedString::from(tr!(
                        "Audio quality: {}, click to choose",
                        actual(model)
                    )))
                })
                .disabled(move || model.status.with(|s| s.track.is_none()))
                .px(px(6.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .hover(|s| s.bg(theme().muted))
                .active(|s| s.opacity(0.6))
                .child(
                    text(move || {
                        model.status.with(|s| {
                            s.audio_quality
                                .as_ref()
                                .map_or_else(|| tr!("Audio quality").into(), |q| q.short_label())
                        })
                    })
                    .text_size(px(10.0))
                    .number_of_lines(1)
                    .text_color(move || theme().muted_foreground),
                ),
        )
        .child(controls(model))
}

pub(super) fn controls(model: Model) -> impl IntoElement {
    view()
        .gap(px(8.0))
        .child(
            view()
                .gap(px(4.0))
                .child(section(tr!("Stream quality")))
                .child(
                    text(move || actual(model))
                        .text_size(px(13.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .number_of_lines(2)
                        .text_color(move || theme().foreground),
                ),
        )
        .child(section(tr!("Playback preference")))
        .child(view().flex_row().gap(px(4.0)).child(each(
            || Quality::choices(true).to_vec(),
            |quality| *quality as u8,
            move |quality, _| {
                button()
                    .accessibility_label(move || {
                        Some(SharedString::from(tr!(
                            "Switch quality to {}",
                            quality.get().label()
                        )))
                    })
                    .disabled(move || {
                        model.quality_busy.get()
                            || model.starting.get()
                            || model.status.with(|s| s.switching)
                    })
                    .selected(move || {
                        model
                            .status
                            .with(|s| s.preferred_quality.for_plugin() == quality.get())
                    })
                    .selected_style(|s| {
                        s.bg(theme().primary).text_color(theme().primary_foreground)
                    })
                    .on_press(move || model.set_quality(quality.get()))
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(5.0))
                    .hover(|s| s.opacity(0.75))
                    .disabled_style(|s| s.opacity(0.45))
                    .child(text(move || quality.get().label()).text_size(px(11.0)))
            },
        )))
        .child(
            text(move || {
                if model.error.with(|e| !e.is_empty()) {
                    return model.error.get();
                }
                if model.quality_busy.get() {
                    return tr!("Changing quality…").into();
                }
                model.status.with(|s| {
                    let preference = s.preferred_quality;
                    let request = s.audio_quality.as_ref().map(|q| q.requested);
                    if preference == Quality::Hires {
                        tr!("Hi-Res saved; this source will use lossless").into()
                    } else if let Some(request) = request {
                        tr!(
                            "Requesting {} for this track; the next uses your preference",
                            request.label()
                        )
                    } else {
                        tr!("Applies when resuming and to the next track").into()
                    }
                })
            })
            .text_size(px(10.0))
            .number_of_lines(2)
            .text_color(move || theme().muted_foreground),
        )
}
