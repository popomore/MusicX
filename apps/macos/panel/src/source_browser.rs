//! The source-management page is disposable. Only a live sign-in owns a temporary VM.
use crate::protocol::tr;
use crate::{
    protocol::{
        MusicSource, MusicSourceAction as Action, MusicSourceCatalog, SignInMethod, SourceAccount,
        SourceLogin, SourcePlaylist,
    },
    remote,
};
use nsh_base::{button, input, theme};
use nsh_core::{element::ImageSrc, prelude::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::oneshot;
#[derive(Default)]
struct LoginJobs(BTreeMap<u64, String>);

impl LoginJobs {
    fn take(&mut self, owner: u64) -> Option<String> {
        self.0.remove(&owner)
    }
    fn finish(&mut self, owner: u64, job: &str) {
        if self.0.get(&owner).is_some_and(|current| current == job) {
            self.0.remove(&owner);
        }
    }
    #[cfg(any(target_os = "macos", test))]
    fn drain(&mut self) -> Vec<String> {
        std::mem::take(&mut self.0).into_values().collect()
    }
}

static LOGIN_JOBS: Mutex<LoginJobs> = Mutex::new(LoginJobs(BTreeMap::new()));

#[derive(Clone, Copy)]
struct LoginOwner(u64);
impl LoginOwner {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
    fn remember(self, job: String) {
        self.cancel();
        LOGIN_JOBS.lock().unwrap().0.insert(self.0, job);
    }
    fn finish(self, job: &str) {
        LOGIN_JOBS.lock().unwrap().finish(self.0, job);
    }
    fn cancel(self) {
        let job = LOGIN_JOBS.lock().unwrap().take(self.0);
        if let Some(job) = job {
            remote::cancel_source_login(job);
        }
    }
}

/// Process shutdown cancels every remaining browser's job.
#[cfg(target_os = "macos")]
pub fn cancel_active_logins() {
    let jobs = LOGIN_JOBS.lock().unwrap().drain();
    for job in jobs {
        remote::cancel_source_login(job);
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
    login_owner: LoginOwner,
    root: Page,
    managing: Signal<bool>,
    configuring: Signal<bool>,
    imported: Signal<bool>,
    disconnecting: Signal<bool>,
    removing: Signal<bool>,
    owner: Owner,
    page: Signal<Page>,
    busy: Signal<bool>,
    message: Signal<String>,
    rows: Signal<Vec<MusicSource>>,
    catalog_refreshing: Signal<bool>,
    catalog_loading: Signal<bool>,
    catalog_warning: Signal<Option<String>>,
    repository: Signal<String>,
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
                    Err(_) => self
                        .message
                        .set(tr!("Source request ended; please try again").into()),
                }
            })
        });
    }
    fn list(self, catalog: bool) {
        self.login_owner.cancel();
        self.login.set(None);
        self.selected.set(None);
        self.account.set(None);
        self.page
            .set(if catalog { Page::Catalog } else { Page::List });
        self.rows.set(Vec::new());
        if catalog {
            self.message.set(String::new());
            self.catalog_warning.set(None);
            self.read_catalog(false);
            return;
        }
        self.work(
            move || request::<Vec<MusicSource>>(Action::List),
            |this, rows| this.rows.set(rows),
        );
    }
    fn read_catalog(self, refresh: bool) {
        if self.catalog_loading.get() || self.busy.get() {
            return;
        }
        self.catalog_loading.set(true);
        let (send, receive) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = send.send(request::<MusicSourceCatalog>(if refresh {
                Action::RefreshCatalog
            } else {
                Action::CatalogView
            }));
        });
        self.owner.run(|| {
            spawn(async move {
                let result = receive.await;
                self.catalog_loading.set(false);
                if self.page.get() == Page::Catalog {
                    match result {
                        Ok(Ok(catalog)) => {
                            self.rows.set(catalog.sources);
                            self.catalog_refreshing.set(catalog.refreshing);
                            self.catalog_warning.set(catalog.warning);
                        }
                        result => {
                            self.catalog_refreshing.set(false);
                            self.catalog_warning.set(Some(match result {
                                Ok(Err(error)) => error.to_string(),
                                _ => tr!("Catalog request ended; please try again").into(),
                            }));
                        }
                    }
                }
            })
        });
    }
    fn populate(self, source: MusicSource) {
        self.imported.set(false);
        self.removing.set(false);
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
        self.install_repository(repository);
    }
    fn install_repository(self, repository: String) {
        self.work(
            move || request::<MusicSource>(Action::Install { repository }),
            |this, source| {
                this.populate(source);
                this.message.set(tr!("Source added").into());
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
                    this.message.set(
                        value["warning"]
                            .as_str()
                            .unwrap_or(tr!("Configuration saved"))
                            .into(),
                    );
                }
            },
        );
    }
    fn sign_in(self, method: SignInMethod) {
        if self.busy.get() {
            return;
        }
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
        self.login_owner.remember(job.clone());
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
        if self.page.get() == Page::Catalog && self.catalog_refreshing.get() && !self.busy.get() {
            self.read_catalog(false);
            return;
        }
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
                    this.login_owner.finish(&job.job);
                    this.message.set(if success {
                        tr!("Account connected").into()
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
        self.login_owner.cancel();
        self.login.set(None);
        self.page.set(Page::Detail);
        self.message.set(tr!("Account connection canceled").into());
    }
    fn sign_out(self) {
        let Some(source) = self.selected.with(|v| v.as_ref().map(|v| v.id.clone())) else {
            return;
        };
        self.work(
            move || request::<serde_json::Value>(Action::SignOut { source }),
            |this, _| {
                this.account.set(Some(SourceAccount::default()));
                this.message
                    .set(tr!("Account disconnected; local playlists kept").into());
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
                this.message.set(tr!(
                    "Imported “{name}” · {tracks}; local copy saved";
                    name = list.display_name(),
                    tracks = crate::protocol::i18n::tracks(list.tracks.len())
                ))
            },
        );
    }
    fn import_collection(self, liked: bool) {
        let Some(source) = self.selected.with(|v| v.as_ref().map(|v| v.id.clone())) else {
            return;
        };
        self.work(
            move || {
                request::<crate::protocol::LocalPlaylist>(if liked {
                    Action::ImportLiked { source }
                } else {
                    Action::ImportRecent { source }
                })
            },
            |this, list| {
                this.imported.set(true);
                this.message.set(tr!(
                    "Imported “{name}” · {tracks}; local copy saved";
                    name = list.display_name(),
                    tracks = crate::protocol::i18n::tracks(list.tracks.len())
                ));
            },
        );
    }
    fn check_update(self) {
        let Some(source) = self.selected.get() else {
            return;
        };
        self.work(
            move || request::<serde_json::Value>(Action::CheckUpdate { source: source.id }),
            |this, result| {
                let available = result["available"].as_bool().unwrap_or(false);
                this.message.set(if available {
                    tr!(
                        "Update available: {}",
                        result["latest_version"].as_str().unwrap_or_default()
                    )
                } else {
                    tr!("Up to date").into()
                });
            },
        );
    }
    fn update(self) {
        let Some(source) = self.selected.get() else {
            return;
        };
        self.work(
            move || request::<MusicSource>(Action::Update { source: source.id }),
            |this, source| {
                this.populate(source);
                this.message
                    .set(tr!("Source updated; account and settings kept").into());
            },
        );
    }
    fn uninstall(self) {
        let Some(source) = self.selected.get() else {
            return;
        };
        self.work(
            move || request::<serde_json::Value>(Action::Uninstall { source: source.id }),
            |this, _| {
                this.removing.set(false);
                this.selected.set(None);
                this.list(false);
                this.message
                    .set(tr!("Source removed; local playlists kept").into());
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
        ("search", tr!("Search")),
        ("stream", tr!("Play")),
        ("lyrics", tr!("Lyrics")),
        ("album", tr!("Album")),
        ("artist", tr!("Artist")),
        ("liked", tr!("Liked tracks import")),
        ("recent", tr!("History import")),
        ("shows", tr!("Podcasts")),
        ("playlists", tr!("Playlists")),
        ("signIn", tr!("Account")),
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
    open_source: impl Fn(Action) + Copy + 'static,
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
        login_owner: LoginOwner::new(),
        root,
        managing: signal(false),
        configuring: signal(false),
        imported: signal(false),
        disconnecting: signal(false),
        removing: signal(false),
        owner: Owner::current(),
        page: signal(Page::List),
        busy: signal(false),
        message: signal(String::new()),
        rows: signal(Vec::new()),
        catalog_refreshing: signal(false),
        catalog_loading: signal(false),
        catalog_warning: signal(None),
        repository: signal(String::new()),
        selected: signal(None),
        draft: signal(BTreeMap::new()),
        dirty: signal(BTreeSet::new()),
        account: signal(None),
        login: signal(None),
        playlists: signal(Vec::new()),
        playlist_id: signal(String::new()),
    };
    cleanup(move || this.login_owner.cancel());
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
            .child(action(tr!("Back"),move||this.busy.get(),move||{let page=this.page.get();if page==this.root||(page==Page::Detail&&this.root==Page::Playlists){back();}else{match page{Page::Catalog|Page::List=>back(),Page::Detail=>this.list(true),Page::Login=>this.cancel(),Page::Playlists=>this.page.set(Page::Detail)}}}))
            .child(when(move||this.page.get()==Page::List,move||action(tr!("Add source"),move||this.busy.get(),move||this.list(true)))))
        .child(when(move||matches!(this.page.get(),Page::List|Page::Catalog),move||{
            view().flex_1().min_h(px(0.0)).gap(px(6.0))
                .child(caption(move||if this.page.get()==Page::Catalog{tr!("MyGo plugin catalog · Service availability varies")}else{tr!("Added sources")}))
                .child(when(move||this.page.get()==Page::Catalog,move||view().gap(px(5.0))
                    .child(input().label(tr!("GitHub plugin repository")).placeholder(tr!("owner/repo or GitHub URL")).value(move||this.repository.get()).on_change(move|value:&str|this.repository.set(value.into()))
                        .w_full().min_w(px(0.0)).h(px(32.0)).px(px(8.0)).rounded(px(5.0)).bg(move||theme().muted).text_color(move||theme().foreground))
                    .child(view().flex_row().items_center().justify_between()
                        .child(action(tr!("Add repository"),move||this.busy.get()||this.repository.with(|value|value.trim().is_empty()),move||this.install_repository(this.repository.get())))
                        .child(action(tr!("Refresh catalog"),move||this.busy.get()||this.catalog_loading.get()||this.catalog_refreshing.get(),move||this.read_catalog(true))))
                    .child(when(move||this.catalog_refreshing.get()||this.catalog_loading.get(),move||caption(move||if this.catalog_refreshing.get(){tr!("Updating catalog…")}else{tr!("Loading catalog…")})))
                    .child(when(move||this.catalog_warning.with(Option::is_some),move||caption(move||this.catalog_warning.get().unwrap_or_default())))))
                .child(when(move||this.page.get()==Page::Catalog&&this.message.with(|message|!message.is_empty()),move||caption(move||this.message.get())))
                .child(when(move||!this.busy.get()&&!this.catalog_loading.get()&&this.rows.with(Vec::is_empty),move||caption(move||if this.page.get()==Page::Catalog{tr!("No sources in the catalog. You can add a repository directly.")}else{tr!("No plugin sources yet. Choose “Add source”.")})))
                .child(virtual_list(move||this.rows.with(Vec::len),move|at|{
                    let source=move||this.rows.with(|r|r.get(at.index()).cloned());
                    button().accessibility_label(move||Some(SharedString::from(tr!("Configure source {}", source().map_or_else(String::new,|s|s.name)))))
                        .disabled(move||this.busy.get()).on_press(move||{if let Some(source)=source(){this.select(source);}})
                        .h(px(70.0)).w_full().justify_center().px(px(8.0)).gap(px(4.0)).rounded(px(6.0)).hover(|s|s.bg(theme().muted))
                        .child(text(move||source().map_or_else(String::new,|s|s.name)).number_of_lines(1).text_size(px(14.0)).text_color(move||theme().foreground))
                        .child(caption(move||source().map_or_else(String::new,|s|if s.installed{tr!("Added").into()}else if s.can_install{if s.capabilities.is_empty(){s.description}else{capabilities(&s)}}else{tr!("This type is not supported yet").into()})))
                }).flex_1().min_h(px(0.0)).estimated_item_size(px(70.0)).label(tr!("Music source list")))
        }))
        .child(when(move||this.page.get()==Page::Detail,move||{
            scroll().flex_1().min_h(px(0.0)).child(view().gap(px(10.0)).pb(px(12.0))
                .child(view().flex_row().items_center().gap(px(9.0))
                    .child(crate::controls::source_badge(move||this.selected.with(|s|s.as_ref().map_or_else(String::new,|s|s.id.clone()))))
                    .child(text(move||this.selected.with(|s|s.as_ref().map_or_else(String::new,|s|crate::controls::source_name(&s.id,&s.name)))).number_of_lines(2).text_size(px(19.0)).font_weight(FontWeight::SEMIBOLD).text_color(move||theme().foreground)))
                .child(when(move||this.managing.get()||this.selected.with(|s|s.as_ref().is_some_and(|s|!s.installed)),move||caption(move||this.selected.with(|s|s.as_ref().map_or_else(String::new,|s|format!("{} · {}",s.version,capabilities(s)))))))
                .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|!s.description.is_empty())),move||caption(move||this.selected.with(|s|s.as_ref().map_or_else(String::new,|s|s.description.clone())))))
                .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.repository.is_some()&&(!s.installed||this.managing.get()))),move||button()
                    .accessibility_label(Some(tr!("Open source project").into())).on_press(move||{if let Some(url)=this.selected.with(|s|s.as_ref().and_then(|s|s.repository.clone())){spawn(async move{let _=open_url(url).await;});}})
                    .py(px(6.0)).rounded(px(4.0)).hover(|s|s.bg(theme().muted)).child(caption(move||this.selected.with(|s|s.as_ref().and_then(|s|s.repository.clone()).unwrap_or_default())))))
                .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.limitation.is_some())),move||caption(move||this.selected.with(|s|s.as_ref().and_then(|s|s.limitation.clone()).unwrap_or_default()))))
                .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|!s.installed)),move||{
                    view().gap(px(8.0))
                        .child(action(tr!("Add this source"),move||this.busy.get()||this.selected.with(|s|s.as_ref().is_none_or(|s|!s.can_install)),move||this.install()))
                        .child(each(move||this.selected.with(|s|s.as_ref().map_or_else(Vec::new,|s|s.settings.clone())),|f|f.definition.key.clone(),move|field,_|view().gap(px(4.0))
                            .child(text(move||field.with(|f|f.definition.label.clone())).text_size(px(12.0)).text_color(move||theme().foreground))
                            .child(caption(move||field.with(|f|f.definition.help.clone())))))
                }))
                .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.installed)),move||{
                    view().gap(px(10.0))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().any(|v|v=="account"||v=="signIn"))),move||{
                            view().gap(px(6.0)).pt(px(6.0))
                                .child(text(tr!("Account")).text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).text_color(move||theme().foreground))
                                .child(caption(move||this.account.with(|v| match v {
                                    Some(a) if a.connected => a.name.as_ref().map_or_else(||tr!("Connected").into(), |name| tr!("Connected · {name}"; name = name)),
                                    Some(_) => tr!("Account not connected").into(),
                                    None if this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().all(|v|v!="account"))) => tr!("This source has no account status").into(),
                                    None if this.busy.get() => tr!("Checking account…").into(),
                                    None => tr!("Account status unavailable").into(),
                                })))
                                .child(view().flex_row().gap(px(4.0))
                                    .child(when(move||this.managing.get(),move||action(tr!("Refresh account"),move||this.busy.get(),move||this.refresh_account())))
                                    .child(when(move||this.account.with(|a|a.as_ref().is_none_or(|a|!a.connected)),move||view().flex_row().gap(px(4.0))
                                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.web_sign_in)),move||crate::controls::primary(tr!("Web sign-in"),move||this.busy.get(),move||this.sign_in(SignInMethod::Web))))
                                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.oauth_sign_in)),move||action(tr!("Browser sign-in"),move||this.busy.get(),move||this.sign_in(SignInMethod::Oauth))))
                                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.qr_sign_in)),move||action(tr!("QR sign-in"),move||this.busy.get(),move||this.sign_in(SignInMethod::Qr))))))
                                    .child(when(move||this.managing.get()&&this.account.with(|s|s.as_ref().is_some_and(|a|a.connected)),move||action(tr!("Disconnect account"),move||this.busy.get(),move||this.disconnecting.set(true))))
                                    .child(when(move||this.disconnecting.get(),move||action(tr!("Confirm account disconnection"),move||this.busy.get(),move||{this.disconnecting.set(false);this.sign_out();}))))
                                .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|!s.qr_sign_in&&!s.web_sign_in&&!s.oauth_sign_in)),||caption(tr!("This sign-in method is not supported yet. Use the source’s account settings if available."))))
                        }))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().any(|v|v=="playlist"))),move||{
                            view().gap(px(6.0)).pt(px(6.0))
                                .child(crate::controls::section(tr!("Playlists")))
                                .child(caption(tr!("Imports are saved to My playlists.")))
                                .child(action(tr!("Import source playlists"),move||this.busy.get(),move||this.playlists()))
                        }))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|!s.settings.is_empty())),move||action(tr!("Source settings"),move||this.busy.get(),move||this.configuring.update(|v|*v = !*v))))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().any(|v|v=="liked"))),move||action(tr!("Import cloud likes"),move||this.busy.get(),move||this.import_collection(true))))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().any(|v|v=="recent"))),move||action(tr!("Import cloud history"),move||this.busy.get(),move||this.import_collection(false))))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().any(|v|v=="shows"))),move||action(tr!("Browse podcasts"),move||this.busy.get(),move||{if let Some(source)=this.selected.with(|s|s.as_ref().map(|s|s.id.clone())){open_source(Action::Shows{source});}})))
                        .child(when(move||this.configuring.get(),move||view().gap(px(10.0))
                        .child(each(move||this.selected.with(|s|s.as_ref().map_or_else(Vec::new,|s|s.settings.clone())),|f|f.definition.key.clone(),move|field,_|{
                            view().gap(px(5.0))
                                .child(text(move||field.with(|f|f.definition.label.clone())).text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).text_color(move||theme().foreground))
                                .child(input().label(move||SharedString::from(tr!("Configure {}", field.with(|f|f.definition.label.clone()))))
                                    .value(move||field.with(|f|this.draft.with(|v|v.get(&f.definition.key).cloned().unwrap_or_default())))
                                    .placeholder(move||field.with(|f|if f.secret&&f.configured{tr!("Saved; type to replace").to_owned()}else{f.definition.value.clone().unwrap_or_default()}))
                                    .masked(move||field.with(|f|f.secret)).on_change(move|value:&str|{let key=field.with(|f|f.definition.key.clone());this.draft.update(|v|{v.insert(key.clone(),value.into());});this.dirty.update(|keys|{keys.insert(key);});})
                                    .h(px(34.0)).px(px(8.0)).rounded(px(5.0)).bg(move||theme().muted).text_color(move||theme().foreground))
                                .child(caption(move||field.with(|f|f.definition.help.clone())))
                                .child(when(move||field.with(|f|f.secret&&f.configured),move||action(tr!("Clear this value"),move||this.busy.get(),move||{let key=field.with(|f|f.definition.key.clone());this.draft.update(|v|{v.insert(key.clone(),String::new());});this.dirty.update(|keys|{keys.insert(key);});})))
                        }))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|!s.settings.is_empty())),move||action(tr!("Save settings"),move||this.busy.get()||this.dirty.with(BTreeSet::is_empty),move||this.save())))
                        .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.settings.is_empty())),||caption(tr!("No additional settings for this source"))))
                        ))
                        .child(action(tr!("More options"),move||this.busy.get(),move||{this.managing.update(|v|*v = !*v);this.disconnecting.set(false);} ))
                        .child(when(move||this.managing.get(),move||view().gap(px(5.0))
                            .child(action(tr!("Check for updates"),move||this.busy.get(),move||this.check_update()))
                            .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.repository.is_some())),move||action(tr!("Update source"),move||this.busy.get(),move||this.update())))
                            .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.repository.is_some())),||caption(tr!("Update older plugins manually once to enable automatic updates. Updating replaces plugin files."))))
                            .child(action(tr!("Remove source"),move||this.busy.get(),move||this.removing.set(true)))
                            .child(when(move||this.removing.get(),move||view().gap(px(4.0))
                                .child(caption(tr!("Local playlists will be kept. Add the source again to play its tracks.")))
                                .child(action(tr!("Confirm source removal"),move||this.busy.get(),move||this.uninstall()))))))
                }))
            )
        }))
        .child(when(move||this.page.get()==Page::Login,move||scroll().flex_1().min_h(px(0.0)).child(view().gap(px(12.0)).items_center()
            .child(text(tr!("Connect source account")).text_size(px(18.0)).font_weight(FontWeight::SEMIBOLD).text_color(move||theme().foreground))
            .child(when(move||this.login.with(|j|j.as_ref().is_some_and(|j|j.svg.is_some())),move||image(move||this.login.with(|j|ImageSrc::svg_markup(j.as_ref().and_then(|j|j.svg.as_deref()).unwrap_or_default())))
                .w(px(220.0)).h(px(220.0)).label(tr!("Source sign-in QR code")).on_load(|_:&nsh_core::Size<DevicePixels>|{nsh_core::image::loader::loader().clear();nsh_core::image::budget().clear();})))
            .child(caption(move||this.login.with(|j|j.as_ref().map_or_else(String::new,|j|j.text.clone()))))
            .child(action(tr!("Cancel connection"),move||this.busy.get(),move||this.cancel()))
        )))
        .child(when(move||this.page.get()==Page::Playlists,move||view().flex_1().min_h(px(0.0)).gap(px(8.0))
            .child(crate::controls::heading(tr!("Import source content")))
            .child(caption(tr!("Import supported likes, history, or playlists into My playlists.")))
            .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().any(|v|v=="liked"))),move||action(tr!("Import cloud likes"),move||this.busy.get(),move||this.import_collection(true))))
            .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().any(|v|v=="recent"))),move||action(tr!("Import cloud history"),move||this.busy.get(),move||this.import_collection(false))))
            .child(when(move||!this.busy.get()&&this.playlists.with(Vec::is_empty)&&this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().any(|v|v=="playlists"))),move||action(tr!("View source account and settings"),move||this.busy.get(),move||{this.page.set(Page::Detail);this.refresh_account();})))
            .child(when(move||this.selected.with(|s|s.as_ref().is_some_and(|s|s.capabilities.iter().any(|v|v=="playlist"))),move||view().flex_row().items_center().gap(px(4.0))
                .child(input().label(tr!("Source playlist ID")).placeholder(tr!("Source playlist ID")).value(move||this.playlist_id.get()).on_change(move|v:&str|this.playlist_id.set(v.into())).flex_1().min_w(px(0.0)).h(px(32.0)).px(px(8.0)).rounded(px(5.0)).bg(move||theme().muted).text_color(move||theme().foreground))
                .child(action(tr!("Import"),move||this.busy.get()||this.playlist_id.with(|v|v.trim().is_empty()),move||this.import(this.playlist_id.get())))))
            .child(virtual_list(move||this.playlists.with(Vec::len),move|at|{
                let item=move||this.playlists.with(|rows|rows.get(at.index()).cloned());
                view().flex_row().items_center().gap(px(4.0)).h(px(65.0))
                    .child(crate::controls::artwork(move||item().and_then(|p|p.cover_url),move||tr!("Artwork for {}", item().map_or_else(String::new,|p|p.name)),40.0))
                    .child(view().flex_1().min_w(px(0.0)).gap(px(3.0))
                        .child(text(move||item().map_or_else(String::new,|p|p.name)).text_size(px(13.0)).number_of_lines(1).text_color(move||theme().foreground))
                        .child(caption(move||item().map_or_else(String::new,|p|crate::protocol::i18n::tracks(p.track_count)))))
                    .child(button().accessibility_label(move||Some(SharedString::from(tr!("Import source playlist {}", item().map_or_else(String::new,|p|p.name)))))
                        .disabled(move||this.busy.get()).on_press(move||{if let Some(p)=item(){this.import(p.id);}}).px(px(8.0)).py(px(7.0)).rounded(px(5.0)).hover(|s|s.bg(theme().muted))
                        .child(text(tr!("Import")).text_size(px(11.0)).text_color(move||theme().foreground)))
            }).flex_1().min_h(px(0.0)).estimated_item_size(px(65.0)).label(tr!("Source playlists")))
        ))
        .child(when(move||this.imported.get(),move||crate::controls::primary(tr!("View imported playlists"),move||this.busy.get(),done)))
        .child(when(move||this.busy.get(),||caption(tr!("Loading or saving…"))))
        .child(when(move||this.page.get()!=Page::Catalog,move||caption(move||this.message.get())))
}

#[cfg(test)]
mod login_tests {
    use super::*;

    #[test]
    fn closing_an_unrelated_browser_keeps_the_settings_login() {
        let settings = LoginOwner::new();
        let imports = LoginOwner::new();
        settings.remember("settings-login".into());
        imports.cancel();
        imports.remember("import-login".into());
        imports.cancel();
        assert_eq!(
            LOGIN_JOBS
                .lock()
                .unwrap()
                .0
                .get(&settings.0)
                .map(String::as_str),
            Some("settings-login")
        );
        settings.finish("old-login");
        assert_eq!(
            LOGIN_JOBS
                .lock()
                .unwrap()
                .0
                .get(&settings.0)
                .map(String::as_str),
            Some("settings-login")
        );
        settings.finish("settings-login");
        assert!(!LOGIN_JOBS.lock().unwrap().0.contains_key(&settings.0));
    }

    #[test]
    fn process_shutdown_drains_every_remaining_login() {
        let mut jobs = LoginJobs::default();
        jobs.0.insert(1, "settings-login".into());
        jobs.0.insert(2, "import-login".into());
        assert_eq!(jobs.drain(), vec!["settings-login", "import-login"]);
        assert!(jobs.drain().is_empty());
    }
}
