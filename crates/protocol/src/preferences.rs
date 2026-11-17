//! Cross-process serialization for preferences shared by the helper and panel.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
};

pub fn locked<T>(dir: &Path, action: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    fs::create_dir_all(dir)?;
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(dir.join(".preferences.lock"))?;
    lock.lock()?;
    action()
}

/// Caller holds `locked`; the rename is atomic for readers in other processes.
pub fn write_locked(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    if name.contains('/') || name.contains('\\') || name.starts_with('.') {
        return Err(io::Error::other("Invalid preference filename"));
    }
    let temp = dir.join(format!(".{name}.{}", std::process::id()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file: File = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, dir.join(name))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub fn save(name: &str, bytes: &[u8]) -> io::Result<()> {
    let dir =
        crate::data_dir().ok_or_else(|| io::Error::other("Configuration directory not found"))?;
    save_at(&dir, name, bytes)
}

pub fn save_at(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    locked(dir, || {
        if dir.join(".icloud-applying").exists() {
            return Err(io::Error::other(crate::tr!(
                "A sync update is being recovered; try again in a moment"
            )));
        }
        write_locked(dir, name, bytes)
    })
}
