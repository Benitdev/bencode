//! The user's own MCP servers for a Claude account profile. A profile's
//! `CLAUDE_CONFIG_DIR` makes Claude read user and local servers from the
//! profile's `.claude.json`, not `~/.claude.json`, so a thread on another
//! account would start without them. Every account shares the default
//! one's servers: they go to the CLI as an `--mcp-config` file in the
//! profile (it may hold tokens, so never on argv).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value, json};

use crate::harness::accounts::AccountProfile;

/// `~/.claude.json` keeps per-project history; past this it is skipped.
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
/// The file written into the profile, read by Claude as it starts.
const FILE_NAME: &str = "bencode-mcp.json";

/// Writes the default account's servers for `cwd` into `profile` and
/// returns the file, or None when there are none (or it could not be done,
/// logged). Reads one small file: called as the child starts.
pub fn config_file(profile: &AccountProfile, cwd: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let config = read_config(&home.join(".claude.json"))?;
    let servers = shared_servers(&config, cwd);
    if servers.is_empty() {
        return None;
    }
    let path = profile.dir.join(FILE_NAME);
    match write_private(&path, &json!({ "mcpServers": servers })) {
        Ok(()) => Some(path),
        Err(err) => {
            log::warn!("could not share MCP servers with an account: {err:#}");
            None
        }
    }
}

fn read_config(path: &Path) -> Option<Value> {
    let len = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            log::warn!("mcp: cannot read {}: {err}", path.display());
            return None;
        }
    };
    if len > MAX_CONFIG_BYTES {
        log::warn!("mcp: {} is {len} bytes, not shared", path.display());
        return None;
    }
    let parsed = std::fs::read(path)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| Ok(serde_json::from_slice(&bytes)?));
    parsed
        .inspect_err(|err| log::warn!("mcp: cannot read {}: {err:#}", path.display()))
        .ok()
}

/// The user's servers, then the project's local ones over them (Claude's
/// own precedence), less those turned off for the project.
fn shared_servers(config: &Value, cwd: &str) -> Map<String, Value> {
    let project = config.get("projects").and_then(|p| p.get(cwd));
    let servers_of = |entry: Option<&Value>| {
        entry
            .and_then(|e| e.get("mcpServers"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    };
    let mut servers = servers_of(Some(config));
    servers.extend(servers_of(project));
    let disabled = project
        .and_then(|p| p.get("disabledMcpServers"))
        .and_then(Value::as_array);
    for name in disabled.into_iter().flatten().filter_map(Value::as_str) {
        servers.remove(name);
    }
    servers
}

/// Replaces `path` whole, readable by the user alone: a turn starting
/// meanwhile reads the old file or the new one, never half of either.
fn write_private(path: &Path, value: &Value) -> anyhow::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let temp = dir.join(format!(
        ".{FILE_NAME}.{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options
        .open(&temp)
        .and_then(|mut file| file.write_all(&serde_json::to_vec(value)?))
        .and_then(|()| std::fs::rename(&temp, path));
    if written.is_err() {
        // Already gone after a rename; the error that matters is above.
        let _ = std::fs::remove_file(&temp);
    }
    Ok(written?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_servers_override_user_ones_and_disabled_ones_drop() {
        let config = json!({
            "mcpServers": {
                "backlog": { "command": "npx", "args": ["backlog"] },
                "docs": { "command": "docs" },
            },
            "projects": {
                "/repo": {
                    "mcpServers": { "docs": { "command": "docs-local" } },
                    "disabledMcpServers": ["backlog"],
                },
            },
        });
        let here = shared_servers(&config, "/repo");
        assert_eq!(here.keys().collect::<Vec<_>>(), ["docs"]);
        assert_eq!(here["docs"]["command"], "docs-local");
        let elsewhere = shared_servers(&config, "/other");
        assert_eq!(elsewhere.len(), 2);
        assert_eq!(elsewhere["docs"]["command"], "docs");
    }

    #[test]
    fn no_servers_is_empty() {
        assert!(shared_servers(&json!({}), "/repo").is_empty());
    }

    #[test]
    fn the_file_is_private_and_whole() {
        let dir = std::env::temp_dir().join(format!("bencode-mcp-{}", std::process::id()));
        let path = dir.join("profile").join(FILE_NAME);
        write_private(&path, &json!({ "mcpServers": {} })).unwrap();
        write_private(&path, &json!({ "mcpServers": { "a": {} } })).unwrap();
        let read: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(read["mcpServers"]["a"].is_object());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Only the file itself is left: no temporary copies.
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
