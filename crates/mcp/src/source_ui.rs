use crate::Services;
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use musicx_core::protocol::{MusicSource, MusicSourceAction};
use serde_json::{Value, json};

type Failure = (StatusCode, Json<Value>);
fn error(e: impl std::fmt::Display) -> Failure {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":e.to_string()})),
    )
}
pub fn routes() -> Router<Services> {
    Router::new().route("/ui/music-sources", get(source_list).post(source_action))
}
async fn source_list(State(services): State<Services>) -> Result<Json<Vec<MusicSource>>, Failure> {
    tokio::task::spawn_blocking(move || services.sources.sources())
        .await
        .map_err(error)?
        .map(Json)
        .map_err(error)
}
pub(super) async fn source_action(
    State(services): State<Services>,
    Json(input): Json<MusicSourceAction>,
) -> Result<Json<Value>, Failure> {
    tokio::task::spawn_blocking(move || musicx_core::sources::execute(services.sources, input))
        .await
        .map_err(error)?
        .map(Json)
        .map_err(error)
}
