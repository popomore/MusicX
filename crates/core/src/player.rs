use anyhow::{Result, anyhow, ensure};
use musicx_plugin_api::{DataSource, Media};
use musicx_protocol::tr;
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

use crate::{
    audio::{AudioOutput, AudioSnapshot, AudioState},
    history,
    protocol::{HistoryProgress, MAX_QUEUE_TRACKS, Track},
};

pub use crate::protocol::{State, Status};

pub enum Action {
    Status,
    DisplayStatus,
    MergeTrackMetadata(Track),
    Play(Media),
    PlayTrack(Track),
    Enqueue(Media),
    ReplaceQueue {
        tracks: Vec<Track>,
        index: usize,
        context: crate::protocol::QueueContext,
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
    Normalization(bool),
}

pub type Reply = Option<oneshot::Sender<Result<Status>>>;

const AUTO_ADVANCE_MAX_ATTEMPTS: usize = 8;
const AUTO_ADVANCE_RETRY_WINDOW: Duration = Duration::from_secs(20);
const SESSION_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(5);

pub struct AutoAdvance {
    index: usize,
    attempt: usize,
    seek_serial: u64,
    retry_until: Instant,
}

pub enum Message {
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

pub struct Player<A: AudioOutput> {
    audio: Option<A>,
    audio_lease: Option<Arc<dyn Send + Sync>>,
    dispatch: fn(Message),
    source: &'static dyn DataSource,
    volume: f32,
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
    preserve_restored_history: bool,
    saved_session: Option<crate::session::Session>,
    session_saved_at: Instant,
    // A disk checkpoint is provisional until a startup handoff or user control.
    startup_handoff_available: bool,
}

impl<A: AudioOutput> Player<A> {
    pub fn new(dispatch: fn(Message), source: &'static dyn DataSource) -> Self {
        let mut player = Self {
            audio: None,
            audio_lease: None,
            volume: 1.0,
            dispatch,
            source,
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
            preserve_restored_history: false,
            saved_session: None,
            session_saved_at: Instant::now(),
            startup_handoff_available: true,
        };
        match crate::session::Session::load() {
            Ok(Some(session)) => {
                player.queue = session.queue.clone();
                player.queue_context = session.queue_context.clone();
                player.index = session.index;
                player.volume = session.volume;
                player.deferred_position = session.index.map(|_| session.position);
                player.history_recorded = session.index.is_some_and(|index| {
                    history::restore(&session.queue[index]).unwrap_or_else(|error| {
                        eprintln!(
                            "{}",
                            tr!("Could not restore listening history: {error}"; error = format!("{:#}", error))
                        );
                        false
                    })
                });
                player.preserve_restored_history = player.history_recorded;
                player.saved_session = Some(session);
            }
            Ok(None) => {}
            Err(error) => eprintln!(
                "{}",
                tr!("Could not restore playback session: {error}"; error = format!("{:#}", error))
            ),
        }
        player
    }

    fn audio(&self) -> &A {
        self.audio
            .as_ref()
            .expect("a loaded track owns its audio output")
    }

    /// Maintains history and queue progression even when no UI exists.
    pub fn tick(&mut self) -> Status {
        self.handle(Message::Action(Action::Status, None));
        let mut status = self.status();
        self.capture_history(&status, false);
        status.queue = Arc::default();
        status
    }

    pub fn shutdown(&mut self) {
        self.flush_history();
        self.save_session(true);
        self.audio.take();
        self.audio_lease.take();
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
                normalization_enabled: crate::audio_quality::normalization_enabled(),
                ..Status::default()
            };
        }
        let track = self.index.map(|index| self.queue[index].clone());
        let AudioSnapshot {
            position,
            duration,
            state: control,
            volume,
            error,
        } = self.audio().snapshot();
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
        } else if control == AudioState::Playing {
            State::Playing
        } else if self.pending_seek.is_none()
            && self.requested_play
            && duration > 0.0
            && position >= duration - 0.12
        {
            State::Ended
        } else if control == AudioState::Waiting {
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
            normalization_enabled: crate::audio_quality::normalization_enabled(),
            audio_quality: self.audio_quality.clone(),
            error: error.or_else(|| self.error.clone()),
            switching: self.switching,
        }
    }

    fn load(&mut self, media: &Media) -> Result<()> {
        self.load_with_play(media, true)
    }
    fn load_with_play(&mut self, media: &Media, play: bool) -> Result<()> {
        self.flush_history();
        A::load(&mut self.audio, media, play, self.volume)?;
        self.audio_lease = media.lease.clone();
        self.seek_serial += 1;
        self.pending_seek = None;
        self.audio_quality = Some(media.quality.clone());
        self.deferred_position = None;
        self.end_handled = false;
        self.history_recorded = false;
        self.preserve_restored_history = false;
        self.requested_play = true;
        Ok(())
    }

    fn prepare_selection(&mut self) {
        // Checkpoint the old track before changing the queue or its index.
        self.flush_history();
        if let Some(audio) = self.audio.take() {
            audio.pause();
        }
        self.audio_lease = None;
        self.audio_quality = None;
        self.seek_serial += 1;
        self.pending_seek = None;
        self.deferred_position = Some(0.0);
        self.end_handled = false;
        self.history_recorded = false;
        self.preserve_restored_history = false;
        self.requested_play = true;
        self.switching = true;
        self.error = None;
    }

    fn resolve_selection(&self, reply: Reply) {
        let index = self.index.expect("a selection owns a queue index");
        let track = self.queue[index].clone();
        let dispatch = self.dispatch;
        let source = self.source;
        thread::spawn(move || {
            dispatch(Message::Switched {
                index,
                media: source.resolve_selection(track, crate::audio_quality::preferred()),
                reply,
            });
        });
    }

    fn seek(&mut self, seconds: f64) {
        self.seek_serial += 1;
        let serial = self.seek_serial;
        self.pending_seek = Some((serial, seconds));
        // Publish the requested position until the native seek completes.
        let dispatch = self.dispatch;
        self.audio().seek(seconds, move |finished| {
            dispatch(Message::Seeked { serial, finished });
        });
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
                    if crate::metadata::merge_metadata(&mut track, &metadata) {
                        Arc::make_mut(&mut self.queue)[index] = track;
                        self.queue_revision += 1;
                    }
                }
            }
            Action::Restore { snapshot, media } => {
                ensure!(
                    self.startup_handoff_available && self.audio.is_none() && !self.switching,
                    tr!("A session can only be transferred to an unused player")
                );
                crate::audio_quality::set_preferred(snapshot.preferred_quality)?;
                if let Some(media) = media {
                    // Load paused without changing the checkpoint volume on failure.
                    self.load_with_play(&media, false)?;
                }
                self.queue = snapshot.queue;
                self.queue_revision = snapshot.queue_revision;
                self.queue_context = snapshot.queue_context;
                self.index = snapshot.index;
                self.deferred_position = if self.audio.is_none() {
                    self.index.map(|_| snapshot.position)
                } else {
                    None
                };
                self.volume = snapshot.volume;
                self.history_recorded = self.index.is_some_and(|index| {
                    history::restore(&self.queue[index]).unwrap_or_else(|error| {
                        eprintln!(
                            "{}",
                            tr!("Could not restore listening history: {error}"; error = format!("{error:#}"))
                        );
                        false
                    })
                });
                self.preserve_restored_history = self.audio.is_none() && self.history_recorded;
                self.end_handled = snapshot.state == State::Ended;
                self.requested_play = matches!(
                    snapshot.state,
                    State::Playing | State::Loading | State::Ended
                );
                if self.audio.is_some() {
                    self.seek(snapshot.position);
                    self.audio().set_volume(self.volume);
                    if matches!(snapshot.state, State::Playing | State::Loading) {
                        self.audio().play();
                    }
                }
                self.startup_handoff_available = false;
            }
            Action::Play(media) => {
                ensure!(!self.switching, tr!("Changing track"));
                ensure!(
                    self.queue.len() < MAX_QUEUE_TRACKS,
                    tr!("The queue supports up to {MAX_QUEUE_TRACKS} tracks"; MAX_QUEUE_TRACKS = MAX_QUEUE_TRACKS)
                );
                self.load(&media)?;
                let target = self.index.map_or(0, |index| index + 1);
                Arc::make_mut(&mut self.queue).insert(target, media.track);
                self.index = Some(target);
                self.queue_revision += 1;
            }
            Action::Enqueue(media) => {
                ensure!(
                    self.queue.len() < MAX_QUEUE_TRACKS,
                    tr!("The queue supports up to {MAX_QUEUE_TRACKS} tracks"; MAX_QUEUE_TRACKS = MAX_QUEUE_TRACKS)
                );
                if self.index.is_none() {
                    self.load(&media)?;
                    self.index = Some(0);
                }
                Arc::make_mut(&mut self.queue).push(media.track);
                self.queue_revision += 1;
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
                ensure!(self.index.is_some(), tr!("Choose a track first"));
                self.flush_history();
                self.requested_play = false;
                if let Some(audio) = &self.audio {
                    audio.pause();
                }
            }
            Action::Resume => {
                ensure!(self.index.is_some(), tr!("Choose a track first"));
                self.audio().prepare_resume()?;
                let status = self.status();
                if status.duration > 0.0 && status.position >= status.duration - 0.12 {
                    self.seek(0.0);
                    self.end_handled = false;
                }
                self.requested_play = true;
                self.audio().play();
            }
            Action::Seek { seconds, track_key } => {
                if let Some(expected) = track_key {
                    ensure!(
                        !self.switching
                            && self.index.is_some_and(|i| {
                                crate::protocol::track_key(&self.queue[i]) == expected
                            }),
                        tr!("The track changed; select a lyric from the current track")
                    );
                }
                let status = self.status();
                ensure!(
                    status.duration > 0.0 && seconds.is_finite() && seconds >= 0.0,
                    tr!("Invalid playback position")
                );
                let position = seconds.min(status.duration);
                if position < status.duration - 0.12 {
                    self.end_handled = false;
                }
                if self.deferred_position.is_some() {
                    if self.deferred_position != Some(position) {
                        self.preserve_restored_history = false;
                    }
                    self.deferred_position = Some(position);
                } else {
                    self.seek(position);
                }
            }
            Action::Normalization(enabled) => {
                crate::audio_quality::set_normalization(enabled)?;
                if let Some(audio) = &self.audio {
                    audio.set_normalization(enabled);
                }
            }
            Action::Volume(value) => {
                ensure!(
                    (0.0..=1.0).contains(&value),
                    tr!("Volume must be between 0 and 1")
                );
                self.volume = value;
                if let Some(audio) = &self.audio {
                    audio.set_volume(value);
                }
            }
            Action::PlayTrack(_)
            | Action::ReplaceQueue { .. }
            | Action::Previous
            | Action::Next
            | Action::Select { .. }
            | Action::Quality(_) => {
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
        // Metadata duration can differ from the actual duration in history.
        // Passive startup must not rewrite that progress before a seek or load.
        if self.preserve_restored_history
            || self.pending_seek.is_some()
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
            eprintln!(
                "{}",
                tr!("Could not save listening history: {error}"; error = format!("{:#}", error))
            );
        }
    }

    fn save_session(&mut self, force: bool) {
        if !force && self.session_saved_at.elapsed() < SESSION_CHECKPOINT_INTERVAL {
            return;
        }
        self.session_saved_at = Instant::now();
        let session = crate::session::Session::capture(&self.status());
        if self.saved_session.as_ref() == Some(&session) {
            return;
        }
        match session.save() {
            Ok(()) => self.saved_session = Some(session),
            Err(error) => eprintln!(
                "{}",
                tr!("Could not save playback session: {error}"; error = format!("{:#}", error))
            ),
        }
    }

    pub fn handle(&mut self, message: Message) {
        if let Message::Action(action, _) = &message
            && !matches!(
                action,
                Action::Status
                    | Action::DisplayStatus
                    | Action::MergeTrackMetadata(_)
                    | Action::Restore { .. }
            )
        {
            // Consume before dispatching work so an in-flight resolve cannot
            // later apply to a session replaced by an upgrade handoff.
            self.startup_handoff_available = false;
        }
        let checkpoint = !matches!(
            &message,
            Message::Action(Action::Status | Action::DisplayStatus, _)
        );
        let dispatch = self.dispatch;
        let source = self.source;
        match message {
            Message::Action(Action::PlayTrack(track), reply) => {
                if self.switching {
                    self.respond(Err(anyhow!(tr!("Changing track"))), reply);
                } else if self.queue.len() >= MAX_QUEUE_TRACKS {
                    self.respond(
                        Err(anyhow!(tr!("The queue supports up to {MAX_QUEUE_TRACKS} tracks"; MAX_QUEUE_TRACKS = MAX_QUEUE_TRACKS))),
                        reply,
                    );
                } else {
                    self.prepare_selection();
                    let index = self.index.map_or(0, |index| index + 1);
                    Arc::make_mut(&mut self.queue).insert(index, track);
                    self.index = Some(index);
                    self.queue_revision += 1;
                    self.resolve_selection(reply);
                }
            }
            Message::Action(
                Action::ReplaceQueue {
                    tracks,
                    index,
                    context,
                },
                reply,
            ) => {
                if self.switching {
                    self.respond(Err(anyhow!(tr!("Changing track"))), reply);
                } else if index >= tracks.len() || tracks.len() > MAX_QUEUE_TRACKS {
                    self.respond(Err(anyhow!(tr!("Invalid playlist"))), reply);
                } else {
                    self.prepare_selection();
                    self.queue = Arc::new(tracks);
                    self.index = Some(index);
                    self.queue_context = Some(context);
                    self.queue_revision += 1;
                    self.resolve_selection(reply);
                }
            }
            Message::Action(action @ (Action::Resume | Action::Toggle), reply)
                if self.deferred_position.is_some()
                    || self
                        .audio
                        .as_ref()
                        .is_some_and(|audio| audio.snapshot().error.is_some()) =>
            {
                if matches!(action, Action::Toggle)
                    && self.deferred_position.is_some()
                    && self.requested_play
                {
                    let result = self.apply(Action::Pause);
                    self.respond(result, reply);
                } else if self.switching {
                    self.respond(
                        Err(anyhow!(tr!("Getting audio stream; please wait"))),
                        reply,
                    );
                } else if let Some(track) = self.index.and_then(|i| self.queue.get(i)).cloned() {
                    if self.deferred_position.is_none() {
                        // A failed native item cannot be played again. Keep the
                        // song, progress and history while resolving a fresh item.
                        let position = self.status().position;
                        if let Some(audio) = self.audio.take() {
                            audio.pause();
                        }
                        self.audio_lease = None;
                        self.audio_quality = None;
                        self.seek_serial += 1;
                        self.pending_seek = None;
                        self.deferred_position = Some(position);
                    }
                    self.switching = true;
                    self.requested_play = true;
                    self.error = None;
                    thread::spawn(move || {
                        dispatch(Message::ResumeReady {
                            media: source.resolve(track, crate::audio_quality::preferred()),
                            reply,
                        })
                    });
                } else {
                    self.respond(Err(anyhow!(tr!("Choose a track first"))), reply);
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
                    self.respond(
                        Err(anyhow!(tr!(
                            "Changing track; please wait before selecting quality"
                        ))),
                        reply,
                    );
                } else if self.deferred_position.is_some() {
                    self.respond(crate::audio_quality::set_preferred(quality), reply);
                } else if let Some(track) = self.index.and_then(|i| self.queue.get(i)).cloned() {
                    self.switching = true;
                    self.error = None;
                    thread::spawn(move || {
                        dispatch(Message::QualityReady {
                            quality,
                            media: source.resolve(track, quality),
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
                        self.respond(
                            Err(anyhow!(tr!("The playlist changed; please select again"))),
                            reply,
                        );
                        return;
                    }
                    Action::Next => self.index.and_then(|i| i.checked_add(1)),
                    _ => self.index.and_then(|i| i.checked_sub(1)),
                };
                if self.switching {
                    self.respond(Err(anyhow!(tr!("Changing track"))), reply);
                } else if let Some(index) = target.filter(|&i| i < self.queue.len()) {
                    self.prepare_selection();
                    self.index = Some(index);
                    self.resolve_selection(reply);
                } else {
                    self.respond(Err(anyhow!(tr!("No other track available"))), reply);
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
                self.error = None;
                let result = media.and_then(|media| {
                    let play = self.requested_play;
                    let position = self.deferred_position.unwrap_or(0.0);
                    self.load_with_play(&media, false)?;
                    self.requested_play = play;
                    if self.queue[index] != media.track {
                        Arc::make_mut(&mut self.queue)[index] = media.track;
                        self.queue_revision += 1;
                    }
                    if position > 0.0 {
                        self.seek(position);
                    }
                    if play {
                        self.audio().play();
                    }
                    Ok(())
                });
                if result.is_err() {
                    self.requested_play = false;
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
                            self.error = Some(
                                tr!("Could not continue playback after trying {} tracks. {error}", pending.attempt; error = error),
                            );
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
                    self.seek(
                        if media.track.duration > 0.0 && position >= media.track.duration - 0.12 {
                            0.0
                        } else {
                            position
                        },
                    );
                    if play {
                        self.audio().play();
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
                    if requested_play && status.state != State::Ended {
                        self.audio().play();
                    }
                    Ok(())
                });
                self.respond(result, reply);
            }
        }
        self.save_session(checkpoint);
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
        let dispatch = self.dispatch;
        let source = self.source;
        thread::spawn(move || {
            let media = source.resolve(track, crate::audio_quality::preferred());
            dispatch(Message::AutoAdvanceReady { pending, media });
        });
    }
}
