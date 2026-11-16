//! Serialize reviewed approvals, installation swaps, revocation and helper launch.
use anyhow::{Context, Result, ensure};
use musicx_core::protocol::tr;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Mutex, MutexGuard},
};
static WRITER: Mutex<()> = Mutex::new(());
pub(crate) fn lock() -> Result<MutexGuard<'static, ()>> {
    WRITER
        .lock()
        .map_err(|_| anyhow::anyhow!(tr!("Source permissions are busy; try again")))
}
fn digest(files: &BTreeMap<String, String>) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(files)?)))
}
pub(crate) fn fingerprint(root: &Path) -> Result<String> {
    digest(&crate::plugin_files::hashes(root)?)
}
pub(crate) fn reviewed_files(
    root: &Path,
    expected: Option<&str>,
    _guard: &MutexGuard<'_, ()>,
) -> Result<BTreeMap<String, String>> {
    let files = crate::plugin_files::hashes(root)?;
    ensure!(
        expected.context(tr!("Refresh the source before granting permission"))? == digest(&files)?,
        tr!("Source files changed; reopen its settings before granting permission")
    );
    Ok(files)
}
