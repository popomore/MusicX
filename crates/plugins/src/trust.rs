//! Curated functional verification. A plugin cannot grant trust to itself.
use crate::mygo::{self, Manifest};
use musicx_core::protocol::SourceVerification;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Read, path::Path, sync::OnceLock};

#[derive(Clone, Deserialize)]
struct Record {
    repository: String,
    id: String,
    #[serde(flatten)]
    verification: SourceVerification,
    manifest_sha256: String,
    entry_sha256: String,
    locales: BTreeMap<String, String>,
    #[serde(default)]
    assets: BTreeMap<String, String>,
}

fn records() -> &'static [Record] {
    static RECORDS: OnceLock<Vec<Record>> = OnceLock::new();
    RECORDS.get_or_init(|| {
        serde_json::from_str(include_str!("verified_plugins.json"))
            .expect("valid curated verification records")
    })
}

fn repository_key(value: &str) -> Option<String> {
    crate::repository::parse_repository(value)
        .ok()
        .map(|s| s.to_ascii_lowercase())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn manifest_digest(manifest: &Manifest) -> Option<String> {
    let mut value = serde_json::to_value(manifest).ok()?;
    let object = value.as_object_mut()?;
    // Only host-owned installation annotations and the relocated entry path differ.
    for key in ["repository", "sourceVersion", "sourceCommit", "main"] {
        object.remove(key);
    }
    Some(digest(&serde_json::to_vec(&value).ok()?))
}

fn file_digest(root: &Path, name: &str, limit: u64) -> Option<String> {
    let path = root.join(name);
    if !path
        .canonicalize()
        .ok()?
        .starts_with(root.canonicalize().ok()?)
    {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= limit).then(|| digest(&bytes))
}

fn matches(record: &Record, manifest: &Manifest, dir: &Path) -> bool {
    if record.id != manifest.id
        || record.verification.version != manifest.version
        || manifest.repository.as_deref().and_then(repository_key)
            != repository_key(&record.repository)
        || manifest_digest(manifest).as_deref() != Some(&record.manifest_sha256)
        || file_digest(dir, &manifest.main, 2 * 1024 * 1024).as_deref()
            != Some(&record.entry_sha256)
    {
        return false;
    }
    for locale in ["en.json", "zh-CN.json"] {
        let actual = file_digest(dir, &format!("locales/{locale}"), 65536);
        if actual.as_ref() != record.locales.get(locale) {
            return false;
        }
    }
    let Ok(mut files) = crate::plugin_files::hashes(dir) else {
        return false;
    };
    for name in [
        "plugin.json",
        "index.js",
        "locales/en.json",
        "locales/zh-CN.json",
    ] {
        files.remove(name);
    }
    if files != record.assets {
        return false;
    }
    true
}

pub(crate) fn for_repository(repository: &str) -> Option<SourceVerification> {
    let key = repository_key(repository)?;
    records()
        .iter()
        .find(|r| repository_key(&r.repository).as_ref() == Some(&key))
        .map(|r| r.verification.clone())
}

pub(crate) fn for_manifest(manifest: &Manifest) -> Option<SourceVerification> {
    let evidence = for_repository(manifest.repository.as_deref()?)?;
    (manifest.version == evidence.version).then_some(evidence)
}

pub(crate) fn installed(manifest: &Manifest) -> Option<SourceVerification> {
    let dir = mygo::plugin_root().ok()?.join(&manifest.id);
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    records()
        .iter()
        .find(|r| r.verification.platform == platform && matches(r, manifest, &dir))
        .map(|r| {
            let mut verification = r.verification.clone();
            verification.installed_payload_matches = true;
            verification
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn verification_is_bound_to_repository_version_permissions_code_and_locales() {
        let root = std::env::temp_dir().join(format!(
            "musicx-trust-{}-{}",
            std::process::id(),
            musicx_core::storage::random_hex::<8>()
        ));
        fs::create_dir_all(root.join("locales")).unwrap();
        fs::write(
            root.join("index.js"),
            "export function stream(){return {url:'https://example.com/a.mp3'}}",
        )
        .unwrap();
        fs::write(root.join("locales/en.json"), "{}").unwrap();
        let mut manifest:Manifest=serde_json::from_value(serde_json::json!({"id":"fixture","name":"Fixture","version":"1.0.0","repository":"https://github.com/author/source","permissions":{"network":["example.com"]}})).unwrap();
        let record = Record {
            repository: manifest.repository.clone().unwrap(),
            id: manifest.id.clone(),
            verification: SourceVerification {
                version: manifest.version.clone(),
                ..Default::default()
            },
            manifest_sha256: manifest_digest(&manifest).unwrap(),
            entry_sha256: file_digest(&root, "index.js", 65536).unwrap(),
            locales: BTreeMap::from([("en.json".into(), digest(b"{}"))]),
            assets: BTreeMap::new(),
        };
        assert!(matches(&record, &manifest, &root));
        manifest
            .metadata
            .insert("sourceVersion".into(), Value::String("v1.0.0".into()));
        assert!(matches(&record, &manifest, &root));
        manifest.version = "1.0.1".into();
        assert!(!matches(&record, &manifest, &root));
        manifest.version = "1.0.0".into();
        manifest.repository = Some("https://github.com/impostor/source".into());
        assert!(!matches(&record, &manifest, &root));
        manifest.repository = Some(record.repository.clone());
        manifest
            .permissions
            .network
            .push("other.example.com".into());
        assert!(!matches(&record, &manifest, &root));
        manifest.permissions.network.pop();
        fs::write(
            root.join("index.js"),
            "export function stream(){return {url:'https://example.com/changed.mp3'}}",
        )
        .unwrap();
        assert!(!matches(&record, &manifest, &root));
        fs::write(
            root.join("index.js"),
            "export function stream(){return {url:'https://example.com/a.mp3'}}",
        )
        .unwrap();
        fs::write(root.join("locales/en.json"), "{\"a\":\"changed\"}").unwrap();
        assert!(!matches(&record, &manifest, &root));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn curated_records_have_unique_repositories_and_explicit_limits() {
        let records = records();
        assert!(records.len() >= 9);
        let keys: std::collections::BTreeSet<_> = records
            .iter()
            .map(|r| repository_key(&r.repository).unwrap())
            .collect();
        assert_eq!(keys.len(), records.len());
        assert!(records.iter().all(
            |r| r.verification.checks.iter().any(|c| c == "playback_sample")
                && !r.verification.installed_payload_matches
        ));
        assert_eq!(
            for_repository("https://github.com/dddaniel1/mygo-plugin-qqmusic")
                .unwrap()
                .limitations,
            ["preview_only"]
        );
        assert!(for_repository("https://github.com/unverified/source").is_none());
    }
}
