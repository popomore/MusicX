//! One tiny persisted preference; no stream URLs or per-track probe caches.
use crate::protocol::Quality;
use anyhow::{Context, Result};
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
        .map_err(|_| anyhow::anyhow!("音质设置不可用"))?;
    crate::storage::save_private("audio-quality.json", &serde_json::to_string(&quality)?)
        .context("无法保存音质设置")?;
    *value = quality;
    Ok(())
}
