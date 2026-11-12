//! Shared NSH screen state. Catalog loads never block transport controls.
use crate::protocol::tr;
#[cfg(target_os = "macos")]
use crate::protocol::{FavoriteState, LocalLibraryAction};
use crate::{
    protocol::{AlbumPage, HistoryEntry, Status, Track, TrackInfo},
    remote::{self, Action},
};
use nsh_core::prelude::*;
use tokio::sync::oneshot;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Player,
    Info,
    History,
    Album,
    Discovery,
    Sources,
    Library,
    #[cfg(target_os = "macos")]
    Queue,
    #[cfg(target_os = "macos")]
    Lyrics,
    #[cfg(target_os = "macos")]
    Settings,
}

#[derive(Clone, Copy)]
pub struct Model {
    owner: Owner,
    pub language: Signal<crate::protocol::i18n::Locale>,
    pub status: Signal<Status>,
    #[cfg(target_os = "macos")]
    pub status_ready: Signal<bool>,
    pub page: Signal<Page>,
    #[cfg(target_os = "macos")]
    pub drawer_open: Signal<bool>,
    #[cfg(target_os = "macos")]
    pub drawer_entered: Signal<bool>,
    #[cfg(target_os = "macos")]
    pub sampled_at: Signal<std::time::Instant>,
    #[cfg(target_os = "macos")]
    pub details_left: Signal<bool>,
    #[cfg(target_os = "macos")]
    details_request: Signal<u64>,
    #[cfg(target_os = "macos")]
    pub lyric_clock: Signal<u8>,
    #[cfg(target_os = "macos")]
    pub show_mini_lyrics: Signal<bool>,
    jobs: Signal<[bool; 4]>,
    pub starting: Signal<bool>,
    pub error: Signal<String>,
    pub info: Signal<Option<TrackInfo>>,
    pub album: Signal<Option<AlbumPage>>,
    pub discovery: Signal<Option<crate::protocol::MusicSourceAction>>,
    pub discovery_back: Signal<Page>,
    pub history: Signal<Vec<(usize, HistoryEntry)>>,
    #[cfg(target_os = "macos")]
    pub queue: Signal<Option<Status>>,
    #[cfg(target_os = "macos")]
    pub quality_busy: Signal<bool>,
    #[cfg(target_os = "macos")]
    pub favorite: Signal<FavoriteState>,
    #[cfg(target_os = "macos")]
    pub favorite_busy: Signal<bool>,
    #[cfg(target_os = "macos")]
    favorite_generation: Signal<u64>,
    pub scrub: Signal<Option<f64>>,
    pub cover_failed: Signal<bool>,
}

enum Loaded {
    Info(Box<TrackInfo>),
    Album(AlbumPage),
    History(Vec<HistoryEntry>),
    #[cfg(target_os = "macos")]
    Queue(Box<Status>),
}

pub fn key(track: Option<&Track>) -> String {
    track.map_or_else(String::new, crate::protocol::track_key)
}

impl Model {
    pub fn new() -> Self {
        Self {
            owner: Owner::current(),
            language: signal(crate::protocol::i18n::locale()),
            status: signal(Status::default()),
            #[cfg(target_os = "macos")]
            status_ready: signal(false),
            page: signal(Page::Player),
            #[cfg(target_os = "macos")]
            drawer_open: signal(false),
            #[cfg(target_os = "macos")]
            drawer_entered: signal(false),
            #[cfg(target_os = "macos")]
            sampled_at: signal(std::time::Instant::now()),
            #[cfg(target_os = "macos")]
            details_left: signal(false),
            #[cfg(target_os = "macos")]
            details_request: signal(0),
            #[cfg(target_os = "macos")]
            lyric_clock: signal(0),
            #[cfg(target_os = "macos")]
            show_mini_lyrics: signal(
                crate::protocol::data_dir()
                    .and_then(|d| std::fs::read_to_string(d.join("panel-preferences.json")).ok())
                    .and_then(|s| serde_json::from_str::<bool>(&s).ok())
                    .unwrap_or(true),
            ),
            jobs: signal([false; 4]),
            starting: signal(false),
            error: signal(String::new()),
            info: signal(None),
            album: signal(None),
            discovery: signal(None),
            discovery_back: signal(Page::Player),
            history: signal(Vec::new()),
            #[cfg(target_os = "macos")]
            queue: signal(None),
            #[cfg(target_os = "macos")]
            quality_busy: signal(false),
            #[cfg(target_os = "macos")]
            favorite: signal(FavoriteState::default()),
            #[cfg(target_os = "macos")]
            favorite_busy: signal(false),
            #[cfg(target_os = "macos")]
            favorite_generation: signal(0),
            scrub: signal(None),
            cover_failed: signal(false),
        }
    }
    pub fn refresh_language(self) {
        crate::protocol::i18n::refresh();
        let language = crate::protocol::i18n::locale();
        if self.language.get() != language {
            self.error.set(String::new());
            self.language.set(language);
            #[cfg(target_os = "macos")]
            crate::desktop::refresh_language();
        }
    }

    pub fn open(self, page: Page) {
        #[cfg(target_os = "macos")]
        {
            let opening = page != Page::Player && !self.drawer_open.get();
            self.drawer_open.set(page != Page::Player);
            if opening {
                let request = self.details_request.get().wrapping_add(1);
                self.details_request.set(request);
                let direction = crate::desktop::request_details_left();
                self.owner.run(|| {
                    spawn(async move {
                        if let Ok(left) = direction.await
                            && self.drawer_open.get()
                            && self.details_request.get() == request
                        {
                            self.details_left.set(left);
                            crate::desktop::set_details_expanded(true, left);
                        }
                    });
                });
            }
            if page == Page::Player {
                // Keep content alive until the sheet finishes sliding out.
                // A canceled entrance never reaches Sheet::Open, so its
                // lifecycle also emits no Close. Collapse that host directly.
                if !self.drawer_entered.get() {
                    self.drawer_closed();
                }
                return;
            }
        }
        self.page.set(page);
        self.error.set(String::new());
        #[cfg(target_os = "ios")]
        self.info.set(None);
        self.album.set(None);
        self.history.set(Vec::new());
        #[cfg(target_os = "macos")]
        self.queue.set(None);
        if page == Page::Player {
            return;
        }
        self.load();
    }
    #[cfg(target_os = "macos")]
    pub fn drawer_closed(self) {
        if !self.drawer_open.get() {
            self.drawer_entered.set(false);
            crate::desktop::set_details_expanded(false, self.details_left.get());
            self.page.set(Page::Player);
            self.album.set(None);
            self.history.set(Vec::new());
            self.queue.set(None);
            self.error.set(String::new());
        }
    }
    pub fn load(self) {
        let page = self.page.get();
        #[cfg(target_os = "macos")]
        if matches!(page, Page::Info | Page::Lyrics) {
            self.load_info();
            return;
        }
        let Some(slot) = Self::slot(page) else {
            return;
        };
        if self.jobs.with(|jobs| jobs[slot]) {
            return;
        }
        self.jobs.update(|jobs| jobs[slot] = true);
        let requested_key = self.status.with(|s| key(s.track.as_ref()));
        let (send, receive) = oneshot::channel();
        std::thread::spawn(move || {
            let result = match page {
                Page::Info => {
                    remote::read_json("/ui/info").map(|info| Loaded::Info(Box::new(info)))
                }
                Page::Album => remote::read_json("/ui/album").map(Loaded::Album),
                Page::History => remote::read_json("/ui/history").map(Loaded::History),
                #[cfg(target_os = "macos")]
                Page::Queue => remote::read_json("/ui/queue").map(|q| Loaded::Queue(Box::new(q))),
                #[cfg(target_os = "macos")]
                Page::Lyrics | Page::Settings => unreachable!(),
                Page::Player | Page::Sources | Page::Library | Page::Discovery => unreachable!(),
            };
            let _ = send.send(result);
        });
        self.owner.run(|| {
            spawn(async move {
                let result = receive.await;
                self.jobs.update(|jobs| jobs[slot] = false);
                let still_current = self.page.get() == page
                    && (matches!(Self::slot(page), Some(1 | 3))
                        || self.status.with(|s| key(s.track.as_ref())) == requested_key);
                if !still_current {
                    if self.page.get() == page {
                        self.load();
                    }
                    return;
                }
                match result {
                    Ok(Ok(Loaded::Info(info))) => self.info.set(Some(*info)),
                    Ok(Ok(Loaded::Album(album))) => self.album.set(Some(album)),
                    Ok(Ok(Loaded::History(rows))) => {
                        self.history.set(rows.into_iter().enumerate().collect())
                    }
                    #[cfg(target_os = "macos")]
                    Ok(Ok(Loaded::Queue(snapshot))) => {
                        if snapshot.queue_revision == self.status.with(|s| s.queue_revision) {
                            self.queue.set(Some(*snapshot));
                        } else {
                            self.load();
                        }
                    }
                    Ok(Err(error)) => self.error.set(error.to_string()),
                    Err(_) => self
                        .error
                        .set(tr!("Loading ended; please try again").into()),
                }
            })
        });
    }
    fn slot(page: Page) -> Option<usize> {
        match page {
            Page::Player | Page::Sources | Page::Library | Page::Discovery => None,
            Page::Info => Some(0),
            #[cfg(target_os = "macos")]
            Page::Lyrics => Some(0),
            #[cfg(target_os = "macos")]
            Page::Settings => None,
            Page::History => Some(1),
            Page::Album => Some(2),
            #[cfg(target_os = "macos")]
            Page::Queue => Some(3),
        }
    }
    pub fn is_loading(self) -> bool {
        Self::slot(self.page.get()).is_some_and(|slot| self.jobs.with(|jobs| jobs[slot]))
    }
    pub fn update(self, status: Status) {
        #[cfg(target_os = "macos")]
        let library_changed = self.status.with(|s| s.library_revision) != status.library_revision;
        #[cfg(target_os = "macos")]
        let queue_changed = self.status.with(|s| s.queue_revision) != status.queue_revision;
        let changed = self.status.with(|s| key(s.track.as_ref())) != key(status.track.as_ref());
        if changed {
            #[cfg(target_os = "macos")]
            self.info.set(None);
            self.scrub.set(None);
            self.cover_failed.set(false);
        }
        self.status.set(status);
        #[cfg(target_os = "macos")]
        self.status_ready.set(true);
        #[cfg(target_os = "macos")]
        if changed || library_changed {
            if changed {
                self.favorite.set(FavoriteState::default());
            }
            self.refresh_favorite();
        }
        #[cfg(target_os = "macos")]
        self.sampled_at.set(std::time::Instant::now());
        #[cfg(target_os = "macos")]
        if changed {
            self.load_info();
        }
        #[cfg(target_os = "macos")]
        if queue_changed && self.page.get() == Page::Queue && self.drawer_open.get() {
            self.load();
        }
        let showing_catalog = matches!(self.page.get(), Page::Info | Page::Album);
        #[cfg(target_os = "macos")]
        let showing_catalog = showing_catalog && self.drawer_open.get();
        if changed && showing_catalog {
            self.open(self.page.get());
        }
    }
    #[cfg(target_os = "macos")]
    pub fn refresh_favorite(self) {
        let requested = self.status.with(|s| key(s.track.as_ref()));
        if requested.is_empty() {
            self.favorite.set(FavoriteState::default());
            return;
        }
        self.favorite_generation.update(|v| *v = v.wrapping_add(1));
        let generation = self.favorite_generation.get();
        let (send, receive) = oneshot::channel();
        let requested_key = requested.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<FavoriteState> {
                let mut url = reqwest::Url::parse("http://localhost/ui/favorite")?;
                url.query_pairs_mut().append_pair("key", &requested_key);
                remote::read_json(&format!(
                    "{}?{}",
                    url.path(),
                    url.query().unwrap_or_default()
                ))
            })();
            let _ = send.send(result);
        });
        self.owner.run(|| {
            spawn(async move {
                if let Ok(Ok(value)) = receive.await
                    && self.favorite_generation.get() == generation
                    && self.status.with(|s| key(s.track.as_ref())) == requested
                {
                    self.favorite.set(value);
                }
            })
        });
    }
    #[cfg(target_os = "macos")]
    pub fn toggle_favorite(self) {
        if self.favorite_busy.get() {
            return;
        }
        let Some(track) = self.status.with(|s| s.track.clone()) else {
            return;
        };
        let requested = key(Some(&track));
        let liked = !self.favorite.with(|f| f.key == requested && f.liked);
        self.favorite_generation.update(|v| *v = v.wrapping_add(1));
        self.favorite_busy.set(true);
        let (send, receive) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = send.send(remote::post_json::<_, FavoriteState>(
                "/ui/local-playlists",
                &LocalLibraryAction::SetFavorite {
                    track: Box::new(track),
                    liked,
                },
            ));
        });
        self.owner.run(|| {
            spawn(async move {
                let result = receive.await;
                self.favorite_busy.set(false);
                if self.status.with(|s| key(s.track.as_ref())) != requested {
                    return;
                }
                match result {
                    Ok(Ok(value)) => self.favorite.set(value),
                    Ok(Err(e)) => self.error.set(tr!("Like not saved: {e}"; e = e)),
                    Err(_) => self
                        .error
                        .set(tr!("Like request ended; please try again").into()),
                }
            })
        });
    }
    #[cfg(target_os = "macos")]
    pub fn load_info(self) {
        if self.jobs.with(|j| j[0]) || self.status.with(|s| s.track.is_none()) {
            return;
        }
        self.jobs.update(|j| j[0] = true);
        let requested = self.status.with(|s| key(s.track.as_ref()));
        let (send, receive) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = send.send(remote::read_json::<TrackInfo>("/ui/info"));
        });
        self.owner.run(|| {
            spawn(async move {
                self.jobs.update(|j| j[0] = true);
                let result = receive.await;
                self.jobs.update(|j| j[0] = false);
                if self.status.with(|s| key(s.track.as_ref())) != requested {
                    self.load_info();
                    return;
                }
                match result {
                    Ok(Ok(info)) => self.info.set(Some(info)),
                    Ok(Err(e)) => self.error.set(e.to_string()),
                    Err(_) => self
                        .error
                        .set(tr!("Lyrics request ended; please try again").into()),
                }
            })
        });
    }
    #[cfg(target_os = "macos")]
    pub fn toggle_mini_lyrics(self) {
        self.show_mini_lyrics.update(|value| *value = !*value);
        let value = self.show_mini_lyrics.get();
        static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static WRITER: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        std::thread::spawn(move || {
            use std::{io::Write, os::unix::fs::OpenOptionsExt};
            let Ok(_guard) = WRITER.lock() else {
                return;
            };
            if GENERATION.load(std::sync::atomic::Ordering::SeqCst) != generation {
                return;
            }
            if let Some(dir) = crate::protocol::data_dir() {
                let temp = dir.join(format!(".panel-preferences.{}", std::process::id()));
                let result = (|| -> std::io::Result<()> {
                    let mut f = std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .mode(0o600)
                        .open(&temp)?;
                    write!(f, "{value}")?;
                    std::fs::rename(&temp, dir.join("panel-preferences.json"))
                })();
                if result.is_err() {
                    let _ = std::fs::remove_file(temp);
                }
            }
        });
    }
    pub fn play_local(self, id: String, index: usize) {
        self.play_action(Action::PlayLocalPlaylist { id, index });
    }
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn play(self, track: Track) {
        self.play_action(Action::PlayTrack(track));
    }
    pub fn open_source(self, action: crate::protocol::MusicSourceAction) {
        if self.page.get() != Page::Discovery {
            self.discovery_back.set(self.page.get());
        }
        self.discovery.set(Some(action));
        self.open(Page::Discovery);
    }
    pub fn play_source(self, page: crate::protocol::SourcePage, source: String, index: usize) {
        self.play_action(Action::PlayList {
            tracks: page.tracks,
            index,
            context: crate::protocol::QueueContext::SourceCollection {
                source,
                id: page.id,
                name: page.title,
            },
        });
    }
    #[cfg(target_os = "macos")]
    pub fn play_browsed(self, index: usize) {
        use crate::protocol::QueueContext;
        let selection = match self.page.get() {
            Page::History => Some((
                self.history
                    .with(|rows| rows.iter().map(|r| r.1.track.clone()).collect()),
                QueueContext::History,
            )),
            Page::Album => self.album.with(|album| {
                album.as_ref().map(|a| {
                    (
                        a.tracks.clone(),
                        QueueContext::Album {
                            id: a.album.id.clone(),
                            name: a.album.name.clone(),
                        },
                    )
                })
            }),
            _ => None,
        };
        if let Some((tracks, context)) = selection {
            self.play_action(Action::PlayList {
                tracks,
                index,
                context,
            });
        }
    }
    #[cfg(target_os = "macos")]
    pub fn play_queue(self, index: usize) {
        if let Some(revision) = self.queue.with(|q| q.as_ref().map(|q| q.queue_revision)) {
            self.play_action(Action::PlayQueue { index, revision });
        }
    }
    #[cfg(target_os = "macos")]
    pub fn set_quality(self, quality: crate::protocol::Quality) {
        if self.quality_busy.get() || self.starting.get() || self.status.with(|s| s.switching) {
            return;
        }
        self.quality_busy.set(true);
        self.error.set(String::new());
        let (send, receive) = oneshot::channel();
        remote::submit(Action::Quality(quality), Some(send));
        self.owner.run(|| {
            spawn(async move {
                match receive.await {
                    Ok(Ok(status)) => self.update(status),
                    Ok(Err(error)) => self.error.set(error.to_string()),
                    Err(_) => self
                        .error
                        .set(tr!("Quality request ended; please try again").into()),
                }
                self.quality_busy.set(false);
            })
        });
    }
    fn play_action(self, action: Action) {
        if self.starting.get() {
            return;
        }
        self.starting.set(true);
        self.error.set(String::new());
        let (send, receive) = oneshot::channel();
        remote::submit(action, Some(send));
        self.owner.run(|| {
            spawn(async move {
                match receive.await {
                    Ok(Ok(status)) => {
                        #[cfg(target_os = "macos")]
                        self.open(Page::Player);
                        self.update(status);
                        self.open(Page::Player);
                    }
                    Ok(Err(error)) => self.error.set(error.to_string()),
                    Err(_) => self.error.set(tr!("Playback request ended").into()),
                }
                self.starting.set(false);
            })
        });
    }
}

pub fn plain_lyrics(text: &str) -> String {
    text.lines()
        .filter_map(|line| {
            let mut line = line.trim();
            while let Some(end) = line
                .strip_prefix('[')
                .and_then(|v| v.find(']').map(|i| i + 1))
            {
                let tag = &line[1..end];
                if !tag.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    return None;
                }
                line = line[end + 1..].trim();
            }
            (!line.is_empty()).then_some(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn ago(timestamp: u64) -> String {
    let now = (nsh_core::time::unix_ms() / 1000.0) as u64;
    let seconds = now.saturating_sub(timestamp);
    crate::protocol::i18n::relative_time(seconds)
}
