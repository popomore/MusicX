use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudSyncPhase {
    #[default]
    Disabled,
    Waiting,
    Syncing,
    Ready,
    Unavailable,
    AccountChanged,
    Error,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSyncStatus {
    pub enabled: bool,
    pub phase: CloudSyncPhase,
    pub last_sync_unix: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CloudSyncAction {
    SetEnabled { enabled: bool },
    SyncNow,
}
