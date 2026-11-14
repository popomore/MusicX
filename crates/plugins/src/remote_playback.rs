//! Source RPC stays off the native audio thread. Each session owns its controls.
use crate::{mygo, protocol::Track};
use musicx_plugin_api::{RemotePlayback, RemoteSnapshot};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender},
    },
    time::{Duration, Instant},
};

// None means an optional control is unavailable; Some(Null) is a successful RPC.
type Call = Arc<dyn Fn(&str, Value, u64) -> anyhow::Result<Option<Value>> + Send + Sync>;
enum Command {
    Start(bool, f32),
    Play,
    Pause,
    Volume(f32),
    Seek(f64, Box<dyn Fn(bool) + Send>),
    Stop,
}
struct Cache {
    snapshot: RemoteSnapshot,
    at: Instant,
}
impl Cache {
    fn advance(&mut self) {
        if self.snapshot.playing {
            self.snapshot.position = (self.snapshot.position + self.at.elapsed().as_secs_f64())
                .min(self.snapshot.duration.unwrap_or(f64::INFINITY));
        }
        self.at = Instant::now();
    }
}
pub(crate) struct Session {
    send: Sender<Command>,
    cache: Arc<Mutex<Cache>>,
    plugin: String,
    generation: u64,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.send.send(Command::Stop);
    }
}

fn owners() -> &'static Mutex<BTreeMap<String, u64>> {
    static OWNERS: OnceLock<Mutex<BTreeMap<String, u64>>> = OnceLock::new();
    OWNERS.get_or_init(Default::default)
}

impl Session {
    fn new(plugin: String, id: String, duration: f64, call: Call) -> Arc<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let generation = NEXT.fetch_add(1, Ordering::Relaxed);
        let cache = Arc::new(Mutex::new(Cache {
            snapshot: RemoteSnapshot {
                duration: (duration > 0.0).then_some(duration),
                volume: 1.0,
                ..Default::default()
            },
            at: Instant::now(),
        }));
        let state = cache.clone();
        let (send, receive) = mpsc::channel();
        let owner_plugin = plugin.clone();
        std::thread::spawn(move || {
            let call = move |method: &str, args: Value| call(method, args, generation);
            let begin = |position: f64, volume: f32| -> anyhow::Result<bool> {
                let volume_supported = call(
                    "remoteVolume",
                    json!({"percent":(volume*100.0).round() as u32}),
                )?
                .is_some();
                call(
                    "remotePlay",
                    json!({"ids":[id],"start":0,"position":(position*1000.0) as u64}),
                )?;
                Ok(volume_supported)
            };
            let mut started = false;
            let mut playing = false;
            let mut volume = 1.0;
            let mut reported_volume = 1.0;
            let mut desired_position = 0.0;
            let mut owns = false;
            let mut failures = 0u32;
            let owned = || owners().lock().unwrap().get(&plugin).copied() == Some(generation);
            loop {
                let command = match receive
                    .recv_timeout(Duration::from_secs((1u64 << failures.min(4)).min(16)))
                {
                    Ok(command) => Some(command),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(_) => Some(Command::Stop),
                };
                if matches!(command, Some(Command::Stop)) {
                    if owns && owned() {
                        if started {
                            let _ = call("remotePause", json!({}));
                        }
                        let mut owners = owners().lock().unwrap();
                        if owners.get(&plugin) == Some(&generation) {
                            owners.remove(&plugin);
                        }
                    }
                    break;
                }
                // A new source selection must prevent an older session's late pause.
                if owns && !owned() {
                    if let Some(Command::Seek(_, done)) = command {
                        done(false);
                    }
                    continue;
                }
                let mut completion = None;
                let mut unsupported = false;
                let is_command = command.is_some();
                let result = match command {
                    Some(Command::Start(play, value)) => {
                        owns = play;
                        volume = value;
                        playing = play;
                        if play {
                            let result = begin(desired_position, volume);
                            started = result.is_ok();
                            if let Ok(supported) = result {
                                if supported {
                                    reported_volume = volume;
                                } else {
                                    volume = reported_volume;
                                }
                            }
                            result.map(|_| ())
                        } else {
                            Ok(())
                        }
                    }
                    Some(Command::Play) => {
                        owns = true;
                        playing = true;
                        if started {
                            call("remoteResume", json!({})).map(|_| ())
                        } else {
                            let result = begin(desired_position, volume);
                            started = result.is_ok();
                            if let Ok(supported) = result {
                                if supported {
                                    reported_volume = volume;
                                } else {
                                    volume = reported_volume;
                                }
                            }
                            result.map(|_| ())
                        }
                    }
                    Some(Command::Pause) => {
                        playing = false;
                        if started {
                            call("remotePause", json!({})).map(|_| ())
                        } else {
                            Ok(())
                        }
                    }
                    Some(Command::Volume(value)) => {
                        if started {
                            call(
                                "remoteVolume",
                                json!({"percent":(value*100.0).round() as u32}),
                            )
                            .map(|response| {
                                if response.is_some() {
                                    volume = value;
                                    reported_volume = value;
                                } else {
                                    unsupported = true;
                                }
                            })
                        } else {
                            volume = value;
                            Ok(())
                        }
                    }
                    Some(Command::Seek(position, done)) => {
                        completion = Some(done);
                        if started {
                            call("remoteSeek", json!({"position":(position*1000.0) as u64})).map(
                                |response| {
                                    if response.is_some() {
                                        desired_position = position;
                                    } else {
                                        unsupported = true;
                                    }
                                },
                            )
                        } else {
                            desired_position = position;
                            Ok(())
                        }
                    }
                    None if started => call("remoteState", json!({})).and_then(|value| {
                        let value = value.ok_or_else(|| {
                            anyhow::anyhow!(crate::protocol::tr!(
                                "This source does not support this capability"
                            ))
                        })?;
                        let mut cache = state.lock().unwrap();
                        cache.advance();
                        if value.is_null() {
                            if cache.snapshot.position
                                >= cache.snapshot.duration.unwrap_or(f64::INFINITY) - 0.12
                            {
                                cache.snapshot.position =
                                    cache.snapshot.duration.unwrap_or(cache.snapshot.position);
                                cache.snapshot.playing = false;
                            }
                        } else {
                            let remote_id = value["trackId"].as_str().unwrap_or("");
                            if !remote_id.is_empty() && remote_id != id {
                                anyhow::bail!(
                                    "{}",
                                    crate::protocol::tr!(
                                        "The remote device switched to another track"
                                    )
                                );
                            }
                            if let Some(position) = value["position"]
                                .as_f64()
                                .filter(|n| n.is_finite() && *n >= 0.0)
                            {
                                cache.snapshot.position = position / 1000.0;
                            }
                            if let Some(duration) = value["duration"]
                                .as_f64()
                                .filter(|n| n.is_finite() && *n > 0.0)
                            {
                                cache.snapshot.duration = Some(duration / 1000.0);
                            }
                            cache.snapshot.playing =
                                value["playing"].as_bool().unwrap_or(cache.snapshot.playing);
                            cache.snapshot.volume = value["volume"]
                                .as_f64()
                                .filter(|n| n.is_finite())
                                .map(|n| (n / 100.0).clamp(0.0, 1.0) as f32)
                                .unwrap_or(reported_volume);
                            reported_volume = cache.snapshot.volume;
                            volume = reported_volume;
                        }
                        cache.at = Instant::now();
                        Ok(())
                    }),
                    None => Ok(()),
                    Some(Command::Stop) => unreachable!(),
                };
                let ok = result.is_ok();
                let mut cache = state.lock().unwrap();
                cache.snapshot.waiting = false;
                if unsupported {
                    if let Some(done) = completion {
                        drop(cache);
                        done(false);
                    }
                    continue;
                }
                match result {
                    Ok(()) => {
                        failures = 0;
                        cache.snapshot.error = None;
                    }
                    Err(error) => {
                        failures = (failures + 1).min(4);
                        cache.snapshot.error = Some(error.to_string());
                    }
                }
                if let Some(done) = completion {
                    if ok {
                        cache.snapshot.position = desired_position;
                        cache.at = Instant::now();
                    }
                    drop(cache);
                    done(ok);
                } else if is_command {
                    cache.advance();
                    cache.snapshot.playing = playing && ok;
                    cache.snapshot.volume = volume;
                    if ok {
                        cache.at = Instant::now();
                    }
                } else {
                    playing = cache.snapshot.playing;
                }
            }
        });
        Arc::new(Self {
            send,
            cache,
            plugin: owner_plugin,
            generation,
        })
    }
    fn command(&self, command: Command, waiting: Option<bool>) {
        {
            let mut state = self.cache.lock().unwrap();
            if let Some(waiting) = waiting {
                state.snapshot.waiting = waiting;
            }
            if matches!(command, Command::Start(..) | Command::Play) {
                state.snapshot.error = None;
            }
        }
        let _ = self.send.send(command);
    }
}

impl RemotePlayback for Session {
    fn start(&self, play: bool, volume: f32) -> anyhow::Result<()> {
        if play {
            owners()
                .lock()
                .unwrap()
                .insert(self.plugin.clone(), self.generation);
        }
        self.command(Command::Start(play, volume), Some(play));
        Ok(())
    }
    fn snapshot(&self) -> RemoteSnapshot {
        let state = self.cache.lock().unwrap();
        let mut snapshot = state.snapshot.clone();
        if snapshot.playing && !snapshot.waiting {
            snapshot.position = (snapshot.position + state.at.elapsed().as_secs_f64())
                .min(snapshot.duration.unwrap_or(f64::INFINITY));
            if snapshot.duration.is_some_and(|d| snapshot.position >= d) {
                snapshot.playing = false;
            }
        }
        snapshot
    }
    fn play(&self) {
        owners()
            .lock()
            .unwrap()
            .insert(self.plugin.clone(), self.generation);
        self.command(Command::Play, Some(true));
    }
    fn pause(&self) {
        self.command(Command::Pause, Some(false));
    }
    fn set_volume(&self, volume: f32) {
        self.command(Command::Volume(volume.clamp(0.0, 1.0)), None);
    }
    fn seek(&self, seconds: f64, complete: Box<dyn Fn(bool) + Send>) {
        if !seconds.is_finite() || seconds < 0.0 {
            complete(false);
            return;
        }
        self.command(Command::Seek(seconds, complete), Some(true));
    }
}

pub(crate) fn for_track(track: &Track) -> Option<Arc<dyn RemotePlayback>> {
    let source = track.source.as_ref()?;
    let manifest = mygo::manifest(&mygo::plugin_root().ok()?.join(&source.plugin)).ok()?;
    if manifest.playback != "remote" {
        return None;
    }
    let plugin = source.plugin.clone();
    let callback = Arc::new(move |method: &str, args: Value, generation: u64| {
        if matches!(method, "remoteVolume" | "remoteSeek") {
            // This work runs on the session worker, including capability discovery.
            if !crate::music_sources::get_source(&format!("mygo:{plugin}"))?
                .host_capabilities
                .iter()
                .any(|c| c == method)
            {
                return Ok(None);
            }
        }
        mygo::invoke_guarded(&plugin, method, args, || {
            owners().lock().unwrap().get(&plugin) == Some(&generation)
        })
        .map(Some)
    });
    Some(Session::new(
        source.plugin.clone(),
        source.id.clone(),
        track.duration,
        callback,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait(check: impl Fn() -> bool) {
        let until = Instant::now() + Duration::from_secs(3);
        while !check() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(check());
    }

    #[test]
    fn controls_are_queued_and_convert_seconds_to_source_milliseconds() {
        let plugin = format!("remote-{}", musicx_core::storage::random_hex::<8>());
        let log = Arc::new(Mutex::new(Vec::<(String, Value, u64)>::new()));
        let calls = log.clone();
        let (entered, started) = mpsc::channel();
        let (release, continue_start) = mpsc::channel();
        let gate = Mutex::new(continue_start);
        let callback: Call = Arc::new(move |method, args, generation| {
            calls
                .lock()
                .unwrap()
                .push((method.into(), args, generation));
            if method == "remotePlay" {
                let _ = entered.send(());
                let _ = gate.lock().unwrap().recv_timeout(Duration::from_secs(3));
            }
            Ok(Some(if method == "remoteState" {
                json!({"trackId":"song","playing":true,"position":12000,"duration":60000,"volume":40})
            } else {
                Value::Null
            }))
        });
        let session = Session::new(plugin, "song".into(), 60.0, callback);
        session.start(false, 0.7).unwrap();
        let (done, complete) = mpsc::channel();
        session.seek(
            7.0,
            Box::new(move |ok| {
                done.send(ok).unwrap();
            }),
        );
        assert!(complete.recv_timeout(Duration::from_secs(2)).unwrap());
        assert!(
            log.lock().unwrap().is_empty(),
            "loading and seeking a paused session must not control another device"
        );
        session.play();
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        // These controls return while the provider's play request is still blocked.
        session.set_volume(0.4);
        session.pause();
        release.send(()).unwrap();
        wait(|| log.lock().unwrap().iter().any(|c| c.0 == "remotePause"));
        let calls = log.lock().unwrap();
        assert_eq!(calls[0].0, "remoteVolume");
        assert_eq!(calls[0].1, json!({"percent":70}));
        assert_eq!(calls[1].0, "remotePlay");
        assert_eq!(
            calls[1].1,
            json!({"ids":["song"],"start":0,"position":7000})
        );
        assert!(
            calls
                .iter()
                .any(|c| c.0 == "remoteVolume" && c.1["percent"] == 40)
        );
        drop(calls);
        let (done, complete) = mpsc::channel();
        session.seek(
            12.0,
            Box::new(move |ok| {
                let _ = done.send(ok);
            }),
        );
        assert!(complete.recv_timeout(Duration::from_secs(2)).unwrap());
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|c| c.0 == "remoteSeek" && c.1["position"] == 12000)
        );
        session.play();
        wait(|| log.lock().unwrap().iter().any(|c| c.0 == "remoteResume"));
        wait(|| log.lock().unwrap().iter().any(|c| c.0 == "remoteState"));
        let snapshot = session.snapshot();
        assert!((snapshot.position - 12.0).abs() < 2.0);
        assert_eq!(snapshot.duration, Some(60.0));
        assert_eq!(snapshot.volume, 0.4);
    }

    #[test]
    fn failed_start_retries_the_selected_track_instead_of_resuming_another() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let log = calls.clone();
        let attempts = Arc::new(AtomicU64::new(0));
        let tried = attempts.clone();
        let callback: Call = Arc::new(move |method, args, _| {
            log.lock().unwrap().push((method.to_owned(), args));
            if method == "remotePlay" && tried.fetch_add(1, Ordering::Relaxed) == 0 {
                anyhow::bail!("offline");
            }
            Ok(Some(Value::Null))
        });
        let session = Session::new(
            format!("remote-retry-{}", musicx_core::storage::random_hex::<8>()),
            "selected".into(),
            60.0,
            callback,
        );
        session.start(true, 0.0).unwrap();
        wait(|| session.snapshot().error.is_some());
        session.play();
        wait(|| session.snapshot().playing && session.snapshot().error.is_none());
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        assert!(!calls.lock().unwrap().iter().any(|c| c.0 == "remoteResume"));
    }

    #[test]
    fn null_state_retains_elapsed_progress_and_reaches_the_end() {
        let polls = Arc::new(AtomicU64::new(0));
        let observed = polls.clone();
        let callback: Call = Arc::new(move |method, _, _| {
            if method == "remoteState" {
                observed.fetch_add(1, Ordering::Relaxed);
            }
            Ok(Some(Value::Null))
        });
        let session = Session::new(
            format!("remote-clock-{}", musicx_core::storage::random_hex::<8>()),
            "selected".into(),
            10.0,
            callback,
        );
        session.start(true, 0.4).unwrap();
        wait(|| session.snapshot().playing);
        session.cache.lock().unwrap().at = Instant::now() - Duration::from_millis(1500);
        wait(|| {
            polls.load(Ordering::Relaxed) >= 1
                && session.cache.lock().unwrap().at.elapsed() < Duration::from_secs(1)
        });
        assert!(
            session.snapshot().position >= 2.0,
            "a null poll must not reset the interpolated clock"
        );
        session.cache.lock().unwrap().at = Instant::now() - Duration::from_secs(20);
        wait(|| {
            let cache = session.cache.lock().unwrap();
            !cache.snapshot.playing && cache.snapshot.position == 10.0
        });
    }

    #[test]
    fn missing_optional_controls_preserve_playback_volume_and_errors() {
        let volumes = Arc::new(AtomicU64::new(0));
        let seen = volumes.clone();
        let callback: Call = Arc::new(move |method, _, _| {
            if method == "remoteVolume" {
                seen.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
            if method == "remoteSeek" {
                return Ok(None);
            }
            Ok(Some(Value::Null))
        });
        let session = Session::new(
            format!(
                "remote-optional-{}",
                musicx_core::storage::random_hex::<8>()
            ),
            "selected".into(),
            60.0,
            callback,
        );
        session.start(true, 0.0).unwrap();
        wait(|| session.snapshot().playing);
        session.set_volume(0.2);
        wait(|| volumes.load(Ordering::Relaxed) >= 2);
        assert_eq!(session.snapshot().volume, 1.0);
        assert!(session.snapshot().playing && session.snapshot().error.is_none());
        session.cache.lock().unwrap().snapshot.error = Some("previous error".into());
        let (done, complete) = mpsc::channel();
        session.seek(
            5.0,
            Box::new(move |ok| {
                done.send(ok).unwrap();
            }),
        );
        assert!(!complete.recv_timeout(Duration::from_secs(2)).unwrap());
        assert_eq!(session.snapshot().error.as_deref(), Some("previous error"));
        assert!(session.snapshot().playing);
    }

    #[test]
    fn non_seek_controls_do_not_rewind_the_interpolated_clock() {
        let callback: Call = Arc::new(|_, _, _| Ok(Some(Value::Null)));
        let session = Session::new(
            format!(
                "remote-control-clock-{}",
                musicx_core::storage::random_hex::<8>()
            ),
            "selected".into(),
            60.0,
            callback,
        );
        session.start(true, 0.5).unwrap();
        wait(|| session.snapshot().playing);
        session.cache.lock().unwrap().at = Instant::now() - Duration::from_millis(800);
        let before = session.snapshot().position;
        session.set_volume(0.4);
        wait(|| session.snapshot().volume == 0.4);
        assert!(session.snapshot().position >= before);
        session.pause();
        wait(|| !session.snapshot().playing);
        assert!(session.snapshot().position >= before);
    }

    #[test]
    fn an_old_session_cannot_pause_its_replacement() {
        let plugin = format!("remote-owner-{}", musicx_core::storage::random_hex::<8>());
        let log = Arc::new(Mutex::new(Vec::new()));
        let calls = log.clone();
        let key = plugin.clone();
        let callback: Call = Arc::new(move |method, _, generation| {
            ensure_owner(&key, generation)?;
            calls.lock().unwrap().push((method.to_owned(), generation));
            Ok(Some(Value::Null))
        });
        let old = Session::new(plugin.clone(), "old".into(), 60.0, callback.clone());
        old.start(true, 0.5).unwrap();
        wait(|| {
            log.lock()
                .unwrap()
                .iter()
                .any(|c| c.0 == "remotePlay" && c.1 == old.generation)
        });
        let current = Session::new(plugin, "new".into(), 60.0, callback);
        current.start(true, 0.5).unwrap();
        wait(|| {
            log.lock()
                .unwrap()
                .iter()
                .any(|c| c.0 == "remotePlay" && c.1 == current.generation)
        });
        let old_generation = old.generation;
        old.pause();
        drop(old);
        current.pause();
        wait(|| {
            log.lock()
                .unwrap()
                .iter()
                .any(|c| c.0 == "remotePause" && c.1 == current.generation)
        });
        assert!(
            !log.lock()
                .unwrap()
                .iter()
                .any(|c| c.0 == "remotePause" && c.1 == old_generation)
        );
    }
    fn ensure_owner(plugin: &str, generation: u64) -> anyhow::Result<()> {
        anyhow::ensure!(
            owners().lock().unwrap().get(plugin) == Some(&generation),
            "old session"
        );
        Ok(())
    }
}
