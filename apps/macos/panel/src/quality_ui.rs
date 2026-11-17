//! Quality selection is anchored to the playback page, independent of detail navigation.
use crate::protocol::tr;
use crate::{
    controls::section,
    model::Model,
    protocol::{Quality, State},
};
use nsh_base::{SelectEntry, SelectGroup, SelectItem, select, theme};
use nsh_core::prelude::*;

fn actual(model: Model) -> String {
    model.status.with(|s| match &s.audio_quality {
        Some(quality) => quality.summary(),
        None if s.state == State::Paused => tr!("Available when playback resumes").into(),
        None => tr!("This source has no audio quality details").into(),
    })
}
pub(super) fn badge(model: Model) -> impl IntoElement {
    select()
        .accessibility_label(move || {
            Some(SharedString::from(tr!(
                "Audio quality: {}, click to choose",
                actual(model)
            )))
        })
        .disabled(move || model.status.with(|s| s.track.is_none()))
        .items(move || {
            let busy = model.quality_busy.get()
                || model.starting.get()
                || model.status.with(|s| s.switching);
            Quality::choices(true)
                .iter()
                .map(|quality| {
                    SelectItem::new(quality.plugin_quality(), quality.label())
                        .group(tr!("Playback preference"))
                        .disabled(busy)
                })
                .collect::<Vec<_>>()
        })
        .value(move || model.status.with(|s| s.preferred_quality.plugin_quality()))
        .on_change(move |value: &Option<SharedString>| {
            if let Some(quality) = value.as_deref().and_then(|value| value.parse().ok()) {
                model.set_quality(quality);
            }
        })
        .placement(Placement::Top)
        .align(Align::End)
        .offset(px(6.0))
        .margin(px(12.0))
        .popup_style(move |s| {
            s.w(px(252.0))
                .p(px(8.0))
                .bg(theme().background)
                .border_1()
                .border_color(theme().muted)
                .rounded(px(10.0))
        })
        .trigger(
            view()
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
        .group_style(|s| s.px(px(4.0)).pt(px(4.0)).pb(px(8.0)))
        .render_group(move |_: &SelectGroup| preference_header(model))
        .item_class("focus:bg-muted")
        .item_style(|s| {
            s.px(px(8.0))
                .py(px(7.0))
                .rounded(px(4.0))
                .text_size(px(12.0))
                .text_color(theme().foreground)
        })
        .render_item(move |entry: &SelectEntry| {
            let value = entry.value.clone();
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(text(entry.title.clone()).flex_1().number_of_lines(1))
                .child(
                    text(move || {
                        if model
                            .status
                            .with(|s| s.preferred_quality.plugin_quality() == value.as_ref())
                        {
                            "✓"
                        } else {
                            ""
                        }
                    })
                    .w(px(12.0)),
                )
        })
}

fn preference_header(model: Model) -> impl IntoElement {
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
        .child(section(tr!("Playback preference")))
}
