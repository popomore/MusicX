use anyhow::{Context, Result};
use musicx_protocol::tr;
use rand::{RngCore, rngs::OsRng};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
};

pub fn state_dir() -> Result<PathBuf> {
    let path = crate::protocol::data_dir().context(tr!("User directory not found"))?;
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
