//! Host-owned provenance and hashes of the installed payload, excluding user data.
use crate::{music_sources, mygo::Manifest};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Read, path::Path};

pub(crate) const FILE: &str = "musicx-install.json";
const PAYLOAD: &[&str] = &[
    "plugin.json",
    "index.js",
    "locales/en.json",
    "locales/zh-CN.json",
];

#[derive(Deserialize, Serialize)]
struct Record {
    schema: u32,
    repository: String,
    version: String,
    revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    files: BTreeMap<String, String>,
}

pub(crate) struct Snapshot {
    pub manifest: Vec<u8>,
    record: Vec<u8>,
}

fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "插件安装记录或文件过大");
    Ok(bytes)
}

fn hashes(dir: &Path) -> Result<BTreeMap<String, String>> {
    let root = dir.canonicalize()?;
    let mut files = BTreeMap::new();
    for &name in PAYLOAD {
        let path = dir.join(name);
        if name.starts_with("locales/") && !path.try_exists()? {
            continue;
        }
        ensure!(path.canonicalize()?.starts_with(&root), "插件文件路径无效");
        let bytes = read(&path, 2 * 1024 * 1024)?;
        files.insert(name.into(), format!("{:x}", Sha256::digest(&bytes)));
    }
    Ok(files)
}

pub(crate) fn write(dir: &Path, manifest: &Manifest) -> Result<()> {
    let record = Record {
        schema: 1,
        repository: manifest.repository.clone().context("缺少插件仓库")?,
        version: manifest.version.clone(),
        revision: music_sources::repository_version(manifest)
            .context("缺少插件仓库版本")?
            .into(),
        commit: manifest
            .metadata
            .get("sourceCommit")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        files: hashes(dir)?,
    };
    fs::write(dir.join(FILE), serde_json::to_vec(&record)?)?;
    Ok(())
}

impl Snapshot {
    pub fn capture(dir: &Path) -> Result<Self> {
        let snapshot = Self {
            manifest: read(&dir.join("plugin.json"), 2 * 1024 * 1024)?,
            record: read(&dir.join(FILE), 65536)?,
        };
        snapshot.verify(dir)?;
        Ok(snapshot)
    }

    pub fn verify(&self, dir: &Path) -> Result<()> {
        ensure!(
            read(&dir.join(FILE), 65536)? == self.record
                && read(&dir.join("plugin.json"), 2 * 1024 * 1024)? == self.manifest,
            "来源安装记录已变化，已跳过自动更新"
        );
        let record: Record = serde_json::from_slice(&self.record)?;
        let manifest: Manifest = serde_json::from_slice(&self.manifest)?;
        ensure!(
            record.schema == 1
                && Some(record.repository.as_str()) == manifest.repository.as_deref()
                && record.version == manifest.version
                && record.revision == music_sources::source_version(&manifest),
            "来源安装记录无效，需手动从仓库更新"
        );
        ensure!(
            hashes(dir)? == record.files,
            "来源文件已被修改，已暂停自动更新；手动更新将替换这些文件"
        );
        Ok(())
    }
}
