use anyhow::{Result, bail};
use musicx_core::{
    audio::{AudioOutput, AudioSnapshot, AudioState},
    player::{Action, Message, Player, State},
    protocol::{AlbumPage, AudioQuality, Quality, QueueContext, Track, TrackInfo},
};
use musicx_plugin_api::{DataSource, Media};
use std::{
    cell::Cell,
    process::Command,
    sync::{
        OnceLock,
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
        _ => panic!("unknown test phase"),
    }
}
