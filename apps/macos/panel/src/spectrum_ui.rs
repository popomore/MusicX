//! Live PCM levels, drawn natively without rebuilding the player or caching images.
use crate::{
    model::{Model, key},
    protocol::tr,
    protocol::{State, spectrum::*},
    remote,
};
use nsh_base::theme;
use nsh_core::{element::Painter, prelude::*};
use std::time::{Duration, Instant};

pub(super) fn spectrum(model: Model) -> impl IntoElement {
    let frame = signal(Spectrum::default());
    let disconnected = signal(false);
    let received = signal(Instant::now());
    let painted = signal(Instant::now() - Duration::from_secs(1));
    if let Ok(feed) = remote::spectrum_feed() {
        set_interval(Duration::from_millis(50), move || {
            if let Some(result) = feed.try_iter().last() {
                match result {
                    Ok(next) => {
                        if !reduce_motion() || painted.get().elapsed() >= Duration::from_millis(250)
                        {
                            frame.set(next);
                            painted.set(Instant::now());
                        }
                        received.set(Instant::now());
                        disconnected.set(false);
                    }
                    Err(_) => {
                        frame.set(Spectrum::default());
                        disconnected.set(true);
                    }
                }
            }
            if received.get().elapsed() > Duration::from_secs(1) {
                disconnected.set(true);
            }
        });
    } else {
        disconnected.set(true);
    }
    content(model, frame, disconnected)
}

pub(crate) fn content(
    model: Model,
    frame: Signal<Spectrum>,
    disconnected: Signal<bool>,
) -> impl IntoElement {
    let current = computed(move || {
        let frame = frame.get();
        let status = model.status.get();
        if disconnected.get()
            || frame.track_key != key(status.track.as_ref())
            || status.state != State::Playing
            || status.switching
            || model.scrub.get().is_some()
        {
            Spectrum {
                track_key: key(status.track.as_ref()),
                ..Spectrum::default()
            }
        } else {
            frame
        }
    });
    let size = signal(Size::new(px(256.0), px(250.0)));
    scroll().flex_1().scrollbar(false).child(
        view().pt(px(20.0)).gap(px(12.0))
            .child(text(move || model.status.with(|s| s.track.as_ref().map_or_else(
                || tr!("Nothing playing yet").to_owned(), |t| t.title.clone())))
                .text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).number_of_lines(2)
                .text_color(move || theme().foreground))
            .child(canvas(move || draw(&current.get(), size.get()))
                .w_full().h(px(250.0)).min_h(px(250.0))
                .on_layout(move |bounds: &Bounds<Points>| { size.set(bounds.size); })
                .accessible(true)
                .label(tr!("Audio frequency spectrum, 20 Hz to 20 kHz, -80 to 0 dBFS")))
            .child(text(move || {
                if disconnected.get() { return tr!("Spectrum connection lost"); }
                if model.status.with(|s| s.track.is_none()) { return tr!("Play a track to see its spectrum"); }
                if model.status.with(|s| s.state == State::Paused) { return tr!("Paused"); }
                match current.get().state {
                    SpectrumState::Live => tr!("Live audio · before volume adjustment"),
                    SpectrumState::Unavailable => tr!("Spectrum is unavailable for this stream"),
                    SpectrumState::Idle | SpectrumState::Waiting => tr!("Waiting for audio…"),
                }
            }).text_size(px(11.0)).text_color(move || theme().muted_foreground))
            .child(text(tr!("Low frequencies on the left, high frequencies on the right. Taller bars mean stronger sound."))
                .text_size(px(11.0)).text_color(move || theme().muted_foreground))
    )
}

fn draw(frame: &Spectrum, size: Size<Points>) -> Painter {
    let (left, top, bottom) = (30.0, 28.0, size.height.0 - 30.0);
    let right = (size.width.0 - 8.0).max(left + 32.0);
    let (width, height) = (right - left, bottom - top);
    let colors = theme();
    let mut painter = Painter::new().font("10px system-ui");
    for db in [0, -20, -40, -60, -80] {
        let y = top + (-db as f32 / -SPECTRUM_FLOOR_DB) * height;
        painter = painter
            .fill_color(color(colors.muted_foreground.opacity(0.16)))
            .fill_rect(left, y, width, 0.5)
            .fill_color(color(colors.muted_foreground))
            .fill_text(&db.to_string(), 0.0, y + 3.0);
    }
    painter = painter
        .fill_color(color(colors.muted_foreground))
        .fill_text("dBFS", 0.0, top - 14.0);
    for (hz, label, shift) in [
        (20.0_f32, "20", 0.0),
        (100.0, "100", 8.0),
        (1_000.0, "1k", 5.0),
        (20_000.0, "20k Hz", 30.0),
    ] {
        let x = left
            + (hz / SPECTRUM_MIN_HZ).log10() / (SPECTRUM_MAX_HZ / SPECTRUM_MIN_HZ).log10() * width;
        painter = painter.fill_text(label, x - shift, bottom + 19.0);
    }
    if frame.state == SpectrumState::Live {
        let step = width / SPECTRUM_BANDS as f32;
        painter = painter.fill_color(color(colors.foreground.opacity(0.78)));
        for (i, db) in frame.bands.iter().enumerate() {
            let level = if db.is_finite() {
                db.clamp(SPECTRUM_FLOOR_DB, 0.0)
            } else {
                SPECTRUM_FLOOR_DB
            };
            let bar = (level - SPECTRUM_FLOOR_DB) / -SPECTRUM_FLOOR_DB * height;
            if bar > 0.0 {
                painter = painter.fill_rect(
                    left + i as f32 * step + 0.8,
                    bottom - bar,
                    (step - 1.6).max(1.0),
                    bar,
                );
            }
        }
    }
    painter
}
