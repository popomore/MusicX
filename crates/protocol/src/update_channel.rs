//! One shared preference for the helper and its disposable settings panel.
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    #[default]
    Stable,
    All,
}

impl UpdateChannel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "稳定版",
            Self::All => "全部版本",
        }
    }

    pub fn load() -> Self {
        crate::data_dir().map_or(Self::default(), |dir| Self::read(&dir))
    }

    pub fn save(self) -> std::io::Result<()> {
        let dir = crate::data_dir().ok_or_else(|| std::io::Error::other("找不到配置目录"))?;
        self.write(&dir)
    }

    fn read(dir: &Path) -> Self {
        match std::fs::read_to_string(dir.join("update-channel"))
            .as_deref()
            .map(str::trim)
        {
            Ok("all") => Self::All,
            _ => Self::Stable,
        }
    }

    fn write(self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let temp = dir.join(format!(".update-channel.{}", std::process::id()));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            file.write_all(match self {
                Self::Stable => b"stable\n",
                Self::All => b"all\n",
            })?;
            file.sync_all()?;
            std::fs::rename(&temp, dir.join("update-channel"))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_invalid_and_saved_channel_preferences() {
        let dir =
            std::env::temp_dir().join(format!("musicx-update-channel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(UpdateChannel::read(&dir), UpdateChannel::Stable);
        UpdateChannel::All.write(&dir).unwrap();
        assert_eq!(UpdateChannel::read(&dir), UpdateChannel::All);
        std::fs::write(dir.join("update-channel"), b"unknown").unwrap();
        assert_eq!(UpdateChannel::read(&dir), UpdateChannel::Stable);
        UpdateChannel::Stable.write(&dir).unwrap();
        assert_eq!(UpdateChannel::read(&dir), UpdateChannel::Stable);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
