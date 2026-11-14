use anyhow::{Result, bail};
use musicx_core::{
    audio::{AudioOutput, AudioSnapshot, AudioState},
    player::{Action, Message, Player, State},
    protocol::{
        AlbumPage, AudioQuality, HistoryProgress, Quality, QueueContext, Status, Track, TrackInfo,
    },
};
use musicx_plugin_api::{DataSource, Media};
use std::{
    cell::Cell,
    process::Command,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

static MESSAGES: OnceLock<mpsc::Sender<Message>> = OnceLock::new();
static LOADS: AtomicUsize = AtomicUsize::new(0);
static RESOLVES: AtomicUsize = AtomicUsize::new(0);
static PAUSES: AtomicUsize = AtomicUsize::new(0);
static FAIL_RESOLVE: AtomicBool = AtomicBool::new(false);
static FAIL_LOAD: AtomicBool = AtomicBool::new(false);
static NATIVE_AUDIO_ERROR: AtomicBool = AtomicBool::new(false);
static NATIVE_AUDIO_ENDED: AtomicBool = AtomicBool::new(false);
static NATIVE_DURATION: AtomicU64 = AtomicU64::new(180.0_f64.to_bits());
static PLAYBACK_ELAPSED: AtomicU64 = AtomicU64::new(0);

fn dispatch(message: Message) {
    MESSAGES.get().unwrap().send(message).unwrap();
}

struct Source;
static SOURCE: Source = Source;
impl DataSource for Source {
    fn resolve(&self, track: Track, _: Quality) -> Result<Media> {
        RESOLVES.fetch_add(1, Ordering::Relaxed);
        if FAIL_RESOLVE.load(Ordering::Relaxed) {
            bail!("offline");
        }
        Ok(media(track))
    }
    fn info(&self, _: Track) -> TrackInfo {
        unreachable!()
    }
    fn album(&self, _: Track) -> Result<AlbumPage> {
        unreachable!()
    }
}

struct Audio {
    position: Cell<f64>,
    playing: Cell<bool>,
    volume: Cell<f32>,
}
impl AudioOutput for Audio {
    fn load(output: &mut Option<Self>, _: &Media, play: bool, volume: f32) -> Result<()> {
        LOADS.fetch_add(1, Ordering::Relaxed);
        if FAIL_LOAD.load(Ordering::Relaxed) {
            bail!("audio load failed");
        }
        NATIVE_AUDIO_ERROR.store(false, Ordering::Relaxed);
        NATIVE_AUDIO_ENDED.store(false, Ordering::Relaxed);
        NATIVE_DURATION.store(180.0_f64.to_bits(), Ordering::Relaxed);
        PLAYBACK_ELAPSED.store(0, Ordering::Relaxed);
        *output = Some(Self {
            position: Cell::new(0.0),
            playing: Cell::new(play),
            volume: Cell::new(volume),
        });
        Ok(())
    }
    fn snapshot(&self) -> AudioSnapshot {
        AudioSnapshot {
            position: self.position.get()
                + f64::from_bits(PLAYBACK_ELAPSED.load(Ordering::Relaxed)),
            duration: Some(f64::from_bits(NATIVE_DURATION.load(Ordering::Relaxed))),
            state: if NATIVE_AUDIO_ENDED.load(Ordering::Relaxed) {
                AudioState::Ended
            } else if self.playing.get() {
                AudioState::Playing
            } else {
                AudioState::Paused
            },
            volume: self.volume.get(),
            error: NATIVE_AUDIO_ERROR
                .load(Ordering::Relaxed)
                .then(|| "native item failed".into()),
        }
    }
    fn prepare_resume(&self) -> Result<()> {
        Ok(())
    }
    fn play(&self) {
        self.playing.set(true);
    }
    fn pause(&self) {
        PAUSES.fetch_add(1, Ordering::Relaxed);
        self.playing.set(false);
    }
    fn set_volume(&self, volume: f32) {
        self.volume.set(volume);
    }
    fn seek(&self, seconds: f64, complete: impl Fn(bool) + Send + 'static) {
        NATIVE_AUDIO_ENDED.store(false, Ordering::Relaxed);
        PLAYBACK_ELAPSED.store(0, Ordering::Relaxed);
        self.position.set(seconds);
        complete(true);
    }
}

fn track(id: u64) -> Track {
    Track {
        id,
        title: format!("Track {id}"),
        artist: "Test".into(),
        artists: Vec::new(),
        episode: None,
        duration: 180.0,
        cover_url: None,
        album: None,
        source_url: None,
        source: None,
    }
}
fn media(track: Track) -> Media {
    Media {
        lease: None,
        track,
        url: "https://example.invalid/expiring-audio?secret=not-persisted"
            .parse()
            .unwrap(),
        user_agent: Some("private-agent".into()),
        uses_http: false,
        quality: AudioQuality::default(),
        gain_db: None,
        peak: None,
    }
}

fn handoff(player: &mut Player<Audio>, state: State) -> Result<Status> {
    let snapshot = Status {
        state,
        track: Some(track(3)),
        queue: Arc::new(vec![track(3), track(4)]),
        index: Some(0),
        queue_length: 2,
        queue_revision: 42,
        queue_context: Some(QueueContext::History),
        position: 95.5,
        duration: 180.0,
        volume: 0.6,
        ..Status::default()
    };
    let (send, reply) = tokio::sync::oneshot::channel();
    player.handle(Message::Action(
        Action::Restore {
            snapshot: Box::new(snapshot),
            media: (state == State::Playing).then(|| media(track(3))),
        },
        Some(send),
    ));
    reply.blocking_recv().unwrap()
}

fn command(player: &mut Player<Audio>, action: Action) -> Result<Status> {
    let (send, reply) = tokio::sync::oneshot::channel();
    player.handle(Message::Action(action, Some(send)));
    reply.blocking_recv().unwrap()
}

fn resolve_command(
    player: &mut Player<Audio>,
    action: Action,
    messages: &mpsc::Receiver<Message>,
) -> Result<Status> {
    let (send, reply) = tokio::sync::oneshot::channel();
    player.handle(Message::Action(action, Some(send)));
    player.handle(messages.recv_timeout(Duration::from_secs(2)).unwrap());
    reply.blocking_recv().unwrap()
}

fn run_worker(directory: &std::path::Path, phase: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "session_worker", "--nocapture"])
        .env("MUSICX_DATA_DIR", directory)
        .env("MUSICX_SESSION_TEST_PHASE", phase)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{phase}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_scenario(phases: &[&str]) {
    let directory = std::env::temp_dir().join(format!(
        "musicx-regression-{}",
        musicx_core::storage::random_hex::<12>()
    ));
    std::fs::create_dir(&directory).unwrap();
    for phase in phases {
        run_worker(&directory, phase);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn native_end_advances_the_queue_even_when_duration_is_inaccurate() {
    for phase in [
        "native_end",
        "native_end_display",
        "native_end_status",
        "native_end_unknown",
    ] {
        run_scenario(&[phase]);
    }
}

#[test]
fn pause_and_seek_take_priority_over_native_end_and_pending_advance() {
    for phase in [
        "native_end_pause",
        "native_end_seek",
        "native_end_pending_pause",
        "native_end_pending_seek",
    ] {
        run_scenario(&[phase]);
    }
}

#[test]
fn http_audio_indicator_follows_the_loaded_source() {
    run_scenario(&["http_transport"]);
}

#[test]
fn loaded_media_resources_live_until_replacement_or_shutdown() {
    run_scenario(&["media_lease"]);
}

#[test]
fn empty_handoff_clears_the_old_checkpoint_position() {
    run_scenario(&["save", "handoff_empty", "verify_empty"]);
}

#[test]
fn failed_handoff_preserves_checkpoint_volume_and_can_retry() {
    run_scenario(&["save", "handoff_load_failure", "retry_failed_handoff"]);
}

#[test]
fn runtime_queue_limit_keeps_the_session_saveable() {
    run_scenario(&["queue_limit", "verify_queue_limit"]);
}

#[test]
fn tick_checkpoints_natural_playback_progress() {
    run_scenario(&["periodic_tick"]);
}

#[test]
fn display_status_checkpoints_natural_playback_progress() {
    run_scenario(&["periodic_display"]);
}

#[test]
fn selection_is_visible_before_resolution_and_keeps_failed_track_for_retry() {
    run_scenario(&["select_resolve_failure"]);
    run_scenario(&["select_load_failure"]);
    run_scenario(&["list_resolve_failure"]);
    run_scenario(&["track_resolve_failure"]);
}

#[test]
fn pause_and_seek_during_selection_apply_to_the_new_track() {
    run_scenario(&["select_pause"]);
}

#[test]
fn failed_native_items_resolve_again_on_resume_and_toggle() {
    for phase in [
        "native_retry_resume",
        "native_retry_toggle",
        "native_retry_offline",
    ] {
        run_scenario(&[phase]);
    }
}

#[test]
fn passive_restore_preserves_completed_history() {
    let directory = std::env::temp_dir().join(format!(
        "musicx-history-{}",
        musicx_core::storage::random_hex::<12>()
    ));
    std::fs::create_dir(&directory).unwrap();
    for phase in ["completed_history_unknown", "completed_history_long"] {
        run_worker(&directory, "save");
        run_worker(&directory, phase);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn startup_session_allows_upgrade_handoff_until_user_control() {
    let directory = std::env::temp_dir().join(format!(
        "musicx-handoff-{}",
        musicx_core::storage::random_hex::<12>()
    ));
    std::fs::create_dir(&directory).unwrap();
    for phase in [
        "handoff_playing",
        "handoff_paused",
        "handoff_after_seek",
        "handoff_during_resume",
    ] {
        for step in ["save", phase] {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "session_worker", "--nocapture"])
                .env("MUSICX_DATA_DIR", &directory)
                .env("MUSICX_SESSION_TEST_PHASE", step)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{step}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn session_survives_process_restart() {
    let directory = std::env::temp_dir().join(format!(
        "musicx-session-{}",
        musicx_core::storage::random_hex::<12>()
    ));
    std::fs::create_dir(&directory).unwrap();
    for phase in ["save", "restore"] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "session_worker", "--nocapture"])
            .env("MUSICX_DATA_DIR", &directory)
            .env("MUSICX_SESSION_TEST_PHASE", phase)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{phase}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let saved = std::fs::read_to_string(directory.join("playback-session.json")).unwrap();
    assert!(!saved.contains("expiring-audio"));
    assert!(!saved.contains("private-agent"));
    let mut unknown_duration: serde_json::Value = serde_json::from_str(&saved).unwrap();
    unknown_duration["queue"][1]["duration"] = 0.0.into();
    std::fs::write(
        directory.join("playback-session.json"),
        unknown_duration.to_string(),
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "session_worker", "--nocapture"])
        .env("MUSICX_DATA_DIR", &directory)
        .env("MUSICX_SESSION_TEST_PHASE", "restore")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "unknown duration: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut invalid_index: serde_json::Value = serde_json::from_str(&saved).unwrap();
    invalid_index["index"] = 100.into();
    for invalid in ["{partial".to_string(), invalid_index.to_string()] {
        std::fs::write(directory.join("playback-session.json"), invalid).unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "session_worker", "--nocapture"])
            .env("MUSICX_DATA_DIR", &directory)
            .env("MUSICX_SESSION_TEST_PHASE", "corrupt")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn session_worker() {
    let Ok(phase) = std::env::var("MUSICX_SESSION_TEST_PHASE") else {
        return;
    };
    let (send, receive) = mpsc::channel();
    MESSAGES.set(send).ok().unwrap();
    let completed = HistoryProgress {
        position: 180.0,
        duration: 180.0,
        completed: true,
    };
    if phase.starts_with("completed_history_") {
        let path = musicx_core::protocol::data_dir()
            .unwrap()
            .join("playback-session.json");
        let mut saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        saved["queue"][1]["duration"] = if phase.ends_with("unknown") {
            0.0
        } else {
            240.0
        }
        .into();
        saved["position"] = 180.0.into();
        musicx_core::history::record(
            serde_json::from_value(saved["queue"][1].clone()).unwrap(),
            completed,
        )
        .unwrap();
        std::fs::write(path, saved.to_string()).unwrap();
    }
    let mut player = Player::<Audio>::new(dispatch, &SOURCE);
    let action = |player: &mut Player<Audio>, action| player.handle(Message::Action(action, None));
    match phase.as_str() {
        "native_end" | "native_end_display" | "native_end_status" | "native_end_unknown" => {
            let mut first = track(1);
            if phase == "native_end_unknown" {
                first.duration = 0.0;
            }
            command(&mut player, Action::Play(media(first))).unwrap();
            command(&mut player, Action::Enqueue(media(track(2)))).unwrap();
            if phase == "native_end_unknown" {
                NATIVE_DURATION.store(f64::NAN.to_bits(), Ordering::Relaxed);
            }
            PLAYBACK_ELAPSED.store(1.0_f64.to_bits(), Ordering::Relaxed);
            player.tick();
            // A native end signal is authoritative even when the final timestamp
            // falls short of the duration reported by the media container.
            PLAYBACK_ELAPSED.store(179.5_f64.to_bits(), Ordering::Relaxed);
            NATIVE_AUDIO_ENDED.store(true, Ordering::Relaxed);
            match phase.as_str() {
                "native_end_display" => {
                    command(&mut player, Action::DisplayStatus).unwrap();
                }
                "native_end_status" => {
                    command(&mut player, Action::Status).unwrap();
                }
                _ => {
                    player.tick();
                }
            }
            assert!(
                player.status().switching,
                "a completed native item must resolve the next queued track"
            );
            // Polling again while resolution is pending must not skip a track
            // or start a second request for the same end notification.
            player.tick();
            command(&mut player, Action::DisplayStatus).unwrap();
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            let status = player.status();
            assert_eq!(status.index, Some(1));
            assert_eq!(status.track.as_ref().map(|track| track.id), Some(2));
            assert!(status.state == State::Playing);
            let history = musicx_core::history::recent(2).unwrap();
            assert!(
                history
                    .iter()
                    .find(|entry| entry.track.id == 1)
                    .unwrap()
                    .progress
                    .unwrap()
                    .completed
            );
            // Replacement clears the previous item's end signal. The final
            // track may end repeatedly without starting another resolution.
            player.tick();
            assert!(player.status().state == State::Playing);
            PLAYBACK_ELAPSED.store(179.5_f64.to_bits(), Ordering::Relaxed);
            NATIVE_AUDIO_ENDED.store(true, Ordering::Relaxed);
            player.tick();
            player.tick();
            assert!(player.status().state == State::Ended && !player.status().switching);
            assert!(receive.try_recv().is_err());
            assert_eq!(RESOLVES.load(Ordering::Relaxed), 1);
            assert_eq!(LOADS.load(Ordering::Relaxed), 2);
        }
        "native_end_pause"
        | "native_end_seek"
        | "native_end_pending_pause"
        | "native_end_pending_seek" => {
            command(&mut player, Action::Play(media(track(1)))).unwrap();
            command(&mut player, Action::Enqueue(media(track(2)))).unwrap();
            PLAYBACK_ELAPSED.store(179.5_f64.to_bits(), Ordering::Relaxed);
            NATIVE_AUDIO_ENDED.store(true, Ordering::Relaxed);
            let pending = phase.contains("pending");
            if pending {
                player.tick();
                assert!(player.status().switching);
            }
            let pause = phase.ends_with("pause");
            command(
                &mut player,
                if pause {
                    Action::Pause
                } else {
                    Action::Seek {
                        seconds: 42.0,
                        track_key: None,
                    }
                },
            )
            .unwrap();
            for _ in 0..usize::from(pending) + usize::from(!pause) {
                player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            }
            player.tick();
            assert_eq!(player.status().index, Some(0));
            assert!(!player.status().switching);
            assert_eq!(LOADS.load(Ordering::Relaxed), 1);
            if pause {
                assert!(player.status().state == State::Paused);
                command(&mut player, Action::Resume).unwrap();
                player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
                assert_eq!(player.status().position, 0.0);
            } else {
                assert_eq!(player.status().position, 42.0);
            }
            assert!(player.status().state == State::Playing);
            // Replaying or rewinding must re-arm automatic advancement.
            PLAYBACK_ELAPSED.store(179.5_f64.to_bits(), Ordering::Relaxed);
            NATIVE_AUDIO_ENDED.store(true, Ordering::Relaxed);
            player.tick();
            assert!(player.status().switching);
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            assert_eq!(player.status().index, Some(1));
            assert!(player.status().state == State::Playing);
        }
        "native_retry_resume" | "native_retry_toggle" | "native_retry_offline" => {
            let lease = Arc::new(());
            let weak = Arc::downgrade(&lease);
            let mut initial = media(track(1));
            initial.lease = Some(lease);
            command(&mut player, Action::Play(initial)).unwrap();
            command(&mut player, Action::Pause).unwrap();
            command(&mut player, Action::Resume).unwrap();
            assert_eq!(LOADS.load(Ordering::Relaxed), 1);
            assert_eq!(RESOLVES.load(Ordering::Relaxed), 0);
            command(
                &mut player,
                Action::Seek {
                    seconds: 42.0,
                    track_key: None,
                },
            )
            .unwrap();
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            player.tick();
            NATIVE_AUDIO_ERROR.store(true, Ordering::Relaxed);
            assert!(player.status().state == State::Error);
            FAIL_RESOLVE.store(phase == "native_retry_offline", Ordering::Relaxed);
            let (send, reply) = tokio::sync::oneshot::channel();
            player.handle(Message::Action(
                if phase == "native_retry_toggle" {
                    Action::Toggle
                } else {
                    Action::Resume
                },
                Some(send),
            ));
            let loading = player.status();
            assert!(
                loading.state == State::Loading && loading.switching,
                "retry must resolve new media instead of playing a failed native item"
            );
            assert_eq!(loading.track.as_ref().map(|t| t.id), Some(1));
            assert_eq!(loading.position, 42.0);
            assert!(loading.audio_quality.is_none() && loading.error.is_none());
            assert!(
                weak.upgrade().is_none(),
                "retry must release the failed item's lease"
            );
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            let result = reply.blocking_recv().unwrap();
            let resumed = if phase == "native_retry_offline" {
                assert!(result.is_err());
                assert_eq!(player.status().position, 42.0);
                FAIL_RESOLVE.store(false, Ordering::Relaxed);
                resolve_command(&mut player, Action::Resume, &receive).unwrap()
            } else {
                result.unwrap()
            };
            assert!(resumed.state == State::Playing && resumed.error.is_none());
            assert_eq!(resumed.track.as_ref().map(|t| t.id), Some(1));
            assert_eq!(resumed.position, 42.0);
            assert_eq!(LOADS.load(Ordering::Relaxed), 2);
            assert_eq!(
                RESOLVES.load(Ordering::Relaxed),
                if phase == "native_retry_offline" {
                    2
                } else {
                    1
                }
            );
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            PLAYBACK_ELAPSED.store(1.0_f64.to_bits(), Ordering::Relaxed);
            player.tick();
            let history = musicx_core::history::recent(10).unwrap();
            assert_eq!(
                history.len(),
                1,
                "retry must keep the existing history entry"
            );
            assert_eq!(history[0].progress.unwrap().position, 43.0);
        }
        "select_resolve_failure"
        | "select_load_failure"
        | "list_resolve_failure"
        | "track_resolve_failure"
        | "select_pause" => {
            resolve_command(
                &mut player,
                Action::ReplaceQueue {
                    tracks: vec![track(1), track(2)],
                    index: 0,
                    context: QueueContext::History,
                },
                &receive,
            )
            .unwrap();
            command(
                &mut player,
                Action::Seek {
                    seconds: 42.0,
                    track_key: None,
                },
            )
            .unwrap();
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            player.tick();
            FAIL_RESOLVE.store(phase.ends_with("resolve_failure"), Ordering::Relaxed);
            FAIL_LOAD.store(phase == "select_load_failure", Ordering::Relaxed);
            let selection = match phase.as_str() {
                "list_resolve_failure" => Action::ReplaceQueue {
                    tracks: vec![track(2), track(3)],
                    index: 0,
                    context: QueueContext::LocalPlaylist {
                        id: "new-list".into(),
                        name: "New list".into(),
                    },
                },
                "track_resolve_failure" => Action::PlayTrack(track(2)),
                _ => Action::Select {
                    index: 1,
                    revision: player.status().queue_revision,
                },
            };
            let pauses = PAUSES.load(Ordering::Relaxed);
            let (send, reply) = tokio::sync::oneshot::channel();
            player.handle(Message::Action(selection, Some(send)));
            let loading = player.status();
            assert_eq!(
                loading.track.as_ref().map(|t| t.id),
                Some(2),
                "the selected track must be visible before resolving its audio"
            );
            assert!(loading.state == State::Loading && loading.switching);
            assert_eq!(loading.position, 0.0);
            assert!(loading.audio_quality.is_none() && loading.error.is_none());
            assert_eq!(PAUSES.load(Ordering::Relaxed), pauses + 1);
            let saved: serde_json::Value = serde_json::from_slice(
                &std::fs::read(
                    musicx_core::protocol::data_dir()
                        .unwrap()
                        .join("playback-session.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                saved["queue"][saved["index"].as_u64().unwrap() as usize]["id"],
                2
            );
            assert_eq!(saved["position"], 0.0);
            if phase == "select_pause" {
                command(&mut player, Action::Pause).unwrap();
                command(
                    &mut player,
                    Action::Seek {
                        seconds: 17.0,
                        track_key: None,
                    },
                )
                .unwrap();
                player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
                let paused = reply.blocking_recv().unwrap().unwrap();
                assert!(paused.state == State::Paused && !paused.switching);
                assert!(paused.error.is_none());
                assert_eq!(paused.track.as_ref().map(|t| t.id), Some(2));
                assert_eq!(paused.position, 17.0);
                player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
                let resumed = command(&mut player, Action::Resume).unwrap();
                assert!(resumed.state == State::Playing);
                return;
            }
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            assert!(reply.blocking_recv().unwrap().is_err());
            let failed = player.status();
            assert_eq!(failed.track.as_ref().map(|t| t.id), Some(2));
            assert!(failed.state == State::Paused && !failed.switching);
            assert_eq!(failed.position, 0.0);
            assert!(failed.audio_quality.is_none());
            assert_eq!(
                failed.error.as_deref(),
                Some(if phase.ends_with("resolve_failure") {
                    "offline"
                } else {
                    "audio load failed"
                })
            );
            let history = musicx_core::history::recent(10).unwrap();
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].track.id, 1);
            assert_eq!(history[0].progress.unwrap().position, 42.0);
            FAIL_RESOLVE.store(false, Ordering::Relaxed);
            FAIL_LOAD.store(false, Ordering::Relaxed);
            let (send, reply) = tokio::sync::oneshot::channel();
            player.handle(Message::Action(Action::Resume, Some(send)));
            assert!(player.status().state == State::Loading);
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            let resumed = reply.blocking_recv().unwrap().unwrap();
            assert_eq!(resumed.track.as_ref().map(|t| t.id), Some(2));
            assert!(resumed.state == State::Playing && resumed.error.is_none());
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
        }
        "http_transport" => {
            let mut http = media(track(1));
            http.uses_http = true;
            http.url = "http://127.0.0.1/audio".parse().unwrap();
            assert!(
                command(&mut player, Action::Play(http.clone()))
                    .unwrap()
                    .audio_uses_http
            );
            command(&mut player, Action::Enqueue(media(track(2)))).unwrap();
            assert!(player.status().audio_uses_http);

            let mut https_proxy = media(track(2));
            https_proxy.url = "http://127.0.0.1/private-https-audio".parse().unwrap();
            FAIL_LOAD.store(true, Ordering::Relaxed);
            assert!(command(&mut player, Action::Play(https_proxy.clone())).is_err());
            assert!(player.status().audio_uses_http);
            FAIL_LOAD.store(false, Ordering::Relaxed);
            assert!(
                !command(&mut player, Action::Play(https_proxy))
                    .unwrap()
                    .audio_uses_http
            );

            command(&mut player, Action::Play(http)).unwrap();
            FAIL_RESOLVE.store(true, Ordering::Relaxed);
            let (send, reply) = tokio::sync::oneshot::channel();
            player.handle(Message::Action(Action::PlayTrack(track(3)), Some(send)));
            assert!(
                !player.status().audio_uses_http,
                "a pending selection must clear the old indicator"
            );
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            assert!(reply.blocking_recv().unwrap().is_err());
            assert!(!player.status().audio_uses_http);
        }
        "media_lease" => {
            let lease = Arc::new(());
            let weak = Arc::downgrade(&lease);
            let mut current = media(track(1));
            current.lease = Some(lease);
            command(&mut player, Action::Play(current)).unwrap();
            assert!(
                weak.upgrade().is_some(),
                "loaded URL resources must survive resolution"
            );
            let queued = Arc::new(());
            let queued_weak = Arc::downgrade(&queued);
            let mut pending = media(track(2));
            pending.lease = Some(queued);
            command(&mut player, Action::Enqueue(pending)).unwrap();
            assert!(
                queued_weak.upgrade().is_none(),
                "unused enqueue resources must be released"
            );
            assert!(weak.upgrade().is_some());
            FAIL_LOAD.store(true, Ordering::Relaxed);
            assert!(command(&mut player, Action::Play(media(track(3)))).is_err());
            FAIL_LOAD.store(false, Ordering::Relaxed);
            assert!(
                weak.upgrade().is_some(),
                "failed replacement must keep the current lease"
            );
            command(&mut player, Action::Play(media(track(3)))).unwrap();
            assert!(weak.upgrade().is_none());
            let lease = Arc::new(());
            let weak = Arc::downgrade(&lease);
            let mut current = media(track(4));
            current.lease = Some(lease);
            command(&mut player, Action::Play(current)).unwrap();
            player.shutdown();
            assert!(
                weak.upgrade().is_none(),
                "shutdown must release the loaded URL resources"
            );
        }
        "save" => {
            resolve_command(
                &mut player,
                Action::ReplaceQueue {
                    tracks: vec![track(1), track(2)],
                    index: 1,
                    context: QueueContext::History,
                },
                &receive,
            )
            .unwrap();
            action(
                &mut player,
                Action::Seek {
                    seconds: 73.25,
                    track_key: None,
                },
            );
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            action(&mut player, Action::Volume(0.37));
            // The last seek must survive even if the process exits without shutdown.
            let saved = std::fs::read_to_string(
                musicx_core::protocol::data_dir()
                    .unwrap()
                    .join("playback-session.json"),
            )
            .expect("checkpoint must exist before shutdown");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&saved).unwrap()["position"],
                73.25
            );
        }
        "restore" => {
            let status = player.status();
            assert!(status.state == State::Paused);
            assert_eq!(status.track.as_ref().map(|t| t.id), Some(2));
            assert_eq!(status.queue.len(), 2);
            assert_eq!(status.index, Some(1));
            assert_eq!(status.position, 73.25);
            assert_eq!(status.volume, 0.37);
            assert!(status.queue_context == Some(QueueContext::History));
            assert_eq!(
                LOADS.load(Ordering::Relaxed),
                0,
                "cold startup must not load or play audio"
            );
            // An offline resume retains progress and can be retried.
            FAIL_RESOLVE.store(true, Ordering::Relaxed);
            action(&mut player, Action::Resume);
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            assert!(player.status().state == State::Paused);
            assert_eq!(player.status().position, 73.25);
            assert_eq!(player.status().queue.len(), 2);
            FAIL_RESOLVE.store(false, Ordering::Relaxed);
            action(&mut player, Action::Resume);
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            assert!(player.status().state == State::Playing);
            assert_eq!(player.status().position, 73.25);
            assert_eq!(player.status().volume, 0.37);
            player.shutdown();
        }
        "corrupt" => {
            assert!(player.status().state == State::Idle);
            action(&mut player, Action::Play(media(track(1))));
            assert!(player.status().state == State::Playing);
        }
        "completed_history_unknown" | "completed_history_long" => {
            player.tick();
            action(&mut player, Action::DisplayStatus);
            action(&mut player, Action::Volume(0.25));
            action(&mut player, Action::Pause);
            player.shutdown();
            assert!(
                musicx_core::history::recent(1).unwrap()[0].progress == Some(completed),
                "passive restoration must preserve the completed history progress"
            );
            if phase.ends_with("long") {
                action(
                    &mut player,
                    Action::Seek {
                        seconds: 60.0,
                        track_key: None,
                    },
                );
                player.tick();
                let progress = musicx_core::history::recent(1).unwrap()[0]
                    .progress
                    .unwrap();
                assert_eq!(progress.position, 60.0);
                assert!(
                    !progress.completed,
                    "an explicit rewind must update completion"
                );
            }
        }
        "handoff_playing" | "handoff_paused" => {
            assert_eq!(
                player.status().track.as_ref().map(|track| track.id),
                Some(2)
            );
            // Polling and background metadata enrichment must not consume the handoff.
            player.tick();
            action(&mut player, Action::DisplayStatus);
            let mut enriched = track(2);
            enriched.cover_url = Some("https://example.invalid/cover".into());
            action(&mut player, Action::MergeTrackMetadata(enriched));
            let state = if phase == "handoff_playing" {
                State::Playing
            } else {
                State::Paused
            };
            let restored = handoff(&mut player, state)
                .expect("startup checkpoint must accept upgrade handoff");
            assert!(restored.state == state);
            assert_eq!(restored.track.as_ref().map(|track| track.id), Some(3));
            assert_eq!(
                restored
                    .queue
                    .iter()
                    .map(|track| track.id)
                    .collect::<Vec<_>>(),
                vec![3, 4]
            );
            assert_eq!(restored.position, 95.5);
            assert_eq!(restored.volume, 0.6);
            assert_eq!(restored.index, Some(0));
            assert_eq!(restored.queue_revision, 42);
            if state == State::Playing {
                player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            } else {
                assert_eq!(LOADS.load(Ordering::Relaxed), 0);
            }
            assert!(
                handoff(&mut player, State::Paused).is_err(),
                "handoff is accepted only once"
            );
            assert!(player.status().state == state);
            assert_eq!(player.status().position, 95.5);
        }
        "handoff_empty" => {
            let (send, reply) = tokio::sync::oneshot::channel();
            player.handle(Message::Action(
                Action::Restore {
                    snapshot: Box::new(Status {
                        state: State::Idle,
                        volume: 0.8,
                        ..Status::default()
                    }),
                    media: None,
                },
                Some(send),
            ));
            let status = reply.blocking_recv().unwrap().unwrap();
            assert!(
                status.state == State::Idle && status.track.is_none() && status.queue.is_empty()
            );
            assert_eq!(
                status.position, 0.0,
                "empty handoff must clear deferred position"
            );
            let saved: serde_json::Value = serde_json::from_slice(
                &std::fs::read(
                    musicx_core::protocol::data_dir()
                        .unwrap()
                        .join("playback-session.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(saved["position"], 0.0);
        }
        "verify_empty" => {
            let status = player.status();
            assert!(status.state == State::Idle && status.queue.is_empty());
            assert_eq!(status.position, 0.0);
            assert_eq!(status.volume, 0.8);
        }
        "queue_limit" => {
            resolve_command(
                &mut player,
                Action::ReplaceQueue {
                    tracks: (1..=9_998).map(track).collect(),
                    index: 0,
                    context: QueueContext::History,
                },
                &receive,
            )
            .unwrap();
            command(&mut player, Action::Play(media(track(10_001)))).unwrap();
            command(&mut player, Action::Enqueue(media(track(10_002)))).unwrap();
            command(
                &mut player,
                Action::Seek {
                    seconds: 33.5,
                    track_key: None,
                },
            )
            .unwrap();
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            let loads = LOADS.load(Ordering::Relaxed);
            assert!(
                command(&mut player, Action::Play(media(track(10_003)))).is_err(),
                "Play must reject an oversized queue"
            );
            assert!(
                command(&mut player, Action::Enqueue(media(track(10_004)))).is_err(),
                "Enqueue must reject an oversized queue"
            );
            assert_eq!(
                LOADS.load(Ordering::Relaxed),
                loads,
                "rejected actions must not replace audio"
            );
            assert_eq!(player.status().queue.len(), 10_000);
            assert_eq!(player.status().position, 33.5);
            command(&mut player, Action::Volume(0.19)).unwrap();
            let saved: serde_json::Value = serde_json::from_slice(
                &std::fs::read(
                    musicx_core::protocol::data_dir()
                        .unwrap()
                        .join("playback-session.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(saved["queue"].as_array().unwrap().len(), 10_000);
            assert!((saved["volume"].as_f64().unwrap() - 0.19).abs() < 0.0001);
            assert_eq!(saved["position"], 33.5);
        }
        "verify_queue_limit" => {
            let status = player.status();
            assert!(status.state == State::Paused);
            assert_eq!(status.queue.len(), 10_000);
            assert_eq!(status.track.as_ref().map(|track| track.id), Some(10_001));
            assert_eq!(status.index, Some(1));
            assert_eq!(status.queue.last().unwrap().id, 10_002);
            assert_eq!(status.position, 33.5);
            assert_eq!(status.volume, 0.19);
        }
        "periodic_tick" | "periodic_display" => {
            command(&mut player, Action::Play(media(track(1)))).unwrap();
            let path = musicx_core::protocol::data_dir()
                .unwrap()
                .join("playback-session.json");
            let saved_position = || {
                serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap()
                    ["position"]
                    .as_f64()
                    .unwrap()
            };
            PLAYBACK_ELAPSED.store(42.5_f64.to_bits(), Ordering::Relaxed);
            let poll = |player: &mut Player<Audio>| {
                if phase == "periodic_tick" {
                    player.tick();
                } else {
                    command(player, Action::DisplayStatus).unwrap();
                }
            };
            poll(&mut player);
            assert_eq!(
                saved_position(),
                0.0,
                "polling before the interval should not write progress"
            );
            std::thread::sleep(Duration::from_millis(5_100));
            poll(&mut player);
            assert_eq!(
                saved_position(),
                42.5,
                "natural progress must be saved before shutdown"
            );
        }
        "handoff_load_failure" | "retry_failed_handoff" => {
            if phase == "handoff_load_failure" {
                FAIL_LOAD.store(true, Ordering::Relaxed);
                assert!(handoff(&mut player, State::Playing).is_err());
            }
            let status = player.status();
            assert!(status.state == State::Paused);
            assert_eq!(status.track.as_ref().map(|track| track.id), Some(2));
            assert_eq!(status.position, 73.25);
            assert_eq!(
                status.volume, 0.37,
                "failed handoff must preserve checkpoint volume"
            );
            let saved: serde_json::Value = serde_json::from_slice(
                &std::fs::read(
                    musicx_core::protocol::data_dir()
                        .unwrap()
                        .join("playback-session.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert!((saved["volume"].as_f64().unwrap() - 0.37).abs() < 0.0001);
            if phase == "retry_failed_handoff" {
                FAIL_LOAD.store(true, Ordering::Relaxed);
                assert!(handoff(&mut player, State::Playing).is_err());
                FAIL_LOAD.store(false, Ordering::Relaxed);
                assert!(handoff(&mut player, State::Playing).unwrap().state == State::Playing);
            }
        }
        "handoff_after_seek" => {
            action(
                &mut player,
                Action::Seek {
                    seconds: 12.0,
                    track_key: None,
                },
            );
            assert!(handoff(&mut player, State::Paused).is_err());
            assert_eq!(
                player.status().track.as_ref().map(|track| track.id),
                Some(2)
            );
            assert_eq!(player.status().position, 12.0);
            assert_eq!(LOADS.load(Ordering::Relaxed), 0);
        }
        "handoff_during_resume" => {
            action(&mut player, Action::Resume);
            assert!(player.status().switching);
            assert!(handoff(&mut player, State::Paused).is_err());
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            player.handle(receive.recv_timeout(Duration::from_secs(2)).unwrap());
            assert!(player.status().state == State::Playing);
            assert_eq!(
                player.status().track.as_ref().map(|track| track.id),
                Some(2)
            );
            assert_eq!(player.status().position, 73.25);
        }
        _ => panic!("unknown test phase"),
    }
}
