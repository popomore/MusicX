//! Host-owned provenance and hashes of the installed payload, excluding user data.
use crate::{music_sources, mygo::Manifest};
use anyhow::{Context, Result, ensure};
use musicx_core::protocol::tr;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Read, path::Path};

pub(crate) const FILE: &str = "musicx-install.json";

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
    ensure!(
        bytes.len() as u64 <= limit,
        tr!("Plugin install record or file is too large")
    );
    Ok(bytes)
}

fn hashes(dir: &Path) -> Result<BTreeMap<String, String>> {
    crate::plugin_files::hashes(dir)
}

pub(crate) fn write(dir: &Path, manifest: &Manifest) -> Result<()> {
    let record = Record {
        schema: 2,
        repository: manifest
            .repository
            .clone()
            .context(tr!("Plugin repository is missing"))?,
        version: manifest.version.clone(),
        revision: music_sources::repository_version(manifest)
            .context(tr!("Plugin repository revision is missing"))?
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
            record: read(&dir.join(FILE), 1024 * 1024)?,
        };
        snapshot.verify(dir)?;
        Ok(snapshot)
    }

    pub fn verify(&self, dir: &Path) -> Result<()> {
        ensure!(
            read(&dir.join(FILE), 1024 * 1024)? == self.record
                && read(&dir.join("plugin.json"), 2 * 1024 * 1024)? == self.manifest,
            tr!("Source installation record changed; automatic update skipped")
        );
        let record: Record = serde_json::from_slice(&self.record)?;
        let manifest: Manifest = serde_json::from_slice(&self.manifest)?;
        ensure!(
            matches!(record.schema, 1 | 2)
                && Some(record.repository.as_str()) == manifest.repository.as_deref()
                && record.version == manifest.version
                && record.revision == music_sources::source_version(&manifest),
            tr!("Invalid source installation record; update manually from the repository")
        );
        ensure!(
            hashes(dir)? == record.files,
            tr!(
                "Source files were modified; automatic updates paused. Manual updates will replace these files."
            )
        );
        Ok(())
    }
}
