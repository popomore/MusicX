//! Resolve one repository independently of discovery, then read its bounded package.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use std::{
    io::{Cursor, Read},
    path::{Component, Path},
};

const API: &str = "https://api.github.com";
const MAX_ARCHIVE: u64 = 16 * 1024 * 1024;
const MAX_SCRIPT: u64 = 2 * 1024 * 1024;

pub(crate) struct Package {
    pub manifest: Vec<u8>,
    pub script: Vec<u8>,
    pub version: String,
    pub commit: Option<String>,
}

struct Release {
    url: String,
    version: String,
    commit: Option<String>,
}

pub(crate) fn parse_repository(input: &str) -> Result<String> {
    let input = input.trim().trim_end_matches('/');
    let repo = if input.contains("://") {
        let url = reqwest::Url::parse(input).context("仓库地址无效")?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && matches!(url.host_str(), Some("github.com" | "www.github.com")),
            "请输入 GitHub 仓库地址或 owner/repo"
        );
        let parts: Vec<_> = url
            .path_segments()
            .context("仓库地址无效")?
            .take(2)
            .collect();
        parts.join("/")
    } else {
        input.to_owned()
    };
    let repo = repo.strip_suffix(".git").unwrap_or(&repo);
    ensure!(
        crate::source_catalog::valid_repository(repo),
        "请输入有效的 owner/repo"
    );
    Ok(repo.to_owned())
}

pub(crate) fn download(repo: &str) -> Result<Package> {
    download_from(repo, API)
}

fn download_from(repo: &str, api: &str) -> Result<Package> {
    let release = latest(repo, api)?;
    let bytes = crate::http::fetch(&release.url, MAX_ARCHIVE).context("插件下载失败")?;
    let (manifest, script) = unpack(bytes)?;
    Ok(Package {
        manifest,
        script,
        version: release.version,
        commit: release.commit,
    })
}

pub(crate) fn pinned(repo: &str, commit: &str) -> Result<Package> {
    ensure!(
        commit.len() == 40 && commit.bytes().all(|c| c.is_ascii_hexdigit()),
        "来源版本无效"
    );
    let root = format!("https://raw.githubusercontent.com/{repo}/{commit}/");
    let manifest = crate::http::fetch(&format!("{root}plugin.json"), 65536)?;
    let main = entry_name(&manifest)?;
    let script = crate::http::fetch(&format!("{root}{main}"), MAX_SCRIPT)?;
    Ok(Package {
        manifest,
        script,
        version: commit.to_owned(),
        commit: Some(commit.to_owned()),
    })
}

fn latest(repo: &str, api: &str) -> Result<Release> {
    ensure!(
        crate::source_catalog::valid_repository(repo),
        "无效的插件仓库路径"
    );
    let response = crate::http::request(
        &format!("{api}/repos/{repo}/releases/latest"),
        None,
        Some("application/vnd.github+json"),
    )?;
    if response.status() != reqwest::StatusCode::NOT_FOUND {
        #[derive(Deserialize)]
        struct Asset {
            name: String,
            browser_download_url: String,
        }
        #[derive(Deserialize)]
        struct Latest {
            tag_name: String,
            zipball_url: String,
            #[serde(default)]
            assets: Vec<Asset>,
        }
        let release: Latest = serde_json::from_slice(&crate::http::read(
            response.error_for_status()?,
            2 * 1024 * 1024,
        )?)?;
        ensure!(!release.tag_name.is_empty(), "Release 版本缺失");
        let names: Vec<_> = release.assets.iter().map(|a| a.name.as_str()).collect();
        let url = pick_asset(&names, std::env::consts::OS, std::env::consts::ARCH)
            .map(|at| release.assets[at].browser_download_url.clone())
            .unwrap_or(release.zipball_url);
        return Ok(Release {
            url,
            version: release.tag_name,
            commit: None,
        });
    }
    let bytes = crate::http::fetch(&format!("{api}/repos/{repo}/commits/HEAD"), 2 * 1024 * 1024)?;
    let revision: Value = serde_json::from_slice(&bytes)?;
    let sha = revision["sha"].as_str().context("来源版本缺失")?;
    ensure!(
        sha.len() == 40 && sha.bytes().all(|c| c.is_ascii_hexdigit()),
        "来源版本无效"
    );
    Ok(Release {
        url: format!("{api}/repos/{repo}/zipball/{sha}"),
        version: sha.to_owned(),
        commit: Some(sha.to_owned()),
    })
}

fn pick_asset(names: &[&str], os: &str, arch: &str) -> Option<usize> {
    let operating_systems = match os {
        "macos" | "darwin" => ["darwin", "macos"],
        _ => [os, os],
    };
    let architectures = match arch {
        "aarch64" | "arm64" => ["aarch64", "arm64"],
        "x86_64" | "amd64" => ["x86_64", "amd64"],
        _ => [arch, arch],
    };
    let mut general = None;
    for (at, name) in names.iter().enumerate() {
        let name = name.to_ascii_lowercase();
        if !name.ends_with(".zip") {
            continue;
        }
        if operating_systems.iter().any(|os| name.contains(os))
            && architectures.iter().any(|arch| name.contains(arch))
        {
            return Some(at);
        }
        if ![
            "darwin", "macos", "linux", "windows", "ios", "arm64", "aarch64", "amd64", "x86_64",
        ]
        .iter()
        .any(|part| name.contains(part))
            && general.is_none()
        {
            general = Some(at);
        }
    }
    general
}

fn entry_name(manifest: &[u8]) -> Result<String> {
    let manifest: Value = serde_json::from_slice(manifest).context("插件清单 JSON 无效")?;
    let main = manifest["main"].as_str().unwrap_or("index.js");
    let path = Path::new(main);
    ensure!(
        !path.is_absolute()
            && !main.contains('\\')
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_) | Component::CurDir)),
        "插件入口路径无效"
    );
    ensure!(main.ends_with(".js"), "当前支持单文件 JavaScript 插件入口");
    Ok(main.to_owned())
}

fn unpack(bytes: Vec<u8>) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).context("插件下载内容不是有效的 ZIP")?;
    ensure!(archive.len() <= 4096, "插件包包含过多文件");
    let names = archive
        .file_names()
        .map(|name| name.map(|name| name.into_owned()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let manifest_path = if names.iter().any(|name| name == "plugin.json") {
        "plugin.json".to_owned()
    } else {
        let roots: Vec<_> = names
            .iter()
            .filter(|name| {
                name.ends_with("/plugin.json")
                    && name.split('/').count() == 2
                    && !name.starts_with('.')
            })
            .collect();
        ensure!(roots.len() == 1, "插件包顶层没有唯一的 plugin.json");
        roots[0].to_string()
    };
    let manifest = archive_file(&mut archive, &manifest_path, 65536)?;
    let main = entry_name(&manifest)?;
    let prefix = manifest_path.strip_suffix("plugin.json").unwrap();
    let script = archive_file(
        &mut archive,
        &format!("{prefix}{}", main.trim_start_matches("./")),
        MAX_SCRIPT,
    )?;
    Ok((manifest, script))
}

fn archive_file(
    archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>> {
    let file = archive
        .by_name(name)
        .with_context(|| format!("插件包缺少 {name}"))?;
    ensure!(
        file.enclosed_name().is_some()
            && file.is_file()
            && file
                .unix_mode()
                .is_none_or(|mode| mode & 0o170000 != 0o120000),
        "插件包文件路径或类型无效"
    );
    ensure!(file.size() <= limit, "插件包文件超过大小限制");
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "插件包文件超过大小限制");
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        net::TcpListener,
        thread::{self, JoinHandle},
        time::Duration,
    };

    const MANIFEST: &[u8] = br#"{"id":"example","name":"Example","version":"1.0","main":"index.js","playback":"stream"}"#;
    const SCRIPT: &[u8] = b"export function stream() { return {url:'https://example.com/a.mp3'}; }";

    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in files {
            writer
                .start_file(
                    name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    type Route = (String, u16, Vec<u8>);
    fn server(routes: impl FnOnce(&str) -> Vec<Route>) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let routes = routes(&api);
        let server = thread::spawn(move || {
            for (path, status, body) in routes {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                    let count = socket.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                }
                assert!(
                    request.starts_with(format!("GET {path} HTTP/1.1\r\n").as_bytes()),
                    "{}",
                    String::from_utf8_lossy(&request)
                );
                write!(
                    socket,
                    "HTTP/1.1 {status} Response\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                socket.write_all(&body).unwrap();
            }
        });
        (api, server)
    }

    #[test]
    fn repository_input_accepts_github_links_and_rejects_other_hosts_or_paths() {
        for input in [
            "owner/repo",
            "https://github.com/owner/repo.git",
            "https://github.com/owner/repo/tree/main",
            "http://www.github.com/owner/repo/",
        ] {
            assert_eq!(parse_repository(input).unwrap(), "owner/repo");
        }
        for input in [
            "../repo",
            "owner/repo/extra",
            "owner",
            "https://evil.example/owner/repo",
            "https://github.com.evil.example/owner/repo",
            "owner/repo?key=value",
        ] {
            assert!(parse_repository(input).is_err(), "{input}");
        }
    }

    #[test]
    fn platform_package_wins_over_generic_and_other_platforms_are_not_selected() {
        let names = [
            "plugin-linux-amd64.zip",
            "plugin.zip",
            "plugin-darwin-arm64.zip",
            "plugin-darwin-amd64.zip",
        ];
        assert_eq!(pick_asset(&names, "macos", "aarch64"), Some(2));
        assert_eq!(pick_asset(&names, "macos", "x86_64"), Some(3));
        assert_eq!(pick_asset(&names, "ios", "aarch64"), Some(1));
        assert_eq!(
            pick_asset(&["plugin-linux-arm64.zip"], "macos", "aarch64"),
            None
        );
    }

    #[test]
    fn release_assets_accept_every_mac_platform_and_architecture_alias() {
        for os in ["darwin", "macos"] {
            for arch in ["arm64", "aarch64"] {
                let name = format!("plugin-{os}-{arch}.zip");
                assert_eq!(
                    pick_asset(
                        &["plugin-linux-arm64.zip", "plugin.zip", &name],
                        "macos",
                        "aarch64"
                    ),
                    Some(2),
                    "{name}"
                );
            }
            for arch in ["amd64", "x86_64"] {
                let name = format!("plugin-{os}-{arch}.zip");
                assert_eq!(
                    pick_asset(
                        &["plugin-linux-amd64.zip", "plugin.zip", &name],
                        "macos",
                        "x86_64"
                    ),
                    Some(2),
                    "{name}"
                );
            }
        }
        assert_eq!(
            pick_asset(
                &["plugin-darwin-x86_64.zip", "plugin-linux-aarch64.zip"],
                "macos",
                "aarch64"
            ),
            None
        );
    }

    #[test]
    fn latest_release_asset_is_downloaded_without_resolving_head() {
        let (api, server) = server(|api| {
            vec![
            ("/repos/owner/repo/releases/latest".into(), 200, serde_json::to_vec(&serde_json::json!({
                "tag_name":"v1", "zipball_url":format!("{api}/source.zip"),
                "assets":[{"name":"plugin.zip","browser_download_url":format!("{api}/plugin.zip")}]
            })).unwrap()),
            ("/plugin.zip".into(), 200, archive(&[("plugin.json", MANIFEST), ("index.js", SCRIPT)])),
        ]
        });
        let package = download_from("owner/repo", &api).unwrap();
        server.join().unwrap();
        assert_eq!(package.version, "v1");
        assert!(package.commit.is_none());
        assert_eq!(package.script, SCRIPT);
    }

    #[test]
    fn release_source_archive_is_used_when_no_asset_matches() {
        let (api, server) = server(|api| {
            vec![
                (
                    "/repos/owner/repo/releases/latest".into(),
                    200,
                    serde_json::to_vec(&serde_json::json!({
                        "tag_name":"v2", "zipball_url":format!("{api}/source.zip"), "assets":[]
                    }))
                    .unwrap(),
                ),
                (
                    "/source.zip".into(),
                    200,
                    archive(&[
                        ("owner-repo-hash/plugin.json", MANIFEST),
                        ("owner-repo-hash/index.js", SCRIPT),
                    ]),
                ),
            ]
        });
        let package = download_from("owner/repo", &api).unwrap();
        server.join().unwrap();
        assert_eq!(package.version, "v2");
        assert_eq!(package.manifest, MANIFEST);
    }

    #[test]
    fn missing_release_resolves_and_downloads_one_fixed_commit() {
        let sha = "1234567890123456789012345678901234567890";
        let (api, server) = server(|_| {
            vec![
                (
                    "/repos/owner/repo/releases/latest".into(),
                    404,
                    b"{}".to_vec(),
                ),
                (
                    "/repos/owner/repo/commits/HEAD".into(),
                    200,
                    serde_json::to_vec(&serde_json::json!({"sha":sha})).unwrap(),
                ),
                (
                    format!("/repos/owner/repo/zipball/{sha}"),
                    200,
                    archive(&[("repo/plugin.json", MANIFEST), ("repo/index.js", SCRIPT)]),
                ),
            ]
        });
        let package = download_from("owner/repo", &api).unwrap();
        server.join().unwrap();
        assert_eq!(package.commit.as_deref(), Some(sha));
        assert_eq!(package.version, sha);
    }

    #[test]
    fn rate_limit_errors_do_not_fall_back_to_another_api_call() {
        let (api, server) = server(|_| {
            vec![(
                "/repos/owner/repo/releases/latest".into(),
                403,
                b"{}".to_vec(),
            )]
        });
        assert!(download_from("owner/repo", &api).is_err());
        server.join().unwrap();
    }

    #[test]
    fn archives_cannot_supply_escaping_typescript_or_oversized_entries() {
        for main in ["../outside.js", "/outside.js", "..\\outside.js", "index.ts"] {
            let manifest = serde_json::to_vec(&serde_json::json!({"main":main})).unwrap();
            assert!(
                unpack(archive(&[("plugin.json", &manifest), ("index.js", SCRIPT)])).is_err(),
                "{main}"
            );
        }
        assert!(
            unpack(archive(&[
                ("a/plugin.json", MANIFEST),
                ("b/plugin.json", MANIFEST)
            ]))
            .is_err()
        );
        let oversized = vec![b' '; MAX_SCRIPT as usize + 1];
        assert!(
            unpack(archive(&[
                ("plugin.json", MANIFEST),
                ("index.js", &oversized)
            ]))
            .is_err()
        );
    }

    #[test]
    fn archive_entry_cannot_be_a_symlink() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("plugin.json", options).unwrap();
        writer.write_all(MANIFEST).unwrap();
        writer
            .add_symlink("index.js", "../../outside.js", options)
            .unwrap();
        assert!(unpack(writer.finish().unwrap().into_inner()).is_err());
    }
}
