use anyhow::Result;
use axum::{Json, Router, extract::Query, http::StatusCode, routing::get};
use musicx_core::{
    local_library::{favorite, lists, playlist},
    protocol::{FavoriteState, LocalLibraryAction},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Selection {
    id: Option<String>,
}
type Failure = (StatusCode, Json<Value>);
fn failure(e: impl std::fmt::Display) -> Failure {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":e.to_string()})),
    )
}
#[derive(Deserialize)]
struct FavoriteSelection {
    key: String,
}
async fn get_favorite(
    Query(input): Query<FavoriteSelection>,
) -> Result<Json<FavoriteState>, Failure> {
    tokio::task::spawn_blocking(move || favorite(&input.key))
        .await
        .map_err(failure)?
        .map(Json)
        .map_err(failure)
}
pub fn routes() -> Router {
    Router::new()
        .route("/ui/local-playlists", get(get_library).post(edit_library))
        .route("/ui/favorite", get(get_favorite))
}
async fn get_library(Query(input): Query<Selection>) -> Result<Json<Value>, Failure> {
    tokio::task::spawn_blocking(move || match input.id {
        Some(id) => playlist(&id).and_then(|p| Ok(serde_json::to_value(p)?)),
        None => lists().and_then(|p| Ok(serde_json::to_value(p)?)),
    })
    .await
    .map_err(failure)?
    .map(Json)
    .map_err(failure)
}
async fn edit_library(Json(input): Json<LocalLibraryAction>) -> Result<Json<Value>, Failure> {
    tokio::task::spawn_blocking(move || musicx_plugins::change_library(input))
        .await
        .map_err(failure)?
        .map(Json)
        .map_err(failure)
}
