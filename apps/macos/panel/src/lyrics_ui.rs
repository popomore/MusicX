//! Mac-only lyric following. Its clock and list exist only in the open drawer.
use crate::protocol::tr;
#[path = "lrc.rs"]
mod lrc;
use crate::{
    model::{Model, Page, key, plain_lyrics},
    protocol::State,
    remote::{Action, submit},
};
use lrc::Timeline;
use nsh_base::{button, theme};
use nsh_core::prelude::*;
use std::rc::Rc;

fn playback_time(model: Model) -> f64 {
    model.status.with(|s| {
        let elapsed = if s.state == State::Playing && !s.switching {
            model
                .sampled_at
                .with(|at| at.elapsed().as_secs_f64().min(0.6))
        } else {
            0.0
        };
        (s.position + elapsed).min(s.duration.max(0.0))
    })
}

fn timed(model: Model, timeline: Memo<Rc<Timeline>>) -> impl IntoElement {
    let active = computed(move || {
        model.lyric_clock.get();
        timeline.with(|t| t.active(playback_time(model)))
    });
    let following = signal(true);
    let ready = signal(false);
    let list = element_ref();
    let first_scroll = stored(true);
    let last_scroll = stored(None::<usize>);
    effect(move || {
        if !following.get() {
            last_scroll.replace(None);
            return;
        }
        if ready.get()
            && let Some(index) = active.get()
            && timeline.with(|t| index < t.cues.len())
            && last_scroll.replace(Some(index)) != Some(index)
        {
            let animate = !first_scroll.replace(false) && !reduce_motion();
            list.scroll_to_index(index as u32, percentage(0.35), px(0.0), animate);
        }
    });
    view()
        .flex_1()
        .min_h(px(0.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .mt(px(12.0))
                .mb(px(8.0))
                .child(
                    view().flex_1().child(
                        text(move || {
                            active
                                .get()
                                .and_then(|i| {
                                    timeline.with(|t| {
                                        t.cues.get(i).map(|cue| {
                                            tr!("Synced · {}", super::time(cue.ms as f64 / 1000.0))
                                        })
                                    })
                                })
                                .unwrap_or_else(|| tr!("Intro").into())
                        })
                        .text_size(px(10.0))
                        .number_of_lines(1)
                        .text_color(move || theme().muted_foreground),
                    ),
                )
                .child(
                    button()
                        .accessibility_label(move || {
                            Some(
                                if following.get() {
                                    tr!("Pause follow")
                                } else {
                                    tr!("Follow current")
                                }
                                .into(),
                            )
                        })
                        .selected(move || following.get())
                        .on_press(move || following.update(|v| *v = !*v))
                        .h(px(26.0))
                        .px(px(6.0))
                        .items_center()
                        .justify_center()
                        .rounded(px(5.0))
                        .hover(|s| s.bg(theme().muted))
                        .child(
                            text(move || {
                                if following.get() {
                                    tr!("Pause follow")
                                } else {
                                    tr!("Follow current")
                                }
                            })
                            .text_size(px(10.0))
                            .text_color(move || theme().muted_foreground),
                        ),
                ),
        )
        .child(
            virtual_list(
                move || timeline.with(|t| t.cues.len()),
                move |at| {
                    let current = move || active.get() == Some(at.index());
                    button()
                        .py(px(10.0))
                        .px(px(8.0))
                        .w_full()
                        .rounded(px(7.0))
                        .accessibility_label(move || {
                            timeline.with(|t| {
                                t.cues.get(at.index()).map(|cue| {
                                    SharedString::from(tr!(
                                        "{}, seek to {}, {}",
                                        if current() {
                                            tr!("Current lyric")
                                        } else {
                                            tr!("Lyrics")
                                        },
                                        lrc::stamp(cue.ms),
                                        cue.text
                                    ))
                                })
                            })
                        })
                        .disabled(move || {
                            model
                                .status
                                .with(|s| s.track.is_none() || s.duration <= 0.0 || s.switching)
                        })
                        .on_press(move || {
                            let cue = timeline.with(|t| t.cues.get(at.index()).map(|cue| cue.ms));
                            let track_key = model
                                .info
                                .with(|info| info.as_ref().map(|info| key(Some(&info.track))));
                            if let (Some(ms), Some(track_key)) = (cue, track_key) {
                                submit(
                                    Action::SeekLyric {
                                        seconds: ms as f64 / 1000.0,
                                        track_key,
                                    },
                                    None,
                                );
                            }
                        })
                        .bg(move || theme().muted.opacity(if current() { 0.55 } else { 0.0 }))
                        .hover(|s| s.bg(theme().muted.opacity(0.55)))
                        .active(|s| s.opacity(0.75))
                        .child(
                            text(move || {
                                timeline.with(|t| {
                                    t.cues
                                        .get(at.index())
                                        .map(|c| {
                                            if !c.words.is_empty() {
                                                String::new()
                                            } else if c.text.is_empty() {
                                                "♪".to_owned()
                                            } else {
                                                c.text.clone()
                                            }
                                        })
                                        .unwrap_or_default()
                                })
                            })
                            .child(each(
                                move || {
                                    timeline.with(|t| {
                                        t.cues.get(at.index()).map_or_else(Vec::new, |c| {
                                            c.words.iter().cloned().enumerate().collect()
                                        })
                                    })
                                },
                                |(index, _)| *index,
                                move |word, _| {
                                    text(move || word.with(|(_, w)| w.text.clone())).text_color(
                                        move || {
                                            model.lyric_clock.get();
                                            let ms =
                                                (playback_time(model).max(0.0) * 1000.0) as u64;
                                            if current() && word.with(|(_, w)| ms >= w.start) {
                                                theme().foreground
                                            } else {
                                                theme().muted_foreground
                                            }
                                        },
                                    )
                                },
                            ))
                            .text_size(px(17.0))
                            .style_with(move |s| {
                                s.font_weight(if current() {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                            })
                            .text_color(move || {
                                if current() {
                                    theme().foreground
                                } else {
                                    theme().muted_foreground
                                }
                            }),
                        )
                        .child(when(
                            move || {
                                timeline.with(|t| {
                                    t.cues
                                        .get(at.index())
                                        .is_some_and(|c| !c.translation.is_empty())
                                })
                            },
                            move || {
                                text(move || {
                                    timeline.with(|t| {
                                        t.cues
                                            .get(at.index())
                                            .map(|c| c.translation.clone())
                                            .unwrap_or_default()
                                    })
                                })
                                .text_size(px(12.0))
                                .mt(px(5.0))
                                .text_color(move || theme().muted_foreground)
                            },
                        ))
                },
            )
            .flex_1()
            .min_h(px(0.0))
            .estimated_item_size(px(48.0))
            .content_padding_y(px(24.0))
            .scrollbar(false)
            .label(tr!("Synced lyrics"))
            .element_ref(&list)
            .on_layout(move |_: &Bounds<Points>| ready.set(true))
            .on_scroll_drag_start(move |_: &Point<Points>| following.set(false)),
        )
}

fn plain(model: Model) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .child(when(
            move || {
                model.info.with(|info| {
                    info.as_ref().is_some_and(|info| {
                        info.lyrics_error.is_none() && !info.lyrics.text.trim().is_empty()
                    })
                })
            },
            || {
                view()
                    .pt(px(12.0))
                    .child(crate::controls::caption(tr!("Full lyrics · No timestamps")))
            },
        ))
        .child(
            scroll().flex_1().child(
                text(move || {
                    model.info.with(|i| {
                        let Some(i) = i else {
                            return if model.is_loading() {
                                tr!("Loading lyrics…").into()
                            } else {
                                String::new()
                            };
                        };
                        if let Some(error) = &i.lyrics_error {
                            return error.clone();
                        }
                        let text = plain_lyrics(&i.lyrics.text);
                        let translation = plain_lyrics(&i.lyrics.translation);
                        if text.is_empty() {
                            tr!("No lyrics for this track").into()
                        } else if translation.is_empty() {
                            text
                        } else {
                            tr!("{text}\n\nTranslation\n{translation}"; text = text, translation = translation)
                        }
                    })
                })
                .text_size(px(15.0))
                .pt(px(12.0))
                .pb(px(20.0))
                .text_color(move || theme().foreground),
            ),
        )
}

pub(super) fn lyrics(model: Model) -> impl IntoElement {
    let timeline = computed(move || {
        Rc::new(model.info.with(|i| {
            i.as_ref()
                .map(|i| Timeline::from_lyrics(&i.lyrics))
                .unwrap_or_default()
        }))
    });
    view()
        .flex_1()
        .min_h(px(0.0))
        .child(when(
            move || timeline.with(|t| !t.cues.is_empty()),
            move || timed(model, timeline),
        ))
        .child(when(
            move || timeline.with(|t| t.cues.is_empty()),
            move || plain(model),
        ))
}

pub(super) fn mini(model: Model) -> impl IntoElement {
    let timeline = computed(move || {
        Rc::new(model.info.with(|info| {
            info.as_ref()
                .map(|i| Timeline::from_lyrics(&i.lyrics))
                .unwrap_or_default()
        }))
    });
    button()
        .accessibility_label(Some(tr!("Open synced lyrics").into()))
        .disabled(move || model.status.with(|s| s.track.is_none()))
        .on_press(move || model.open(Page::Lyrics))
        .w_full()
        .h(px(28.0))
        .justify_center()
        .rounded(px(4.0))
        .text_color(move || theme().muted_foreground)
        .hover(|s| s.text_color(theme().foreground).cursor_pointer())
        .active(|s| s.opacity(0.65))
        .child(
            text(move || {
                model.lyric_clock.get();
                timeline.with(|t| {
                    if let Some(index) = t.active(playback_time(model)) {
                        let line = &t.cues[index].text;
                        if line.is_empty() {
                            tr!("Interlude").to_owned()
                        } else {
                            line.clone()
                        }
                    } else if !t.cues.is_empty() {
                        tr!("Intro").into()
                    } else if model.info.with(Option::is_none) {
                        tr!("Loading lyrics…").into()
                    } else {
                        tr!("No synced lyrics").into()
                    }
                })
            })
            .number_of_lines(1)
            .text_size(px(12.0)),
        )
}
