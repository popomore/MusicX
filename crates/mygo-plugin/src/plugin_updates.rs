//! The persistent host updates repository installs independently of the player UI.
use crate::{music_sources, mygo, repository};
use anyhow::{Result, ensure};
use musicx_core::protocol::tr;
use musicx_core::{protocol::PluginUpdateSettings, storage};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const FILE: &str = "plugin-updates.json";
const CHECK_INTERVAL: u64 = 24 * 60 * 60;
const RETRY_INTERVAL: u64 = 60 * 60;
static WRITER: Mutex<()> = Mutex::new(());
static STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Deserialize)]
struct State {
    enabled: bool,
    #[serde(default)]
    checks: BTreeMap<String, Check>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            enabled: true,
            checks: BTreeMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Check {
    repository: String,
    version: String,
    at: u64,
    failed: bool,
}

fn lock() -> Result<MutexGuard<'static, ()>> {
    WRITER
        .lock()
        .map_err(|_| anyhow::anyhow!(tr!("Plugin update settings are unavailable")))
}

fn load() -> Result<State> {
    match fs::read(storage::state_dir()?.join(FILE)) {
        Ok(bytes) => {
            ensure!(
                bytes.len() <= 2 * 1024 * 1024,
                tr!("Plugin update record is too large")
            );
            Ok(serde_json::from_slice(&bytes)?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(error) => Err(error.into()),
    }
}

fn save(state: &State) -> Result<()> {
    storage::save_private(FILE, &serde_json::to_string(state)?)
}

pub fn settings() -> Result<PluginUpdateSettings> {
    Ok(PluginUpdateSettings {
        enabled: load()?.enabled,
    })
}

pub fn set_enabled(enabled: bool) -> Result<PluginUpdateSettings> {
    let _guard = lock()?;
    let mut state = load()?;
    state.enabled = enabled;
    save(&state)?;
    Ok(PluginUpdateSettings { enabled })
}

// Held only during the final local install, so disabling also cancels downloads
// that have not yet started replacing an installed plugin.
pub(crate) fn enabled_guard() -> Result<MutexGuard<'static, ()>> {
    let guard = lock()?;
    ensure!(
        load()?.enabled,
        tr!("Automatic plugin updates are disabled")
    );
    Ok(guard)
}

pub fn start() -> Result<()> {
    if STARTED.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let result = std::thread::Builder::new()
        .name("plugin-updates".into())
        .spawn(|| {
            // Startup and the first playback request take priority over maintenance.
            std::thread::sleep(Duration::from_secs(30));
            loop {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if let Err(error) = run_due(now, repository::API) {
                    eprintln!(
                        "{}",
                        tr!("Automatic plugin updates are unavailable: {error}"; error = error)
                    );
                }
                std::thread::sleep(Duration::from_secs(60));
            }
        });
    if let Err(error) = result {
        STARTED.store(false, Ordering::Release);
        return Err(error.into());
    }
    Ok(())
}

fn due(check: Option<&Check>, repository: &str, version: &str, now: u64) -> bool {
    check.is_none_or(|check| {
        check.repository != repository
            || check.version != version
            || now < check.at
            || now - check.at
                >= if check.failed {
                    RETRY_INTERVAL
                } else {
                    CHECK_INTERVAL
                }
    })
}

fn run_due(now: u64, api: &str) -> Result<()> {
    if !settings()?.enabled {
        return Ok(());
    }
    for item in fs::read_dir(mygo::plugin_root()?)? {
        let item = item?;
        let id = item.file_name().to_string_lossy().into_owned();
        if !item.file_type()?.is_dir() || !mygo::valid_id(&id) || music_sources::auth_active(&id) {
            continue;
        }
        // A single damaged or locally installed source must not block the others.
        let Ok(expected) = crate::installation::Snapshot::capture(&item.path()) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_slice::<mygo::Manifest>(&expected.manifest) else {
            continue;
        };
        let Some(version) = music_sources::repository_version(&manifest) else {
            continue;
        };
        let Some(repository) = manifest
            .repository
            .as_deref()
            .and_then(|repo| repository::parse_repository(repo).ok())
        else {
            continue;
        };
        if manifest.id != id {
            continue;
        }
        {
            let _guard = lock()?;
            let mut state = load()?;
            if !state.enabled {
                break;
            }
            if !due(state.checks.get(&id), &repository, version, now) {
                continue;
            }
            // Persist before network I/O so restarting cannot bypass retry limits.
            state.checks.insert(
                id.clone(),
                Check {
                    repository: repository.clone(),
                    version: version.into(),
                    at: now,
                    failed: true,
                },
            );
            save(&state)?;
        }
        let result = music_sources::auto_update(&id, api, &expected);
        let _guard = lock()?;
        let mut state = load()?;
        if let Some(check) = state.checks.get_mut(&id) {
            match result {
                Ok(version) => {
                    check.version = version;
                    check.failed = false;
                }
                Err(error) => eprintln!(
                    "{}",
                    tr!("Automatic update for plugin {id} failed; keeping the installed version: {error}"; id = id, error = error)
                ),
            }
        }
        save(&state)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "plugin_updates_tests.rs"]
mod tests;
