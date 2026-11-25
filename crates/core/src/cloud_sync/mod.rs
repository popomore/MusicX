//! Opt-in iCloud sync. Transport is injected by the Apple platform; local data
//! remains usable when iCloud is unavailable. No account or playback-session files
//! are included in a replica.
pub mod merge;
mod snapshot;
#[cfg(test)]
mod tests;

use crate::{
    history, local_library,
    protocol::{self, CloudSyncPhase as Phase, CloudSyncStatus},
    storage,
};
use anyhow::{Context, Result, ensure};
use merge::{Replica, Snapshot};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs,
    io::{ErrorKind, Read},
    path::Path,
    sync::{Mutex, OnceLock},
};

pub const MAX_BYTES: u64 = 128 * 1024 * 1024;
const STATE_FILE: &str = "icloud-state.json";
const SETTINGS_FILE: &str = "icloud-settings.json";
pub const HISTORY_FILE: &str = "icloud-history.json";
const APPLY_MARKER: &str = ".icloud-applying";
static CONTROL: Mutex<()> = Mutex::new(());
static STATUS: Mutex<Option<CloudSyncStatus>> = Mutex::new(None);
static WAKE: OnceLock<fn()> = OnceLock::new();

#[derive(Default, Serialize, Deserialize)]
struct Settings {
    enabled: bool,
    account: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Pending {
    before: Snapshot,
    after: Snapshot,
}

#[derive(Serialize, Deserialize)]
struct State {
    replica: Replica,
    baseline: Snapshot,
    pending: Option<Pending>,
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::File::open(path) {
        Ok(file) => {
            ensure!(
                file.metadata()?.len() <= MAX_BYTES,
                "iCloud sync file is too large"
            );
            let mut bytes = Vec::new();
            file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= MAX_BYTES,
                "iCloud sync file is too large"
            );
            Ok(Some(serde_json::from_slice(&bytes).context(
                "Could not read iCloud sync data; the original file was preserved",
            )?))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn settings() -> Result<Settings> {
    Ok(read_json(&storage::state_dir()?.join(SETTINGS_FILE))?.unwrap_or_default())
}

pub fn enabled() -> bool {
    settings().is_ok_and(|s| s.enabled)
}

pub fn status() -> CloudSyncStatus {
    STATUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| CloudSyncStatus {
            enabled: enabled(),
            phase: if enabled() {
                Phase::Waiting
            } else {
                Phase::Disabled
            },
            ..Default::default()
        })
}

pub fn set_phase(phase: Phase, error: Option<String>) {
    let mut status = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    let enabled = enabled();
    let last_sync_unix = status.as_ref().and_then(|s| s.last_sync_unix);
    *status = Some(CloudSyncStatus {
        enabled,
        phase: if enabled { phase } else { Phase::Disabled },
        last_sync_unix,
        error: if enabled { error } else { None },
    });
}

/// Ready means local changes have been handed to iCloud, not an upload receipt.
pub fn exchanged(waiting_for_downloads: bool) {
    set_phase(
        if waiting_for_downloads {
            Phase::Waiting
        } else {
            Phase::Ready
        },
        None,
    );
    if let Some(status) = STATUS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        status.last_sync_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs());
    }
}

pub fn set_waker(waker: fn()) {
    let _ = WAKE.set(waker);
}
pub fn sync_now() {
    if let Some(wake) = WAKE.get() {
        wake();
    }
}

pub fn set_enabled(enabled: bool) -> Result<()> {
    let _guard = CONTROL
        .lock()
        .map_err(|_| anyhow::anyhow!("iCloud settings unavailable"))?;
    let mut settings = settings()?;
    if settings.enabled != enabled {
        settings.enabled = enabled;
        // A deliberate off/on is required before binding a different Apple ID.
        if !enabled {
            settings.account = None;
        }
        storage::save_private(SETTINGS_FILE, &serde_json::to_string(&settings)?)?;
    }
    set_phase(
        if enabled {
            Phase::Waiting
        } else {
            Phase::Disabled
        },
        None,
    );
    sync_now();
    Ok(())
}

/// Do not carry data into a different Apple ID after an account change.
pub fn bind_account(account: &str) -> Result<bool> {
    let _guard = CONTROL
        .lock()
        .map_err(|_| anyhow::anyhow!("iCloud settings unavailable"))?;
    let mut settings = settings()?;
    if !settings.enabled {
        return Ok(false);
    }
    if let Some(previous) = &settings.account {
        if previous != account {
            set_phase(Phase::AccountChanged, None);
            return Ok(false);
        }
    } else {
        settings.account = Some(account.to_owned());
        storage::save_private(SETTINGS_FILE, &serde_json::to_string(&settings)?)?;
    }
    Ok(true)
}

pub fn device_id() -> Result<String> {
    let path = storage::state_dir()?.join("icloud-device");
    match fs::read_to_string(&path) {
        Ok(value) => {
            ensure!(
                merge::valid_device(&value),
                "Invalid local iCloud device identifier"
            );
            Ok(value)
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let value = storage::random_hex::<16>();
            storage::save_private("icloud-device", &value)?;
            Ok(value)
        }
        Err(e) => Err(e.into()),
    }
}

fn save_state(state: &State) -> Result<()> {
    let json = serde_json::to_string(state)?;
    ensure!(
        json.len() as u64 <= MAX_BYTES,
        "iCloud sync state is too large"
    );
    storage::save_private(STATE_FILE, &json)
}

/// Cloud reads happen before entering this transaction. Local library writes and
/// cross-process preference writes are serialized while capturing and applying.
pub fn synchronize(incoming: &[Replica]) -> Result<Option<Replica>> {
    let _guard = CONTROL
        .lock()
        .map_err(|_| anyhow::anyhow!("iCloud settings unavailable"))?;
    synchronize_locked(incoming, false)
}

pub fn has_pending_apply() -> bool {
    protocol::data_dir().is_some_and(|dir| dir.join(APPLY_MARKER).exists())
}

/// Run before accepting local mutations, even with sync disabled/unavailable.
pub fn recover_pending() -> Result<()> {
    let _guard = CONTROL
        .lock()
        .map_err(|_| anyhow::anyhow!("iCloud settings unavailable"))?;
    recover_pending_locked()
}

fn recover_pending_locked() -> Result<()> {
    let dir = storage::state_dir()?;
    let state: Option<State> = read_json(&dir.join(STATE_FILE))?;
    if state.is_some_and(|state| state.pending.is_some()) {
        synchronize_locked(&[], true)?;
    } else {
        clear_apply_marker(&dir)?;
    }
    Ok(())
}

pub(crate) fn local_edit<T>(edit: impl FnOnce() -> Result<T>) -> Result<T> {
    let _guard = CONTROL
        .lock()
        .map_err(|_| anyhow::anyhow!("iCloud settings unavailable"))?;
    if has_pending_apply() {
        recover_pending_locked()?;
    }
    edit()
}

fn clear_apply_marker(dir: &Path) -> Result<()> {
    protocol::preferences::locked(dir, || match fs::remove_file(dir.join(APPLY_MARKER)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    })?;
    Ok(())
}

fn synchronize_locked(incoming: &[Replica], recovering: bool) -> Result<Option<Replica>> {
    if !recovering && !enabled() {
        return Ok(None);
    }
    let dir = storage::state_dir()?;
    let device = device_id()?;
    let mut state: State = read_json(&dir.join(STATE_FILE))?.unwrap_or_else(|| State {
        replica: Replica::new(device.clone()),
        baseline: Snapshot::new(),
        pending: None,
    });
    state.replica.validate()?;
    ensure!(
        state.replica.device == device,
        "Local iCloud device identity changed"
    );
    for replica in incoming {
        replica.validate()?;
    }
    local_library::synchronize(|playlists| {
        protocol::preferences::locked(&dir, || {
            let mut apply = || -> Result<()> {
                let own_history = history::sync_entries(&device)?;
                let mut actual = snapshot::capture(playlists, &own_history, &dir, &state.replica)?;
                let current = if let Some(pending) = state.pending.take() {
                    // The journal may contain a legacy value captured before its
                    // standalone preference was written. Its absence is not a
                    // local deletion while this apply is still pending.
                    let key = "s/plugin-auto-update.json";
                    if !actual.contains_key(key)
                        && let Some(value) = pending.before.get(key)
                    {
                        actual.insert(key.into(), value.clone());
                    }
                    let recovered = merge::recover(&pending.before, &pending.after, &actual);
                    state.replica.capture(&pending.after, &recovered)?;
                    state.baseline = recovered.clone();
                    recovered
                } else {
                    actual.clone()
                };
                state.replica.capture(&state.baseline, &current)?;
                for replica in incoming {
                    state.replica.merge(replica)?;
                }
                let all = state.replica.snapshot();
                let next = snapshot::playlists(&all, playlists)?;
                local_library::validate_sync(&next)?;
                let history = snapshot::history(&all)?;
                snapshot::validate_settings(&all)?;
                let after = snapshot::local_view(&all, &device);
                // Persist intent before touching materialized files. Retrying a
                // partial apply preserves edits made since its captured input.
                state.pending = Some(Pending {
                    before: actual,
                    after: after.clone(),
                });
                storage::save_private(APPLY_MARKER, "1")?;
                save_state(&state)?;
                snapshot::apply_settings(&all, &dir)?;
                let encoded = serde_json::to_string(&history)?;
                if fs::read(dir.join(HISTORY_FILE)).ok().as_deref() != Some(encoded.as_bytes()) {
                    storage::save_private(HISTORY_FILE, &encoded)?;
                    history::changed();
                }
                *playlists = next;
                state.baseline = after;
                Ok(())
            };
            apply().map_err(std::io::Error::other)
        })
        .map_err(Into::into)
    })?;
    state.pending = None;
    finish_apply(&state, &dir)?;
    Ok(Some(state.replica))
}

fn finish_apply(state: &State, dir: &Path) -> Result<()> {
    save_state(state)?;
    crate::audio_quality::reload();
    protocol::i18n::refresh();
    clear_apply_marker(dir)?;
    Ok(())
}
