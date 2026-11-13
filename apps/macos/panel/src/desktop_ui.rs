//! Mac-only presentation. The shared iPhone layout stays independent.
use super::{PLACEHOLDER, time};
use crate::protocol::tr;
use crate::{
    model::{Model, Page, key},
    protocol::{State, Track},
    remote::{Action, submit},
};
use nsh_base::{SliderValue, button, slider, theme};
use nsh_core::{element::ImageSrc, prelude::*};

#[derive(Clone, Copy)]
enum Icon {
    Play,
    Pause,
    Previous,
    Next,
    History,
    Queue,
    Playlists,
    Heart,
    HeartFilled,
    Refresh,
    Minimize,
    Close,
}

impl Icon {
    fn markup(self) -> &'static str {
        match self {
            Self::Play => {
                r#"<svg viewBox="0 0 24 24"><path d="M8 4.8a1 1 0 0 1 1.5-.86l12 7.2a1 1 0 0 1 0 1.72l-12 7.2a1 1 0 0 1-1.5-.86Z" fill="currentColor"/></svg>"#
            }
            Self::Pause => {
                r#"<svg viewBox="0 0 24 24"><rect x="6" y="4" width="4" height="16" rx="1" fill="currentColor"/><rect x="14" y="4" width="4" height="16" rx="1" fill="currentColor"/></svg>"#
            }
            Self::Previous => {
                r#"<svg viewBox="0 0 24 24"><path d="M7 5v14M19 5.5 8.5 12 19 18.5Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round" stroke-linecap="round" fill="currentColor"/></svg>"#
            }
            Self::Next => {
                r#"<svg viewBox="0 0 24 24"><path d="M17 5v14M5 5.5 15.5 12 5 18.5Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round" stroke-linecap="round" fill="currentColor"/></svg>"#
            }
            Self::History => {
                r#"<svg viewBox="0 0 24 24"><path d="M3.5 10a8.5 8.5 0 1 1 1.3 7.2M3.5 4.5V10H9M12 7.5V12l3 2" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>"#
            }
            Self::Queue => {
                r#"<svg viewBox="0 0 24 24"><path d="M4 6h16M4 12h7M4 18h7" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/><path d="m14 10.5 7 4.5-7 4.5Z" fill="currentColor" stroke="currentColor" stroke-width=".7" stroke-linejoin="round"/></svg>"#
            }
            Self::Playlists => {
                r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><path d="M16 4H6a2 2 0 0 0-2 2v10"/><rect x="8" y="8" width="12" height="12" rx="2"/><circle cx="14" cy="14" r="2.5"/></svg>"#
            }
            Self::Heart | Self::HeartFilled => {
                if matches!(self, Self::HeartFilled) {
                    r#"<svg viewBox="0 0 24 24"><path d="M20.3 5.7a5.3 5.3 0 0 0-7.5 0L12 6.5l-.8-.8a5.3 5.3 0 0 0-7.5 7.5L12 21l8.3-7.8a5.3 5.3 0 0 0 0-7.5Z" fill="currentColor"/></svg>"#
                } else {
                    r#"<svg viewBox="0 0 24 24"><path d="M20.3 5.7a5.3 5.3 0 0 0-7.5 0L12 6.5l-.8-.8a5.3 5.3 0 0 0-7.5 7.5L12 21l8.3-7.8a5.3 5.3 0 0 0 0-7.5Z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>"#
                }
            }
            Self::Refresh => {
                r#"<svg viewBox="0 0 24 24"><path d="M19 9a7.5 7.5 0 1 0 0 7M19 4v5h-5" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>"#
            }
            Self::Minimize => {
                r#"<svg viewBox="0 0 24 24"><path d="M6 12h12" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>"#
            }
            Self::Close => {
                r#"<svg viewBox="0 0 24 24"><path d="m7 7 10 10M17 7 7 17" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>"#
            }
        }
    }
}

fn icon(which: impl Fn() -> Icon + 'static, size: f32, inverted: bool) -> impl IntoElement {
    svg(move || ImageSrc::svg_markup(which().markup()))
        .w(px(size))
        .h(px(size))
        .text_color(move || {
            if inverted {
                theme().primary_foreground
            } else {
                theme().foreground
            }
        })
}

fn hint(label: &'static str) -> nsh_core::element::Text {
    text(label)
        .text_size(px(11.0))
        .px(px(6.0))
        .py(px(4.0))
        .bg(move || theme().primary)
        .text_color(move || theme().primary_foreground)
        .rounded(px(4.0))
}

fn small_button(
    label: &'static str,
    which: Icon,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view().tooltip(hint(label)).child(
        button()
            .accessibility_label(Some(label.into()))
            .on_press(on_press)
            .w(px(26.0))
            .min_w(px(26.0))
            .h(px(26.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .hover(|s| s.bg(theme().muted))
            .active(|s| s.opacity(0.6))
            .child(icon(move || which, 16.0, false)),
    )
}

fn header(model: Model) -> impl IntoElement {
    view()
        .flex_row()
        .window_drag(true)
        .items_center()
        .h(px(26.0))
        .gap(px(2.0))
        .child(view().flex_1().min_w(px(12.0)).h_full().window_drag(true))
        .child(now_playing_button(model))
        .child(
            view()
                .w(px(1.0))
                .h(px(10.0))
                .mx(px(3.0))
                .bg(move || theme().muted),
        )
        .child(small_button(
            tr!("Minimize window"),
            Icon::Minimize,
            nsh_core::element::desktop::minimize,
        ))
        .child(small_button(
            tr!("Close window"),
            Icon::Close,
            nsh_core::element::desktop::close_window,
        ))
}

fn now_playing_button(model: Model) -> impl IntoElement {
    let cover = computed(move || {
        model
            .status
            .with(|s| s.track.as_ref().and_then(|t| t.cover_url.clone()))
    });
    view().tooltip(hint(tr!("Now playing"))).child(
        button()
            .accessibility_label(Some(tr!("Open now playing").into()))
            .on_press(move || {
                if model.drawer_open.get() && model.page.get() == Page::Queue {
                    model.open(Page::Player);
                } else {
                    model.open(Page::Queue);
                }
            })
            .selected(move || model.drawer_open.get() && model.page.get() == Page::Queue)
            .selected_style(|s| s.bg(theme().muted))
            .w(px(26.0))
            .h(px(26.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .hover(|s| s.bg(theme().muted).cursor_pointer())
            .active(|s| s.opacity(0.65))
            .child(
                when(
                    move || cover.with(Option::is_some),
                    move || {
                        crate::controls::artwork(
                            move || cover.get(),
                            || tr!("Current track artwork").into(),
                            22.0,
                        )
                    },
                )
                .otherwise(|| icon(|| Icon::Queue, 15.0, false)),
            ),
    )
}

fn transport(model: Model, previous: bool) -> impl IntoElement {
    button()
        .accessibility_label(Some(
            if previous {
                tr!("Previous track")
            } else {
                tr!("Next track")
            }
            .into(),
        ))
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
        .w(px(44.0))
        .h(px(44.0))
        .items_center()
        .justify_center()
        .rounded_full()
        .hover(|s| s.bg(theme().muted))
        .active(|s| s.opacity(0.6))
        .disabled_style(|s| s.opacity(0.22))
        .child(icon(
            move || if previous { Icon::Previous } else { Icon::Next },
            22.0,
            false,
        ))
}

fn destination(model: Model, label: &'static str, which: Icon, page: Page) -> impl IntoElement {
    view().tooltip(hint(label)).child(
        button()
            .accessibility_label(Some(label.into()))
            .on_press(move || {
                if model.page.get() != page {
                    model.open(page);
                }
            })
            .selected(move || model.page.get() == page)
            .selected_style(|s| s.bg(theme().muted.opacity(0.68)))
            .w(px(24.0))
            .min_w(px(24.0))
            .h(px(26.0))
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .hover(|s| s.bg(theme().muted.opacity(0.68)))
            .active(|s| s.opacity(0.6))
            .child(icon(move || which, 15.0, false)),
    )
}

fn details_navigation(model: Model) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .child(destination(
            model,
            tr!("Now playing"),
            Icon::Queue,
            Page::Queue,
        ))
        .child(destination(
            model,
            tr!("History"),
            Icon::History,
            Page::History,
        ))
        .child(destination(
            model,
            tr!("My playlists"),
            Icon::Playlists,
            Page::Library,
        ))
}

pub(super) fn shell(model: Model) -> impl IntoElement {
    let hint_left = signal(crate::desktop::preferred_details_left());
    set_interval(std::time::Duration::from_millis(150), move || {
        if !model.drawer_open.get() {
            let left = crate::desktop::preferred_details_left();
            if hint_left.get() != left {
                hint_left.set(left);
            }
        }
    });
    set_interval(std::time::Duration::from_millis(100), move || {
        if model.status.with(|s| s.state == State::Playing)
            && (model.show_mini_lyrics.get()
                || (model.page.get() == Page::Lyrics && model.drawer_open.get()))
        {
            model.lyric_clock.update(|n| *n = n.wrapping_add(1));
        }
    });
    view()
        .flex_1()
        .overflow_hidden()
        .child(
            view()
                .w(px(crate::desktop::PLAYER_WIDTH))
                .min_w(px(crate::desktop::PLAYER_WIDTH))
                .h_full()
                .ml(move || {
                    px(
                        if model.details_left.get()
                            && window().width.0 >= crate::desktop::EXPANDED_WIDTH - 1.0
                        {
                            crate::desktop::DETAILS_WIDTH + crate::desktop::DETAILS_GAP
                        } else {
                            0.0
                        },
                    )
                })
                .bg(move || theme().background)
                .rounded(px(16.0))
                .overflow_hidden()
                .px(px(24.0))
                .pt(px(12.0))
                .pb(px(12.0))
                .child(header(model))
                // The player remains mounted while its details are open.
                .child(player(model)),
        )
        .child(when(
            move || {
                !model.drawer_open.get() && window().width.0 < crate::desktop::EXPANDED_WIDTH - 1.0
            },
            move || {
                crate::drawer::details_hint()
                    .left(hint_left)
                    .on_press(move || model.open(Page::Queue))
            },
        ))
        .child(
            crate::drawer::details_drawer()
                .open(move || {
                    model.drawer_open.get()
                        && window().width.0 >= crate::desktop::EXPANDED_WIDTH - 1.0
                })
                .left(move || model.details_left.get())
                .on_open_change(move |open: &bool| {
                    if !open {
                        model.open(Page::Player);
                    }
                })
                .on_open(move || {
                    if model.drawer_open.get() {
                        model.drawer_entered.set(true);
                    }
                })
                .on_close(move || model.drawer_closed())
                .child({
                    view()
                        .flex_1()
                        .min_h(px(0.0))
                        .h_full()
                        .overflow_hidden()
                        .px(px(20.0))
                        .pt(px(12.0))
                        .pb(px(16.0))
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .h(px(28.0))
                                .child(
                                    view().flex_1().min_w(px(0.0)).child(
                                        text(move || match model.page.get() {
                                            Page::Info => tr!("Track information"),
                                            Page::Lyrics => tr!("Lyrics"),
                                            Page::Sources => tr!("Music sources"),
                                            Page::History => tr!("History"),
                                            Page::Library => tr!("My playlists"),
                                            Page::Queue => tr!("Now playing"),
                                            Page::Album => tr!("Album tracks"),
                                            _ => tr!("Details"),
                                        })
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                    ),
                                )
                                .child(details_navigation(model))
                                .child(view().w(px(26.0)).min_w(px(26.0)).child(when(
                                    move || {
                                        matches!(
                                            model.page.get(),
                                            Page::Info | Page::Lyrics | Page::History | Page::Album
                                        )
                                    },
                                    move || {
                                        small_button(
                                            tr!("Refresh details"),
                                            Icon::Refresh,
                                            move || model.open(model.page.get()),
                                        )
                                    },
                                )))
                                .child(small_button(
                                    tr!("Close details"),
                                    Icon::Close,
                                    move || model.open(Page::Player),
                                )),
                        )
                        .child(when(
                            move || model.page.get() == Page::Lyrics,
                            move || super::lyrics_ui::lyrics(model),
                        ))
                        .child(when(
                            move || model.page.get() == Page::Sources,
                            move || {
                                crate::source_browser::sources_view(
                                    move |action| model.open_source(action),
                                    move || model.open(Page::Player),
                                    move || model.open(Page::Library),
                                    None,
                                    crate::source_browser::Start::Catalog,
                                )
                            },
                        ))
                        .child(when(
                            move || model.page.get() == Page::Info,
                            move || super::info(model),
                        ))
                        .child(when(
                            move || model.page.get() == Page::Discovery,
                            move || crate::discovery_ui::browse(model),
                        ))
                        .child(when(
                            move || model.page.get() == Page::History,
                            move || super::history(model),
                        ))
                        .child(when(
                            move || model.page.get() == Page::Album,
                            move || super::album(model),
                        ))
                        .child(when(
                            move || model.page.get() == Page::Queue,
                            move || queue(model),
                        ))
                        .child(when(
                            move || model.page.get() == Page::Library,
                            move || crate::library::library_view(model),
                        ))
                        .child(when(
                            move || model.is_loading() || model.starting.get(),
                            move || {
                                text(move || {
                                    if model.starting.get() {
                                        tr!("Preparing playback…")
                                    } else {
                                        tr!("Loading…")
                                    }
                                })
                                .text_size(px(11.0))
                                .text_color(move || theme().muted_foreground)
                            },
                        ))
                        .child(when(
                            move || model.error.with(|e| !e.is_empty()),
                            move || {
                                text(move || model.error.get())
                                    .text_size(px(11.0))
                                    .number_of_lines(3)
                                    .text_color(move || theme().muted_foreground)
                            },
                        ))
                }),
        )
}

pub(super) fn player(model: Model) -> impl IntoElement {
    when(
        move || model.status_ready.get(),
        move || {
            when(
                move || model.status.with(|s| s.track.is_some()),
                move || active_player(model),
            )
            .otherwise(move || empty_player(model))
        },
    )
    .otherwise(move || {
        view()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .child(crate::controls::caption(tr!("Connecting to player…")))
            .child(when(
                move || model.error.with(|error| !error.is_empty()),
                move || crate::controls::caption(move || model.error.get()),
            ))
    })
}

fn setup_action(label: &'static str, press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .accessibility_label(Some(label.into()))
        .on_press(press)
        .w_full()
        .h(px(40.0))
        .items_center()
        .justify_center()
        .rounded(px(7.0))
        .bg(move || theme().primary)
        .hover(|s| s.opacity(0.86))
        .active(|s| s.opacity(0.65))
        .child(
            text(label)
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().primary_foreground),
        )
}

fn empty_player(model: Model) -> impl IntoElement {
    // This is an empty listening session, not an album or lyrics loading state.
    view()
        .flex_1()
        .justify_center()
        .pb(px(24.0))
        .child(
            view()
                .items_center()
                .child(
                    svg(|| {
                        ImageSrc::svg_markup(
                            &include_str!("../../../../assets/musicx-mark.svg")
                                .replace("#1b2421", "currentColor"),
                        )
                    })
                    .w(px(48.0))
                    .h(px(48.0))
                    .text_color(move || theme().foreground)
                    .label("MusicX"),
                )
                .child(
                    text(tr!("Add a music source"))
                        .mt(px(24.0))
                        .text_size(px(20.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(tr!("Connect your music service\nand start listening."))
                        .mt(px(12.0))
                        .text_size(px(12.0))
                        .text_center()
                        .number_of_lines(2)
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            view()
                .mt(px(24.0))
                .child(setup_action(tr!("Add music source"), move || {
                    model.open(Page::Sources)
                })),
        )
        .child(
            view()
                .items_center()
                .mt(px(20.0))
                .gap(px(4.0))
                .child(crate::controls::action(
                    tr!("Connect your agent"),
                    || false,
                    || crate::desktop::open_settings(crate::sources_ui::SettingsGroup::Mcp),
                ))
                .child(
                    text(tr!("Choose music and control playback through chat"))
                        .text_size(px(11.0))
                        .text_center()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(when(
            move || model.error.with(|error| !error.is_empty()),
            move || crate::controls::caption(move || model.error.get()),
        ))
}

fn active_player(model: Model) -> impl IntoElement {
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
            .pb(px(8.0))
            .child(
                view().items_center().mt(px(8.0)).child(
                    button()
                        .accessibility_label(Some(tr!("Browse this album").into()))
                        .disabled(move || {
                            model
                                .status
                                .with(|s| s.track.as_ref().is_none_or(|t| t.album.is_none()))
                        })
                        .on_press(move || model.open(Page::Album))
                        .rounded(px(8.0))
                        .active(|s| s.opacity(0.9))
                        .child(
                            image(move || cover.get())
                                .w(px(248.0))
                                .h(px(248.0))
                                .rounded(px(8.0))
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
                    .mt(px(12.0))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                button()
                                    .accessibility_label(Some(tr!("View track information").into()))
                                    .disabled(move || model.status.with(|s| s.track.is_none()))
                                    .on_press(move || model.open(Page::Info))
                                    .self_start()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .rounded(px(4.0))
                                    .hover(|s| s.bg(theme().muted))
                                    .active(|s| s.opacity(0.65))
                                    .child(
                                        text(move || title.get())
                                            .text_size(px(20.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .number_of_lines(1)
                                            .text_color(move || theme().foreground),
                                    ),
                            )
                            .child(super::playback_notices(model))
                            .child(
                                button()
                                    .accessibility_label(move || {
                                        Some(SharedString::from(
                                            if model.favorite.with(|f| {
                                                f.liked
                                                    && f.key
                                                        == model
                                                            .status
                                                            .with(|s| key(s.track.as_ref()))
                                            }) {
                                                tr!("Unlike this track")
                                            } else {
                                                tr!("Like this track")
                                            },
                                        ))
                                    })
                                    .disabled(move || {
                                        model.favorite_busy.get()
                                            || model.status.with(|s| s.track.is_none())
                                    })
                                    .on_press(move || model.toggle_favorite())
                                    .w(px(28.0))
                                    .h(px(28.0))
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(5.0))
                                    .hover(|s| s.bg(theme().muted))
                                    .active(|s| s.opacity(0.65))
                                    .disabled_style(|s| s.opacity(0.4))
                                    .selected(move || {
                                        model.favorite.with(|f| {
                                            f.liked
                                                && f.key
                                                    == model.status.with(|s| key(s.track.as_ref()))
                                        })
                                    })
                                    .child(icon(
                                        move || {
                                            if model.favorite.with(|f| {
                                                f.liked
                                                    && f.key
                                                        == model
                                                            .status
                                                            .with(|s| key(s.track.as_ref()))
                                            }) {
                                                Icon::HeartFilled
                                            } else {
                                                Icon::Heart
                                            }
                                        },
                                        18.0,
                                        false,
                                    )),
                            ),
                    )
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .mt(px(3.0))
                            .child(
                                text(move || {
                                    model.status.with(|s| {
                                        s.track.as_ref().map_or_else(
                                            || tr!("Choose music with your agent").into(),
                                            |t| t.artist.clone(),
                                        )
                                    })
                                })
                                .max_w(px(88.0))
                                .number_of_lines(1)
                                .text_size(px(11.0))
                                .text_color(move || theme().muted_foreground),
                            )
                            .child(when(
                                move || {
                                    model.status.with(|s| {
                                        s.track.as_ref().is_some_and(|t| t.album.is_some())
                                    })
                                },
                                move || {
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .flex_row()
                                        .items_center()
                                        .gap(px(2.0))
                                        .child(
                                            text("·")
                                                .text_size(px(11.0))
                                                .text_color(move || theme().muted_foreground),
                                        )
                                        .child(
                                            button()
                                                .accessibility_label(Some(
                                                    tr!("View album tracks").into(),
                                                ))
                                                .on_press(move || model.open(Page::Album))
                                                .flex_1()
                                                .min_w(px(0.0))
                                                .py(px(4.0))
                                                .px(px(3.0))
                                                .rounded(px(4.0))
                                                .hover(|s| s.bg(theme().muted))
                                                .active(|s| s.opacity(0.65))
                                                .child(
                                                    text(move || {
                                                        model.status.with(|s| {
                                                            s.track
                                                                .as_ref()
                                                                .and_then(|t| t.album.as_ref())
                                                                .map_or_else(String::new, |a| {
                                                                    a.name.clone()
                                                                })
                                                        })
                                                    })
                                                    .number_of_lines(1)
                                                    .text_size(px(11.0))
                                                    .text_color(move || theme().muted_foreground),
                                                ),
                                        )
                                },
                            ))
                            .child(super::quality_ui::badge(model)),
                    )
                    .child(view().h(px(28.0)).child(when(
                        move || model.show_mini_lyrics.get(),
                        move || super::lyrics_ui::mini(model),
                    ))),
            )
            .child(
                view()
                    .mt(px(6.0))
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
                    .gap(px(24.0))
                    .mt(px(8.0))
                    .child(transport(model, true))
                    .child(
                        button()
                            .accessibility_label(move || {
                                Some(
                                    if super::playback_busy(model) {
                                        tr!("Loading audio")
                                    } else if playing() {
                                        tr!("Pause")
                                    } else {
                                        tr!("Play")
                                    }
                                    .into(),
                                )
                            })
                            .disabled(move || {
                                model.status.with(|s| {
                                    s.track.is_none()
                                        || (model.starting.get() && !s.switching)
                                        || (s.switching && !playing())
                                })
                            })
                            .on_press(move || {
                                if !playing() {
                                    model.error.set(String::new());
                                }
                                submit(
                                    if playing() {
                                        Action::Pause
                                    } else {
                                        Action::Resume
                                    },
                                    None,
                                )
                            })
                            .w(px(48.0))
                            .h(px(48.0))
                            .rounded_full()
                            .items_center()
                            .justify_center()
                            .bg(move || theme().primary)
                            .hover(|s| s.opacity(0.88))
                            .active(|s| s.opacity(0.6))
                            .disabled_style(|s| s.opacity(0.22))
                            .child(
                                when(
                                    move || super::playback_busy(model),
                                    crate::controls::loading_indicator,
                                )
                                .otherwise(move || {
                                    icon(
                                        move || if playing() { Icon::Pause } else { Icon::Play },
                                        22.0,
                                        true,
                                    )
                                }),
                            ),
                    )
                    .child(transport(model, false)),
            ),
    )
}

fn queue(model: Model) -> impl IntoElement {
    let list = element_ref();
    let ready = signal(false);
    let positioned = signal(false);
    effect(move || {
        if ready.get() && !positioned.get() && model.queue.with(Option::is_some) {
            if let Some(index) = model.status.with(|s| s.index) {
                list.scroll_to_index(index as u32, percentage(0.35), px(0.0), false);
            }
            positioned.set(true);
        }
    });
    let source = computed(move || {
        model.queue.with(|snapshot| {
            snapshot.as_ref().map(|queue| {
                use crate::protocol::QueueContext;
                let (name, kind) = match &queue.queue_context {
                    Some(QueueContext::Album { name, .. }) => (name.clone(), Some(tr!("Album"))),
                    Some(QueueContext::LocalPlaylist { id, name }) => {
                        (if id == "liked" { tr!("Liked Songs").into() } else { name.clone() }, Some(tr!("Local playlist")))
                    }
                    Some(QueueContext::Playlist { name, .. }) => {
                        (name.clone(), Some(tr!("Source playlist")))
                    }
                    Some(QueueContext::History) => (tr!("History").into(), None),
                    Some(QueueContext::SourceCollection { name, .. }) => {
                        (name.clone(), Some(tr!("Source tracks")))
                    }
                    None => (tr!("Queue").into(), None),
                };
                let count = queue.queue.len();
                let detail = match kind {
                    Some(kind) => tr!("{kind} · {tracks}"; kind = kind, tracks = crate::protocol::i18n::tracks(count)),
                    None => crate::protocol::i18n::tracks(count),
                };
                (name, detail)
            })
        })
    });
    view()
        .flex_1()
        .min_h(px(0.0))
        .pt(px(12.0))
        .gap(px(8.0))
        .child(when(
            move || {
                model
                    .queue
                    .with(|q| q.as_ref().is_some_and(|q| !q.queue.is_empty()))
            },
            move || {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap(px(3.0))
                            .child(
                                text(move || {
                                    source.with(|value| {
                                        value
                                            .as_ref()
                                            .map_or_else(String::new, |(name, _)| name.clone())
                                    })
                                })
                                .number_of_lines(2)
                                .text_size(px(15.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                            )
                            .child(crate::controls::caption(move || {
                                source.with(|value| {
                                    value
                                        .as_ref()
                                        .map_or_else(String::new, |(_, detail)| detail.clone())
                                })
                            })),
                    )
                    .child(when(
                        move || {
                            !model.show_mini_lyrics.get()
                                && model.status.with(|s| s.track.is_some())
                        },
                        move || {
                            crate::controls::action(
                                tr!("View lyrics"),
                                || false,
                                move || model.open(Page::Lyrics),
                            )
                        },
                    ))
                    .child(crate::controls::action(
                        tr!("Find current"),
                        move || model.status.with(|s| s.index.is_none()),
                        move || {
                            if let Some(index) = model.status.with(|s| s.index) {
                                list.scroll_to_index(
                                    index as u32,
                                    percentage(0.35),
                                    px(0.0),
                                    !reduce_motion(),
                                );
                            }
                        },
                    ))
            },
        ))
        .child(when(
            move || {
                !model.is_loading()
                    && model
                        .queue
                        .with(|q| q.as_ref().is_some_and(|q| q.queue.is_empty()))
            },
            || {
                view()
                    .gap(px(6.0))
                    .child(crate::controls::heading(tr!("The queue is empty")))
                    .child(crate::controls::caption(tr!(
                        "Choose a track from an album, history, or playlist to see the playback order here."
                    )))
            },
        ))
        .child(
            virtual_list(
                move || {
                    model
                        .queue
                        .with(|q| q.as_ref().map_or(0, |q| q.queue.len()))
                },
                move |at| {
                    let track = move || {
                        model
                            .queue
                            .with(|q| q.as_ref().and_then(|q| q.queue.get(at.index()).cloned()))
                    };
                    let current = move || {
                        model.queue.with(|q| {
                            q.as_ref().is_some_and(|q| {
                                model.status.with(|s| {
                                    s.queue_revision == q.queue_revision
                                        && s.index == Some(at.index())
                                })
                            })
                        })
                    };
                    button()
                        .accessibility_label(move || {
                            Some(SharedString::from(tr!(
                                "Play queue track {}, {}",
                                at.index() + 1,
                                track().map_or_else(String::new, |t| t.title)
                            )))
                        })
                        .disabled(move || {
                            model.starting.get() || model.status.with(|s| s.switching)
                        })
                        .on_press(move || model.play_queue(at.index()))
                        .w_full()
                        .h(px(52.0))
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(7.0))
                        .rounded(px(6.0))
                        .selected(current)
                        .selected_style(|s| s.bg(theme().muted))
                        .hover(|s| s.bg(theme().muted))
                        .active(|s| s.opacity(0.65))
                        .child(
                            view()
                                .w(px(18.0))
                                .items_center()
                                .justify_center()
                                .child(when(current, move || {
                                    icon(
                                        move || {
                                            if model.status.with(|s| s.state == State::Playing) {
                                                Icon::Play
                                            } else {
                                                Icon::Pause
                                            }
                                        },
                                        12.0,
                                        false,
                                    )
                                }))
                                .child(when(
                                    move || !current(),
                                    move || {
                                        text(move || format!("{:02}", at.index() + 1))
                                            .text_size(px(10.0))
                                            .text_color(move || theme().muted_foreground)
                                    },
                                )),
                        )
                        .child(crate::controls::artwork(
                            move || track().and_then(|t| t.cover_url),
                            move || tr!("Artwork for {}", track().map_or_else(String::new, |t| t.title)),
                            36.0,
                        ))
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .gap(px(3.0))
                                .child(
                                    text(move || track().map_or_else(String::new, |t| t.title))
                                        .number_of_lines(1)
                                        .text_size(px(13.0))
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    text(move || track().map_or_else(String::new, |t| t.artist))
                                        .number_of_lines(1)
                                        .text_size(px(11.0))
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(
                            text(move || {
                                if current() {
                                    tr!("Current").into()
                                } else {
                                    track().map_or_else(String::new, |t| time(t.duration))
                                }
                            })
                            .text_size(px(10.0))
                            .text_color(move || theme().muted_foreground),
                        )
                },
            )
            .flex_1()
            .min_h(px(0.0))
            .estimated_item_size(px(52.0))
            .scrollbar(false)
            .label(tr!("Current queue"))
            .element_ref(&list)
            .on_layout(move |_: &Bounds<Points>| ready.set(true)),
        )
}

pub(super) fn track_row(
    track: impl Fn() -> Track + Copy + 'static,
    subtitle: impl Fn() -> String + 'static,
    model: Model,
    index: impl Fn() -> usize + Copy + 'static,
    progress: impl Fn() -> Option<String> + Copy + 'static,
) -> impl IntoElement {
    // A history can contain the same song many times. Only an album has
    // a unique track row that can unambiguously carry the current marker.
    let current = move || {
        model.page.get() == Page::Album
            && model.status.with(|s| key(s.track.as_ref())) == key(Some(&track()))
    };
    button()
        .accessibility_label(move || {
            let track = track();
            let mut label = tr!("Play {}, {}", track.title, track.artist);
            if let Some(progress) = progress() {
                label.push('，');
                label.push_str(&progress);
            }
            Some(SharedString::from(label))
        })
        .disabled(move || model.starting.get())
        .selected(current)
        .selected_style(|s| s.bg(theme().muted))
        .on_press(move || model.play_browsed(index()))
        .w_full()
        .min_h(px(62.0))
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .py(px(12.0))
        .px(px(10.0))
        .rounded(px(7.0))
        .hover(|s| s.bg(theme().muted))
        .active(|s| s.opacity(0.65))
        .disabled_style(|s| s.opacity(0.4))
        .child(crate::controls::artwork(
            move || track().cover_url,
            move || tr!("Artwork for {}", track().title),
            36.0,
        ))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(4.0))
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
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(when(
            move || current() || progress().is_some(),
            move || {
                text(move || {
                    if current() {
                        tr!("Current").into()
                    } else {
                        progress().unwrap_or_default()
                    }
                })
                .text_size(px(10.0))
                .text_color(move || theme().muted_foreground)
            },
        ))
}

pub(super) fn song_info(model: Model) -> impl IntoElement {
    scroll().flex_1().min_h(px(0.0)).child(
        view()
            .pt(px(16.0))
            .gap(px(10.0))
            .child(crate::discovery_ui::artist_links(model))
            .child(crate::discovery_ui::episode_details(model))
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
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
            )
            .child(
                button()
                    .accessibility_label(Some(tr!("View album information").into()))
                    .on_press(move || model.open(Page::Album))
                    .disabled(move || {
                        model
                            .status
                            .with(|s| s.track.as_ref().is_none_or(|t| t.album.is_none()))
                    })
                    .self_start()
                    .py(px(6.0))
                    .rounded(px(4.0))
                    .hover(|s| s.bg(theme().muted))
                    .child(
                        text(move || {
                            model.status.with(|s| {
                                s.track.as_ref().and_then(|t| t.album.as_ref()).map_or_else(
                                    || tr!("No album information").into(),
                                    |a| a.name.clone(),
                                )
                            })
                        })
                        .number_of_lines(2)
                        .text_size(px(13.0))
                        .text_color(move || theme().foreground),
                    ),
            )
            .child(
                text(tr!("Track source"))
                    .mt(px(8.0))
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().foreground),
            )
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
                                .and_then(|i| i.source_link.as_ref().map(|l| l.url.clone()))
                        }) {
                            spawn(async move {
                                let _ = open_url(url).await;
                            });
                        }
                    })
                    .py(px(8.0))
                    .px(px(6.0))
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(5.0))
                    .hover(|s| s.bg(theme().muted))
                    .child(crate::controls::source_badge(move || {
                        model.status.with(|s| {
                            s.track.as_ref().map_or_else(String::new, |t| {
                                t.source
                                    .as_ref()
                                    .map_or_else(String::new, |src| src.plugin.clone())
                            })
                        })
                    }))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap(px(3.0))
                            .child(
                                text(move || {
                                    model.info.with(|i| {
                                        i.as_ref().map_or_else(
                                            || tr!("Loading source…").into(),
                                            |i| {
                                                let source = i
                                                    .track
                                                    .source
                                                    .as_ref()
                                                    .map_or("", |src| src.plugin.as_str());
                                                crate::controls::source_name(source, &i.source_name)
                                            },
                                        )
                                    })
                                })
                                .number_of_lines(1)
                                .text_size(px(13.0))
                                .text_color(move || theme().foreground),
                            )
                            .child(
                                text(move || {
                                    model.info.with(|i| {
                                        i.as_ref()
                                            .and_then(|i| {
                                                i.source_link.as_ref().map(|l| l.url.clone())
                                            })
                                            .unwrap_or_default()
                                    })
                                })
                                .number_of_lines(2)
                                .text_size(px(10.0))
                                .text_color(move || theme().muted_foreground),
                            ),
                    ),
            )
            .child(
                button()
                    .accessibility_label(Some(tr!("Add current track to a local playlist").into()))
                    .on_press(move || model.open(Page::Library))
                    .py(px(9.0))
                    .px(px(8.0))
                    .rounded(px(6.0))
                    .hover(|s| s.bg(theme().muted))
                    .child(
                        text(tr!("Add to my playlists"))
                            .text_size(px(12.0))
                            .text_color(move || theme().foreground),
                    ),
            ),
    )
}
