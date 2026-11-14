//! Wildcard sources may use only server origins explicitly saved by the user.
use crate::mygo::{self, Manifest};
use serde_json::Value;
use url::Url;
const SERVER: &str = "@server:";
const ALL: &str = "@network:all";
pub(crate) fn granted(manifest: &Manifest) -> bool {
    let check = (|| -> anyhow::Result<bool> {
        use std::{fs, io::Read};
        let root = mygo::plugin_root()?.join(&manifest.id);
        let mut bytes = Vec::new();
        fs::File::open(
            musicx_core::storage::state_dir()?
                .join(format!("mygo-{}-network-permission.json", manifest.id)),
        )?
        .take(1024 * 1024)
        .read_to_end(&mut bytes)?;
        let record: std::collections::BTreeMap<String, String> = serde_json::from_slice(&bytes)?;
        Ok(manifest.permissions.network.iter().any(|g| g == "*")
            && record == crate::plugin_files::hashes(&root)?)
    })();
    check.unwrap_or(false)
}
pub(crate) fn set(
    id: &str,
    enabled: bool,
    expected: Option<&str>,
    guard: &std::sync::MutexGuard<'_, ()>,
) -> anyhow::Result<()> {
    use musicx_core::{protocol::tr, storage};
    anyhow::ensure!(mygo::valid_id(id), tr!("Invalid plugin ID"));
    let root = mygo::plugin_root()?.join(id);
    let manifest = mygo::manifest(&root)?;
    anyhow::ensure!(
        !enabled || manifest.permissions.network.iter().any(|g| g == "*"),
        tr!("This source has no unrestricted network permission")
    );
    let name = format!("mygo-{id}-network-permission.json");
    if enabled {
        storage::save_private(
            &name,
            &serde_json::to_string(&crate::permissions::reviewed_files(&root, expected, guard)?)?,
        )?;
    } else if let Err(error) = std::fs::remove_file(storage::state_dir()?.join(name))
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(error.into());
    }
    Ok(())
}
pub(crate) fn explicit(url: &Url, grants: &[String]) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && (grants.iter().any(|g| g == ALL)
            || grants
                .iter()
                .filter_map(|g| g.strip_prefix(SERVER))
                .any(|origin| url.origin().ascii_serialization() == origin))
}
pub(crate) fn effective(manifest: &Manifest, saved: &Value) -> Vec<String> {
    let mut grants: Vec<_> = manifest
        .permissions
        .network
        .iter()
        .filter(|g| g.as_str() != "*" && !g.starts_with('@'))
        .cloned()
        .collect();
    if manifest.permissions.network.iter().any(|g| g == "*") {
        for (key, value) in saved.as_object().into_iter().flatten() {
            let key = key.to_ascii_lowercase();
            if !matches!(
                key.as_str(),
                "server"
                    | "serverurl"
                    | "server_url"
                    | "baseurl"
                    | "base_url"
                    | "endpoint"
                    | "host"
                    | "url"
                    | "localserver"
                    | "remoteserver"
                    | "serverlocal"
                    | "serverremote"
            ) {
                continue;
            }
            let Some(value) = value.as_str() else {
                continue;
            };
            let Ok(url) = Url::parse(value.trim()) else {
                continue;
            };
            if matches!(url.scheme(), "http" | "https")
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
            {
                grants.push(format!("{SERVER}{}", url.origin().ascii_serialization()));
            }
        }
    }
    grants.sort();
    grants.dedup();
    grants
}
pub(crate) fn for_manifest(manifest: &Manifest, saved: &Value) -> Vec<String> {
    let mut grants = effective(manifest, saved);
    if granted(manifest) {
        grants.push(ALL.into());
    }
    grants
}
pub(crate) fn for_plugin(plugin: &str) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(
        mygo::valid_id(plugin),
        musicx_core::protocol::tr!("Invalid plugin ID")
    );
    let root = mygo::plugin_root()?.join(plugin);
    let manifest = mygo::manifest(&root)?;
    use std::io::Read;
    let mut bytes = Vec::new();
    if let Ok(file) = std::fs::File::open(root.join("settings.json")) {
        file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    }
    anyhow::ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        musicx_core::protocol::tr!("Plugin settings exceed the size limit")
    );
    let saved = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
    Ok(for_manifest(&manifest, &saved))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn private_network_access_requires_an_explicit_server_origin_and_port() {
        let manifest:Manifest=serde_json::from_value(json!({"id":"server","permissions":{"network":["*","@server:http://127.0.0.1:80"]},"settings":[{"key":"server","label":"Server","value":"http://127.0.0.1:4533"}]})).unwrap();
        assert!(!explicit(
            &Url::parse("http://127.0.0.1:4533/a").unwrap(),
            &effective(&manifest, &json!({}))
        ));
        let grants = effective(
            &manifest,
            &json!({"server":"http://127.0.0.1:4533/music/","password":"http://evil.example"}),
        );
        assert!(explicit(
            &Url::parse("http://127.0.0.1:4533/rest/search").unwrap(),
            &grants
        ));
        for value in [
            "http://127.0.0.1:4534/",
            "http://localhost:4533/",
            "https://127.0.0.1:4533/",
            "http://user:pass@127.0.0.1:4533/",
            "http://evil.example/",
        ] {
            assert!(!explicit(&Url::parse(value).unwrap(), &grants), "{value}");
        }
    }
}
