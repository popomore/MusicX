use anyhow::{Context, Result};
use rand::{RngCore, rngs::OsRng};
use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

/// Move the old default directory once. Hold the caller's guard across the
/// move so the previous desktop helper cannot be serving requests there.
pub fn migrate_legacy_data<G>(acquire_guard: impl FnOnce() -> Result<G>) -> Result<()> {
    let Some(legacy) = crate::protocol::legacy_data_dir() else {
        return Ok(());
    };
    let current = crate::protocol::data_dir().context("找不到用户目录")?;
    if !needs_migration(&legacy, &current)? {
        return Ok(());
    }
    let _guard = acquire_guard()?;
    if needs_migration(&legacy, &current)? {
        fs::rename(&legacy, &current).context("无法将旧数据目录迁移到 MusicX")?;
    }
    Ok(())
}

fn needs_migration(legacy: &Path, current: &Path) -> Result<bool> {
    match current.symlink_metadata() {
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    match legacy.metadata() {
        Ok(metadata) => {
            anyhow::ensure!(metadata.is_dir(), "旧数据路径不是目录");
            Ok(true)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub fn state_dir() -> Result<PathBuf> {
    let path = crate::protocol::data_dir().context("找不到用户目录")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&path)?;
    Ok(path)
}

pub fn random_hex<const N: usize>() -> String {
    let mut bytes = [0; N];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn save_private(name: &str, value: &str) -> Result<()> {
    let dir = state_dir()?;
    let temporary = dir.join(format!(".{name}.{}", random_hex::<12>()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(value.as_bytes())?;
        fs::rename(&temporary, dir.join(name))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
