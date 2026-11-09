use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rmcp::{
    ErrorData, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
    },
};
use serde::{Deserialize, Serialize};
use std::{fs, net::TcpListener, sync::Arc, time::Duration};
use tokio::sync::oneshot;

use crate::{
    netease,
    player::{Action, Status, submit},
};

use crate::protocol::address;

pub fn read_token() -> Result<String> {
    fs::read_to_string(netease::state_dir()?.join("mcp-token"))
        .context("请先启动播放器，再运行 mini-cloudmusic mcp-token")
}

async fn authorize(State(expected): State<Arc<String>>, request: Request, next: Next) -> Response {
    if request.headers().contains_key(header::ORIGIN) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        != Some(expected.as_str())
    {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response();
    }
    next.run(request).await
}

pub fn start() -> Result<()> {
    let address = address();
    let listener = TcpListener::bind(address).context("无法监听 MCP，本机可能已启动播放器")?;
    crate::mygo::ensure_netease()?;
    listener.set_nonblocking(true)?;
    let path = netease::state_dir()?.join("mcp-token");
    let token = match fs::read_to_string(&path) {
        Ok(token) => token,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let token = netease::random_hex::<32>();
            netease::save_private("mcp-token", &token)?;
            token
        }
        Err(error) => return Err(error.into()),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(4)
        .thread_keep_alive(Duration::from_secs(5))
        .enable_all()
        .build()?;
    let auth = Arc::new(format!("Bearer {token}"));
    std::thread::spawn(move || {
        let result = runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener)?;
            let service = StreamableHttpService::new(
                move || {
                    Ok(PlayerTools {
                        tool_router: PlayerTools::tool_router(),
                    })
                },
                Arc::new(NeverSessionManager::default()),
                StreamableHttpServerConfig::default()
                    .with_stateful_mode(false)
                    .with_json_response(true),
            );
            let app = Router::new()
                .nest_service("/mcp", service)
                .route("/ui/state", get(ui_state))
                .route("/ui/info", get(ui_info))
                .route("/ui/album", get(ui_album))
                .route("/ui/history", get(ui_history))
                .route("/ui/queue", get(ui_queue))
                .route("/ui/action", post(ui_action))
                .route("/ui/session/restore", post(ui_restore))
                .merge(crate::account_ui::routes())
                .merge(crate::local_library::routes())
                .merge(crate::music_sources::routes())
                .layer(middleware::from_fn_with_state(auth, authorize));
            axum::serve(listener, app).await
        });
        if let Err(error) = result {
            eprintln!("MCP 服务退出：{error}");
        }
    });
    eprintln!("Streamable HTTP MCP：http://{address}/mcp");
    Ok(())
}

// The panel's private loopback transport shares the exact same bearer and
// Origin checks as MCP. It creates no extra MCP sessions or response caches.
type UiResult = std::result::Result<Json<Status>, (StatusCode, Json<serde_json::Value>)>;
fn ui_error(error: impl std::fmt::Display) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": error.to_string()})),
    )
}
async fn ui_player(action: Action) -> UiResult {
    ui_status(action, false).await
}
async fn ui_queue() -> UiResult {
    ui_status(Action::Status, true).await
}
async fn ui_status(action: Action, include_queue: bool) -> UiResult {
    let (send, receive) = oneshot::channel();
    submit(action, Some(send));
    let status = tokio::time::timeout(Duration::from_secs(28), receive)
        .await
        .map_err(|_| ui_error("播放器响应超时"))?
        .map_err(|_| ui_error("播放器已经退出"))?
        .map_err(ui_error)?;
    let mut status = status;
    if !include_queue {
        status.queue = Arc::default();
    }
    Ok(Json(status))
}
async fn ui_state() -> UiResult {
    ui_player(Action::DisplayStatus).await
}
async fn current_track() -> Result<netease::Track, (StatusCode, Json<serde_json::Value>)> {
    ui_state()
        .await?
        .0
        .track
        .ok_or_else(|| ui_error("还没有播放歌曲"))
}
async fn ui_info() -> Result<Json<crate::protocol::TrackInfo>, (StatusCode, Json<serde_json::Value>)>
{
    let track = current_track().await?;
    let info = tokio::task::spawn_blocking(move || {
        let info = crate::catalog::info(track.clone());
        if info.track != track {
            submit(Action::MergeTrackMetadata(info.track.clone()), None);
        }
        info
    })
    .await
    .map_err(ui_error)?;
    Ok(Json(info))
}
async fn ui_album()
-> Result<Json<crate::protocol::AlbumPage>, (StatusCode, Json<serde_json::Value>)> {
    let track = current_track().await?;
    Ok(Json(
        tokio::task::spawn_blocking(move || crate::catalog::album(track))
            .await
            .map_err(ui_error)?
            .map_err(|_| ui_error("当前来源暂时无法提供这张专辑的曲目，可稍后重试"))?,
    ))
}
async fn ui_history()
-> Result<Json<Vec<crate::protocol::HistoryEntry>>, (StatusCode, Json<serde_json::Value>)> {
    Ok(Json(
        tokio::task::spawn_blocking(|| crate::history::recent(50))
            .await
            .map_err(ui_error)?
            .map_err(ui_error)?,
    ))
}
async fn ui_action(Json(action): Json<crate::protocol::UiAction>) -> UiResult {
    use crate::protocol::UiAction;
    let action = match action {
        UiAction::Play { id } => Action::Play(
            tokio::task::spawn_blocking(move || netease::resolve(id))
                .await
                .map_err(ui_error)?
                .map_err(ui_error)?,
        ),
        UiAction::PlayTrack { track } => Action::Play(
            tokio::task::spawn_blocking(move || crate::catalog::resolve(track))
                .await
                .map_err(ui_error)?
                .map_err(ui_error)?,
        ),
        UiAction::PlayList {
            tracks,
            index,
            context,
        } => {
            validate_list(&tracks, index).map_err(ui_error)?;
            let track = tracks[index].clone();
            let media = tokio::task::spawn_blocking(move || crate::catalog::resolve(track))
                .await
                .map_err(ui_error)?
                .map_err(ui_error)?;
            Action::ReplaceQueue {
                tracks,
                index,
                context,
                media,
            }
        }
        UiAction::PlayPlaylist { id, track_id } => tokio::task::spawn_blocking(move || {
            let page = netease::playlist_queue(id)?;
            let index = page
                .tracks
                .iter()
                .position(|t| t.id == track_id)
                .ok_or_else(|| anyhow::anyhow!("所选歌曲已不在歌单中，请刷新歌单"))?;
            validate_list(&page.tracks, index)?;
            let media = crate::catalog::resolve(page.tracks[index].clone())?;
            Ok::<_, anyhow::Error>(Action::ReplaceQueue {
                tracks: page.tracks,
                index,
                media,
                context: crate::protocol::QueueContext::Playlist {
                    id,
                    name: page.name,
                },
            })
        })
        .await
        .map_err(ui_error)?
        .map_err(ui_error)?,
        UiAction::PlayLocalPlaylist { id, index } => {
            tokio::task::spawn_blocking(move || local_queue(id, index))
                .await
                .map_err(ui_error)?
                .map_err(ui_error)?
        }
        UiAction::PlayQueue { index, revision } => Action::Select { index, revision },
        UiAction::Quality { quality } => Action::Quality(quality),
        UiAction::Pause => Action::Pause,
        UiAction::Resume => Action::Resume,
        UiAction::Toggle => Action::Toggle,
        UiAction::Previous => Action::Previous,
        UiAction::Next => Action::Next,
        UiAction::Seek { seconds, track_key } => Action::Seek { seconds, track_key },
        UiAction::Volume { value } => Action::Volume(value),
        UiAction::Quit => {
            let result = ui_state().await;
            crate::quit();
            return result;
        }
    };
    ui_player(action).await
}

fn local_queue(id: String, index: usize) -> Result<Action> {
    let list = crate::local_library::playlist(&id)?;
    validate_list(&list.tracks, index)?;
    let media = crate::catalog::resolve(list.tracks[index].clone())?;
    Ok(Action::ReplaceQueue {
        tracks: list.tracks,
        index,
        media,
        context: crate::protocol::QueueContext::LocalPlaylist {
            id,
            name: list.name,
        },
    })
}

fn validate_list(tracks: &[crate::protocol::Track], index: usize) -> Result<()> {
    anyhow::ensure!(
        !tracks.is_empty() && tracks.len() <= 10000,
        "播放列表应包含1至10000首歌"
    );
    anyhow::ensure!(index < tracks.len(), "无效的播放位置");
    anyhow::ensure!(
        tracks.iter().all(|t| !t.title.is_empty()
            && t.title.len() <= 2048
            && t.artist.len() <= 4096
            && t.duration.is_finite()
            && t.duration >= 0.0),
        "无效的曲目信息"
    );
    Ok(())
}

// Local upgrade handoff: authenticated like all UI routes, and only accepted by
// an empty player. It carries metadata, never expiring audio URLs or credentials.
async fn ui_restore(Json(snapshot): Json<Status>) -> UiResult {
    validate_snapshot(&snapshot).map_err(ui_error)?;
    let track = snapshot.index.map(|index| snapshot.queue[index].clone());
    let quality = snapshot.preferred_quality;
    // A paused upgrade must survive offline/expired plugin authentication.
    // Resolve on explicit resume, retaining queue, seek and volume on failure.
    let deferred = snapshot.state == crate::protocol::State::Paused;
    let media = tokio::task::spawn_blocking(move || {
        track
            .filter(|_| !deferred)
            .map(|t| crate::catalog::resolve_with_quality(t, quality))
            .transpose()
    })
    .await
    .map_err(ui_error)?
    .map_err(ui_error)?;
    ui_player(Action::Restore {
        snapshot: Box::new(snapshot),
        media,
    })
    .await
}

fn validate_snapshot(snapshot: &Status) -> Result<()> {
    anyhow::ensure!(snapshot.queue.len() <= 10000, "队列过长");
    anyhow::ensure!(
        snapshot
            .index
            .is_none_or(|index| index < snapshot.queue.len()),
        "无效的队列位置"
    );
    anyhow::ensure!(
        snapshot.queue.is_empty() == snapshot.index.is_none(),
        "会话缺少当前曲目"
    );
    anyhow::ensure!(
        (0.0..=1.0).contains(&snapshot.volume)
            && snapshot.position.is_finite()
            && snapshot.position >= 0.0,
        "无效的音量或进度"
    );
    anyhow::ensure!(
        snapshot.queue.iter().all(|track| !track.title.is_empty()
            && track.title.len() <= 2048
            && track.artist.len() <= 4096
            && track.duration.is_finite()
            && track.duration >= 0.0),
        "无效的曲目信息"
    );
    Ok(())
}

#[derive(Clone)]
struct PlayerTools {
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Search {
    query: String,
    limit: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PluginSearch {
    plugin: String,
    query: String,
    offset: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PluginSong {
    plugin: String,
    id: String,
    name: String,
    artist: Option<String>,
    duration_ms: Option<u64>,
    enqueue: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Song {
    id: u64,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Page {
    offset: Option<u64>,
    limit: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct HistoryPage {
    limit: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PlaylistPage {
    id: u64,
    offset: Option<u64>,
    limit: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ControlAction {
    Pause,
    Resume,
    Previous,
    Next,
    Quality,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Control {
    action: ControlAction,
    quality: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Seek {
    seconds: f64,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Volume {
    volume: f32,
}

#[derive(Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum LibraryOperation {
    List,
    Get,
    Create,
    Rename,
    Pin,
    Unpin,
    AddCurrent,
    AddQueue,
    Remove,
    Delete,
    ImportNetease,
    Play,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct LibraryInput {
    action: LibraryOperation,
    playlist_id: Option<String>,
    name: Option<String>,
    index: Option<usize>,
    revision: Option<u64>,
    netease_playlist_id: Option<u64>,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
struct SourceInput {
    action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    values: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    job: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    playlist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<String>,
}

fn tool_error(error: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

fn output(value: impl Serialize) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![Content::json(value)?]))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T, ErrorData> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(tool_error)?
        .map_err(tool_error)
}

impl PlayerTools {
    async fn player(&self, action: Action) -> Result<CallToolResult, ErrorData> {
        let (send, receive) = oneshot::channel();
        submit(action, Some(send));
        let status: Status = tokio::time::timeout(Duration::from_secs(28), receive)
            .await
            .map_err(|_| tool_error("播放器响应超时"))?
            .map_err(|_| tool_error("播放器已退出"))?
            .map_err(tool_error)?;
        output(status)
    }
}

#[tool_router]
impl PlayerTools {
    #[tool(
        description = "管理音乐数据源。action=list/catalog/get/install/configure/account/sign_in/poll/cancel/sign_out/playlists/import_playlist。插件source为mygo:ID；install用目录仓库owner/repo。configure的values按声明字段保存，敏感值不回显；sign_in返回扫码任务，poll/cancel用job。导入仅建本地副本，不回写来源歌单。"
    )]
    async fn manage_music_sources(
        &self,
        Parameters(input): Parameters<SourceInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let action: crate::protocol::MusicSourceAction =
            serde_json::from_value(serde_json::to_value(input).map_err(tool_error)?)
                .map_err(|_| tool_error("无效的数据源操作参数"))?;
        blocking(move || crate::music_sources::execute(action))
            .await
            .and_then(output)
    }

    #[tool(
        description = "管理播放器本地歌单，与网易云歌单独立。action=list/get/create/rename/pin/unpin/add_current/add_queue/remove/delete/import_netease/play。pin和unpin需要playlist_id。歌曲按来源去重，播放为独立快照；导入创建本地副本，不修改第三方歌单。remove需index和当前revision。"
    )]
    async fn manage_local_playlists(
        &self,
        Parameters(input): Parameters<LibraryInput>,
    ) -> Result<CallToolResult, ErrorData> {
        use crate::protocol::LocalLibraryAction as Edit;
        let id = || {
            input
                .playlist_id
                .clone()
                .context("需要playlist_id")
                .map_err(tool_error)
        };
        let title = || input.name.clone().context("需要name").map_err(tool_error);
        if matches!(input.action, LibraryOperation::List) {
            return blocking(crate::local_library::lists).await.and_then(output);
        }
        if matches!(input.action, LibraryOperation::Get) {
            let id = id()?;
            return blocking(move || crate::local_library::playlist(&id))
                .await
                .and_then(output);
        }
        if matches!(input.action, LibraryOperation::Play) {
            let id = id()?;
            let index = input.index.unwrap_or(0);
            return self
                .player(blocking(move || local_queue(id, index)).await?)
                .await;
        }
        let edit = match input.action {
            LibraryOperation::Create => Edit::Create { name: title()? },
            LibraryOperation::Rename => Edit::Rename {
                id: id()?,
                name: title()?,
            },
            LibraryOperation::Pin | LibraryOperation::Unpin => Edit::SetPinned {
                id: id()?,
                pinned: matches!(input.action, LibraryOperation::Pin),
            },
            LibraryOperation::Delete => Edit::Delete { id: id()? },
            LibraryOperation::Remove => Edit::Remove {
                id: id()?,
                index: input.index.context("需要index").map_err(tool_error)?,
                revision: input.revision.context("需要revision").map_err(tool_error)?,
            },
            LibraryOperation::ImportNetease => Edit::ImportNetease {
                id: input
                    .netease_playlist_id
                    .context("需要netease_playlist_id")
                    .map_err(tool_error)?,
            },
            LibraryOperation::AddCurrent | LibraryOperation::AddQueue => {
                let Json(snapshot) = ui_queue().await.map_err(|(_, v)| tool_error(v.0))?;
                let tracks = if matches!(input.action, LibraryOperation::AddCurrent) {
                    vec![snapshot.track.context("当前没有歌曲").map_err(tool_error)?]
                } else {
                    snapshot.queue.as_ref().clone()
                };
                Edit::Add { id: id()?, tracks }
            }
            _ => unreachable!(),
        };
        blocking(move || crate::local_library::change(edit))
            .await
            .and_then(output)
    }

    #[tool(description = "列出本机安装的 MyGo Music 插件和目录兼容状态。")]
    async fn list_music_plugins(&self) -> Result<CallToolResult, ErrorData> {
        blocking(crate::mygo::list).await.and_then(output)
    }

    #[tool(description = "用 MyGo Music 插件搜索歌曲。返回插件曲目 ID 和毫秒时长。")]
    async fn plugin_search(
        &self,
        Parameters(input): Parameters<PluginSearch>,
    ) -> Result<CallToolResult, ErrorData> {
        let query = input.query.trim();
        if query.is_empty() || query.chars().count() > 120 {
            return Err(tool_error("请输入 1 到 120 个字符"));
        }
        let args =
            serde_json::json!({"query":query,"kind":"tracks","offset":input.offset.unwrap_or(0)});
        blocking(move || {
            let result = crate::mygo::invoke(&input.plugin, "search", args)?;
            crate::mygo::remember_artwork(&input.plugin, &result);
            Ok(result)
        })
        .await
        .and_then(output)
    }

    #[tool(
        description = "播放或排队 MyGo Music 插件曲目。使用 plugin_search 返回的原始 ID、名称、艺人及毫秒时长。"
    )]
    async fn plugin_play(
        &self,
        Parameters(input): Parameters<PluginSong>,
    ) -> Result<CallToolResult, ErrorData> {
        if input.id.is_empty()
            || input.id.len() > 512
            || input.name.is_empty()
            || input.name.len() > 512
            || input
                .artist
                .as_ref()
                .is_some_and(|artist| artist.len() > 1024)
        {
            return Err(tool_error("无效的插件曲目"));
        }
        let track = netease::Track {
            id: input.id.parse().unwrap_or(0),
            title: input.name,
            artist: input.artist.unwrap_or_default(),
            duration: input.duration_ms.unwrap_or(0) as f64 / 1000.0,
            cover_url: None,
            album: None,
            source_url: None,
            source: Some(crate::mygo::Source {
                plugin: input.plugin,
                id: input.id,
            }),
        };
        let media = blocking(move || crate::mygo::resolve(track)).await?;
        self.player(if input.enqueue.unwrap_or(false) {
            Action::Enqueue(media)
        } else {
            Action::Play(media)
        })
        .await
    }

    #[tool(description = "按歌名或艺人搜索网易云单曲。返回曲目 ID。")]
    async fn search_songs(
        &self,
        Parameters(input): Parameters<Search>,
    ) -> Result<CallToolResult, ErrorData> {
        let tracks =
            blocking(move || netease::search(input.query, input.limit.unwrap_or(8))).await?;
        output(tracks)
    }

    #[tool(description = "列出当前登录账号创建和收藏的歌单。支持 offset 和 limit。")]
    async fn my_playlists(
        &self,
        Parameters(input): Parameters<Page>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(move || {
            netease::my_playlists(input.offset.unwrap_or(0), input.limit.unwrap_or(30))
        })
        .await
        .and_then(output)
    }

    #[tool(description = "按歌单 ID 读取曲目，按歌单原顺序分页返回。")]
    async fn playlist_tracks(
        &self,
        Parameters(input): Parameters<PlaylistPage>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(move || {
            netease::playlist_tracks(
                input.id,
                input.offset.unwrap_or(0),
                input.limit.unwrap_or(30),
            )
        })
        .await
        .and_then(output)
    }

    #[tool(description = "立即播放指定网易云曲目。")]
    async fn play_song(
        &self,
        Parameters(input): Parameters<Song>,
    ) -> Result<CallToolResult, ErrorData> {
        let media = blocking(move || netease::resolve(input.id)).await?;
        self.player(Action::Play(media)).await
    }

    #[tool(description = "将指定网易云曲目加入队列。队列为空时立即播放。")]
    async fn enqueue_song(
        &self,
        Parameters(input): Parameters<Song>,
    ) -> Result<CallToolResult, ErrorData> {
        let media = blocking(move || netease::resolve(input.id)).await?;
        self.player(Action::Enqueue(media)).await
    }

    #[tool(description = "读取本机最近播放过的曲目，按时间倒序，最多 100 条。")]
    async fn playback_history(
        &self,
        Parameters(input): Parameters<HistoryPage>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(move || crate::history::recent(input.limit.unwrap_or(20)))
            .await
            .and_then(output)
    }

    #[tool(description = "读取当前曲目、播放状态、进度、音量和队列。")]
    async fn player_status(&self) -> Result<CallToolResult, ErrorData> {
        self.player(Action::Status).await
    }

    #[tool(
        description = "暂停、继续、上一首或下一首。action=quality时用quality=standard/high/lossless/hires选择音质，保持曲目与进度。player_status返回偏好和实际音质参数。"
    )]
    async fn control_player(
        &self,
        Parameters(input): Parameters<Control>,
    ) -> Result<CallToolResult, ErrorData> {
        self.player(match input.action {
            ControlAction::Pause => Action::Pause,
            ControlAction::Resume => Action::Resume,
            ControlAction::Previous => Action::Previous,
            ControlAction::Next => Action::Next,
            ControlAction::Quality => Action::Quality(
                input
                    .quality
                    .as_deref()
                    .unwrap_or("")
                    .parse()
                    .map_err(tool_error)?,
            ),
        })
        .await
    }

    #[tool(description = "跳转到当前曲目的指定秒数。")]
    async fn seek_player(
        &self,
        Parameters(input): Parameters<Seek>,
    ) -> Result<CallToolResult, ErrorData> {
        self.player(Action::Seek {
            seconds: input.seconds,
            track_key: None,
        })
        .await
    }

    #[tool(description = "设置音量（0 到 1）。")]
    async fn set_volume(
        &self,
        Parameters(input): Parameters<Volume>,
    ) -> Result<CallToolResult, ErrorData> {
        self.player(Action::Volume(input.volume)).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PlayerTools {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("MusicX", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "选歌和控制本机 MusicX 播放器。导入个人歌单前，请在音乐来源设置中关联账号。",
            )
    }
}
