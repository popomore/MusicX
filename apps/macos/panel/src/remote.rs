//! The disposable NSH process controls the existing background player.
use crate::protocol::tr;
use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use std::sync::{OnceLock, mpsc};
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
    let token = read_token()?;
    CONNECTION
        .set((client.clone(), token.clone()))
        .map_err(|_| anyhow::anyhow!(tr!("Panel connection already started")))?;
    let (send, receive) = mpsc::channel::<Request>();
    REQUESTS
        .set(send)
        .map_err(|_| anyhow::anyhow!(tr!("Panel client already started")))?;
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
    Ok(response.json()?)
}

fn send(action: Option<UiAction>, reply: Reply) {
    if let Some(sender) = REQUESTS.get() {
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
    // Preparing an entire saved playlist must not delay pause/seek requests.
    if matches!(
        action,
        Some(
            UiAction::PlayList { .. }
                | UiAction::PlayLocalPlaylist { .. }
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
