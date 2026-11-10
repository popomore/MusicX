//! Streamable HTTP MCP and authenticated panel routes.
mod local_library_ui;
mod source_ui;

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

use musicx_core::{
    audio_quality, history, local_library,
    player::{Action, Reply, Status},
    storage,
};
use musicx_plugin_api::DataSource;
use musicx_plugins::{music_sources, mygo};
use musicx_protocol::{self as protocol, address};

#[derive(Clone, Copy)]
pub struct Services {
    pub submit: fn(Action, Reply),
    pub quit: fn(),
    pub sources: &'static dyn DataSource,
}

pub fn read_token() -> Result<String> {
    fs::read_to_string(storage::state_dir()?.join("mcp-token"))
        .context("请先启动播放器，再运行 musicx mcp-token")
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

pub fn start(services: Services) -> Result<()> {
    let address = address();
    let listener = TcpListener::bind(address).context("无法监听 MCP，本机可能已启动播放器")?;
    listener.set_nonblocking(true)?;
    let path = storage::state_dir()?.join("mcp-token");
    let token = match fs::read_to_string(&path) {
        Ok(token) => token,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let token = storage::random_hex::<32>();
            storage::save_private("mcp-token", &token)?;
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
                        services,
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
                .with_state(services)
                .merge(local_library_ui::routes())
                .merge(source_ui::routes())
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
async fn ui_player(services: Services, action: Action) -> UiResult {
    ui_status(services, action, false).await
}
async fn ui_queue(State(services): State<Services>) -> UiResult {
    ui_status(services, Action::Status, true).await
}
async fn ui_status(services: Services, action: Action, include_queue: bool) -> UiResult {
    let (send, receive) = oneshot::channel();
    (services.submit)(action, Some(send));
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
async fn ui_state(State(services): State<Services>) -> UiResult {
    ui_player(services, Action::DisplayStatus).await
}
async fn current_track(
    services: Services,
) -> Result<protocol::Track, (StatusCode, Json<serde_json::Value>)> {
    ui_state(State(services))
        .await?
        .0
        .track
        .ok_or_else(|| ui_error("还没有播放歌曲"))
}
async fn ui_info(
    State(services): State<Services>,
) -> Result<Json<protocol::TrackInfo>, (StatusCode, Json<serde_json::Value>)> {
    let track = current_track(services).await?;
    let info = tokio::task::spawn_blocking(move || {
        let info = services.sources.info(track.clone());
        if info.track != track {
            (services.submit)(Action::MergeTrackMetadata(info.track.clone()), None);
        }
        info
    })
    .await
    .map_err(ui_error)?;
    Ok(Json(info))
}
async fn ui_album(
    State(services): State<Services>,
) -> Result<Json<protocol::AlbumPage>, (StatusCode, Json<serde_json::Value>)> {
    let track = current_track(services).await?;
    Ok(Json(
        tokio::task::spawn_blocking(move || services.sources.album(track))
            .await
            .map_err(ui_error)?
            .map_err(|_| ui_error("当前来源暂时无法提供这张专辑的曲目，可稍后重试"))?,
    ))
}
async fn ui_history()
-> Result<Json<Vec<protocol::HistoryEntry>>, (StatusCode, Json<serde_json::Value>)> {
    Ok(Json(
        tokio::task::spawn_blocking(|| history::recent(50))
            .await
            .map_err(ui_error)?
            .map_err(ui_error)?,
    ))
}
async fn ui_action(
    State(services): State<Services>,
    Json(action): Json<protocol::UiAction>,
) -> UiResult {
    use protocol::UiAction;
    let action = match action {
        UiAction::PlayTrack { track } => Action::Play(
            tokio::task::spawn_blocking(move || {
                services.sources.resolve(track, audio_quality::preferred())
            })
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
            let media = tokio::task::spawn_blocking(move || {
                services.sources.resolve(track, audio_quality::preferred())
            })
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
        UiAction::PlayLocalPlaylist { id, index } => {
            tokio::task::spawn_blocking(move || local_queue(services, id, index))
                .await
                .map_err(ui_error)?
                .map_err(ui_error)?
        }
        UiAction::PlayQueue { index, revision } => Action::Select { index, revision },
        UiAction::Quality { quality } => Action::Quality(quality),
        UiAction::Normalization { enabled } => Action::Normalization(enabled),
        UiAction::Pause => Action::Pause,
        UiAction::Resume => Action::Resume,
        UiAction::Toggle => Action::Toggle,
        UiAction::Previous => Action::Previous,
        UiAction::Next => Action::Next,
        UiAction::Seek { seconds, track_key } => Action::Seek { seconds, track_key },
        UiAction::Volume { value } => Action::Volume(value),
        UiAction::Quit => {
            let result = ui_state(State(services)).await;
            (services.quit)();
            return result;
        }
    };
    ui_player(services, action).await
}

fn local_queue(services: Services, id: String, index: usize) -> Result<Action> {
    let list = local_library::playlist(&id)?;
    validate_list(&list.tracks, index)?;
    let media = services
        .sources
        .resolve(list.tracks[index].clone(), audio_quality::preferred())?;
    Ok(Action::ReplaceQueue {
        tracks: list.tracks,
        index,
        media,
        context: protocol::QueueContext::LocalPlaylist {
            id,
            name: list.name,
        },
    })
}

fn validate_list(tracks: &[protocol::Track], index: usize) -> Result<()> {
    anyhow::ensure!(
        !tracks.is_empty() && tracks.len() <= protocol::MAX_QUEUE_TRACKS,
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

// Local upgrade handoff: authenticated like all UI routes, and accepted before
// user control or an earlier handoff, including over a cold-start checkpoint.
// It carries metadata, never expiring audio URLs or credentials.
async fn ui_restore(State(services): State<Services>, Json(snapshot): Json<Status>) -> UiResult {
    validate_snapshot(&snapshot).map_err(ui_error)?;
    let track = snapshot.index.map(|index| snapshot.queue[index].clone());
    let quality = snapshot.preferred_quality;
    // A paused upgrade must survive offline/expired plugin authentication.
    // Resolve on explicit resume, retaining queue, seek and volume on failure.
    let deferred = snapshot.state == protocol::State::Paused;
    let media = tokio::task::spawn_blocking(move || {
        track
            .filter(|_| !deferred)
            .map(|t| services.sources.resolve(t, quality))
            .transpose()
    })
    .await
    .map_err(ui_error)?
    .map_err(ui_error)?;
    ui_player(
        services,
        Action::Restore {
            snapshot: Box::new(snapshot),
            media,
        },
    )
    .await
}

fn validate_snapshot(snapshot: &Status) -> Result<()> {
    anyhow::ensure!(
        snapshot.queue.len() <= protocol::MAX_QUEUE_TRACKS,
        "队列过长"
    );
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
    services: Services,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PluginSearch {
    plugin: String,
    query: String,
    offset: Option<u64>,
    kind: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Song {
    track: protocol::Track,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct HistoryPage {
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
    ImportSource,
    Play,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct LibraryInput {
    action: LibraryOperation,
    playlist_id: Option<String>,
    name: Option<String>,
    index: Option<usize>,
    revision: Option<u64>,
    source: Option<String>,
    source_playlist_id: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<u64>,
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
        .map_err(|error| {
            ErrorData::internal_error(
                error.to_string(),
                mygo::error_code(&error).map(|code| serde_json::json!({"plugin_code":code})),
            )
        })
}

impl PlayerTools {
    async fn play_track(
        &self,
        track: protocol::Track,
        enqueue: bool,
    ) -> Result<CallToolResult, ErrorData> {
        validate_list(std::slice::from_ref(&track), 0).map_err(tool_error)?;
        let sources = self.services.sources;
        let media =
            blocking(move || sources.resolve_selection(track, audio_quality::preferred())).await?;
        self.player(if enqueue {
            Action::Enqueue(media)
        } else {
            Action::Play(media)
        })
        .await
    }

    async fn player(&self, action: Action) -> Result<CallToolResult, ErrorData> {
        let (send, receive) = oneshot::channel();
        (self.services.submit)(action, Some(send));
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
        description = "管理音乐数据源。action=list/catalog/catalog_view/refresh_catalog/get/install/check_update/update/uninstall/configure/account/sign_in/poll/cancel/sign_out/playlists/import_playlist/import_liked/import_recent/artist/album/shows/show。artist/album/show需要id，show可用offset分页。catalog_view/refresh_catalog返回sources、refreshing和warning，旧缓存立即可读；install可直接用owner/repo或GitHub仓库URL，无需在目录中。插件source为mygo:ID。configure的values按声明字段保存，敏感值不回显；sign_in返回扫码任务，poll/cancel用job。导入仅建本地副本，不回写来源歌单。"
    )]
    async fn manage_music_sources(
        &self,
        Parameters(input): Parameters<SourceInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let action: protocol::MusicSourceAction =
            serde_json::from_value(serde_json::to_value(input).map_err(tool_error)?)
                .map_err(|_| tool_error("无效的数据源操作参数"))?;
        blocking(move || music_sources::execute(action))
            .await
            .and_then(output)
    }

    #[tool(
        description = "管理播放器本地歌单。action=list/get/create/rename/pin/unpin/add_current/add_queue/remove/delete/import_source/play。import_source需要source=mygo:插件ID和source_playlist_id。pin和unpin需要playlist_id。歌曲按来源去重，播放为独立快照；导入创建本地副本，不修改第三方歌单。remove需index和当前revision。"
    )]
    async fn manage_local_playlists(
        &self,
        Parameters(input): Parameters<LibraryInput>,
    ) -> Result<CallToolResult, ErrorData> {
        use protocol::LocalLibraryAction as Edit;
        let id = || {
            input
                .playlist_id
                .clone()
                .context("需要playlist_id")
                .map_err(tool_error)
        };
        let title = || input.name.clone().context("需要name").map_err(tool_error);
        if matches!(input.action, LibraryOperation::List) {
            return blocking(local_library::lists).await.and_then(output);
        }
        if matches!(input.action, LibraryOperation::Get) {
            let id = id()?;
            return blocking(move || local_library::playlist(&id))
                .await
                .and_then(output);
        }
        if matches!(input.action, LibraryOperation::Play) {
            let id = id()?;
            let index = input.index.unwrap_or(0);
            let services = self.services;
            return self
                .player(blocking(move || local_queue(services, id, index)).await?)
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
            LibraryOperation::ImportSource => Edit::ImportSource {
                source: input.source.context("需要source").map_err(tool_error)?,
                playlist_id: input
                    .source_playlist_id
                    .context("需要source_playlist_id")
                    .map_err(tool_error)?,
            },
            LibraryOperation::AddCurrent | LibraryOperation::AddQueue => {
                let Json(snapshot) = ui_queue(State(self.services))
                    .await
                    .map_err(|(_, v)| tool_error(v.0))?;
                let tracks = if matches!(input.action, LibraryOperation::AddCurrent) {
                    vec![snapshot.track.context("当前没有歌曲").map_err(tool_error)?]
                } else {
                    snapshot.queue.as_ref().clone()
                };
                Edit::Add { id: id()?, tracks }
            }
            _ => unreachable!(),
        };
        blocking(move || musicx_plugins::change_library(edit))
            .await
            .and_then(output)
    }

    #[tool(description = "列出本机安装的 MyGo Music 插件和目录兼容状态。")]
    async fn list_music_plugins(&self) -> Result<CallToolResult, ErrorData> {
        blocking(mygo::list).await.and_then(output)
    }

    #[tool(
        description = "通过指定 MyGo 插件搜索。kind=tracks(默认)/albums/artists/playlists/shows，offset 分页。歌曲 tracks 返回统一 Track，duration 单位为秒，可直接传给 play_song/enqueue_song。歌手/专辑/播客 ID 可用于 manage_music_sources 的 artist/album/show。"
    )]
    async fn plugin_search(
        &self,
        Parameters(input): Parameters<PluginSearch>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(move || {
            mygo::search_kind(
                &input.plugin,
                &input.query,
                input.kind.as_deref().unwrap_or("tracks"),
                input.offset.unwrap_or(0),
            )
        })
        .await
        .and_then(output)
    }

    #[tool(
        description = "播放完整曲目 Track，使用来源搜索返回的对象，保留封面、专辑及来源信息。通过统一数据源接口解析音频，不推断来源。"
    )]
    async fn play_song(
        &self,
        Parameters(input): Parameters<Song>,
    ) -> Result<CallToolResult, ErrorData> {
        self.play_track(input.track, false).await
    }

    #[tool(
        description = "将完整曲目 Track 加入播放队列，保留全部元数据；队列为空时立即播放。使用来源搜索返回的对象。"
    )]
    async fn enqueue_song(
        &self,
        Parameters(input): Parameters<Song>,
    ) -> Result<CallToolResult, ErrorData> {
        self.play_track(input.track, true).await
    }

    #[tool(description = "读取本机最近播放过的曲目，按时间倒序，最多 100 条。")]
    async fn playback_history(
        &self,
        Parameters(input): Parameters<HistoryPage>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(move || history::recent(input.limit.unwrap_or(20)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use musicx_plugin_api::Media;
    use std::sync::Mutex;

    #[test]
    fn source_and_player_tools_have_separate_inputs() {
        let tools = PlayerTools::tool_router().list_all();
        let search = tools
            .iter()
            .find(|tool| tool.name == "plugin_search")
            .unwrap();
        assert!(
            search.input_schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field == "plugin")
        );
        for name in ["play_song", "enqueue_song"] {
            let tool = tools.iter().find(|tool| tool.name == name).unwrap();
            let schema = serde_json::to_value(&tool.input_schema).unwrap();
            assert_eq!(schema["required"], serde_json::json!(["track"]));
            assert!(schema["properties"].get("plugin").is_none());
            let track = &schema["properties"]["track"];
            let track =
                track
                    .get("$ref")
                    .and_then(|value| value.as_str())
                    .map_or(track, |reference| {
                        schema
                            .pointer(reference.strip_prefix('#').unwrap())
                            .unwrap()
                    });
            for field in [
                "title",
                "artist",
                "duration",
                "cover_url",
                "album",
                "source_url",
                "source",
            ] {
                assert!(
                    track["properties"].get(field).is_some(),
                    "missing track metadata: {field}"
                );
            }
        }
        for name in [
            "plugin_play",
            "search_songs",
            "my_playlists",
            "playlist_tracks",
        ] {
            assert!(
                !tools.iter().any(|tool| tool.name == name),
                "source-specific player tool: {name}"
            );
        }
    }

    fn track() -> protocol::Track {
        serde_json::from_value(serde_json::json!({
            "id":0,"title":"完整曲目","artist":"Fixture artist","duration":125.5,
            "cover_url":"https://example.com/cover.jpg",
            "album":{"id":"album:opaque","name":"Fixture album"},
            "source_url":"https://example.com/tracks/opaque",
            "source":{"plugin":"uninstalled-fixture-source","id":"track:opaque"}
        }))
        .unwrap()
    }

    struct FixtureSource {
        seen: Mutex<Vec<protocol::Track>>,
        fail: bool,
    }
    impl DataSource for FixtureSource {
        fn resolve(&self, track: protocol::Track, quality: protocol::Quality) -> Result<Media> {
            self.seen.lock().unwrap().push(track.clone());
            anyhow::ensure!(!self.fail, "fixture source unavailable");
            Ok(Media {
                lease: None,
                gain_db: None,
                peak: None,
                track,
                url: "https://example.com/resolved.mp3".parse().unwrap(),
                user_agent: Some("Independent source fixture".into()),
                quality: protocol::AudioQuality {
                    requested: quality,
                    ..Default::default()
                },
            })
        }
        fn info(&self, _: protocol::Track) -> protocol::TrackInfo {
            unreachable!()
        }
        fn album(&self, _: protocol::Track) -> Result<protocol::AlbumPage> {
            unreachable!()
        }
    }

    fn submit_media(action: Action, reply: Reply, enqueue: bool) {
        let media = match (action, enqueue) {
            (Action::Play(media), false) | (Action::Enqueue(media), true) => media,
            _ => panic!("wrong player action"),
        };
        assert!(
            media.track == track(),
            "all track metadata must reach the player"
        );
        assert_eq!(media.url.as_str(), "https://example.com/resolved.mp3");
        assert_eq!(
            media.user_agent.as_deref(),
            Some("Independent source fixture")
        );
        reply
            .unwrap()
            .send(Ok(Status {
                track: Some(media.track),
                ..Default::default()
            }))
            .ok()
            .unwrap();
    }
    fn submit_play(action: Action, reply: Reply) {
        submit_media(action, reply, false);
    }
    fn submit_enqueue(action: Action, reply: Reply) {
        submit_media(action, reply, true);
    }
    fn no_submit(_: Action, _: Reply) {
        panic!("resolution failure must not change the player");
    }

    fn fixture(submit: fn(Action, Reply), fail: bool) -> (PlayerTools, &'static FixtureSource) {
        let source = Box::leak(Box::new(FixtureSource {
            seen: Mutex::new(Vec::new()),
            fail,
        }));
        (
            PlayerTools {
                services: Services {
                    submit,
                    quit: || {},
                    sources: source,
                },
                tool_router: PlayerTools::tool_router(),
            },
            source,
        )
    }

    #[tokio::test]
    async fn play_song_uses_the_injected_source_and_preserves_full_metadata() {
        let (tools, source) = fixture(submit_play, false);
        tools
            .play_song(Parameters(Song { track: track() }))
            .await
            .unwrap();
        assert!(source.seen.lock().unwrap().as_slice() == [track()]);
    }

    #[tokio::test]
    async fn enqueue_song_uses_the_injected_source_and_preserves_full_metadata() {
        let (tools, source) = fixture(submit_enqueue, false);
        tools
            .enqueue_song(Parameters(Song { track: track() }))
            .await
            .unwrap();
        assert!(source.seen.lock().unwrap().as_slice() == [track()]);
    }

    #[tokio::test]
    async fn failed_source_resolution_never_submits_play_or_enqueue() {
        let (tools, source) = fixture(no_submit, true);
        assert!(
            tools
                .play_song(Parameters(Song { track: track() }))
                .await
                .unwrap_err()
                .message
                .contains("fixture source unavailable")
        );
        assert!(
            tools
                .enqueue_song(Parameters(Song { track: track() }))
                .await
                .unwrap_err()
                .message
                .contains("fixture source unavailable")
        );
        assert_eq!(source.seen.lock().unwrap().len(), 2);
    }
}
