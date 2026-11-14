//! Bounded package assets; user settings and host records are never package files.
use anyhow::{Context, Result, ensure};
use musicx_core::protocol::tr;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path},
};
pub(crate) const MAX_FILE: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_TOTAL: u64 = 128 * 1024 * 1024;
pub(crate) struct Asset {
    pub bytes: Vec<u8>,
    pub executable: bool,
}
pub(crate) fn path_allowed(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\\')
        && !Path::new(name).is_absolute()
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
pub(crate) fn excluded(name: &str) -> bool {
    let first = name
        .split('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(first.as_str(), "settings.json" | "musicx-install.json")
        || name.split('/').next().is_some_and(|s| {
            matches!(
                s.to_ascii_lowercase().as_str(),
                ".git" | ".github" | "node_modules" | "target" | ".goals"
            )
        })
}
pub(crate) fn write(root: &Path, name: &str, asset: &Asset) -> Result<()> {
    ensure!(
        path_allowed(name)
            && !excluded(name)
            && !matches!(
                name.to_ascii_lowercase().as_str(),
                "plugin.json" | "index.js" | "locales/en.json" | "locales/zh-cn.json"
            ),
        tr!("Invalid plugin package file path or type")
    );
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, &asset.bytes)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if asset.executable { 0o700 } else { 0o600 }),
    )?;
    Ok(())
}
fn scan(
    root: &Path,
    dir: &Path,
    files: &mut BTreeMap<String, Asset>,
    total: &mut u64,
) -> Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        let path = item.path();
        let name = path
            .strip_prefix(root)?
            .to_str()
            .context(tr!("Invalid plugin entry path"))?
            .replace('\\', "/");
        if excluded(&name) {
            continue;
        }
        ensure!(
            path_allowed(&name) && !item.file_type()?.is_symlink(),
            tr!("Invalid plugin package file path or type")
        );
        if item.file_type()?.is_dir() {
            scan(root, &path, files, total)?;
            continue;
        }
        ensure!(
            item.file_type()?.is_file() && files.len() < 4096,
            tr!("Plugin package contains too many files")
        );
        let mut bytes = Vec::new();
        fs::File::open(&path)?
            .take(MAX_FILE + 1)
            .read_to_end(&mut bytes)?;
        *total += bytes.len() as u64;
        ensure!(
            bytes.len() as u64 <= MAX_FILE && *total <= MAX_TOTAL,
            tr!("Plugin assets exceed the size limit")
        );
        use std::os::unix::fs::PermissionsExt;
        files.insert(
            name,
            Asset {
                bytes,
                executable: item.metadata()?.permissions().mode() & 0o111 != 0,
            },
        );
    }
    Ok(())
}
pub(crate) fn files(root: &Path) -> Result<BTreeMap<String, Asset>> {
    let mut out = BTreeMap::new();
    scan(root, root, &mut out, &mut 0)?;
    Ok(out)
}
pub(crate) fn copy_assets(source: &Path, target: &Path, entry: &Path) -> Result<()> {
    let root = source.canonicalize()?;
    let entry = entry.canonicalize()?;
    let target = target.canonicalize()?;
    ensure!(
        !target.starts_with(&root) && !root.starts_with(&target),
        tr!("Plugin source and destination must not overlap")
    );
    for (name, asset) in files(source)? {
        if matches!(
            name.as_str(),
            "plugin.json" | "index.js" | "locales/en.json" | "locales/zh-CN.json"
        ) || root.join(&name) == entry
        {
            continue;
        }
        write(&target, &name, &asset)?;
    }
    Ok(())
}
pub(crate) fn hashes(root: &Path) -> Result<BTreeMap<String, String>> {
    Ok(files(root)?
        .into_iter()
        .map(|(name, asset)| (name, format!("{:x}", Sha256::digest(&asset.bytes))))
        .collect())
}

pub(crate) fn owned_file(plugin: &str, url: &url::Url) -> Option<std::path::PathBuf> {
    if !crate::mygo::valid_id(plugin)
        || url.scheme() != "file"
        || url.host_str().is_some()
        || url.query().is_some()
    {
        return None;
    }
    let path = url.to_file_path().ok()?.canonicalize().ok()?;
    let roots = [
        crate::mygo::plugin_root().ok()?.join(plugin),
        musicx_core::storage::state_dir()
            .ok()?
            .join("mygo-data")
            .join(plugin),
    ];
    (path.is_file()
        && roots
            .iter()
            .filter_map(|r| r.canonicalize().ok())
            .any(|r| path.starts_with(r)))
    .then_some(path)
}

/// A helper can reuse its cache filename on the next call; each loaded item gets a copy.
pub(crate) struct AudioFile(std::path::PathBuf);
impl AudioFile {
    pub(crate) fn copy(path: &Path) -> Result<std::sync::Arc<Self>> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let dir = musicx_core::storage::state_dir()?.join("generated-audio");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)?;
        ensure!(
            !fs::symlink_metadata(&dir)?.file_type().is_symlink(),
            tr!("Invalid source data directory")
        );
        let target = dir.join(format!(
            "{}.{}",
            musicx_core::storage::random_hex::<16>(),
            path.extension()
                .and_then(|p| p.to_str())
                .filter(|s| s.len() <= 16 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
                .unwrap_or("audio")
        ));
        let file = std::sync::Arc::new(Self(target));
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file.0)?;
        let mut input = fs::File::open(path)?.take(256 * 1024 * 1024 + 1);
        let size = std::io::copy(&mut input, &mut output)?;
        ensure!(
            size <= 256 * 1024 * 1024,
            tr!("Generated audio exceeds 256 MiB")
        );
        Ok(file)
    }
    pub(crate) fn url(&self) -> Result<url::Url> {
        url::Url::from_file_path(&self.0).map_err(|_| anyhow::anyhow!(tr!("Invalid audio URL")))
    }
}
impl Drop for AudioFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_paths_and_copies_are_bounded_and_do_not_follow_links() {
        for name in [
            "SETTINGS.JSON",
            "sEtTiNgS.JsOn",
            "MUSICX-INSTALL.JSON",
            "SETTINGS.JSON/nested",
        ] {
            assert!(
                excluded(name),
                "filesystem-equivalent host files must stay excluded: {name}"
            );
        }
        for name in ["../bad", "/tmp/bad", "a/../b", "a\\b", ""] {
            assert!(!path_allowed(name), "{name}");
        }
        let root = std::env::temp_dir().join(format!(
            "musicx-assets-{}",
            musicx_core::storage::random_hex::<8>()
        ));
        fs::create_dir_all(&root).unwrap();
        for name in [
            "PLUGIN.JSON",
            "INDEX.JS",
            "LOCALES/EN.JSON",
            "locales/ZH-CN.JSON",
        ] {
            assert!(
                write(
                    &root,
                    name,
                    &Asset {
                        bytes: b"untrusted override".to_vec(),
                        executable: false
                    }
                )
                .is_err()
            );
        }
        fs::write(root.join("index.js"), "export function search(){}").unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        assert!(copy_assets(&root, &root.join("nested"), &root.join("index.js")).is_err());
        std::os::unix::fs::symlink("/etc/hosts", root.join("escape")).unwrap();
        assert!(files(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
