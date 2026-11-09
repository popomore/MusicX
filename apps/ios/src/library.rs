//! Account and paged personal library. No cookies or challenge keys are displayed as text.
use crate::{
    protocol::{
        LoginChallenge, LoginPoll, LoginProgress, PlaylistList, PlaylistPage, PlaylistSummary,
        Track,
    },
    remote,
};
use nsh_base::{button, theme};
use nsh_core::{element::ImageSrc, prelude::*};
use std::time::Duration;
use tokio::sync::oneshot;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Lists,
    Tracks,
    Login,
}
#[derive(Clone, Copy)]
struct Browser {
    owner: Owner,
    mode: Signal<Mode>,
    busy: Signal<bool>,
    error: Signal<String>,
    lists: Signal<Vec<(u64, PlaylistSummary)>>,
    tracks: Signal<Vec<(usize, Track)>>,
    selected: Signal<Option<PlaylistSummary>>,
    offset: Signal<u64>,
    list_offset: Signal<u64>,
    more: Signal<bool>,
    challenge: Signal<Option<LoginChallenge>>,
    progress: Signal<String>,
}
impl Browser {
    fn work<T: Send + 'static>(
        self,
        run: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
        done: impl FnOnce(Self, T) + 'static,
    ) {
        if self.busy.get() {
            return;
        }
        self.busy.set(true);
        self.error.set(String::new());
        let requested = self.mode.get();
        let (send, receive) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = send.send(run());
        });
        self.owner.run(|| {
            spawn(async move {
                let result = receive.await;
                self.busy.set(false);
                if self.mode.get() != requested {
                    return;
                }
                match result {
                    Ok(Ok(value)) => done(self, value),
                    Ok(Err(error)) => self.error.set(error.to_string()),
                    Err(_) => self.error.set("请求已结束，请重试".into()),
                }
            })
        });
    }
    fn lists(self) {
        if self.busy.get() {
            return;
        }
        self.mode.set(Mode::Lists);
        self.lists.set(Vec::new());
        self.tracks.set(Vec::new());
        self.more.set(false);
        self.challenge.set(None);
        let offset = self.list_offset.get();
        self.work(
            move || remote::read_json::<PlaylistList>(&format!("/ui/playlists?offset={offset}")),
            |this, page| {
                this.more.set(page.more);
                this.lists
                    .set(page.playlists.into_iter().map(|p| (p.id, p)).collect());
            },
        );
    }
    fn tracks(self, playlist: PlaylistSummary, offset: u64) {
        if self.busy.get() {
            return;
        }
        self.mode.set(Mode::Tracks);
        self.selected.set(Some(playlist.clone()));
        self.offset.set(offset);
        self.more.set(false);
        self.tracks.set(Vec::new());
        self.work(
            move || {
                remote::read_json::<PlaylistPage>(&format!(
                    "/ui/playlist?id={}&offset={offset}",
                    playlist.id
                ))
            },
            |this, page| {
                this.more.set(page.more);
                this.tracks
                    .set(page.tracks.into_iter().enumerate().collect());
            },
        );
    }
    fn login(self) {
        if self.busy.get() {
            return;
        }
        self.mode.set(Mode::Login);
        self.challenge.set(None);
        self.progress.set("正在生成登录二维码…".into());
        self.work(
            || remote::post_json::<_, LoginChallenge>("/ui/login/start", &serde_json::json!({})),
            |this, challenge| {
                this.challenge.set(Some(challenge));
                this.progress
                    .set("用网易云音乐 App 扫码，并确认登录".into());
            },
        );
    }
    fn poll(self) {
        if self.mode.get() != Mode::Login || self.busy.get() {
            return;
        }
        let Some(challenge) = self.challenge.get() else {
            return;
        };
        if (nsh_core::time::unix_ms() / 1000.0) as u64 >= challenge.expires_at {
            self.challenge.set(None);
            self.progress.set("二维码已过期，请重新生成".into());
            return;
        }
        let input = LoginPoll {
            key: challenge.key,
            device: challenge.device,
        };
        self.work(
            move || remote::post_json::<_, LoginProgress>("/ui/login/poll", &input),
            |this, progress| match progress.status.as_str() {
                "success" => {
                    this.challenge.set(None);
                    this.list_offset.set(0);
                    this.lists();
                }
                "scanned" => this.progress.set("已扫码，请在网易云 App 中确认".into()),
                "expired" => {
                    this.challenge.set(None);
                    this.progress.set("二维码已过期，请重新生成".into());
                }
                _ => {}
            },
        );
    }
    fn logout(self) {
        self.work(
            || remote::post_json::<_, serde_json::Value>("/ui/logout", &serde_json::json!({})),
            |this, _| {
                this.lists.set(Vec::new());
                this.more.set(false);
                this.list_offset.set(0);
                this.error.set("已退出登录".into());
            },
        );
    }
    fn next(self, delta: i64) {
        if self.busy.get() {
            return;
        }
        match self.mode.get() {
            Mode::Lists => {
                self.list_offset
                    .update(|n| *n = n.saturating_add_signed(delta));
                self.lists();
            }
            Mode::Tracks => {
                if let Some(list) = self.selected.get() {
                    self.tracks(list, self.offset.get().saturating_add_signed(delta));
                }
            }
            Mode::Login => {}
        }
    }
}

fn action(
    label: &'static str,
    disabled: impl Fn() -> bool + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .disabled(disabled)
        .h(px(44.0))
        .px(px(8.0))
        .rounded(px(7.0))
        .active(|s| s.opacity(0.6))
        .disabled_style(|s| s.opacity(0.35))
        .child(
            text(label)
                .text_size(px(13.0))
                .text_color(move || theme().foreground),
        )
}
fn row(
    title: impl Fn() -> String + 'static,
    subtitle: impl Fn() -> String + 'static,
    disabled: impl Fn() -> bool + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .disabled(disabled)
        .w_full()
        .min_h(px(66.0))
        .py(px(10.0))
        .px(px(8.0))
        .rounded(px(8.0))
        .active(|s| s.bg(theme().muted))
        .disabled_style(|s| s.opacity(0.4))
        .child(
            text(title)
                .text_size(px(15.0))
                .number_of_lines(1)
                .text_color(move || theme().foreground),
        )
        .child(
            text(subtitle)
                .text_size(px(12.0))
                .number_of_lines(1)
                .mt(px(5.0))
                .text_color(move || theme().muted_foreground),
        )
}
pub fn library_view(on_play: impl Fn(Track) + Copy + 'static) -> impl IntoElement {
    let this = Browser {
        owner: Owner::current(),
        mode: signal(Mode::Lists),
        busy: signal(false),
        error: signal(String::new()),
        lists: signal(Vec::new()),
        tracks: signal(Vec::new()),
        selected: signal(None),
        offset: signal(0),
        list_offset: signal(0),
        more: signal(false),
        challenge: signal(None),
        progress: signal(String::new()),
    };
    this.lists();
    set_interval(Duration::from_secs(2), move || this.poll());
    view()
        .flex_1()
        .pt(px(12.0))
        .child(when(
            move || this.mode.get() == Mode::Lists,
            move || {
                view()
                    .flex_row()
                    .justify_between()
                    .child(action(
                        "登录 / 切换账号",
                        move || this.busy.get(),
                        move || this.login(),
                    ))
                    .child(action(
                        "退出登录",
                        move || this.busy.get(),
                        move || this.logout(),
                    ))
                    .child(action(
                        "刷新",
                        move || this.busy.get(),
                        move || this.lists(),
                    ))
            },
        ))
        .child(when(
            move || this.mode.get() != Mode::Lists,
            move || action("返回歌单", move || this.busy.get(), move || this.lists()),
        ))
        .child(when(
            move || this.mode.get() == Mode::Lists,
            move || {
                scroll().flex_1().child(
                    view()
                        .child(each(
                            move || this.lists.get(),
                            |entry| entry.0,
                            move |entry, _| {
                                row(
                                    move || entry.with(|e| e.1.name.clone()),
                                    move || {
                                        entry.with(|e| {
                                            format!(
                                                "{} 首 · {}",
                                                e.1.track_count,
                                                if e.1.owned {
                                                    "创建的歌单"
                                                } else {
                                                    "收藏的歌单"
                                                }
                                            )
                                        })
                                    },
                                    move || this.busy.get(),
                                    move || this.tracks(entry.with(|e| e.1.clone()), 0),
                                )
                            },
                        ))
                        .child(when(
                            move || {
                                !this.busy.get()
                                    && this.error.with(String::is_empty)
                                    && this.lists.with(Vec::is_empty)
                            },
                            || text("暂无歌单").text_size(px(14.0)),
                        )),
                )
            },
        ))
        .child(when(
            move || this.mode.get() == Mode::Tracks,
            move || {
                view()
                    .flex_1()
                    .child(
                        text(move || {
                            this.selected
                                .with(|s| s.as_ref().map_or_else(String::new, |p| p.name.clone()))
                        })
                        .text_size(px(18.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                    )
                    .child(scroll().flex_1().child(view().child(each(
                        move || this.tracks.get(),
                        |entry| entry.0,
                        move |entry, _| {
                            row(
                                move || entry.with(|e| e.1.title.clone()),
                                move || entry.with(|e| e.1.artist.clone()),
                                move || this.busy.get(),
                                move || on_play(entry.with(|e| e.1.clone())),
                            )
                        },
                    ))))
            },
        ))
        .child(when(
            move || this.mode.get() == Mode::Login,
            move || {
                scroll().flex_1().child(
                    view()
                        .items_center()
                        .gap(px(14.0))
                        .pt(px(20.0))
                        .child(when(
                            move || this.challenge.with(Option::is_some),
                            move || {
                                image(move || {
                                    this.challenge.with(|c| {
                                        ImageSrc::svg_markup(
                                            c.as_ref().map_or("", |c| c.svg.as_str()),
                                        )
                                    })
                                })
                                .w(px(224.0))
                                .h(px(224.0))
                                .label("网易云登录二维码")
                                .on_load(
                                    |_: &nsh_core::Size<DevicePixels>| {
                                        nsh_core::image::loader::loader().clear();
                                        nsh_core::image::budget().clear();
                                    },
                                )
                            },
                        ))
                        .child(
                            text(move || this.progress.get())
                                .text_size(px(15.0))
                                .text_color(move || theme().foreground),
                        )
                        .child(action(
                            "重新生成二维码",
                            move || this.busy.get(),
                            move || this.login(),
                        ))
                        .child(
                            text("登录状态保存在这台 iPhone。可用另一台设备上的网易云 App 扫码。")
                                .text_size(px(12.0))
                                .text_color(move || theme().muted_foreground),
                        ),
                )
            },
        ))
        .child(when(
            move || this.mode.get() != Mode::Login,
            move || {
                view()
                    .flex_row()
                    .justify_between()
                    .child(action(
                        "上一页",
                        move || {
                            this.busy.get()
                                || match this.mode.get() {
                                    Mode::Lists => this.list_offset.get() == 0,
                                    _ => this.offset.get() == 0,
                                }
                        },
                        move || this.next(-50),
                    ))
                    .child(action(
                        "下一页",
                        move || this.busy.get() || !this.more.get(),
                        move || this.next(50),
                    ))
            },
        ))
        .child(when(
            move || this.busy.get(),
            || {
                text("正在加载…")
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground)
            },
        ))
        .child(
            text(move || this.error.get())
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground),
        )
}
