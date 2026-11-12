//! Every music source uses the same MyGo configuration, account and import flow.
use crate::{
    controls::{action, caption, heading, section},
    model::{Model, Page},
    protocol::{MusicSource, MusicSourceAction, PluginUpdateSettings},
    remote,
};
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
    auto_update: Signal<Option<bool>>,
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
                    Err(_) => self.message.set("请求已结束，请重试".into()),
                }
            })
        });
    }
    fn refresh(self) {
        self.work(
            move || {
                let rows = remote::read_json::<Vec<MusicSource>>("/ui/music-sources")?;
                let updates = (!self.import_mode).then(|| {
                    remote::post_json::<_, PluginUpdateSettings>(
                        "/ui/music-sources",
                        &MusicSourceAction::AutoUpdateSettings,
                    )
                });
                Ok((rows, updates))
            },
            |this, (rows, updates)| {
                this.plugins.set(rows);
                match updates {
                    Some(Ok(settings)) => this.auto_update.set(Some(settings.enabled)),
                    Some(Err(error)) => this.message.set(format!("无法读取插件更新设置：{error}")),
                    None => {}
                }
            },
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
                        Err(error) => message.set(format!("无法保存更新设置：{error}")),
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
        .child(section("版本与更新"))
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
                .child(settings_action("检查更新", || false, crate::updates::check)),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    text("接收更新")
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
                    UpdateChannel::All => "包含稳定版和预览版（Preview）。".into(),
                    UpdateChannel::Stable => "仅检查稳定版。已开始的预览版更新仍可能安装。".into(),
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
        .child(section("播放"))
        .child(setting_toggle(
            "音量均衡",
            move || model.status.with(|s| s.normalization_enabled),
            || false,
            move |enabled| remote::submit(remote::Action::Normalization(enabled), None),
        ))
        .child(setting_toggle(
            "单行同步歌词",
            move || model.show_mini_lyrics.get(),
            || false,
            move |_| model.toggle_mini_lyrics(),
        ))
}

fn setting_toggle(
    label: &'static str,
    checked: impl Fn() -> bool + Copy + 'static,
    disabled: impl Fn() -> bool + 'static,
    change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    switch()
        .accessibility_label(label)
        .checked(checked)
        .disabled(disabled)
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
        .disabled_style(|s| s.opacity(0.4))
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
        .child(section("MCP 连接"))
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
                    "复制 MCP 配置",
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
                            message.set("配置已复制");
                        }
                    },
                ))
                .child(settings_action(
                    "复制令牌",
                    || remote::mcp_token().is_none(),
                    move || {
                        if let Some(token) = remote::mcp_token() {
                            set_clipboard(token);
                            message.set("令牌已复制");
                        }
                    },
                )),
        )
        .child(caption(move || match message.get() {
            "" => "用于支持 Streamable HTTP 的客户端。",
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
        auto_update: signal(None),
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
                    .child(action("返回我的歌单", move || this.busy.get(), back))
                    .child(heading("导入歌单"))
                    .child(caption("选择音乐来源，保存一份本地歌单。"))
            },
        ))
        .child(when(
            move || matches!(this.mode.get(), Mode::Settings | Mode::ChooseImport),
            move || {
                scroll().flex_1().min_h(px(0.0)).child(
                    view()
                        .gap(px(8.0))
                        .pb(px(12.0))
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
        .child(when(move || this.busy.get(), || caption("正在读取或保存…")))
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
                    .child(section("音乐来源"))
                    .child(
                        button()
                            .accessibility_label(Some("添加音乐来源".into()))
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
                                text("＋ 添加")
                                    .text_size(px(11.0))
                                    .text_color(move || theme().foreground),
                            ),
                    )
            },
        ))
        .child(when(
            move || !importing,
            move || {
                view()
                    .gap(px(4.0))
                    .child(setting_toggle(
                        "自动更新插件",
                        move || this.auto_update.get().unwrap_or(false),
                        move || this.busy.get() || this.auto_update.get().is_none(),
                        move |enabled| {
                            this.work(
                                move || {
                                    remote::post_json::<_, PluginUpdateSettings>(
                                        "/ui/music-sources",
                                        &MusicSourceAction::SetAutoUpdate { enabled },
                                    )
                                },
                                |this, settings| this.auto_update.set(Some(settings.enabled)),
                            )
                        },
                    ))
                    .child(caption(
                        "每天更新仓库插件，保留账号和设置。旧插件需先手动更新一次。",
                    ))
            },
        ))
        .child(when(
            move || !this.busy.get() && this.plugins.with(|sources| sources.is_empty()),
            || caption("尚未添加音乐来源。添加插件后，可按其能力登录或导入歌单。"),
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
                        Some(SharedString::from(format!(
                            "音乐来源 {}",
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
                                        (true, _, _) => "导入此来源的歌单",
                                        (_, true, true) => "支持登录 · 可导入歌单",
                                        (_, true, false) => "支持登录",
                                        (_, false, true) => "可导入歌单",
                                        _ => "已添加",
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
