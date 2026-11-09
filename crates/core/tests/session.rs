use anyhow::{Result, bail};
use musicx_core::{
    audio::{AudioOutput, AudioSnapshot, AudioState},
    player::{Action, Message, Player, State},
    protocol::{AlbumPage, AudioQuality, Quality, QueueContext, Status, Track, TrackInfo},
};
use musicx_plugin_api::{DataSource, Media};
use std::{
    cell::Cell,
    process::Command,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

static MESSAGES: OnceLock<mpsc::Sender<Message>> = OnceLock::new();
static LOADS: AtomicUsize = AtomicUsize::new(0);
static FAIL_RESOLVE: AtomicBool = AtomicBool::new(false);

fn dispatch(message: Message) {
    MESSAGES.get().unwrap().send(message).unwrap();
}

struct Source;
static SOURCE: Source = Source;
impl DataSource for Source {
    fn search(&self, _: String, _: u64) -> Result<Vec<Track>> {
        unreachable!()
    }
    fn resolve(&self, track: Track, _: Quality) -> Result<Media> {
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
        *output = Some(Self {
            position: Cell::new(0.0),
            playing: Cell::new(play),
            volume: Cell::new(volume),
        });
        Ok(())
    }
    fn snapshot(&self) -> AudioSnapshot {
        AudioSnapshot {
            position: self.position.get(),
            duration: Some(180.0),
            state: if self.playing.get() {
                AudioState::Playing
            } else {
                AudioState::Paused
            },
            volume: self.volume.get(),
            error: None,
        }
    }
    fn prepare_resume(&self) -> Result<()> {
        Ok(())
    }
    fn play(&self) {
        self.playing.set(true);
    }
    fn pause(&self) {
        self.playing.set(false);
    }
    fn set_volume(&self, volume: f32) {
        self.volume.set(volume);
    }
    fn seek(&self, seconds: f64, complete: impl Fn(bool) + Send + 'static) {
        self.position.set(seconds);
        complete(true);
    }
}

fn track(id: u64) -> Track {
    Track {
        id,
        title: format!("Track {id}"),
        artist: "Test".into(),
        duration: 180.0,
        cover_url: None,
        album: None,
        source_url: None,
        source: None,
    }
}
fn media(track: Track) -> Media {
    Media {
        track,
        url: "https://example.invalid/expiring-audio?secret=not-persisted"
            .parse()
            .unwrap(),
        user_agent: Some("private-agent".into()),
        quality: AudioQuality::default(),
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
                .env("MINI_CLOUDMUSIC_DATA_DIR", &directory)
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
            .env("MINI_CLOUDMUSIC_DATA_DIR", &directory)
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
        .env("MINI_CLOUDMUSIC_DATA_DIR", &directory)
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
            .env("MINI_CLOUDMUSIC_DATA_DIR", &directory)
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
    let mut player = Player::<Audio>::new(dispatch, &SOURCE);
    let action = |player: &mut Player<Audio>, action| player.handle(Message::Action(action, None));
    match phase.as_str() {
        "save" => {
            action(
                &mut player,
                Action::ReplaceQueue {
                    tracks: vec![track(1), track(2)],
                    index: 1,
                    context: QueueContext::History,
                    media: media(track(2)),
                },
            );
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
