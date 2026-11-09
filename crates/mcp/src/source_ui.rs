use axum::{Json, Router, http::StatusCode, routing::get};
use musicx_core::protocol::{MusicSource, MusicSourceAction};
use musicx_plugins::music_sources::{execute, list};
use serde_json::{Value, json};

type Failure = (StatusCode, Json<Value>);
fn error(e: impl std::fmt::Display) -> Failure {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":e.to_string()})),
    )
}
pub fn routes() -> Router {
    Router::new().route("/ui/music-sources", get(source_list).post(source_action))
}
async fn source_list() -> Result<Json<Vec<MusicSource>>, Failure> {
    tokio::task::spawn_blocking(list)
        .await
        .map_err(error)?
        .map(Json)
        .map_err(error)
}
async fn source_action(Json(input): Json<MusicSourceAction>) -> Result<Json<Value>, Failure> {
    tokio::task::spawn_blocking(move || execute(input))
        .await
        .map_err(error)?
        .map(Json)
        .map_err(error)
}
