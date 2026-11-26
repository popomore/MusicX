//! One tiny persisted preference; no stream URLs or per-track probe caches.
use crate::protocol::Quality;
use anyhow::{Context, Result};
use musicx_protocol::tr;
use std::sync::{OnceLock, RwLock};

fn preference() -> &'static RwLock<Quality> {
    static VALUE: OnceLock<RwLock<Quality>> = OnceLock::new();
    VALUE.get_or_init(|| {
        RwLock::new(
            crate::protocol::data_dir()
                .and_then(|p| std::fs::read(p.join("audio-quality.json")).ok())
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default(),
        )
    })
}
pub fn preferred() -> Quality {
    *preference().read().unwrap_or_else(|e| e.into_inner())
}
pub fn set_preferred(quality: Quality) -> Result<()> {
    let mut value = preference()
        .write()
        .map_err(|_| anyhow::anyhow!(tr!("Audio quality settings unavailable")))?;
    crate::protocol::preferences::save("audio-quality.json", &serde_json::to_vec(&quality)?)
        .context(tr!("Could not save audio quality settings"))?;
    *value = quality;
    Ok(())
}
fn normalization() -> &'static RwLock<bool> {
    static VALUE: OnceLock<RwLock<bool>> = OnceLock::new();
    VALUE.get_or_init(|| {
        RwLock::new(
            crate::protocol::data_dir()
                .and_then(|p| std::fs::read(p.join("normalization.json")).ok())
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(false),
        )
    })
}
pub fn normalization_enabled() -> bool {
    *normalization().read().unwrap_or_else(|e| e.into_inner())
}
pub fn set_normalization(enabled: bool) -> Result<()> {
    let mut value = normalization()
        .write()
        .map_err(|_| anyhow::anyhow!(tr!("Volume normalization settings unavailable")))?;
    crate::protocol::preferences::save("normalization.json", &serde_json::to_vec(&enabled)?)?;
    *value = enabled;
    Ok(())
}

pub(crate) fn reload() {
    if let Some(dir) = crate::protocol::data_dir() {
        // Match setters' lock order, so a reload cannot overwrite a newer
        // local preference in memory after it has been saved to disk.
        let mut quality = preference().write().unwrap_or_else(|e| e.into_inner());
        let mut normalized = normalization().write().unwrap_or_else(|e| e.into_inner());
        let _ = crate::protocol::preferences::locked(&dir, || {
            *quality = std::fs::read(dir.join("audio-quality.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            *normalized = std::fs::read(dir.join("normalization.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(false);
            Ok(())
        });
    }
}
