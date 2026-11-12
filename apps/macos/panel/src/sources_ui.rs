//! Every music source uses the same MyGo configuration, account and import flow.
use crate::protocol::tr;
use crate::{
    controls::{action, caption, heading, section},
    model::{Model, Page},
    protocol::MusicSource,
    remote,
};
#[cfg(target_os = "macos")]
use nsh_base::switch;
use nsh_base::{button, theme};
use nsh_core::prelude::*;
use tokio::sync::oneshot;
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Settings,
    ChooseImport,
    Plugins,
}
#[derive(Clone, Copy)]
struct Sources {
    owner: Owner,
    mode: Signal<Mode>,
    busy: Signal<bool>,
    message: Signal<String>,
    plugins: Signal<Vec<MusicSource>>,
    selected: Signal<Option<MusicSource>>,
    catalog: Signal<bool>,
    import_mode: bool,
}
impl Sources {
    fn work<T: Send + 'static>(
        self,
        run: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
        done: impl FnOnce(Self, T) + 'static,
    ) {
        if self.busy.get() {
            return;
        }
        self.busy.set(true);
        self.message.set(String::new());
        let (send, receive) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = send.send(run());
        });
        self.owner.run(|| {
            spawn(async move {
                let result = receive.await;
                self.busy.set(false);
                match result {
                    Ok(Ok(v)) => done(self, v),
                    Ok(Err(e)) => self.message.set(e.to_string()),
                    Err(_) => self
                        .message
                        .set(tr!("Request ended; please try again").into()),
                }
            })
        });
    }
    fn refresh(self) {
        self.work(
            || remote::read_json::<Vec<MusicSource>>("/ui/music-sources"),
            |this, rows| this.plugins.set(rows),
        );
    }
    fn home(self) {
        self.message.set(String::new());
        self.mode.set(if self.import_mode {
            Mode::ChooseImport
        } else {
            Mode::Settings
        });
        self.refresh();
    }
    fn choose(self, source: MusicSource) {
        self.message.set(String::new());
        self.selected.set(Some(source));
        self.catalog.set(false);
        self.mode.set(Mode::Plugins);
    }
}
pub fn settings_view(model: Model) -> impl IntoElement {
    flow(model, false, || {})
}

fn language_settings(model: Model) -> impl IntoElement {
    use crate::protocol::i18n::Language;
    let selected = signal(Language::load());
    let message = signal(String::new());
    view()
        .gap(px(8.0))
        .child(section(tr!("Language")))
        .child(Language::ALL.into_iter().fold(
            view().flex_row().flex_wrap().gap(px(4.0)),
            |row, language| {
                row.child(
                    button()
                        .accessibility_label(Some(language.label().into()))
                        .selected(move || selected.get() == language)
                        .on_press(move || match language.save() {
                            Ok(()) => {
                                selected.set(language);
                                message.set(String::new());
                                model.refresh_language();
                            }
                            Err(error) => {
                                message.set(tr!("Could not save language: {error}"; error = error))
                            }
                        })
                        .min_h(px(if cfg!(target_os = "ios") { 44.0 } else { 32.0 }))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .items_center()
                        .justify_center()
                        .bg(move || {
                            if selected.get() == language {
                                theme().muted
                            } else {
                                theme().muted.opacity(0.25)
                            }
                        })
                        .hover(|s| s.bg(theme().muted))
                        .active(|s| s.opacity(0.65))
                        .child(
                            text(language.label())
                                .text_size(px(11.0))
                                .text_color(move || theme().foreground),
                        ),
                )
            },
        ))
        .child(when(
            move || !message.get().is_empty(),
            move || caption(move || message.get()),
        ))
}

#[cfg(target_os = "macos")]
fn update_settings() -> impl IntoElement {
    use crate::protocol::UpdateChannel;
    let channel = signal(UpdateChannel::load());
    let message = signal(String::new());
    let options = view().flex_row().gap(px(4.0));
    let options = [UpdateChannel::All, UpdateChannel::Stable]
        .into_iter()
        .fold(options, |row, value| {
            row.child(
                button()
                    .accessibility_label(Some(value.label().into()))
                    .selected(move || channel.get() == value)
                    .on_press(move || match value.save() {
                        Ok(()) => {
                            channel.set(value);
                            message.set(String::new());
                            crate::updates::channel_changed();
                        }
                        Err(error) => message
                            .set(tr!("Could not save update settings: {error}"; error = error)),
                    })
                    .h(px(28.0))
                    .px(px(8.0))
                    .rounded(px(6.0))
                    .items_center()
                    .justify_center()
                    .bg(move || {
                        if channel.get() == value {
                            theme().muted
                        } else {
                            theme().muted.opacity(0.25)
                        }
                    })
                    .hover(|s| s.bg(theme().muted))
                    .active(|s| s.opacity(0.65))
                    .child(
                        text(value.label())
                            .text_size(px(11.0))
                            .text_color(move || theme().foreground),
                    ),
            )
        });
    view()
        .gap(px(8.0))
        .child(section(tr!("Version & updates")))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(caption(crate::updates::version())),
                )
                .child(settings_action(
                    tr!("Check for updates"),
                    || false,
                    crate::updates::check,
                )),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    text(tr!("Updates"))
                        .text_size(px(12.0))
                        .text_color(move || theme().foreground),
                )
                .child(options),
        )
        .child(caption(move || {
            if !message.get().is_empty() {
                message.get()
            } else {
                match channel.get() {
                    UpdateChannel::All => tr!("Includes stable and preview releases.").into(),
                    UpdateChannel::Stable => {
                        tr!("Stable releases only. A preview update already in progress may still install.").into()
                    }
                }
            }
        }))
}

#[cfg(target_os = "ios")]
fn update_settings() -> impl IntoElement {
    view()
}

#[cfg(target_os = "ios")]
fn playback_settings(_model: Model) -> impl IntoElement {
    view()
}

#[cfg(target_os = "macos")]
fn playback_settings(model: Model) -> impl IntoElement {
    view()
        .gap(px(4.0))
        .child(section(tr!("Play")))
        .child(setting_toggle(
            tr!("Volume normalization"),
            move || model.status.with(|s| s.normalization_enabled),
            move |enabled| remote::submit(remote::Action::Normalization(enabled), None),
        ))
        .child(setting_toggle(
            tr!("Lyric preview"),
            move || model.show_mini_lyrics.get(),
            move |_| model.toggle_mini_lyrics(),
        ))
}

#[cfg(target_os = "macos")]
fn setting_toggle(
    label: &'static str,
    checked: impl Fn() -> bool + Copy + 'static,
    change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    switch()
        .accessibility_label(label)
        .checked(checked)
        .on_change(move |enabled: &bool| change(*enabled))
        .flex_row_reverse()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .w_full()
        .h(px(36.0))
        .px(px(4.0))
        .rounded(px(6.0))
        .hover(|s| s.bg(theme().muted.opacity(0.5)))
        .active(|s| s.opacity(0.65))
        .track_style(move |s| {
            s.w(px(28.0))
                .min_w(px(28.0))
                .h(px(16.0))
                .p(px(2.0))
                .rounded(px(8.0))
                .bg(move || {
                    if checked() {
                        theme().foreground
                    } else {
                        theme().muted
                    }
                })
        })
        .thumb_style(move |s| {
            s.w(px(12.0))
                .h(px(12.0))
                .ml(move || px(if checked() { 12.0 } else { 0.0 }))
                .rounded(px(6.0))
                .bg(move || theme().background)
        })
        .child(
            text(label)
                .text_size(px(12.0))
                .text_color(move || theme().foreground),
        )
}

fn settings_action(
    label: &'static str,
    disabled: impl Fn() -> bool + 'static,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .accessibility_label(Some(label.into()))
        .disabled(disabled)
        .on_press(press)
        .h(px(28.0))
        .px(px(8.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(move || theme().muted.opacity(0.5))
        .hover(|s| s.bg(theme().muted))
        .active(|s| s.opacity(0.65))
        .disabled_style(|s| s.opacity(0.4))
        .child(
            text(label)
                .text_size(px(11.0))
                .text_color(move || theme().foreground),
        )
}

fn mcp_settings() -> impl IntoElement {
    let message = signal("");
    view()
        .gap(px(8.0))
        .child(section(tr!("MCP connection")))
        .child(
            view()
                .px(px(8.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .bg(move || theme().muted.opacity(0.35))
                .child(caption(remote::mcp_endpoint())),
        )
        .child(
            view()
                .flex_row()
                .gap(px(8.0))
                .child(settings_action(
                    tr!("Copy MCP config"),
                    || remote::mcp_token().is_none(),
                    move || {
                        if let Some(token) = remote::mcp_token() {
                            let config = serde_json::json!({
                                "mcpServers": {
                                    "musicx": {
                                        "type": "http",
                                        "url": remote::mcp_endpoint(),
                                        "headers": { "Authorization": format!("Bearer {token}") }
                                    }
                                }
                            });
                            set_clipboard(format!("{config:#}"));
                            message.set(tr!("Configuration copied"));
                        }
                    },
                ))
                .child(settings_action(
                    tr!("Copy token"),
                    || remote::mcp_token().is_none(),
                    move || {
                        if let Some(token) = remote::mcp_token() {
                            set_clipboard(token);
                            message.set(tr!("Token copied"));
                        }
                    },
                )),
        )
        .child(caption(move || match message.get() {
            "" => tr!("For clients that support Streamable HTTP."),
            feedback => feedback,
        }))
}

fn settings_separator() -> impl IntoElement {
    view()
        .w_full()
        .h(px(1.0))
        .min_h(px(1.0))
        .bg(move || theme().muted)
}

pub fn import_view(model: Model, back: impl Fn() + Copy + 'static) -> impl IntoElement {
    flow(model, true, back)
}
fn flow(model: Model, importing: bool, back: impl Fn() + Copy + 'static) -> impl IntoElement {
    let this = Sources {
        owner: Owner::current(),
        mode: signal(if importing {
            Mode::ChooseImport
        } else {
            Mode::Settings
        }),
        busy: signal(false),
        message: signal(String::new()),
        plugins: signal(Vec::new()),
        selected: signal(None),
        catalog: signal(false),
        import_mode: importing,
    };
    this.refresh();
    view()
        .flex_1()
        .min_h(px(0.0))
        .pt(px(8.0))
        .gap(px(10.0))
        .child(when(
            move || this.mode.get() == Mode::ChooseImport,
            move || {
                view()
                    .gap(px(12.0))
                    .child(action(
                        tr!("Back to my playlists"),
                        move || this.busy.get(),
                        back,
                    ))
                    .child(heading(tr!("Import playlist")))
                    .child(caption(tr!(
                        "Choose a music source to save a local playlist."
                    )))
            },
        ))
        .child(when(
            move || matches!(this.mode.get(), Mode::Settings | Mode::ChooseImport),
            move || {
                scroll().flex_1().min_h(px(0.0)).child(
                    view()
                        .gap(px(8.0))
                        .pb(px(12.0))
                        .child(when(move || !importing, move || language_settings(model)))
                        .child(when(move || !importing, settings_separator))
                        .child(when(move || !importing, move || playback_settings(model)))
                        .child(when(
                            move || !importing && cfg!(target_os = "macos"),
                            settings_separator,
                        ))
                        .child(source_settings(this, importing))
                        .child(when(move || !importing, settings_separator))
                        .child(when(move || !importing, mcp_settings))
                        .child(when(
                            move || !importing && cfg!(target_os = "macos"),
                            settings_separator,
                        ))
                        .child(when(move || !importing, update_settings)),
                )
            },
        ))
        .child(when(
            move || this.mode.get() == Mode::Plugins,
            move || {
                crate::source_browser::sources_view(
                    model,
                    move || this.home(),
                    move || {
                        if importing {
                            back()
                        } else {
                            model.open(Page::Library)
                        }
                    },
                    this.selected.get(),
                    if this.catalog.get() {
                        crate::source_browser::Start::Catalog
                    } else if importing {
                        crate::source_browser::Start::Import
                    } else {
                        crate::source_browser::Start::Detail
                    },
                )
            },
        ))
        .child(when(
            move || this.busy.get(),
            || caption(tr!("Loading or saving…")),
        ))
        .child(when(
            move || this.message.with(|v| !v.is_empty()),
            move || caption(move || this.message.get()),
        ))
}

fn source_settings(this: Sources, importing: bool) -> impl IntoElement {
    view()
        .gap(px(4.0))
        .child(when(
            move || !importing,
            move || {
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(section(tr!("Music sources")))
                    .child(
                        button()
                            .accessibility_label(Some(tr!("Add music source").into()))
                            .disabled(move || this.busy.get())
                            .on_press(move || {
                                this.selected.set(None);
                                this.catalog.set(true);
                                this.mode.set(Mode::Plugins);
                            })
                            .h(px(24.0))
                            .px(px(8.0))
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .hover(|s| s.bg(theme().muted))
                            .active(|s| s.opacity(0.65))
                            .disabled_style(|s| s.opacity(0.4))
                            .child(
                                text(tr!("＋ Add"))
                                    .text_size(px(11.0))
                                    .text_color(move || theme().foreground),
                            ),
                    )
            },
        ))
        .child(when(
            move || !this.busy.get() && this.plugins.with(|sources| sources.is_empty()),
            || {
                caption(tr!(
                    "No music sources yet. Add a plugin to sign in or import playlists where supported."
                ))
            },
        ))
        .child(each(
            move || {
                this.plugins.with(|rows| {
                    rows.iter()
                        .filter(|s| {
                            !importing
                                || s.capabilities
                                    .iter()
                                    .any(|v| matches!(v.as_str(), "playlist" | "liked" | "recent"))
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
            },
            |s| s.id.clone(),
            move |row, _| {
                button()
                    .accessibility_label(move || {
                        Some(SharedString::from(tr!(
                            "Music source {}",
                            row.with(|s| crate::controls::source_name(&s.id, &s.name))
                        )))
                    })
                    .on_press(move || this.choose(row.get()))
                    .disabled(move || this.busy.get())
                    .w_full()
                    .h(px(if importing { 56.0 } else { 48.0 }))
                    .px(px(4.0))
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(6.0))
                    .hover(|s| s.bg(theme().muted))
                    .active(|s| s.opacity(0.65))
                    .child(crate::controls::source_badge(move || {
                        row.with(|s| s.id.clone())
                    }))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap(px(2.0))
                            .child(
                                text(move || {
                                    row.with(|s| crate::controls::source_name(&s.id, &s.name))
                                })
                                .number_of_lines(1)
                                .text_size(px(13.0))
                                .text_color(move || theme().foreground),
                            )
                            .child(caption(move || {
                                row.with(|source| {
                                    let sign_in = source.web_sign_in
                                        || source.qr_sign_in
                                        || source.oauth_sign_in;
                                    let playlist =
                                        source.capabilities.iter().any(|v| v == "playlist");
                                    match (importing, sign_in, playlist) {
                                        (true, _, _) => tr!("Import playlists from this source"),
                                        (_, true, true) => tr!("Sign-in · Playlist import"),
                                        (_, true, false) => tr!("Sign-in available"),
                                        (_, false, true) => tr!("Playlist import"),
                                        _ => tr!("Added"),
                                    }
                                })
                            })),
                    )
                    .child(
                        text("›")
                            .text_size(px(18.0))
                            .text_color(move || theme().muted_foreground),
                    )
            },
        ))
}
