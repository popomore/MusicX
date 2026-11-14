//! User-level MCP configuration. Never launch an agent or change its tool approvals.
use crate::protocol::tr;
use anyhow::{Context, Result, bail};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy)]
pub enum Agent {
    Codex,
    ClaudeCode,
}

impl Agent {
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
        }
    }

    pub fn setup_label(self) -> &'static str {
        match self {
            Self::Codex => tr!("Set up Codex"),
            Self::ClaudeCode => tr!("Set up Claude Code"),
        }
    }

    pub fn configure(self) -> Result<()> {
        let token = crate::remote::mcp_token().context(tr!("Start the background player first"))?;
        configure_at(
            self,
            &self.config_path()?,
            &crate::remote::mcp_endpoint(),
            token,
        )
    }

    fn config_path(self) -> Result<PathBuf> {
        // Isolated previews/tests must not modify the user's agent configuration.
        if let Some(root) = std::env::var_os("MUSICX_AGENT_CONFIG_ROOT").filter(|p| !p.is_empty()) {
            return Ok(PathBuf::from(root).join(match self {
                Self::Codex => "codex/config.toml",
                Self::ClaudeCode => "claude.json",
            }));
        }
        let home = std::env::home_dir().context(tr!("User directory not found"))?;
        Ok(match self {
            Self::Codex => std::env::var_os("CODEX_HOME")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"))
                .join("config.toml"),
            Self::ClaudeCode => std::env::var_os("CLAUDE_CONFIG_DIR")
                .filter(|p| !p.is_empty())
                .map(|p| PathBuf::from(p).join(".claude.json"))
                .unwrap_or_else(|| home.join(".claude.json")),
        })
    }
}

fn invalid_config() -> &'static str {
    tr!("Could not read the agent configuration. The original file was kept.")
}

fn render(agent: Agent, original: Option<&str>, endpoint: &str, token: &str) -> Result<String> {
    let authorization = format!("Bearer {}", token.trim());
    match agent {
        Agent::Codex => {
            use toml_edit::{DocumentMut, InlineTable, Item, Table, Value, value};
            let mut document = original
                .unwrap_or("")
                .parse::<DocumentMut>()
                .map_err(|_| anyhow::anyhow!(invalid_config()))?;
            let servers = document
                .entry("mcp_servers")
                .or_insert(Item::Table(Table::new()))
                .as_table_like_mut()
                .context(invalid_config())?;
            let server = servers
                .entry("musicx")
                .or_insert(Item::Value(Value::InlineTable(InlineTable::new())))
                .as_table_like_mut()
                .context(invalid_config())?;
            // Replace only transport/authentication; retain tool restrictions and other options.
            for key in [
                "command",
                "args",
                "env",
                "env_vars",
                "cwd",
                "bearer_token_env_var",
                "http_headers_helper",
            ] {
                server.remove(key);
            }
            server.insert("url", value(endpoint));
            server.insert("enabled", value(true));
            server.insert("environment_id", value("local"));
            if let Some(headers) = server
                .get_mut("env_http_headers")
                .and_then(Item::as_table_like_mut)
            {
                let keys: Vec<_> = headers
                    .iter()
                    .filter(|(key, _)| key.eq_ignore_ascii_case("authorization"))
                    .map(|(key, _)| key.to_owned())
                    .collect();
                for key in keys {
                    headers.remove(&key);
                }
            }
            let headers = server
                .entry("http_headers")
                .or_insert(Item::Value(Value::InlineTable(InlineTable::new())))
                .as_table_like_mut()
                .context(invalid_config())?;
            let keys: Vec<_> = headers
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case("authorization"))
                .map(|(key, _)| key.to_owned())
                .collect();
            for key in keys {
                headers.remove(&key);
            }
            headers.insert("Authorization", value(authorization));
            Ok(document.to_string())
        }
        Agent::ClaudeCode => {
            use serde_json::{Value, json};
            let mut document: Value = serde_json::from_str(original.unwrap_or("{}"))
                .map_err(|_| anyhow::anyhow!(invalid_config()))?;
            let root = document.as_object_mut().context(invalid_config())?;
            let servers = root
                .entry("mcpServers")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .context(invalid_config())?;
            let server = servers
                .entry("musicx")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .context(invalid_config())?;
            for key in ["command", "args", "env", "cwd", "headersHelper"] {
                server.remove(key);
            }
            server.insert("type".into(), json!("http"));
            server.insert("url".into(), json!(endpoint));
            let headers = server
                .entry("headers")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .context(invalid_config())?;
            headers.retain(|key, _| !key.eq_ignore_ascii_case("authorization"));
            headers.insert("Authorization".into(), json!(authorization));
            Ok(format!("{}\n", serde_json::to_string_pretty(&document)?))
        }
    }
}

fn read(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => bail!(invalid_config()),
    }
}

fn configure_at(agent: Agent, path: &Path, endpoint: &str, token: &str) -> Result<()> {
    if token.trim().is_empty() {
        bail!(tr!("Start the background player first"));
    }
    let path = if path.is_symlink() {
        fs::canonicalize(path).context(invalid_config())?
    } else {
        path.to_owned()
    };
    let original = read(&path)?;
    let content = render(agent, original.as_deref(), endpoint, token)?;
    if original.as_deref() == Some(content.as_str()) {
        return Ok(());
    }
    save(&path, original.as_deref(), &content)
}

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct PendingFile(PathBuf);
impl Drop for PendingFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn save(path: &Path, original: Option<&str>, content: &str) -> Result<()> {
    let parent = path.parent().context(invalid_config())?;
    fs::create_dir_all(parent).context(tr!("Could not save the agent configuration."))?;
    let name = path
        .file_name()
        .context(invalid_config())?
        .to_string_lossy();
    let pending = PendingFile(parent.join(format!(
        ".{name}.musicx-{}-{}.tmp",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    )));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&pending.0)
        .context(tr!("Could not save the agent configuration."))?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    if read(path)?.as_deref() != original {
        bail!(tr!(
            "The agent configuration changed during setup. Please try again."
        ));
    }
    if let Some(original) = original {
        let backup = parent.join(format!("{name}.musicx-backup"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(backup)
        {
            Ok(mut backup) => {
                backup.write_all(original.as_bytes())?;
                backup.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    fs::rename(&pending.0, path).context(tr!("Could not save the agent configuration."))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "musicx-agent-test-{}-{}",
                std::process::id(),
                NEXT_FILE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    const ENDPOINT: &str = "http://127.0.0.1:45999/mcp";
    const TOKEN: &str = "test-only-token\"\\value\n";

    #[test]
    fn codex_preserves_comments_other_servers_and_tool_restrictions() {
        let directory = Directory::new();
        let path = directory.0.join("codex/config.toml");
        fs::create_dir(path.parent().unwrap()).unwrap();
        let original = "# Keep this comment\nmodel = 'example'\n[mcp_servers.other]\ncommand = 'other'\n[mcp_servers.musicx]\ncommand = 'old-transport'\ndisabled_tools = ['delete_playlist']\n[mcp_servers.musicx.http_headers]\nX-Custom = 'keep'\nauthorization = 'old'\n";
        fs::write(&path, original).unwrap();
        configure_at(Agent::Codex, &path, ENDPOINT, TOKEN).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        let config = content.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(content.contains("# Keep this comment"));
        assert_eq!(config["model"].as_str(), Some("example"));
        assert_eq!(
            config["mcp_servers"]["other"]["command"].as_str(),
            Some("other")
        );
        let server = &config["mcp_servers"]["musicx"];
        assert_eq!(server["url"].as_str(), Some(ENDPOINT));
        assert_eq!(
            server["http_headers"]["Authorization"].as_str(),
            Some(format!("Bearer {}", TOKEN.trim()).as_str())
        );
        assert_eq!(server["http_headers"]["X-Custom"].as_str(), Some("keep"));
        assert!(server.get("command").is_none());
        assert!(server["http_headers"].get("authorization").is_none());
        assert_eq!(
            server["disabled_tools"][0].as_str(),
            Some("delete_playlist")
        );
        assert_eq!(
            fs::read_to_string(path.with_file_name("config.toml.musicx-backup")).unwrap(),
            original
        );
        configure_at(Agent::Codex, &path, ENDPOINT, TOKEN).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn claude_preserves_settings_and_is_idempotent() {
        let directory = Directory::new();
        let path = directory.0.join("claude.json");
        let original = r#"{"theme":"dark","projects":{"/work":{"allowedTools":[]}},"mcpServers":{"other":{"command":"other"},"musicx":{"type":"stdio","command":"old","headers":{"X-Custom":"keep","authorization":"old"}}}}"#;
        fs::write(&path, original).unwrap();
        configure_at(Agent::ClaudeCode, &path, ENDPOINT, TOKEN).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(config["theme"], "dark");
        assert!(config["projects"]["/work"]["allowedTools"].is_array());
        assert_eq!(config["mcpServers"]["other"]["command"], "other");
        let server = &config["mcpServers"]["musicx"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["url"], ENDPOINT);
        assert_eq!(
            server["headers"]["Authorization"],
            format!("Bearer {}", TOKEN.trim())
        );
        assert_eq!(server["headers"]["X-Custom"], "keep");
        assert!(server.get("command").is_none());
        assert!(server["headers"].get("authorization").is_none());
        configure_at(Agent::ClaudeCode, &path, ENDPOINT, TOKEN).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), content);
    }

    #[test]
    fn malformed_and_conflicting_files_are_not_overwritten_or_exposed() {
        let directory = Directory::new();
        for (agent, content) in [
            (Agent::Codex, "secret = [broken"),
            (Agent::ClaudeCode, "{secret: broken"),
            (Agent::ClaudeCode, "[]"),
            (Agent::Codex, "mcp_servers = 1"),
        ] {
            let path = directory.0.join("invalid");
            fs::write(&path, content).unwrap();
            let error = configure_at(agent, &path, ENDPOINT, TOKEN)
                .unwrap_err()
                .to_string();
            assert!(!error.contains("secret"));
            assert_eq!(fs::read_to_string(&path).unwrap(), content);
        }
        let path = directory.0.join("changed");
        fs::write(&path, "new content").unwrap();
        assert!(save(&path, Some("old content"), "replacement").is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "new content");
    }

    #[test]
    fn new_configs_and_symlink_targets_are_supported() {
        let directory = Directory::new();
        for agent in [Agent::Codex, Agent::ClaudeCode] {
            let target = directory.0.join(agent.label());
            configure_at(agent, &target, ENDPOINT, TOKEN).unwrap();
            let link = directory.0.join(format!("{}.link", agent.label()));
            symlink(&target, &link).unwrap();
            configure_at(agent, &link, ENDPOINT, "new-test-token").unwrap();
            assert!(link.is_symlink());
            assert!(
                fs::read_to_string(target)
                    .unwrap()
                    .contains("new-test-token")
            );
        }
    }

    #[test]
    fn inline_codex_tables_are_preserved() {
        let original = "mcp_servers = { other = { command = 'other' } }\n";
        let content = render(Agent::Codex, Some(original), ENDPOINT, TOKEN).unwrap();
        let config = content.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            config["mcp_servers"]["other"]["command"].as_str(),
            Some("other")
        );
        assert_eq!(
            config["mcp_servers"]["musicx"]["url"].as_str(),
            Some(ENDPOINT)
        );
    }
}
