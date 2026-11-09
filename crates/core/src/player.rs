use anyhow::{Result, anyhow, ensure};
use objc2::{
    MainThreadMarker,
    rc::{Retained, autoreleasepool},
    runtime::AnyObject,
};
use objc2_av_foundation::{
    AVPlayer, AVPlayerItem, AVPlayerTimeControlStatus, AVURLAsset, AVURLAssetHTTPUserAgentKey,
};
use objc2_core_media::CMTime;
use objc2_foundation::{NSDictionary, NSString, NSURL};
use std::{
    cell::RefCell,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

use crate::{
    history,
    netease::{self, Media, Track},
    protocol::HistoryProgress,
};

pub use crate::protocol::{State, Status};

pub enum Action {
    Status,
    DisplayStatus,
    MergeTrackMetadata(Track),
    Play(Media),
    Enqueue(Media),
    ReplaceQueue {
        tracks: Vec<Track>,
        index: usize,
        context: crate::protocol::QueueContext,
        media: Media,
    },
    Select {
        index: usize,
        revision: u64,
    },
    Restore {
        snapshot: Box<Status>,
        media: Option<Media>,
    },
    Pause,
    Resume,
    Toggle,
    Previous,
    Next,
    Seek {
        seconds: f64,
        track_key: Option<String>,
    },
    Volume(f32),
    Quality(crate::protocol::Quality),
}

pub type Reply = Option<oneshot::Sender<Result<Status>>>;

const AUTO_ADVANCE_MAX_ATTEMPTS: usize = 8;
const AUTO_ADVANCE_RETRY_WINDOW: Duration = Duration::from_secs(20);

struct AutoAdvance {
    index: usize,
    attempt: usize,
    seek_serial: u64,
    retry_until: Instant,
}

enum Message {
    Action(Action, Reply),
    Switched {
        index: usize,
        media: Result<Media>,
        reply: Reply,
    },
    AutoAdvanceReady {
        pending: AutoAdvance,
        media: Result<Media>,
    },
    QualityReady {
        quality: crate::protocol::Quality,
        media: Result<Media>,
        reply: Reply,
    },
    ResumeReady {
        media: Result<Media>,
        reply: Reply,
    },
    Seeked {
        serial: u64,
        finished: bool,
    },
}

thread_local! { static PLAYER: RefCell<Player> = RefCell::new(Player::new()); }

pub fn submit(action: Action, reply: Reply) {
    dispatch(Message::Action(action, reply));
}

fn dispatch(message: Message) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        autoreleasepool(|_| PLAYER.with_borrow_mut(|player| player.handle(message)));
    });
}

/// Maintains history and queue progression even when no UI exists.
pub fn tick() -> Status {
    PLAYER.with_borrow_mut(|player| {
        player.handle(Message::Action(Action::Status, None));
        let mut status = player.status();
        player.capture_history(&status, false);
        status.queue = Arc::default();
        status
    })
}

pub fn shutdown() {
    PLAYER.with_borrow_mut(|player| {
        player.flush_history();
        player.audio.take();
    });
}

struct Player {
    audio: Option<Retained<AVPlayer>>,
    volume: f32,
    main_thread: MainThreadMarker,
    queue: Arc<Vec<Track>>,
    queue_revision: u64,
    queue_context: Option<crate::protocol::QueueContext>,
    audio_quality: Option<crate::protocol::AudioQuality>,
    seek_serial: u64,
    pending_seek: Option<(u64, f64)>,
    deferred_position: Option<f64>,
    index: Option<usize>,
    error: Option<String>,
    switching: bool,
    end_handled: bool,
    requested_play: bool,
    history_recorded: bool,
}

impl Player {
    pub fn new() -> Self {
        let main_thread = MainThreadMarker::new().expect("AppKit main thread");
        Self {
            audio: None,
            volume: 1.0,
            main_thread,
            queue: Arc::default(),
            queue_revision: 0,
            queue_context: None,
            audio_quality: None,
            seek_serial: 0,
            pending_seek: None,
            deferred_position: None,
            index: None,
            error: None,
            switching: false,
            end_handled: false,
            requested_play: false,
            history_recorded: false,
        }
    }

    fn audio(&self) -> &AVPlayer {
        self.audio
            .as_deref()
            .expect("a loaded track owns its AVPlayer")
    }

    pub fn status(&self) -> Status {
        if self.audio.is_none() {
            let track = self.index.and_then(|i| self.queue.get(i)).cloned();
            return Status {
                state: if track.is_none() {
                    State::Idle
                } else if self.switching && self.requested_play {
                    State::Loading
                } else {
                    State::Paused
                },
                duration: track.as_ref().map_or(0.0, |t| t.duration),
                track,
                index: self.index,
                position: self.deferred_position.unwrap_or(0.0),
                switching: self.switching,
                volume: self.volume,
                error: self.error.clone(),
                queue: self.queue.clone(),
                queue_length: self.queue.len(),
                queue_revision: self.queue_revision,
                library_revision: crate::local_library::revision(),
                queue_context: self.queue_context.clone(),
                preferred_quality: crate::audio_quality::preferred(),
                ..Status::default()
            };
        }
        let track = self.index.map(|index| self.queue[index].clone());
        // All AVFoundation access stays on the application main thread.
        let (position, duration, control, volume, error) = unsafe {
            let item = self.audio().currentItem();
            let duration = item.as_ref().map(|item| item.duration().seconds());
            let error = item
                .and_then(|item| item.error())
                .map(|err| err.localizedDescription().to_string());
            (
                self.audio().currentTime().seconds(),
                duration,
                self.audio().timeControlStatus(),
                self.audio().volume(),
                error,
            )
        };
        let duration = duration
            .filter(|n| n.is_finite() && *n > 0.0)
            .or_else(|| track.as_ref().map(|track| track.duration))
            .unwrap_or_default();
        let position = if let Some((_, target)) = self.pending_seek {
            target
        } else if position.is_finite() {
            position.max(0.0)
        } else {
            0.0
        };
        let state = if error.is_some() {
            State::Error
        } else if track.is_none() {
            State::Idle
        } else if control == AVPlayerTimeControlStatus::Playing {
            State::Playing
        } else if self.pending_seek.is_none()
            && self.requested_play
            && duration > 0.0
            && position >= duration - 0.12
        {
            State::Ended
        } else if control == AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate {
            State::Loading
        } else {
            State::Paused
        };
        Status {
            state,
            track,
            position,
            duration,
            volume,
            index: self.index,
            queue: self.queue.clone(),
            queue_length: self.queue.len(),
            queue_revision: self.queue_revision,
            library_revision: crate::local_library::revision(),
            queue_context: self.queue_context.clone(),
            preferred_quality: crate::audio_quality::preferred(),
            audio_quality: self.audio_quality.clone(),
            error: error.or_else(|| self.error.clone()),
            switching: self.switching,
        }
    }

    fn load(&mut self, media: &Media) -> Result<()> {
        self.load_with_play(media, true)
    }
    fn load_with_play(&mut self, media: &Media, play: bool) -> Result<()> {
        #[cfg(target_os = "ios")]
        crate::mobile::activate_audio_session()?;
        let url = NSURL::URLWithString(&NSString::from_str(media.url.as_str()))
            .ok_or_else(|| anyhow!("无效的音频地址"))?;
        self.flush_history();
        self.seek_serial += 1;
        self.pending_seek = None;
        if self.audio.is_none() {
            // AVPlayer and all of its accesses stay on AppKit's main thread.
            let audio = unsafe { AVPlayer::new(self.main_thread) };
            unsafe {
                audio.setVolume(self.volume);
            }
            self.audio = Some(audio);
        }
        unsafe {
            let item = if let Some(agent) = &media.user_agent {
                let value = NSString::from_str(agent);
                let object: &AnyObject = &value;
                let options = NSDictionary::from_slices(&[AVURLAssetHTTPUserAgentKey], &[object]);
                let asset = AVURLAsset::URLAssetWithURL_options(&url, Some(&options));
                AVPlayerItem::playerItemWithAsset(&asset, self.main_thread)
            } else {
                AVPlayerItem::playerItemWithURL(&url, self.main_thread)
            };
            self.audio().replaceCurrentItemWithPlayerItem(Some(&item));
            if play {
                self.audio().play();
            } else {
                self.audio().pause();
            }
        }
        self.audio_quality = Some(media.quality.clone());
        self.deferred_position = None;
        self.end_handled = false;
        self.history_recorded = false;
        self.requested_play = true;
        Ok(())
    }

    fn seek(&mut self, seconds: f64) {
        self.seek_serial += 1;
        let serial = self.seek_serial;
        self.pending_seek = Some((serial, seconds));
        // AVPlayer exposes 0 until an asynchronous seek completes. Publish the
        // requested position until its actual completion, never a guessed delay.
        let completion = block2::RcBlock::new(move |finished: objc2::runtime::Bool| {
            dispatch(Message::Seeked {
                serial,
                finished: finished.as_bool(),
            })
        });
        unsafe {
            self.audio()
                .seekToTime_toleranceBefore_toleranceAfter_completionHandler(
                    CMTime::with_seconds(seconds, 1000),
                    CMTime::with_seconds(0.0, 1000),
                    CMTime::with_seconds(0.0, 1000),
                    &completion,
                );
        }
    }

    fn apply(&mut self, action: Action) -> Result<()> {
        if !matches!(action, Action::Status | Action::MergeTrackMetadata(_)) {
            self.error = None;
        }
        match action {
            Action::Status | Action::DisplayStatus => {}
            Action::MergeTrackMetadata(metadata) => {
                if let Some(index) = self.index {
                    let mut track = self.queue[index].clone();
                    if crate::mygo::merge_metadata(&mut track, &metadata) {
                        Arc::make_mut(&mut self.queue)[index] = track;
                        self.queue_revision += 1;
                    }
                }
            }
            Action::Restore { snapshot, media } => {
                ensure!(
                    self.queue.is_empty() && self.audio.is_none(),
                    "只能向空播放器恢复会话"
                );
                crate::audio_quality::set_preferred(snapshot.preferred_quality)?;
                self.volume = 0.0;
                if let Some(media) = media {
                    self.load(&media)?;
                    unsafe {
                        self.audio().pause();
                    }
                }
                self.queue = snapshot.queue;
                self.queue_revision = snapshot.queue_revision;
                self.queue_context = snapshot.queue_context;
                self.index = snapshot.index;
                if self.audio.is_none() && self.index.is_some() {
                    self.deferred_position = Some(snapshot.position);
                }
                self.volume = snapshot.volume;
                self.history_recorded = self.index.is_some_and(|index| {
                    history::restore(&self.queue[index]).unwrap_or_else(|error| {
                        eprintln!("恢复播放历史失败：{error:#}");
                        false
                    })
                });
                self.end_handled = snapshot.state == State::Ended;
                self.requested_play = matches!(
                    snapshot.state,
                    State::Playing | State::Loading | State::Ended
                );
                if self.audio.is_some() {
                    self.seek(snapshot.position);
                    unsafe {
                        self.audio().setVolume(self.volume);
                        if matches!(snapshot.state, State::Playing | State::Loading) {
                            self.audio().play();
                        }
                    }
                }
            }
            Action::Play(media) => {
                ensure!(!self.switching, "正在切换曲目");
                self.load(&media)?;
                let target = self.index.map_or(0, |index| index + 1);
                Arc::make_mut(&mut self.queue).insert(target, media.track);
                self.index = Some(target);
                self.queue_revision += 1;
            }
            Action::Enqueue(media) => {
                if self.index.is_none() {
                    self.load(&media)?;
                    self.index = Some(0);
                }
                Arc::make_mut(&mut self.queue).push(media.track);
                self.queue_revision += 1;
            }
            Action::ReplaceQueue {
                mut tracks,
                index,
                context,
                media,
            } => {
                ensure!(!self.switching, "正在切换曲目");
                ensure!(
                    index < tracks.len() && tracks.len() <= 10000,
                    "无效的播放列表"
                );
                self.load(&media)?;
                tracks[index] = media.track;
                self.queue = Arc::new(tracks);
                self.queue_context = Some(context);
                self.queue_revision += 1;
                self.index = Some(index);
            }
            Action::Toggle => {
                let playing = matches!(self.status().state, State::Playing | State::Loading);
                return self.apply(if playing {
                    Action::Pause
                } else {
                    Action::Resume
                });
            }
            Action::Pause => {
                ensure!(self.index.is_some(), "请先选一首歌");
                self.flush_history();
                self.requested_play = false;
                if let Some(audio) = &self.audio {
                    unsafe {
                        audio.pause();
                    }
                }
            }
            Action::Resume => {
                ensure!(self.index.is_some(), "请先选一首歌");
                #[cfg(target_os = "ios")]
                crate::mobile::activate_audio_session()?;
                let status = self.status();
                if status.duration > 0.0 && status.position >= status.duration - 0.12 {
                    self.seek(0.0);
                    self.end_handled = false;
                }
                self.requested_play = true;
                unsafe {
                    self.audio().play();
                }
            }
            Action::Seek { seconds, track_key } => {
                if let Some(expected) = track_key {
                    ensure!(
                        !self.switching
                            && self.index.is_some_and(|i| {
                                crate::protocol::track_key(&self.queue[i]) == expected
                            }),
                        "歌曲已切换，请选择当前歌词"
                    );
                }
                let status = self.status();
                ensure!(
                    status.duration > 0.0 && seconds.is_finite() && seconds >= 0.0,
                    "无效的播放进度"
                );
                let position = seconds.min(status.duration);
                if position < status.duration - 0.12 {
                    self.end_handled = false;
                }
                if self.deferred_position.is_some() {
                    self.deferred_position = Some(position);
                } else {
                    self.seek(position);
                }
            }
            Action::Volume(value) => {
                ensure!((0.0..=1.0).contains(&value), "音量必须在 0 到 1 之间");
                self.volume = value;
                if let Some(audio) = &self.audio {
                    unsafe {
                        audio.setVolume(value);
                    }
                }
            }
            Action::Previous | Action::Next | Action::Select { .. } | Action::Quality(_) => {
                unreachable!("switching resolves outside the UI thread")
            }
        }
        Ok(())
    }

    fn respond(&mut self, result: Result<()>, reply: Reply) {
        if let Err(error) = &result {
            self.error = Some(error.to_string());
        }
        if let Some(reply) = reply {
            let _ = reply.send(result.map(|()| self.status()));
        }
    }

    fn flush_history(&mut self) {
        if self.index.is_some() {
            let status = self.status();
            self.capture_history(&status, true);
        }
    }

    fn capture_history(&mut self, status: &Status, flush: bool) {
        let Some(track) = &status.track else { return };
        if self.pending_seek.is_some()
            || !matches!(status.state, State::Playing | State::Paused | State::Ended)
        {
            return;
        }
        let completed = status.state == State::Ended || self.end_handled;
        let duration = if status.duration.is_finite() {
            status.duration.max(0.0)
        } else {
            0.0
        };
        let position = if duration > 0.0 {
            status.position.clamp(0.0, duration)
        } else {
            status.position.max(0.0)
        };
        let progress = HistoryProgress {
            position: if completed && duration > 0.0 {
                duration
            } else {
                position
            },
            duration,
            completed,
        };
        let result = if self.history_recorded {
            history::update(progress, flush)
        } else if status.state == State::Playing && position > 0.0 {
            self.history_recorded = true;
            history::record(track.clone(), progress)
        } else {
            return;
        };
        if let Err(error) = result {
            eprintln!("保存播放历史失败：{error:#}");
        }
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Action(action @ (Action::Resume | Action::Toggle), reply)
                if self.deferred_position.is_some() =>
            {
                if matches!(action, Action::Toggle) && self.requested_play {
                    let result = self.apply(Action::Pause);
                    self.respond(result, reply);
                } else if self.switching {
                    self.respond(Err(anyhow!("正在取流，请稍候")), reply);
                } else if let Some(track) = self.index.and_then(|i| self.queue.get(i)).cloned() {
                    self.switching = true;
                    self.requested_play = true;
                    self.error = None;
                    thread::spawn(move || {
                        dispatch(Message::ResumeReady {
                            media: crate::catalog::resolve(track),
                            reply,
                        })
                    });
                } else {
                    self.respond(Err(anyhow!("请先选一首歌")), reply);
                }
            }
            Message::Seeked { serial, finished } => {
                if self
                    .pending_seek
                    .is_some_and(|(current, _)| current == serial)
                {
                    self.pending_seek = None;
                    if finished {
                        self.flush_history();
                    }
                }
            }
            Message::Action(Action::Quality(quality), reply) => {
                if self.switching {
                    self.respond(Err(anyhow!("正在切换，请稍后再选音质")), reply);
                } else if self.deferred_position.is_some() {
                    self.respond(crate::audio_quality::set_preferred(quality), reply);
                } else if let Some(track) = self.index.and_then(|i| self.queue.get(i)).cloned() {
                    self.switching = true;
                    self.error = None;
                    thread::spawn(move || {
                        dispatch(Message::QualityReady {
                            quality,
                            media: crate::catalog::resolve_with_quality(track, quality),
                            reply,
                        })
                    });
                } else {
                    self.respond(crate::audio_quality::set_preferred(quality), reply);
                }
            }
            Message::Action(
                action @ (Action::Previous | Action::Next | Action::Select { .. }),
                reply,
            ) => {
                let target = match action {
                    Action::Select { index, revision } if revision == self.queue_revision => {
                        Some(index)
                    }
                    Action::Select { .. } => {
                        self.respond(Err(anyhow!("播放列表已变化，请重新选择")), reply);
                        return;
                    }
                    Action::Next => self.index.and_then(|i| i.checked_add(1)),
                    _ => self.index.and_then(|i| i.checked_sub(1)),
                };
                if self.switching {
                    self.respond(Err(anyhow!("正在切换曲目")), reply);
                } else if let Some((index, track)) =
                    target.and_then(|i| self.queue.get(i).map(|track| (i, track)))
                {
                    self.switching = true;
                    self.error = None;
                    let track = track.clone();
                    thread::spawn(move || {
                        dispatch(Message::Switched {
                            index,
                            media: if track.source.is_some() {
                                crate::mygo::resolve(track)
                            } else {
                                netease::resolve(track.id)
                            },
                            reply,
                        });
                    });
                } else {
                    self.respond(Err(anyhow!("没有可切换的曲目")), reply);
                }
            }
            Message::Action(Action::DisplayStatus, reply) => {
                self.advance_on_end();
                if let Some(reply) = reply {
                    let mut display = self.status();
                    display.queue = Arc::default();
                    let _ = reply.send(Ok(display));
                }
            }
            Message::Action(action, reply) => {
                if matches!(action, Action::Status) {
                    self.advance_on_end();
                }
                let result = self.apply(action);
                self.respond(result, reply);
            }
            Message::Switched {
                index,
                media,
                reply,
            } => {
                self.switching = false;
                let result = media.and_then(|media| self.load(&media));
                if result.is_ok() {
                    self.index = Some(index);
                }
                self.respond(result, reply);
            }
            Message::AutoAdvanceReady { pending, media } => {
                self.switching = false;
                // A pause or a new seek takes priority over the previous
                // track's end while its successor is being resolved.
                if !self.requested_play || self.seek_serial != pending.seek_serial {
                    return;
                }
                match media.and_then(|media| self.load(&media)) {
                    Ok(()) => {
                        self.index = Some(pending.index);
                        self.error = None;
                    }
                    Err(error) => {
                        let index = pending.index + 1;
                        if index < self.queue.len()
                            && pending.attempt < AUTO_ADVANCE_MAX_ATTEMPTS
                            && Instant::now() < pending.retry_until
                        {
                            self.resolve_automatic_next(AutoAdvance {
                                index,
                                attempt: pending.attempt + 1,
                                ..pending
                            });
                        } else {
                            self.error = Some(format!(
                                "自动续播失败，已尝试 {} 首。{error}",
                                pending.attempt
                            ));
                        }
                    }
                }
            }
            Message::ResumeReady { media, reply } => {
                self.switching = false;
                let result = media.and_then(|media| {
                    let position = self.deferred_position.unwrap_or(0.0);
                    let play = self.requested_play;
                    let recorded = self.history_recorded;
                    self.load_with_play(&media, false)?;
                    self.requested_play = play;
                    self.history_recorded = recorded;
                    self.seek(if position >= media.track.duration - 0.12 {
                        0.0
                    } else {
                        position
                    });
                    if play {
                        unsafe {
                            self.audio().play();
                        }
                    }
                    Ok(())
                });
                if result.is_err() {
                    self.requested_play = false;
                }
                self.respond(result, reply);
            }
            Message::QualityReady {
                quality,
                media,
                reply,
            } => {
                self.switching = false;
                let result = media.and_then(|media| {
                    // Read progress/state at commit time: pause, seek and volume
                    // may have changed while the new stream was resolving.
                    let status = self.status();
                    let history_recorded = self.history_recorded;
                    let end_handled = self.end_handled;
                    let requested_play = self.requested_play;
                    let old_preference = crate::audio_quality::preferred();
                    crate::audio_quality::set_preferred(quality)?;
                    if let Err(error) = self.load_with_play(&media, false) {
                        let _ = crate::audio_quality::set_preferred(old_preference);
                        return Err(error);
                    }
                    self.history_recorded = history_recorded;
                    self.end_handled = end_handled;
                    self.requested_play = requested_play;
                    self.seek(status.position);
                    unsafe {
                        if requested_play && status.state != State::Ended {
                            self.audio().play();
                        }
                    }
                    Ok(())
                });
                self.respond(result, reply);
            }
        }
    }

    fn advance_on_end(&mut self) {
        if !self.end_handled && !self.switching && self.status().state == State::Ended {
            self.end_handled = true;
            if let Some(index) = self.index.map(|index| index + 1)
                && index < self.queue.len()
            {
                self.error = None;
                self.resolve_automatic_next(AutoAdvance {
                    index,
                    attempt: 1,
                    seek_serial: self.seek_serial,
                    retry_until: Instant::now() + AUTO_ADVANCE_RETRY_WINDOW,
                });
            }
        }
    }

    fn resolve_automatic_next(&mut self, pending: AutoAdvance) {
        let track = self.queue[pending.index].clone();
        self.switching = true;
        thread::spawn(move || {
            let media = crate::catalog::resolve(track);
            dispatch(Message::AutoAdvanceReady { pending, media });
        });
    }
}
