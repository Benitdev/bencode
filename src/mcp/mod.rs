//! Model Context Protocol (MCP) server discovery and configuration.
//!
//! Ported from MonoCode (`src-tauri/src/mcp.rs`).
//! Discovers configured MCP servers across the Claude CLI (`~/.claude.json`),
//! Claude Desktop, Cursor, and project-local JSON configurations.
//!
//! This module only *discovers* configured servers. It does not supervise
//! server processes or speak JSON-RPC to them. Codex (`~/.codex/config.toml`)
//! is not read: BenCode has no direct TOML dependency.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConnection {
    pub provider: String,
    pub name: String,
    pub scope: String,
    pub config_path: String,
    pub transport: String,
    pub enabled: bool,
}

fn claude_desktop_config(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support/Claude/claude_desktop_config.json")
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
            .join("Claude/claude_desktop_config.json")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        home.join(".config/Claude/claude_desktop_config.json")
    }
}

/// Config files larger than this are skipped. `~/.claude.json` keeps per-project
/// history and can grow very large; parsing it on the UI thread would stall.
const MAX_CONFIG_BYTES: u64 = 2 * 1024 * 1024;

/// Reads and parses a JSON config, skipping (and logging) files that are
/// missing, oversized, unreadable, or malformed.
fn read_json(path: &Path) -> Option<Value> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            log::warn!("mcp: cannot open {}: {err}", path.display());
            return None;
        }
    };
    let len = match file.metadata() {
        Ok(meta) => meta.len(),
        Err(err) => {
            log::warn!("mcp: cannot stat {}: {err}", path.display());
            return None;
        }
    };
    if len > MAX_CONFIG_BYTES {
        log::warn!(
            "mcp: skipping {} ({len} bytes exceeds {MAX_CONFIG_BYTES} byte limit)",
            path.display()
        );
        return None;
    }
    // Bound the read as well, in case the file grows between stat and read.
    let mut bytes = Vec::new();
    if let Err(err) = file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes) {
        log::warn!("mcp: cannot read {}: {err}", path.display());
        return None;
    }
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        log::warn!("mcp: skipping {} (grew past size limit)", path.display());
        return None;
    }
    match serde_json::from_slice(&bytes) {
        Ok(value) => Some(value),
        Err(err) => {
            log::warn!("mcp: invalid JSON in {}: {err}", path.display());
            None
        }
    }
}

fn add_json_servers(
    connections: &mut Vec<McpConnection>,
    provider: &str,
    scope: &str,
    path: &Path,
    servers: Option<&Value>,
) {
    let Some(servers) = servers.and_then(Value::as_object) else {
        return;
    };
    for (name, entry) in servers {
        let transport = if entry.get("url").and_then(Value::as_str).is_some() {
            "sse"
        } else {
            "stdio"
        };
        let enabled = !entry
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        connections.push(McpConnection {
            provider: provider.to_string(),
            name: name.clone(),
            scope: scope.to_string(),
            config_path: path.to_string_lossy().into_owned(),
            transport: transport.to_string(),
            enabled,
        });
    }
}

fn add_json_file(
    connections: &mut Vec<McpConnection>,
    provider: &str,
    scope: &str,
    path: &Path,
    servers_key: &str,
) {
    if let Some(config) = read_json(path) {
        add_json_servers(connections, provider, scope, path, config.get(servers_key));
    }
}

/// Discovers all available MCP servers configured on the system or in the workspace.
pub fn discover_mcp_servers(cwd: &str) -> Vec<McpConnection> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut connections = Vec::new();

    let Some(home) = home else {
        return connections;
    };

    let project = Path::new(cwd);

    // 1. Claude CLI user and local configs (~/.claude.json)
    let claude_json = home.join(".claude.json");
    if let Some(config) = read_json(&claude_json) {
        add_json_servers(
            &mut connections,
            "claude",
            "user",
            &claude_json,
            config.get("mcpServers"),
        );
        add_json_servers(
            &mut connections,
            "claude",
            "project",
            &claude_json,
            config
                .get("projects")
                .and_then(|p| p.get(project.to_string_lossy().as_ref()))
                .and_then(|entry| entry.get("mcpServers")),
        );
    }

    // 2. Cursor user config (~/.cursor/mcp.json)
    let cursor_json = home.join(".cursor/mcp.json");
    add_json_file(
        &mut connections,
        "cursor",
        "user",
        &cursor_json,
        "mcpServers",
    );

    // 3. Project-level Cursor config (<project>/.cursor/mcp.json)
    let project_cursor = project.join(".cursor/mcp.json");
    add_json_file(
        &mut connections,
        "cursor",
        "project",
        &project_cursor,
        "mcpServers",
    );

    // 4. Claude Desktop config
    let desktop_config = claude_desktop_config(&home);
    add_json_file(
        &mut connections,
        "claude_desktop",
        "user",
        &desktop_config,
        "mcpServers",
    );

    // 5. BenCode project config (<project>/.bencode/mcp.json or <project>/.mcp.json)
    let bencode_mcp = project.join(".bencode/mcp.json");
    add_json_file(
        &mut connections,
        "bencode",
        "project",
        &bencode_mcp,
        "mcpServers",
    );

    let dot_mcp = project.join(".mcp.json");
    add_json_file(
        &mut connections,
        "project",
        "project",
        &dot_mcp,
        "mcpServers",
    );

    connections
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mcp_servers_from_json_value() {
        let json: Value = serde_json::json!({
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
                },
                "remote-docs": {
                    "url": "https://mcp.example.com/sse",
                    "disabled": true
                }
            }
        });

        let mut connections = Vec::new();
        let dummy_path = Path::new("/path/to/config.json");
        add_json_servers(
            &mut connections,
            "test_provider",
            "user",
            dummy_path,
            json.get("mcpServers"),
        );

        assert_eq!(connections.len(), 2);

        let fs = connections.iter().find(|c| c.name == "filesystem").unwrap();
        assert_eq!(fs.provider, "test_provider");
        assert_eq!(fs.scope, "user");
        assert_eq!(fs.transport, "stdio");
        assert!(fs.enabled);

        let remote = connections
            .iter()
            .find(|c| c.name == "remote-docs")
            .unwrap();
        assert_eq!(remote.transport, "sse");
        assert!(!remote.enabled);
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bencode-mcp-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_json_parses_small_config() {
        let dir = temp_dir("small");
        let path = dir.join("mcp.json");
        std::fs::write(&path, r#"{"mcpServers":{"a":{"command":"x"}}}"#).unwrap();

        let value = read_json(&path).expect("small config parses");

        assert!(value.get("mcpServers").is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_json_skips_configs_over_size_cap() {
        let dir = temp_dir("big");
        let path = dir.join("huge.json");
        let padding = " ".repeat(MAX_CONFIG_BYTES as usize + 16);
        std::fs::write(&path, format!(r#"{{"mcpServers":{{}}}}{padding}"#)).unwrap();

        assert!(read_json(&path).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_json_returns_none_for_missing_or_invalid_files() {
        let dir = temp_dir("bad");
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{not json").unwrap();

        assert!(read_json(&dir.join("missing.json")).is_none());
        assert!(read_json(&bad).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
