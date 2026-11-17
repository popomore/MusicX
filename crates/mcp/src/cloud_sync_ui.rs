use crate::{Services, ui_error};
use axum::{Json, Router, http::StatusCode, routing::get};
use musicx_core::{
    cloud_sync,
    protocol::{CloudSyncAction, CloudSyncPhase, CloudSyncStatus},
};

pub fn routes() -> Router<Services> {
    Router::new().route("/ui/icloud", get(status).post(change))
}

async fn status() -> Json<CloudSyncStatus> {
    Json(cloud_sync::status())
}

async fn change(
    Json(action): Json<CloudSyncAction>,
) -> Result<Json<CloudSyncStatus>, (StatusCode, Json<serde_json::Value>)> {
    tokio::task::spawn_blocking(move || {
        match action {
            CloudSyncAction::SetEnabled { enabled } => cloud_sync::set_enabled(enabled)?,
            CloudSyncAction::SyncNow => {
                cloud_sync::set_phase(CloudSyncPhase::Waiting, None);
                cloud_sync::sync_now();
            }
        }
        Ok::<_, anyhow::Error>(Json(cloud_sync::status()))
    })
    .await
    .map_err(ui_error)?
    .map_err(ui_error)
}
