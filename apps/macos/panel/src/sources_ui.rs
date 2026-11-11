//! Every music source uses the same MyGo configuration, account and import flow.
use crate::{
    controls::{action, caption, heading, section},
    model::{Model, Page},
    protocol::MusicSource,
    remote,
};
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
                    Err(_) => self.message.set("请求已结束，请重试".into()),
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

#[cfg(target_os = "macos")]
fn update_settings() -> impl IntoElement {
    view()
        .gap(px(6.0))
        .pb(px(14.0))
        .child(section("版本与更新"))
        .child(caption(crate::updates::version()))
        .child(action("检查更新…", || false, crate::updates::check))
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
        .gap(px(8.0))
        .pb(px(14.0))
        .child(section("播放"))
        .child(
            button()
                .accessibility_label(Some("切换音量均衡".into()))
                .on_press(move || {
                    crate::remote::submit(
                        crate::remote::Action::Normalization(
                            !model.status.with(|s| s.normalization_enabled),
                        ),
                        None,
                    )
                })
                .selected(move || model.status.with(|s| s.normalization_enabled))
                .py(px(8.0))
                .px(px(4.0))
                .rounded(px(5.0))
                .hover(|s| s.bg(theme().muted))
                .child(
                    text(move || {
                        if model.status.with(|s| s.normalization_enabled) {
                            "音量均衡 · 已开启"
                        } else {
                            "音量均衡 · 已关闭"
                        }
                    })
                    .text_size(px(13.0))
                    .text_color(move || theme().foreground),
                ),
        )
        .child(
            button()
                .accessibility_label(Some("切换播放界面单行歌词".into()))
                .on_press(move || model.toggle_mini_lyrics())
                .selected(move || model.show_mini_lyrics.get())
                .flex_row()
                .items_center()
                .justify_between()
                .h(px(40.0))
                .px(px(4.0))
                .rounded(px(5.0))
                .hover(|s| s.bg(theme().muted))
                .child(
                    text("单行同步歌词")
                        .text_size(px(13.0))
                        .text_color(move || theme().foreground),
                )
                .child(
                    view()
                        .w(px(28.0))
                        .h(px(16.0))
                        .p(px(2.0))
                        .rounded(px(8.0))
                        .bg(move || {
                            if model.show_mini_lyrics.get() {
                                theme().foreground
                            } else {
                                theme().muted
                            }
                        })
                        .flex_row()
                        .style_with(move |s| {
                            if model.show_mini_lyrics.get() {
                                s.justify_end()
                            } else {
                                s.justify_start()
                            }
                        })
                        .child(
                            view()
                                .w(px(12.0))
                                .h(px(12.0))
                                .rounded(px(6.0))
                                .bg(move || theme().background),
                        ),
                ),
        )
}

fn mcp_settings() -> impl IntoElement {
    let message = signal("");
    view()
        .gap(px(6.0))
        .pb(px(14.0))
        .child(section("MCP 连接"))
        .child(caption(remote::mcp_endpoint()))
        .child(
            view()
                .flex_row()
                .gap(px(4.0))
                .child(action(
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
                .child(action(
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
            "" => "将配置粘贴到支持 Streamable HTTP 的客户端。",
            feedback => feedback,
        }))
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
        .pt(px(10.0))
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
                        .gap(px(10.0))
                        .pb(px(12.0))
                        .child(when(move || !importing, update_settings))
                        .child(when(move || !importing, move || playback_settings(model)))
                        .child(when(move || !importing, mcp_settings))
                        .child(when(move || !importing, || section("音乐来源")))
                        .child(when(
                            move || {
                                !this.busy.get() && this.plugins.with(|sources| sources.is_empty())
                            },
                            || caption("尚未添加音乐来源。添加插件后，可按其能力登录或导入歌单。"),
                        ))
                        .child(each(
                            move || {
                                this.plugins.with(|rows| {
                                    rows.iter()
                                        .filter(|s| {
                                            !importing
                                                || s.capabilities.iter().any(|v| {
                                                    matches!(
                                                        v.as_str(),
                                                        "playlist" | "liked" | "recent"
                                                    )
                                                })
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
                                            row.with(|s| crate::controls::source_name(
                                                &s.id, &s.name
                                            ))
                                        )))
                                    })
                                    .on_press(move || this.choose(row.get()))
                                    .disabled(move || this.busy.get())
                                    .w_full()
                                    .h(px(56.0))
                                    .px(px(6.0))
                                    .flex_row()
                                    .items_center()
                                    .gap(px(10.0))
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
                                            .gap(px(3.0))
                                            .child(
                                                text(move || {
                                                    row.with(|s| {
                                                        crate::controls::source_name(&s.id, &s.name)
                                                    })
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
                                                    let playlist = source
                                                        .capabilities
                                                        .iter()
                                                        .any(|v| v == "playlist");
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
                            },
                        ))
                        .child(when(
                            move || !importing,
                            move || {
                                action(
                                    "添加音乐来源",
                                    move || this.busy.get(),
                                    move || {
                                        this.selected.set(None);
                                        this.catalog.set(true);
                                        this.mode.set(Mode::Plugins);
                                    },
                                )
                            },
                        )),
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
