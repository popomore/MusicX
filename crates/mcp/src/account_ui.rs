//! Authenticated local UI endpoints; cookies never leave the process.
use crate::{
    netease,
    protocol::{LoginChallenge, LoginPoll, LoginProgress},
};
use axum::{
    Json, Router,
    extract::Query,
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

type Failure = (StatusCode, Json<Value>);
fn failure(message: &str) -> Failure {
    (StatusCode::BAD_REQUEST, Json(json!({"error":message})))
}

#[derive(Deserialize, Default)]
struct Paging {
    #[serde(default)]
    offset: u64,
    #[serde(default)]
    id: u64,
}

pub fn routes() -> Router {
    Router::new()
        .route("/ui/login/start", post(start))
        .route("/ui/login/poll", post(poll))
        .route("/ui/logout", post(logout))
        .route("/ui/playlists", get(playlists))
        .route("/ui/playlist", get(playlist))
}
async fn start() -> Result<Json<LoginChallenge>, Failure> {
    tokio::task::spawn_blocking(netease::begin_login)
        .await
        .map_err(|_| failure("登录请求已结束，请重试"))?
        .map(Json)
        .map_err(|_| failure("登录二维码暂时无法取得，请稍后重试"))
}
async fn poll(Json(input): Json<LoginPoll>) -> Result<Json<LoginProgress>, Failure> {
    tokio::task::spawn_blocking(move || netease::poll_login(&input))
        .await
        .map_err(|_| failure("登录请求已结束，请重试"))?
        .map(Json)
        .map_err(|_| failure("登录状态暂时无法取得，请重试"))
}
async fn logout() -> Result<Json<Value>, Failure> {
    tokio::task::spawn_blocking(netease::logout)
        .await
        .map_err(|_| failure("退出登录失败，请重试"))?
        .map_err(|_| failure("退出登录失败，请重试"))?;
    Ok(Json(json!({"ok":true})))
}
async fn playlists(Query(page): Query<Paging>) -> Result<Json<Value>, Failure> {
    tokio::task::spawn_blocking(move || netease::my_playlists(page.offset, 50))
        .await
        .map_err(|_| failure("歌单请求已结束，请重试"))?
        .map(Json)
        .map_err(|_| failure("请登录网易云后查看个人歌单；已登录时可稍后重试"))
}
async fn playlist(Query(page): Query<Paging>) -> Result<Json<Value>, Failure> {
    if page.id == 0 {
        return Err(failure("请选择歌单"));
    }
    tokio::task::spawn_blocking(move || netease::playlist_tracks(page.id, page.offset, 50))
        .await
        .map_err(|_| failure("歌单请求已结束，请重试"))?
        .map(Json)
        .map_err(|_| failure("歌单暂时无法加载，请确认登录状态或稍后重试"))
}
