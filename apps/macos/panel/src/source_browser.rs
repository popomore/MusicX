//! The source-management page is disposable. Only a live sign-in owns a temporary VM.
use crate::{
    protocol::{
        MusicSource, MusicSourceAction as Action, SignInMethod, SourceAccount, SourceLogin,
        SourcePlaylist,
    },
    remote,
};
use nsh_base::{button, input, theme};
use nsh_core::{element::ImageSrc, prelude::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::oneshot;
static LOGIN_JOB: Mutex<Option<String>> = Mutex::new(None);
pub fn cancel_active_login() {
    if let Ok(mut job) = LOGIN_JOB.lock()
        && let Some(id) = job.take()
    {
        remote::cancel_source_login(id);
    }
}
fn remember_job(job: Option<String>) {
    if let Ok(mut value) = LOGIN_JOB.lock() {
        *value = job;
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Page {
    List,
    Catalog,
    Detail,
    Login,
    Playlists,
}
#[derive(Clone, Copy)]
struct Browser {
    root: Page,
    managing: Signal<bool>,
    configuring: Signal<bool>,
    imported: Signal<bool>,
    disconnecting: Signal<bool>,
    owner: Owner,
    page: Signal<Page>,
    busy: Signal<bool>,
    message: Signal<String>,
    rows: Signal<Vec<MusicSource>>,
    selected: Signal<Option<MusicSource>>,
    draft: Signal<BTreeMap<String, String>>,
    dirty: Signal<BTreeSet<String>>,
    account: Signal<Option<SourceAccount>>,
    login: Signal<Option<SourceLogin>>,
    playlists: Signal<Vec<SourcePlaylist>>,
    playlist_id: Signal<String>,
}
fn request<T: serde::de::DeserializeOwned>(action: Action) -> anyhow::Result<T> {
    remote::post_json("/ui/music-sources", &action)
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
                    Ok(Ok(value)) => done(self, value),
                    Ok(Err(e)) => self.message.set(e.to_string()),
                    Err(_) => self.message.set("数据源请求已结束，请重试".into()),
                }
            })
        });
    }
    fn list(self, catalog: bool) {
        cancel_active_login();
        self.login.set(None);
        self.selected.set(None);
        self.account.set(None);
        self.page
            .set(if catalog { Page::Catalog } else { Page::List });
        self.rows.set(Vec::new());
        self.work(
            move || {
                request::<Vec<MusicSource>>(if catalog {
                    Action::Catalog
                } else {
                    Action::List
                })
            },
            |this, rows| this.rows.set(rows),
        );
    }
    fn populate(self, source: MusicSource) {
        self.imported.set(false);
        self.configuring
            .set(source.settings.iter().any(|f| f.secret && !f.configured));
        let values = source
            .settings
            .iter()
            .map(|f| {
                (
                    f.definition.key.clone(),
                    f.saved_value.clone().unwrap_or_default(),
                )
            })
            .collect();
        self.draft.set(values);
        self.dirty.set(BTreeSet::new());
        self.selected.set(Some(source));
        self.page.set(Page::Detail);
    }
    fn select(self, source: MusicSource) {
        self.account.set(None);
        if source.installed {
            let id = source.id;
            self.work(
                move || request::<MusicSource>(Action::Get { source: id }),
                |this, source| {
                    this.populate(source);
                    this.refresh_account();
                },
            );
        } else {
            self.populate(source);
        }
    }
    fn refresh_account(self) {
        let Some(source) = self.selected.get() else {
            return;
        };
        if !source.installed || !source.capabilities.iter().any(|v| v == "account") {
            return;
        }
        self.work(
            move || request::<SourceAccount>(Action::Account { source: source.id }),
            |this, account| this.account.set(Some(account)),
        );
    }
    fn install(self) {
        let Some(repository) = self
            .selected
            .with(|v| v.as_ref().and_then(|s| s.repository.clone()))
        else {
            return;
        };
        self.work(
            move || request::<MusicSource>(Action::Install { repository }),
            |this, source| {
                this.populate(source);
                this.message.set("数据源已添加".into());
                this.refresh_account();
            },
        );
    }
    fn save(self) {
        let Some(source) = self.selected.with(|v| v.as_ref().map(|v| v.id.clone())) else {
            return;
        };
        let values = self.dirty.with(|keys| {
            self.draft.with(|draft| {
                keys.iter()
                    .map(|key| (key.clone(), draft.get(key).cloned().unwrap_or_default()))
                    .collect()
            })
        });
        self.work(
            move || request::<serde_json::Value>(Action::Configure { source, values }),
            |this, value| {
                if let Ok(source) = serde_json::from_value(value["source"].clone()) {
                    this.populate(source);
                    this.message
                        .set(value["warning"].as_str().unwrap_or("配置已保存").into());
                }
            },
        );
    }
    fn sign_in(self, method: SignInMethod) {
        let Some(source) = self.selected.with(|v| v.as_ref().map(|v| v.id.clone())) else {
            return;
        };
        let job = format!(
            "panel-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        remember_job(Some(job.clone()));
        self.work(
            move || {
                request::<SourceLogin>(Action::SignIn {
                    source,
                    job: Some(job),
                    method,
                })
            },
            |this, job| {
                this.login.set(Some(job));
                this.page.set(Page::Login);
            },
        );
    }
    fn poll(self) {
        if self.page.get() != Page::Login || self.busy.get() {
            return;
        }
        let Some(job) = self.login.get() else {
            return;
        };
        if !matches!(job.status.as_str(), "waiting" | "cancelling") {
            return;
        }
        self.work(
            move || request::<SourceLogin>(Action::Poll { job: job.job }),
            |this, job| {
                let terminal = matches!(job.status.as_str(), "success" | "error" | "cancelled");
                let success = job.status == "success";
                if terminal {
                    remember_job(None);
                    this.message.set(if success {
                        "账号已关联".into()
                    } else {
                        job.text.clone()
                    });
                    this.login.set(None);
                    this.page.set(Page::Detail);
                    if success {
                        this.refresh_account();
                    }
                } else {
                    this.message.set(String::new());
                    this.login.set(Some(job));
                }
            },
        );
    }
    fn cancel(self) {
        cancel_active_login();
        self.login.set(None);
        self.page.set(Page::Detail);
        self.message.set("已取消账号关联".into());
    }
    fn sign_out(self) {
        let Some(source) = self.selected.with(|v| v.as_ref().map(|v| v.id.clone())) else {
            return;
        };
        self.work(
            move || request::<serde_json::Value>(Action::SignOut { source }),
            |this, _| {
                this.account.set(Some(SourceAccount::default()));
                this.message.set("已断开来源账号，本地歌单保留".into());
            },
        );
    }
    fn playlists(self) {
        let Some(source) = self.selected.get() else {
            return;
        };
        self.page.set(Page::Playlists);
        self.playlists.set(Vec::new());
        self.message.set(String::new());
        if source.capabilities.iter().any(|v| v == "playlists") {
            self.work(
                move || request::<Vec<SourcePlaylist>>(Action::Playlists { source: source.id }),
                |this, rows| this.playlists.set(rows),
            );
        }
    }
    fn import(self, id: String) {
        let Some(source) = self.selected.with(|v| v.as_ref().map(|v| v.id.clone())) else {
            return;
        };
        self.work(
            move || {
                request::<crate::protocol::LocalPlaylist>(Action::ImportPlaylist {
                    source,
                    playlist_id: id,
                })
            },
            |this, list| {
                this.imported.set(true);
                this.message.set(format!(
                    "已导入「{}」· {} 首，本地副本已保存",
                    list.name,
                    list.tracks.len()
                ))
            },
        );
    }
}
fn action(
    label: &'static str,
    disabled: impl Fn() -> bool + 'static,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .accessibility_label(Some(label.into()))
        .disabled(disabled)
        .on_press(press)
        .px(px(8.0))
        .py(px(7.0))
        .rounded(px(5.0))
        .hover(|s| s.bg(theme().muted))
        .disabled_style(|s| s.opacity(0.4))
        .child(
            text(label)
                .text_size(px(11.0))
                .text_color(move || theme().foreground),
        )
}
fn caption(value: impl IntoReactive<SharedString>) -> impl IntoElement {
    text(value)
        .text_size(px(11.0))
        .number_of_lines(4)
        .text_color(move || theme().muted_foreground)
}
fn capabilities(source: &MusicSource) -> String {
    [
        ("search", "搜索"),
        ("stream", "播放"),
        ("lyrics", "歌词"),
        ("album", "专辑"),
        ("playlists", "歌单"),
        ("signIn", "账号"),
    ]
    .iter()
    .filter(|(key, _)| source.capabilities.iter().any(|v| v == key))
    .map(|(_, label)| *label)
    .collect::<Vec<_>>()
    .join(" · ")
}
#[derive(Clone, Copy)]
pub enum Start {
    Detail,
    Import,
    Catalog,
}
pub fn sources_view(
    back: impl Fn() + Copy + 'static,
    done: impl Fn() + Copy + 'static,
    initial: Option<MusicSource>,
    start: Start,
) -> impl IntoElement {
    let root = match start {
        Start::Detail => Page::Detail,
        Start::Import => Page::Playlists,
        Start::Catalog => Page::Catalog,
    };
    let this = Browser {
        root,
        managing: signal(false),
        configuring: signal(false),
        imported: signal(false),
        disconnecting: signal(false),
        owner: Owner::current(),
        page: signal(Page::List),
        busy: signal(false),
        message: signal(String::new()),
        rows: signal(Vec::new()),
        selected: signal(None),
        draft: signal(BTreeMap::new()),
        dirty: signal(BTreeSet::new()),
        account: signal(None),
        login: signal(None),
        playlists: signal(Vec::new()),
        playlist_id: signal(String::new()),
    };
    cleanup(cancel_active_login);
    match start {
        Start::Catalog => this.list(true),
        Start::Detail => {
            if let Some(source) = initial {
                this.select(source);
            }
        }
        Start::Import => {
            if let Some(source) = initial {
                this.populate(source);
                this.playlists();
            }
        }
    }
    set_interval(Duration::from_secs(2), move || this.poll());
    view().flex_1().min_h(px(0.0)).gap(px(8.0))
        .child(view().flex_row().items_center().justify_between()
            .child(action("返回",move||this.busy.get(),move||{let page=this.page.get();if page==this.root||(page==Page::Detail&&this.root==Page::Playlists){back();}else{match page{Page::Catalog|Page::List=>back(),Page::Detail=>this.list(true),Page::Login=>this.cancel(),Page::Playlists=>this.page.set(Page::Detail)}}}))
            .child(when(move||this.page.get()==Page::List,move||action("添加数据源",move||this.busy.get(),move||this.list(true)))))
        .child(when(move||matches!(this.page.get(),Page::List|Page::Catalog),move||{
            view().flex_1().min_h(px(0.0)).gap(px(6.0))
                .child(caption(move||if this.page.get()==Page::Catalog{"MyGo 插件目录 · 服务可用性需逐项验证"}else{"已添加的数据源"}))
                .child(when(move||!this.busy.get()&&this.rows.with(Vec::is_empty),||caption("还没有插件数据源，点击添加。")))
                .child(virtual_list(move||this.rows.with(Vec::len),move|at|{
                    let source=move||this.rows.with(|r|r.get(at.index()).cloned());
                    button().accessibility_label(move||Some(SharedString::from(format!("配置数据源 {}",source().map_or_else(String::new,|s|s.name)))))
                        .disabled(move||this.busy.get()).on_press(move||{if let Some(source)=source(){this.select(source);}})
                        .h(px(70.0)).w_full().justify_center().px(px(8.0)).gap(px(4.0)).rounded(px(6.0)).hover(|s|s.bg(theme().muted))
                        .child(text(move||source().map_or_else(String::new,|s|s.name)).number_of_lines(1).text_size(px(14.0)).text_color(move||theme().foreground))
                        .child(caption(move||source().map_or_else(String::new,|s|if s.installed{"已添加".into()}else if s.can_install{capabilities(&s)}else{"尚未支持此类型".into()})))
                }).flex_1().min_h(px(0.0)).estimated_item_size(px(70.0)).label("音乐数据源列表"))
        }))
        .child(when(move||this.page.get()==Page::Detail,move||{
            scroll().flex_1().min_h(px(0.0)).child(view().gap(px(10.0)).pb(px(12.0))
                .child(view().flex_row().items_center().gap(px(9.0))
                    .child(crate::controls::source_badge(move||this.selected.with(|s|s.as_ref().map_or_else(String::new,|s|s.id.clone()))))
                    .child(text(move||this.selected.with(|s|s.as_ref().map_or_else(String::new,|s|crate::controls::source_name(&s.id,&s.name)))).number_of_lines(2).text_size(px(19.0)).font_weight(FontWeight::SEMIBOLD).text_color(move||theme().foreground)))
                .child(when(move||this.managing.get()||this.selected.with(|s|s.as_ref().is_some_and(|s|!s.installed)),move||caption(move||this.selected.with(|s|s.as_ref().map_or_else(String::new,|s|format!("{} · {}",s.version,capabilities(s)))))))
                .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.repository.is_some()&&(!s.installed||this.managing.get()))),move||button()
                    .accessibility_label(Some("打开数据源项目".into())).on_press(move||{if let Some(url)=this.selected.with(|s|s.as_ref().and_then(|s|s.repository.clone())){spawn(async move{let _=open_url(url).await;});}})
                    .py(px(6.0)).rounded(px(4.0)).hover(|s|s.bg(theme().muted)).child(caption(move||this.selected.with(|s|s.as_ref().and_then(|s|s.repository.clone()).unwrap_or_default())))))
                .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.limitation.is_some())),move||caption(move||this.selected.with(|s|s.as_ref().and_then(|s|s.limitation.clone()).unwrap_or_default()))))
                .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|!s.installed)),move||{
                    view().gap(px(8.0))
                        .child(action("添加此数据源",move||this.busy.get()||this.selected.with(|s|s.as_ref().is_none_or(|s|!s.can_install)),move||this.install()))
                        .child(each(move||this.selected.with(|s|s.as_ref().map_or_else(Vec::new,|s|s.settings.clone())),|f|f.definition.key.clone(),move|field,_|view().gap(px(4.0))
                            .child(text(move||field.with(|f|f.definition.label.clone())).text_size(px(12.0)).text_color(move||theme().foreground))
                            .child(caption(move||field.with(|f|f.definition.help.clone())))))
                }))
                .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.installed)),move||{
                    view().gap(px(10.0))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().any(|v|v=="account"||v=="signIn"))),move||{
                            view().gap(px(6.0)).pt(px(6.0))
                                .child(text("账号").text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).text_color(move||theme().foreground))
                                .child(caption(move||this.account.with(|v| match v {
                                    Some(a) if a.connected => a.name.as_ref().map_or_else(||"已连接".into(), |name| format!("已连接 · {name}")),
                                    Some(_) => "账号未连接".into(),
                                    None if this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().all(|v|v!="account"))) => "此来源未提供账号状态".into(),
                                    None if this.busy.get() => "正在检查账号…".into(),
                                    None => "暂时无法读取账号状态".into(),
                                })))
                                .child(view().flex_row().gap(px(4.0))
                                    .child(when(move||this.managing.get(),move||action("刷新账号",move||this.busy.get(),move||this.refresh_account())))
                                    .child(when(move||this.account.with(|a|a.as_ref().is_none_or(|a|!a.connected)),move||view().flex_row().gap(px(4.0))
                                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.web_sign_in)),move||crate::controls::primary("网页登录",move||this.busy.get(),move||this.sign_in(SignInMethod::Web))))
                                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.qr_sign_in)),move||action("扫码登录",move||this.busy.get(),move||this.sign_in(SignInMethod::Qr))))))
                                    .child(when(move||this.managing.get()&&this.account.with(|s|s.as_ref().is_some_and(|a|a.connected)),move||action("断开来源账号",move||this.busy.get(),move||this.disconnecting.set(true))))
                                    .child(when(move||this.disconnecting.get(),move||action("确认断开来源账号",move||this.busy.get(),move||{this.disconnecting.set(false);this.sign_out();}))))
                                .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|!s.qr_sign_in&&!s.web_sign_in)),||caption("此来源的登录方式尚未接入，可先填写它提供的账号配置项")))
                        }))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().any(|v|v=="playlist"))),move||{
                            view().gap(px(6.0)).pt(px(6.0))
                                .child(crate::controls::section("歌单"))
                                .child(caption("导入后保存到我的歌单。"))
                                .child(action("导入来源歌单",move||this.busy.get(),move||this.playlists()))
                        }))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|!s.settings.is_empty())),move||action("来源设置",move||this.busy.get(),move||this.configuring.update(|v|*v = !*v))))
                        .child(when(move||this.configuring.get(),move||view().gap(px(10.0))
                        .child(each(move||this.selected.with(|s|s.as_ref().map_or_else(Vec::new,|s|s.settings.clone())),|f|f.definition.key.clone(),move|field,_|{
                            view().gap(px(5.0))
                                .child(text(move||field.with(|f|f.definition.label.clone())).text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).text_color(move||theme().foreground))
                                .child(input().label(move||SharedString::from(format!("配置 {}",field.with(|f|f.definition.label.clone()))))
                                    .value(move||field.with(|f|this.draft.with(|v|v.get(&f.definition.key).cloned().unwrap_or_default())))
                                    .placeholder(move||field.with(|f|if f.secret&&f.configured{"已保存，输入可替换".to_owned()}else{f.definition.value.clone().unwrap_or_default()}))
                                    .masked(move||field.with(|f|f.secret)).on_change(move|value:&str|{let key=field.with(|f|f.definition.key.clone());this.draft.update(|v|{v.insert(key.clone(),value.into());});this.dirty.update(|keys|{keys.insert(key);});})
                                    .h(px(34.0)).px(px(8.0)).rounded(px(5.0)).bg(move||theme().muted).text_color(move||theme().foreground))
                                .child(caption(move||field.with(|f|f.definition.help.clone())))
                                .child(when(move||field.with(|f|f.secret&&f.configured),move||action("清除此项",move||this.busy.get(),move||{let key=field.with(|f|f.definition.key.clone());this.draft.update(|v|{v.insert(key.clone(),String::new());});this.dirty.update(|keys|{keys.insert(key);});})))
                        }))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|!s.settings.is_empty())),move||action("保存配置",move||this.busy.get()||this.dirty.with(BTreeSet::is_empty),move||this.save())))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.settings.is_empty())),||caption("此来源没有额外配置项")))
                        ))
                        .child(action("更多选项",move||this.busy.get(),move||{this.managing.update(|v|*v = !*v);this.disconnecting.set(false);} ))
                }))
            )
        }))
        .child(when(move||this.page.get()==Page::Login,move||scroll().flex_1().min_h(px(0.0)).child(view().gap(px(12.0)).items_center()
            .child(text("关联来源账号").text_size(px(18.0)).font_weight(FontWeight::SEMIBOLD).text_color(move||theme().foreground))
            .child(when(move||this.login.with(|j|j.as_ref().is_some_and(|j|j.svg.is_some())),move||image(move||this.login.with(|j|ImageSrc::svg_markup(j.as_ref().and_then(|j|j.svg.as_deref()).unwrap_or_default())))
                .w(px(220.0)).h(px(220.0)).label("数据源登录二维码").on_load(|_:&nsh_core::Size<DevicePixels>|{nsh_core::image::loader::loader().clear();nsh_core::image::budget().clear();})))
            .child(caption(move||this.login.with(|j|j.as_ref().map_or_else(String::new,|j|j.text.clone()))))
            .child(action("取消关联",move||this.busy.get(),move||this.cancel()))
        )))
        .child(when(move||this.page.get()==Page::Playlists,move||view().flex_1().min_h(px(0.0)).gap(px(8.0))
            .child(crate::controls::heading("导入来源歌单"))
            .child(caption("选择来源歌单，或按歌单 ID 导入到我的歌单。"))
            .child(when(move||!this.busy.get()&&this.playlists.with(Vec::is_empty)&&this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().any(|v|v=="playlists"))),move||action("查看来源账号与配置",move||this.busy.get(),move||{this.page.set(Page::Detail);this.refresh_account();})))
            .child(view().flex_row().items_center().gap(px(4.0))
                .child(input().label("来源歌单ID").placeholder("来源歌单ID").value(move||this.playlist_id.get()).on_change(move|v:&str|this.playlist_id.set(v.into())).flex_1().min_w(px(0.0)).h(px(32.0)).px(px(8.0)).rounded(px(5.0)).bg(move||theme().muted).text_color(move||theme().foreground))
                .child(action("导入",move||this.busy.get()||this.playlist_id.with(|v|v.trim().is_empty()),move||this.import(this.playlist_id.get()))))
            .child(virtual_list(move||this.playlists.with(Vec::len),move|at|{
                let item=move||this.playlists.with(|rows|rows.get(at.index()).cloned());
                view().flex_row().items_center().gap(px(4.0)).h(px(65.0))
                    .child(crate::controls::artwork(move||item().and_then(|p|p.cover_url),move||format!("{}封面",item().map_or_else(String::new,|p|p.name)),40.0))
                    .child(view().flex_1().min_w(px(0.0)).gap(px(3.0))
                        .child(text(move||item().map_or_else(String::new,|p|p.name)).text_size(px(13.0)).number_of_lines(1).text_color(move||theme().foreground))
                        .child(caption(move||item().map_or_else(String::new,|p|format!("{} 首",p.track_count)))))
                    .child(button().accessibility_label(move||Some(SharedString::from(format!("导入来源歌单 {}",item().map_or_else(String::new,|p|p.name)))))
                        .disabled(move||this.busy.get()).on_press(move||{if let Some(p)=item(){this.import(p.id);}}).px(px(8.0)).py(px(7.0)).rounded(px(5.0)).hover(|s|s.bg(theme().muted))
                        .child(text("导入").text_size(px(11.0)).text_color(move||theme().foreground)))
            }).flex_1().min_h(px(0.0)).estimated_item_size(px(65.0)).label("来源歌单列表"))
        ))
        .child(when(move||this.imported.get()&&this.page.get()==Page::Playlists,move||crate::controls::primary("查看已导入歌单",move||this.busy.get(),done)))
        .child(when(move||this.busy.get(),||caption("正在读取或保存…")))
        .child(caption(move||this.message.get()))
}
