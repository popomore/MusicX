//! The disposable NSH process controls the existing background player.
use crate::protocol::tr;
use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::Duration;

use crate::protocol::{Status, Track, UiAction, address, data_dir};
use tokio::sync::oneshot;
pub enum Action {
    DisplayStatus,
    Pause,
    Resume,
    Previous,
    Next,
    Seek(f64),
    #[cfg(target_os = "macos")]
    SeekLyric {
        seconds: f64,
        track_key: String,
    },
    PlayTrack(Track),
    PlayList {
        tracks: Vec<Track>,
        index: usize,
        context: crate::protocol::QueueContext,
    },
    PlayLocalPlaylist {
        id: String,
        index: usize,
    },
    #[cfg(target_os = "macos")]
    PlayQueue {
        index: usize,
        revision: u64,
    },
    #[cfg(target_os = "macos")]
    Quality(crate::protocol::Quality),
    #[cfg(target_os = "macos")]
    Normalization(bool),
}
pub type Reply = Option<oneshot::Sender<Result<Status>>>;
fn read_token() -> Result<String> {
    let directory = data_dir().context(tr!("User directory not found"))?;
    std::fs::read_to_string(directory.join("mcp-token"))
        .context(tr!("Start the background player first"))
}
type Request = (Option<UiAction>, Reply);
static REQUESTS: OnceLock<mpsc::Sender<Request>> = OnceLock::new();
static NAVIGATION: OnceLock<mpsc::Sender<Request>> = OnceLock::new();
static NAVIGATION_WARNING: Mutex<Option<String>> = Mutex::new(None);
static CONNECTION: OnceLock<(Client, String)> = OnceLock::new();

pub fn mcp_endpoint() -> String {
    format!("http://{}/mcp", address())
}

pub fn mcp_token() -> Option<&'static str> {
    CONNECTION.get().map(|(_, token)| token.as_str())
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()?)
}

pub fn start() -> Result<()> {
    if REQUESTS.get().is_some() {
        return Ok(());
    }
    let client = client()?;
    let status_client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    let token = read_token()?;
    CONNECTION
        .set((client.clone(), token.clone()))
        .map_err(|_| anyhow::anyhow!(tr!("Panel connection already started")))?;
    let (send, receive) = mpsc::channel::<Request>();
    REQUESTS
        .set(send)
        .map_err(|_| anyhow::anyhow!(tr!("Panel client already started")))?;
    let (send, navigation) = mpsc::channel::<Request>();
    NAVIGATION
        .set(send)
        .map_err(|_| anyhow::anyhow!(tr!("Track navigation queue already started")))?;
    let navigation_client = client.clone();
    let navigation_token = token.clone();
    std::thread::spawn(move || {
        navigation_requests(
            navigation,
            |action| request(&navigation_client, &navigation_token, action),
            || request(&status_client, &navigation_token, None),
            |warning| *NAVIGATION_WARNING.lock().unwrap() = warning,
            || std::thread::sleep(Duration::from_millis(250)),
        );
    });
    std::thread::spawn(move || {
        for (action, reply) in receive {
            let result = request(&client, &token, action.as_ref());
            let disconnected = result.as_ref().err().is_some_and(|error| {
                error
                    .chain()
                    .any(|cause| cause.downcast_ref::<reqwest::Error>().is_some())
            });
            if let Some(reply) = reply {
                let _ = reply.send(result);
            }
            if disconnected {
                #[cfg(target_os = "macos")]
                {
                    crate::on_disconnected();
                    break;
                }
            }
        }
    });
    Ok(())
}

fn navigation_requests(
    requests: mpsc::Receiver<Request>,
    mut perform: impl FnMut(Option<&UiAction>) -> Result<Status>,
    mut status: impl FnMut() -> Result<Status>,
    mut warn: impl FnMut(Option<String>),
    mut wait: impl FnMut(),
) {
    for (action, reply) in requests {
        let result = perform(action.as_ref());
        let failed = result.is_err();
        if let Some(reply) = reply {
            let _ = reply.send(result);
        }
        // A timeout does not cancel the backend switch. Retain later intents
        // until an independent status request confirms it has finished.
        if failed {
            loop {
                match status() {
                    Ok(status) if !status.switching => break,
                    Ok(_) => warn(None),
                    Err(_) => warn(Some(tr!("Track change outcome is not yet confirmed; later track changes are queued. Please wait.").into())),
                }
                wait();
            }
        }
        warn(None);
    }
}

fn request(client: &Client, token: &str, action: Option<&UiAction>) -> Result<Status> {
    let base = format!("http://{}", address());
    let request = match action {
        Some(action) => {
            let request = client.post(format!("{base}/ui/action")).json(action);
            if matches!(
                action,
                UiAction::PlayLocalPlaylist { .. } | UiAction::PlayList { .. }
            ) {
                request.timeout(Duration::from_secs(180))
            } else {
                request
            }
        }
        None => client.get(format!("{base}/ui/state")),
    };
    let response = request
        .bearer_auth(token.trim())
        .send()
        .context(tr!("Could not connect to the background player"))?;
    if !response.status().is_success() {
        let error: serde_json::Value = response.json()?;
        bail!(
            "{}",
            error["error"]
                .as_str()
                .unwrap_or(tr!("Background request failed"))
        );
    }
    let mut status: Status = response.json()?;
    if action.is_none()
        && let Some(warning) = NAVIGATION_WARNING.lock().unwrap().clone()
    {
        status.error.get_or_insert(warning);
    }
    Ok(status)
}

fn send(action: Option<UiAction>, reply: Reply) {
    let sender = if matches!(action, Some(UiAction::Previous | UiAction::Next)) {
        NAVIGATION.get()
    } else {
        REQUESTS.get()
    };
    if let Some(sender) = sender {
        if let Err(mpsc::SendError((_, Some(reply)))) = sender.send((action, reply)) {
            let _ = reply.send(Err(anyhow::anyhow!(tr!("Background connection closed"))));
        }
    } else if let Some(reply) = reply {
        let _ = reply.send(Err(anyhow::anyhow!(tr!("Panel client not initialized"))));
    }
}

pub fn submit(action: Action, reply: Reply) {
    let action = match action {
        Action::DisplayStatus => None,
        Action::Pause => Some(UiAction::Pause),
        Action::Resume => Some(UiAction::Resume),
        Action::Previous => Some(UiAction::Previous),
        Action::Next => Some(UiAction::Next),
        Action::Seek(seconds) => Some(UiAction::Seek {
            seconds,
            track_key: None,
        }),
        #[cfg(target_os = "macos")]
        Action::SeekLyric { seconds, track_key } => Some(UiAction::Seek {
            seconds,
            track_key: Some(track_key),
        }),
        Action::PlayTrack(track) => Some(UiAction::PlayTrack { track }),
        Action::PlayList {
            tracks,
            index,
            context,
        } => Some(UiAction::PlayList {
            tracks,
            index,
            context,
        }),
        Action::PlayLocalPlaylist { id, index } => Some(UiAction::PlayLocalPlaylist { id, index }),
        #[cfg(target_os = "macos")]
        Action::PlayQueue { index, revision } => Some(UiAction::PlayQueue { index, revision }),
        #[cfg(target_os = "macos")]
        Action::Quality(quality) => Some(UiAction::Quality { quality }),
        #[cfg(target_os = "macos")]
        Action::Normalization(enabled) => Some(UiAction::Normalization { enabled }),
    };
    // Stream resolution must not delay status polling or pause/seek requests.
    if matches!(
        action,
        Some(
            UiAction::PlayTrack { .. }
                | UiAction::PlayList { .. }
                | UiAction::PlayLocalPlaylist { .. }
                | UiAction::PlayQueue { .. }
                | UiAction::Resume
                | UiAction::Quality { .. }
        )
    ) {
        std::thread::spawn(move || {
            let result = CONNECTION
                .get()
                .context(tr!("Panel connection not started"))
                .and_then(|(client, token)| request(client, token, action.as_ref()));
            if let Some(reply) = reply {
                let _ = reply.send(result);
            }
        });
    } else {
        send(action, reply);
    }
}

#[cfg(target_os = "macos")]
pub fn quit_background() {
    // Keep the UI's main loop alive until the request is sent. If the
    // backend has already gone, the UI still closes after this bounded wait.
    std::thread::spawn(|| {
        if let (Ok(client), Ok(token)) = (
            Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build(),
            read_token(),
        ) {
            let _ = request(&client, &token, Some(&UiAction::Quit));
        }
        crate::on_disconnected();
    });
}

/// Catalog requests use their own short-lived worker so lyrics never delay pause/seek.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T> {
    let (client, token) = CONNECTION
        .get()
        .context(tr!("Panel connection not started"))?;
    let response = client
        .get(format!("http://{}{path}", address()))
        .bearer_auth(token.trim())
        .send()
        .context(tr!("Background connection failed"))?;
    if !response.status().is_success() {
        let error: serde_json::Value = response.json()?;
        bail!(
            "{}",
            error["error"].as_str().unwrap_or(tr!("Request failed"))
        );
    }
    Ok(response.json()?)
}

/// One bounded feed per mounted spectrum. Separate requests keep transport
/// actions independent; dropping the receiver ends polling on the next reply.
#[cfg(target_os = "macos")]
pub fn spectrum_feed() -> Result<mpsc::Receiver<Result<crate::protocol::spectrum::Spectrum>>> {
    let (_, token) = CONNECTION
        .get()
        .context(tr!("Panel connection not started"))?;
    let token = token.clone();
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(400))
        .build()?;
    let (send, receive) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("musicx-spectrum-feed".into())
        .spawn(move || {
            loop {
                let result = client
                    .get(format!("http://{}/ui/spectrum", address()))
                    .bearer_auth(token.trim())
                    .send()
                    .and_then(|response| response.error_for_status())
                    .and_then(|response| response.json())
                    .map_err(anyhow::Error::from);
                match send.try_send(result) {
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                    Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        })?;
    Ok(receive)
}

pub fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T> {
    let (client, token) = CONNECTION
        .get()
        .context(tr!("Panel connection not started"))?;
    let response = client
        .post(format!("http://{}{path}", address()))
        .timeout(Duration::from_secs(180))
        .bearer_auth(token.trim())
        .json(body)
        .send()
        .context(tr!("Background connection failed"))?;
    if !response.status().is_success() {
        let error: serde_json::Value = response.json()?;
        bail!(
            "{}",
            error["error"].as_str().unwrap_or(tr!("Request failed"))
        );
    }
    Ok(response.json()?)
}

pub fn cancel_source_login(job: String) {
    if let Some((client, token)) = CONNECTION.get() {
        let _ = client
            .post(format!("http://{}/ui/music-sources", address()))
            .timeout(Duration::from_secs(2))
            .bearer_auth(token.trim())
            .json(&crate::protocol::MusicSourceAction::Cancel { job })
            .send();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn navigation_intents_wait_for_the_previous_reply_in_fifo_order() {
        let (send, receive) = mpsc::channel();
        let (entered, calls) = mpsc::channel();
        let (finish, unfinished) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut first = true;
            navigation_requests(
                receive,
                |action| {
                    entered
                        .send(match action {
                            Some(UiAction::Next) => "next",
                            Some(UiAction::Previous) => "previous",
                            _ => panic!("unexpected navigation"),
                        })
                        .unwrap();
                    if first {
                        first = false;
                        unfinished.recv_timeout(Duration::from_secs(2)).unwrap();
                    }
                    Ok(Status::default())
                },
                || panic!("successful requests must not need a recovery status check"),
                |_| {},
                || panic!("successful requests must not wait"),
            );
        });
        let (first, first_reply) = oneshot::channel();
        let (second, mut second_reply) = oneshot::channel();
        send.send((Some(UiAction::Next), Some(first))).unwrap();
        send.send((Some(UiAction::Previous), Some(second))).unwrap();
        drop(send);
        assert_eq!(calls.recv_timeout(Duration::from_secs(2)).unwrap(), "next");
        assert!(matches!(
            second_reply.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(matches!(calls.try_recv(), Err(mpsc::TryRecvError::Empty)));
        finish.send(()).unwrap();
        assert!(first_reply.blocking_recv().unwrap().is_ok());
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(2)).unwrap(),
            "previous"
        );
        assert!(second_reply.blocking_recv().unwrap().is_ok());
        worker.join().unwrap();
    }

    #[test]
    fn failed_navigation_retains_later_intents_until_switch_outcome_is_known() {
        let (send, receive) = mpsc::channel();
        let (first, first_reply) = oneshot::channel();
        let (second, second_reply) = oneshot::channel();
        send.send((Some(UiAction::Next), Some(first))).unwrap();
        send.send((Some(UiAction::Next), Some(second))).unwrap();
        drop(send);
        let calls = Cell::new(0);
        let polls = Cell::new(0);
        let waits = Cell::new(0);
        let warnings = RefCell::new(Vec::new());
        navigation_requests(
            receive,
            |_| {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    bail!("request timed out");
                }
                assert_eq!(
                    polls.get(),
                    3,
                    "later intent was sent before switching ended"
                );
                Ok(Status::default())
            },
            || {
                assert_eq!(calls.get(), 1, "queued intent must remain unsent");
                polls.set(polls.get() + 1);
                match polls.get() {
                    1 => bail!("status unavailable"),
                    2 => Ok(Status {
                        switching: true,
                        ..Status::default()
                    }),
                    3 => Ok(Status::default()),
                    _ => panic!("unexpected status check"),
                }
            },
            |warning| warnings.borrow_mut().push(warning),
            || waits.set(waits.get() + 1),
        );
        assert!(first_reply.blocking_recv().unwrap().is_err());
        assert!(second_reply.blocking_recv().unwrap().is_ok());
        assert_eq!(calls.get(), 2);
        assert_eq!(waits.get(), 2);
        assert_eq!(
            warnings.borrow()[0].as_deref(),
            Some(tr!(
                "Track change outcome is not yet confirmed; later track changes are queued. Please wait."
            ))
        );
        assert!(warnings.borrow().last().unwrap().is_none());
    }
}
