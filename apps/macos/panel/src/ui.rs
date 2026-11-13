use crate::protocol::tr;
use crate::{
    model::{Model, ago},
    remote::{Action, submit},
};
#[cfg(target_os = "ios")]
use crate::{
    model::{Page, plain_lyrics},
    protocol::{State, Track},
};
#[cfg(target_os = "ios")]
use nsh_base::button;
use nsh_base::theme;
#[cfg(target_os = "ios")]
use nsh_base::{SliderValue, slider};
#[cfg(target_os = "ios")]
use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;
use std::time::Duration;
use tokio::sync::oneshot;

#[cfg(target_os = "macos")]
#[path = "desktop_ui.rs"]
mod desktop_ui;
#[cfg(target_os = "macos")]
use desktop_ui::track_row;
#[cfg(target_os = "macos")]
#[path = "lyrics_ui.rs"]
mod lyrics_ui;
#[cfg(target_os = "macos")]
use desktop_ui::song_info as info;
#[cfg(target_os = "macos")]
#[path = "quality_ui.rs"]
mod quality_ui;
#[cfg(target_os = "macos")]
#[path = "spectrum_ui.rs"]
pub(crate) mod spectrum_ui;

const PLACEHOLDER: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><rect width="512" height="512" fill="#e9e7e1"/><circle cx="256" cy="256" r="118" fill="#dedbd3"/><circle cx="256" cy="256" r="72" fill="#e9e7e1"/><path d="M282 190v117a30 30 0 1 1-16-27v-78l59-14v22z" fill="#939087"/></svg>"##;

fn time(seconds: f64) -> String {
    let whole = seconds.max(0.0) as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

fn playback_busy(model: Model) -> bool {
    model.starting.get()
        || model
            .status
            .with(|s| s.switching || s.state == crate::protocol::State::Loading)
}

fn playback_notices(model: Model) -> impl IntoElement {
    crate::controls::notice_badge(
        move || model.status.with(|s| crate::model::key(s.track.as_ref())),
        move || {
            use crate::controls::Notice;
            let status = model.status.get();
            let mut notices = Vec::new();
            if let Some(error) = status.error.filter(|error| !error.trim().is_empty()) {
                notices.push(Notice::error(error));
            }
            let error = model.error.get();
            if !error.trim().is_empty() && !notices.iter().any(|notice| notice.message == error) {
                notices.push(Notice::error(error));
            }
            if status.audio_uses_http {
                notices.push(Notice::warning(tr!(
                    "This audio source uses an unencrypted HTTP connection"
                )));
            }
            if model.cover_failed.get() {
                notices.push(Notice::warning(tr!("Artwork unavailable")));
            }
            notices
        },
    )
}
#[cfg(target_os = "ios")]
fn mobile() -> bool {
    cfg!(target_os = "ios")
}
#[cfg(target_os = "ios")]
fn cover_size() -> f32 {
    if mobile() {
        (window().height.0 - 420.0)
            .clamp(180.0, 320.0)
            .min(window().width.0 - 48.0)
    } else {
        224.0
    }
}

#[cfg(target_os = "ios")]
fn action(label: &'static str, on_press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .accessibility_label(Some(label.into()))
        .on_press(on_press)
        .h(px(if mobile() { 44.0 } else { 30.0 }))
        .items_center()
        .justify_center()
        .px(px(10.0))
        .rounded(px(7.0))
        .hover(|s| s.bg(theme().muted))
        .active(|s| s.opacity(0.65))
        .child(
            text(label)
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
}

#[cfg(target_os = "ios")]
fn nav_action(label: &'static str, on_press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .accessibility_label(Some(label.into()))
        .on_press(on_press)
        .w(px(if mobile() { 44.0 } else { 36.0 }))
        .min_w(px(if mobile() { 44.0 } else { 36.0 }))
        .h(px(if mobile() { 44.0 } else { 26.0 }))
        .items_center()
        .justify_center()
        .rounded(px(7.0))
        .hover(|s| s.bg(theme().muted))
        .active(|s| s.opacity(0.65))
        .child(
            text(label)
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
}

#[cfg(target_os = "ios")]
fn header(model: Model) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .h(px(if mobile() { 44.0 } else { 26.0 }))
        .gap(px(6.0))
        .child(when(
            move || model.page.get() != Page::Player,
            move || nav_action(tr!("Back"), move || model.open(Page::Player)),
        ))
        .child(
            view().flex_1().min_w(px(0.0)).window_drag(!mobile()).child(
                text(move || match model.page.get() {
                    Page::Info => tr!("Track information"),
                    Page::History => tr!("History"),
                    Page::Album => tr!("Album tracks"),
                    Page::Discovery => tr!("Source releases"),
                    Page::Sources => tr!("Music sources"),
                    Page::Library => tr!("My playlists"),
                    Page::Player => model.status.with(|s| match s.state {
                        State::Playing => tr!("Now playing"),
                        State::Paused => tr!("Paused"),
                        State::Loading => tr!("Buffering"),
                        State::Ended => tr!("Playback ended"),
                        State::Error => tr!("Playback error"),
                        State::Idle => "MusicX",
                    }),
                })
                .text_size(px(11.0))
                .number_of_lines(1)
                .text_color(move || theme().muted_foreground),
            ),
        )
        .child(when(
            move || {
                !matches!(
                    model.page.get(),
                    Page::Player | Page::Sources | Page::Library | Page::Discovery
                )
            },
            move || nav_action(tr!("Refresh"), move || model.open(model.page.get())),
        ))
        .child(when(
            || !mobile(),
            move || {
                view()
                    .flex_row()
                    .w(px(48.0))
                    .min_w(px(48.0))
                    .h(px(24.0))
                    .child(
                        button()
                            .accessibility_label(Some(tr!("Minimize window").into()))
                            .on_press(nsh_core::element::desktop::minimize)
                            .w(px(24.0))
                            .h(px(24.0))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .child(
                                text("−")
                                    .text_size(px(17.0))
                                    .text_color(move || theme().muted_foreground),
                            ),
                    )
                    .child(
                        button()
                            .accessibility_label(Some(tr!("Close window").into()))
                            .on_press(nsh_core::element::desktop::close_window)
                            .w(px(24.0))
                            .h(px(24.0))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .child(
                                text("×")
                                    .text_size(px(17.0))
                                    .text_color(move || theme().muted_foreground),
                            ),
                    )
            },
        ))
}

#[cfg(target_os = "ios")]
fn transport(model: Model, previous: bool) -> impl IntoElement {
    let label = if previous {
        tr!("Previous track")
    } else {
        tr!("Next track")
    };
    button()
        .accessibility_label(Some(label.into()))
        .disabled(move || {
            model.starting.get()
                || model.status.with(|s| {
                    s.switching
                        || s.index.is_none_or(|i| {
                            if previous {
                                i == 0
                            } else {
                                i + 1 >= s.queue_length
                            }
                        })
                })
        })
        .on_press(move || {
            submit(
                if previous {
                    Action::Previous
                } else {
                    Action::Next
                },
                None,
            )
        })
        .w(px(48.0))
        .h(px(48.0))
        .items_center()
        .justify_center()
        .rounded_full()
        .hover(|s| s.bg(theme().muted))
        .disabled_style(|s| s.opacity(0.25))
        .active(|s| s.opacity(0.65))
        .child(
            text(if previous { "⏮" } else { "⏭" })
                .text_size(px(23.0))
                .text_color(move || theme().foreground),
        )
}

#[cfg(target_os = "ios")]
fn player(model: Model) -> impl IntoElement {
    let playing = move || {
        model
            .status
            .with(|s| matches!(s.state, State::Playing | State::Loading))
    };
    let position = move || {
        model
            .scrub
            .get()
            .unwrap_or_else(|| model.status.with(|s| s.position))
    };
    let title = computed(move || {
        model.status.with(|s| {
            s.track.as_ref().map_or_else(
                || tr!("Nothing playing yet").to_owned(),
                |t| t.title.clone(),
            )
        })
    });
    let artist = computed(move || {
        model.status.with(|s| {
            s.track.as_ref().map_or_else(
                || tr!("Ask your agent to choose a track").to_owned(),
                |t| t.artist.clone(),
            )
        })
    });
    let cover = computed(move || {
        model.status.with(|s| {
            s.track
                .as_ref()
                .and_then(|t| t.cover_url.clone())
                .map_or_else(|| ImageSrc::svg_markup(PLACEHOLDER), ImageSrc::from)
        })
    });
    scroll().flex_1().scrollbar(false).child(
        view()
            .pb(px(10.0))
            .child(
                view().items_center().mt(px(10.0)).child(
                    button()
                        .accessibility_label(Some(tr!("Browse this album").into()))
                        .disabled(move || {
                            model
                                .status
                                .with(|s| s.track.as_ref().is_none_or(|t| t.album.is_none()))
                        })
                        .on_press(move || model.open(Page::Album))
                        .rounded(px(10.0))
                        .child(
                            image(move || cover.get())
                                .w(move || px(cover_size()))
                                .h(move || px(cover_size()))
                                .rounded(px(10.0))
                                .object_fit(ResizeMode::Cover)
                                .label(move || SharedString::from(tr!("Cover for {}", title.get())))
                                .placeholder(ImageSrc::svg_markup(PLACEHOLDER))
                                .on_error(move |_: &str| model.cover_failed.set(true))
                                .on_load(move |_: &nsh_core::Size<DevicePixels>| {
                                    model.cover_failed.set(false);
                                    nsh_core::image::loader::loader().clear();
                                    nsh_core::image::budget().clear();
                                }),
                        ),
                ),
            )
            .child(
                view()
                    .mt(px(16.0))
                    .window_drag(!mobile())
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                text(move || title.get())
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_size(px(if mobile() { 25.0 } else { 21.0 }))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .number_of_lines(1)
                                    .text_color(move || theme().foreground),
                            )
                            .child(playback_notices(model)),
                    )
                    .child(
                        text(move || artist.get())
                            .text_size(px(13.0))
                            .number_of_lines(1)
                            .mt(px(3.0))
                            .text_color(move || theme().muted_foreground),
                    )
                    .child(when(
                        move || {
                            model
                                .status
                                .with(|s| s.track.as_ref().is_some_and(|t| t.album.is_some()))
                        },
                        move || {
                            button()
                                .accessibility_label(Some(tr!("View album tracks").into()))
                                .on_press(move || model.open(Page::Album))
                                .mt(px(4.0))
                                .child(
                                    text(move || {
                                        model.status.with(|s| {
                                            s.track
                                                .as_ref()
                                                .and_then(|t| t.album.as_ref())
                                                .map_or_else(String::new, |a| {
                                                    format!("{}  ›", a.name)
                                                })
                                        })
                                    })
                                    .text_size(px(11.0))
                                    .number_of_lines(1)
                                    .text_color(move || theme().muted_foreground),
                                )
                        },
                    )),
            )
            .child(
                view()
                    .mt(px(12.0))
                    .child(
                        slider()
                            .value(move || Some(SliderValue::Single(position())))
                            .max(move || model.status.with(|s| s.duration.max(1.0)))
                            .disabled(move || model.status.with(|s| s.duration <= 0.0))
                            .accessibility_label(Some(tr!("Playback position").into()))
                            .track_class(Some("h-5 justify-center".into()))
                            .indicator_class(Some("h-1 rounded-full bg-muted".into()))
                            .range_class(Some("rounded-full bg-primary".into()))
                            .thumb_class(Some(
                                "size-2 rounded-full bg-primary disabled:bg-muted-foreground"
                                    .into(),
                            ))
                            .on_change(move |&v: &SliderValue| {
                                if let SliderValue::Single(v) = v {
                                    model.scrub.set(Some(v));
                                }
                            })
                            .on_change_end(move |&v: &SliderValue| {
                                if let SliderValue::Single(v) = v {
                                    submit(Action::Seek(v), None);
                                }
                                model.scrub.set(None);
                            }),
                    )
                    .child(
                        view()
                            .flex_row()
                            .justify_between()
                            .child(
                                text(move || time(position()))
                                    .text_size(px(10.0))
                                    .text_color(move || theme().muted_foreground),
                            )
                            .child(
                                text(move || model.status.with(|s| time(s.duration)))
                                    .text_size(px(10.0))
                                    .text_color(move || theme().muted_foreground),
                            ),
                    ),
            )
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap(px(22.0))
                    .mt(px(12.0))
                    .child(transport(model, true))
                    .child(
                        button()
                            .accessibility_label(move || {
                                Some(if playback_busy(model) {
                                    tr!("Loading audio").into()
                                } else if playing() {
                                    tr!("Pause").into()
                                } else {
                                    tr!("Play").into()
                                })
                            })
                            .disabled(move || {
                                model.starting.get()
                                    || model.status.with(|s| s.track.is_none() || s.switching)
                            })
                            .on_press(move || {
                                submit(
                                    if playing() {
                                        Action::Pause
                                    } else {
                                        Action::Resume
                                    },
                                    None,
                                )
                            })
                            .w(px(52.0))
                            .h(px(52.0))
                            .rounded_full()
                            .items_center()
                            .justify_center()
                            .bg(move || theme().primary)
                            .disabled_style(|s| s.opacity(0.25))
                            .active(|s| s.opacity(0.65))
                            .child(
                                when(
                                    move || playback_busy(model),
                                    crate::controls::loading_indicator,
                                )
                                .otherwise(move || {
                                    text(move || if playing() { "Ⅱ" } else { "▶" })
                                        .text_size(px(21.0))
                                        .text_color(move || theme().primary_foreground)
                                }),
                            ),
                    )
                    .child(transport(model, false)),
            )
            .child(
                view()
                    .flex_row()
                    .justify_center()
                    .gap(px(14.0))
                    .mt(px(12.0))
                    .child(
                        button()
                            .accessibility_label(Some(
                                tr!("View track information and lyrics").into(),
                            ))
                            .disabled(move || model.status.with(|s| s.track.is_none()))
                            .on_press(move || model.open(Page::Info))
                            .h(px(if mobile() { 44.0 } else { 30.0 }))
                            .px(px(10.0))
                            .child(
                                text(tr!("Track information"))
                                    .text_size(px(12.0))
                                    .text_color(move || theme().muted_foreground),
                            ),
                    )
                    .child(action(tr!("History"), move || model.open(Page::History))),
            )
            .child(when(mobile, move || {
                view()
                    .flex_row()
                    .justify_center()
                    .child(action(tr!("My playlists"), move || {
                        model.open(Page::Library)
                    }))
                    .child(action(tr!("Music sources"), move || {
                        model.open(Page::Sources)
                    }))
            })),
    )
}

#[cfg(target_os = "ios")]
fn track_row(
    track: impl Fn() -> Track + Copy + 'static,
    subtitle: impl Fn() -> String + 'static,
    model: Model,
) -> impl IntoElement {
    button()
        .accessibility_label(move || {
            Some(SharedString::from(tr!(
                "Play {}, {}",
                track().title,
                track().artist
            )))
        })
        .disabled(move || model.starting.get())
        .on_press(move || model.play(track()))
        .w_full()
        .min_h(px(62.0))
        .py(px(10.0))
        .px(px(8.0))
        .rounded(px(8.0))
        .hover(|s| s.bg(theme().muted))
        .active(|s| s.opacity(0.7))
        .child(
            text(move || track().title)
                .text_size(px(14.0))
                .number_of_lines(1)
                .text_color(move || theme().foreground),
        )
        .child(
            text(subtitle)
                .text_size(px(11.0))
                .number_of_lines(1)
                .mt(px(4.0))
                .text_color(move || theme().muted_foreground),
        )
}

fn history(model: Model) -> impl IntoElement {
    scroll().flex_1().child(
        view()
            .gap(px(2.0))
            .mt(px(12.0))
            .child(when(
                move || !model.is_loading() && model.history.with(Vec::is_empty),
                || {
                    text(tr!("No listening history yet"))
                        .text_size(px(13.0))
                        .text_color(move || theme().muted_foreground)
                },
            ))
            .child(each(
                move || model.history.get(),
                |row| row.0,
                move |row, _| {
                    track_row(
                        move || row.with(|row| row.1.track.clone()),
                        move || {
                            row.with(|row| {
                                format!("{} · {}", row.1.track.artist, ago(row.1.played_at_unix))
                            })
                        },
                        model,
                        #[cfg(target_os = "macos")]
                        move || row.with(|row| row.0),
                        #[cfg(target_os = "macos")]
                        move || {
                            row.with(|row| {
                                row.1.progress.map(|progress| {
                                    if progress.completed {
                                        tr!("Finished").into()
                                    } else if progress.duration > 0.0 {
                                        format!(
                                            "{} / {}",
                                            time(progress.position),
                                            time(progress.duration)
                                        )
                                    } else {
                                        tr!("Played {}", time(progress.position))
                                    }
                                })
                            })
                        },
                    )
                },
            )),
    )
}

fn album(model: Model) -> impl IntoElement {
    scroll().flex_1().child(
        view()
            .mt(px(14.0))
            .gap(px(4.0))
            .child(
                text(move || {
                    model.album.with(|a| {
                        a.as_ref()
                            .map_or_else(String::new, |a| a.album.name.clone())
                    })
                })
                .text_size(px(21.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
            )
            .child(each(
                move || {
                    model.album.with(|a| {
                        a.as_ref().map_or_else(Vec::new, |a| {
                            a.tracks.iter().cloned().enumerate().collect::<Vec<_>>()
                        })
                    })
                },
                |row| row.0,
                move |row, _| {
                    track_row(
                        move || row.with(|row| row.1.clone()),
                        move || {
                            row.with(|row| format!("{} · {}", row.1.artist, time(row.1.duration)))
                        },
                        model,
                        #[cfg(target_os = "macos")]
                        move || row.with(|row| row.0),
                        #[cfg(target_os = "macos")]
                        || None,
                    )
                },
            ))
            .child(when(
                move || {
                    !model.is_loading()
                        && model
                            .album
                            .with(|a| a.as_ref().is_some_and(|a| a.tracks.is_empty()))
                },
                || {
                    text(tr!("This source returned no album tracks"))
                        .text_size(px(13.0))
                        .text_color(move || theme().muted_foreground)
                },
            )),
    )
}

#[cfg(target_os = "ios")]
fn info(model: Model) -> impl IntoElement {
    scroll().flex_1().child(
        view()
            .child(crate::discovery_ui::artist_links(model))
            .child(crate::discovery_ui::episode_details(model))
            .mt(px(16.0))
            .gap(px(8.0))
            .child(
                text(move || {
                    model.status.with(|s| {
                        s.track
                            .as_ref()
                            .map_or_else(String::new, |t| t.title.clone())
                    })
                })
                .text_size(px(22.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
            )
            .child(
                text(move || {
                    model.status.with(|s| {
                        s.track.as_ref().map_or_else(String::new, |t| {
                            format!("{} · {}", t.artist, time(t.duration))
                        })
                    })
                })
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground),
            )
            .child(when(
                move || {
                    model
                        .status
                        .with(|s| s.track.as_ref().is_some_and(|t| t.album.is_some()))
                },
                move || {
                    action(tr!("View album tracks ›"), move || {
                        model.open(Page::Album)
                    })
                },
            ))
            .child(
                button()
                    .accessibility_label(Some(tr!("Open track source link").into()))
                    .disabled(move || {
                        model
                            .info
                            .with(|i| i.as_ref().is_none_or(|i| i.source_link.is_none()))
                    })
                    .on_press(move || {
                        if let Some(url) = model.info.with(|i| {
                            i.as_ref()
                                .and_then(|i| i.source_link.as_ref().map(|link| link.url.clone()))
                        }) {
                            spawn(async move {
                                let _ = open_url(url).await;
                            });
                        }
                    })
                    .py(px(8.0))
                    .child(
                        text(move || {
                            model.info.with(|i| {
                                i.as_ref().map_or_else(
                                    || tr!("Track source ↗").to_owned(),
                                    |i| {
                                        i.source_link.as_ref().map_or_else(
                                            || i.source_name.clone(),
                                            |link| format!("{} · {} ↗", i.source_name, link.label),
                                        )
                                    },
                                )
                            })
                        })
                        .text_size(px(13.0))
                        .text_color(move || theme().foreground),
                    )
                    .child(
                        text(move || {
                            model.info.with(|i| {
                                i.as_ref()
                                    .and_then(|i| {
                                        i.source_link.as_ref().map(|link| link.url.clone())
                                    })
                                    .unwrap_or_else(|| tr!("This source has no link for this track").into())
                            })
                        })
                        .text_size(px(10.0))
                        .number_of_lines(2)
                        .text_color(move || theme().muted_foreground),
                    ),
            )
            .child(
                text(tr!("Lyrics"))
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .mt(px(16.0))
                    .text_color(move || theme().foreground),
            )
            .child(
                text(move || {
                    model.info.with(|i| {
                        i.as_ref().map_or_else(
                            || {
                                if model.is_loading() {
                                    tr!("Loading lyrics…").into()
                                } else {
                                    String::new()
                                }
                            },
                            |i| {
                                if let Some(error) = &i.lyrics_error {
                                    return error.clone();
                                }
                                let original = plain_lyrics(&i.lyrics.text);
                                let translation = plain_lyrics(&i.lyrics.translation);
                                if original.is_empty() {
                                    tr!("No lyrics for this track").into()
                                } else if translation.is_empty() {
                                    original
                                } else {
                                    tr!("{original}\n\nTranslation\n{translation}"; original = original, translation = translation)
                                }
                            },
                        )
                    })
                })
                .text_size(px(15.0))
                .text_color(move || theme().foreground)
                .pb(px(20.0)),
            ),
    )
}

#[component]
/// Native playback screen with on-demand album, lyrics and local history.
pub fn App() -> impl IntoElement {
    let model = Model::new();
    #[cfg(target_os = "macos")]
    crate::desktop::menus::connect(model);
    #[cfg(target_os = "ios")]
    if let Err(error) = crate::initialize() {
        model
            .error
            .set(tr!("Player failed to start: {error}"; error = error));
    }
    let refreshing = signal(false);
    set_interval(Duration::from_millis(500), move || {
        model.refresh_language();
        if refreshing.get() {
            return;
        }
        refreshing.set(true);
        let (send, receive) = oneshot::channel();
        submit(Action::DisplayStatus, Some(send));
        spawn(async move {
            match receive.await {
                Ok(Ok(status)) => model.update(status),
                Ok(Err(error)) => model.error.set(error.to_string()),
                Err(_) => {}
            }
            refreshing.set(false);
        });
    });
    // Keep the application root at window height while its localized children
    // are replaced. A bare root region otherwise gets an intrinsic-size host.
    view().flex_1().child(dynamic(move || {
        model.language.get();
        // Only a language change should recreate static labels. Playback ticks
        // keep their fine-grained bindings and the same authoritative Model.
        untracked(|| {
            #[cfg(target_os = "macos")]
            {
                desktop_ui::shell(model)
            }
            #[cfg(target_os = "ios")]
            {
                mobile_shell(model)
            }
        })
    }))
}

#[cfg(target_os = "ios")]
fn mobile_shell(model: Model) -> impl IntoElement {
    view()
        .flex_1()
        .bg(move || theme().background)
        .rounded(px(if mobile() { 0.0 } else { 16.0 }))
        .p_safe_area()
        .child(
            view()
                .flex_1()
                .px(px(24.0))
                .pt(px(12.0))
                .pb(px(12.0))
                .child(header(model))
                .child(when(
                    move || model.page.get() == Page::Player,
                    move || player(model),
                ))
                .child(when(
                    move || model.page.get() == Page::Info,
                    move || info(model),
                ))
                .child(when(
                    move || model.page.get() == Page::History,
                    move || history(model),
                ))
                .child(when(
                    move || model.page.get() == Page::Album,
                    move || album(model),
                ))
                .child(when(
                    move || model.page.get() == Page::Sources,
                    move || crate::sources_ui::settings_view(model),
                ))
                .child(when(
                    move || model.page.get() == Page::Discovery,
                    move || crate::discovery_ui::browse(model),
                ))
                .child(when(
                    move || model.page.get() == Page::Library,
                    move || crate::library::library_view(model),
                ))
                .child(when(
                    move || model.page.get() != Page::Player && model.is_loading(),
                    || {
                        text(tr!("Loading…"))
                            .text_size(px(11.0))
                            .text_color(move || theme().muted_foreground)
                    },
                ))
                .child(
                    text(move || {
                        if model.page.get() != Page::Player
                            || model.status.with(|s| s.track.is_none())
                        {
                            model.error.get()
                        } else {
                            String::new()
                        }
                    })
                    .text_size(px(11.0))
                    .number_of_lines(2)
                    .text_color(move || theme().muted_foreground),
                ),
        )
}
