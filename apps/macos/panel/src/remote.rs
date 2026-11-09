//! The disposable NSH process controls the existing background player.
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
    #[cfg(target_os = "macos")]
    PlayList {
        tracks: Vec<Track>,
        index: usize,
        context: crate::protocol::QueueContext,
    },
    #[cfg(target_os = "macos")]
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
    #[cfg_attr(target_os = "macos", allow(dead_code))] // Shared phone link action.
    PlayId(u64),
}
pub type Reply = Option<oneshot::Sender<Result<Status>>>;
fn read_token() -> Result<String> {
    let directory = data_dir().context("找不到用户目录")?;
    std::fs::read_to_string(directory.join("mcp-token")).context("请先启动后台播放器")
}
type Request = (Option<UiAction>, Reply);
static REQUESTS: OnceLock<mpsc::Sender<Request>> = OnceLock::new();
static CONNECTION: OnceLock<(Client, String)> = OnceLock::new();

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
        .map_err(|_| anyhow::anyhow!("界面连接已启动"))?;
    let (send, receive) = mpsc::channel::<Request>();
    REQUESTS
        .set(send)
        .map_err(|_| anyhow::anyhow!("界面客户端已经启动"))?;
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
            if matches!(action, UiAction::PlayPlaylist { .. }) {
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
        .context("后台播放器连接失败")?;
    if !response.status().is_success() {
        let error: serde_json::Value = response.json()?;
        bail!("{}", error["error"].as_str().unwrap_or("后台请求失败"));
    }
    Ok(response.json()?)
}

fn send(action: Option<UiAction>, reply: Reply) {
    if let Some(sender) = REQUESTS.get() {
        if let Err(mpsc::SendError((_, Some(reply)))) = sender.send((action, reply)) {
            let _ = reply.send(Err(anyhow::anyhow!("后台连接已经结束")));
        }
    } else if let Some(reply) = reply {
        let _ = reply.send(Err(anyhow::anyhow!("界面客户端未初始化")));
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
        #[cfg(target_os = "macos")]
        Action::PlayList {
            tracks,
            index,
            context,
        } => Some(UiAction::PlayList {
            tracks,
            index,
            context,
        }),
        #[cfg(target_os = "macos")]
        Action::PlayLocalPlaylist { id, index } => Some(UiAction::PlayLocalPlaylist { id, index }),
        #[cfg(target_os = "macos")]
        Action::PlayQueue { index, revision } => Some(UiAction::PlayQueue { index, revision }),
        #[cfg(target_os = "macos")]
        Action::Quality(quality) => Some(UiAction::Quality { quality }),
        Action::PlayId(id) => Some(UiAction::Play { id }),
    };
    // Preparing an entire saved playlist must not delay pause/seek requests.
    if matches!(
        action,
        Some(
            UiAction::PlayList { .. }
                | UiAction::PlayPlaylist { .. }
                | UiAction::PlayLocalPlaylist { .. }
                | UiAction::Quality { .. }
        )
    ) {
        std::thread::spawn(move || {
            let result = CONNECTION
                .get()
                .context("界面连接尚未启动")
                .and_then(|(client, token)| request(client, token, action.as_ref()));
            if let Some(reply) = reply {
                let _ = reply.send(result);
            }
        });
    } else {
        send(action, reply);
    }
}

/// Accept a numeric ID or a canonical NetEase song link, without fetching pasted URLs.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn song_id(input: &str) -> Result<u64> {
    let input = input.trim();
    let id = if input.chars().all(|c| c.is_ascii_digit()) {
        input.parse::<u64>()?
    } else {
        let url = reqwest::Url::parse(input).context("请粘贴完整的网易云歌曲链接或歌曲 ID")?;
        anyhow::ensure!(
            url.scheme() == "https" && url.host_str() == Some("music.163.com"),
            "目前支持 music.163.com 的完整歌曲链接"
        );
        let song = match url.fragment() {
            Some(fragment) => reqwest::Url::parse(&format!(
                "https://music.163.com/{}",
                fragment.trim_start_matches('/')
            ))?,
            None => url,
        };
        anyhow::ensure!(
            song.path().trim_end_matches('/') == "/song",
            "请使用歌曲链接"
        );
        song.query_pairs()
            .find_map(|(key, value)| (key == "id").then(|| value.parse::<u64>().ok()).flatten())
            .context("链接缺少歌曲 ID")?
    };
    anyhow::ensure!(id > 0, "歌曲 ID 必须大于 0");
    Ok(id)
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
    let (client, token) = CONNECTION.get().context("界面连接尚未启动")?;
    let response = client
        .get(format!("http://{}{path}", address()))
        .bearer_auth(token.trim())
        .send()
        .context("后台连接失败")?;
    if !response.status().is_success() {
        let error: serde_json::Value = response.json()?;
        bail!("{}", error["error"].as_str().unwrap_or("请求失败"));
    }
    Ok(response.json()?)
}

pub fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T> {
    let (client, token) = CONNECTION.get().context("界面连接尚未启动")?;
    let response = client
        .post(format!("http://{}{path}", address()))
        .timeout(Duration::from_secs(180))
        .bearer_auth(token.trim())
        .json(body)
        .send()
        .context("后台连接失败")?;
    if !response.status().is_success() {
        let error: serde_json::Value = response.json()?;
        bail!("{}", error["error"].as_str().unwrap_or("请求失败"));
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
mod link_tests {
    use super::song_id;
    #[test]
    fn accepts_both_shared_link_forms_and_numeric_ids() {
        for link in [
            "29723011",
            "https://music.163.com/#/song?id=29723011",
            "https://music.163.com/song?id=29723011&userid=1",
        ] {
            assert_eq!(song_id(link).unwrap(), 29723011);
        }
    }
    #[test]
    fn rejects_other_resources_and_hosts() {
        for link in [
            "0",
            "https://example.com/song?id=1",
            "https://music.163.com/playlist?id=1",
            "https://music.163.com/#/album?id=1",
        ] {
            assert!(song_id(link).is_err(), "{link}");
        }
    }
}
