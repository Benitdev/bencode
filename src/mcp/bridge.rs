//! The app's end of the browser and desktop MCP servers: a Unix socket,
//! one connection and one line of JSON each way per call. The app picks
//! up calls on the UI thread and hands desktop actions to a worker.
//!
//! The socket sits in the user's temporary folder (private to them), named
//! for this process, so two BenCodes never answer each other's agents.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

/// The longest a call may take: a page load waits up to 30s.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(90);
/// Cap on one tool call's JSON line (matches `app_cli::server::MAX_REQUEST`).
const MAX_REQUEST: usize = 262_144;

#[derive(Default, Serialize, Deserialize)]
pub struct ToolRequest {
    #[serde(default)]
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub computer_turn: Option<String>,
    pub tool: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ToolReply {
    pub ok: bool,
    pub text: String,
    /// A PNG, base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

impl ToolReply {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            ok: true,
            text: text.into(),
            image: None,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            ok: false,
            text: text.into(),
            image: None,
        }
    }
}

/// A call waiting for the app.
pub struct ToolCall {
    pub request: ToolRequest,
    pub reply: std::sync::mpsc::Sender<ToolReply>,
}

impl ToolCall {
    pub fn answer(self, reply: ToolReply) {
        if self.reply.send(reply).is_err() {
            log::debug!("MCP tool answered after its caller left");
        }
    }
}

const SOCKET_PREFIX: &str = "bencode-tools-";
const SOCKET_SUFFIX: &str = ".sock";
const TOKEN_SUFFIX: &str = ".token";

fn socket_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "{SOCKET_PREFIX}{}{SOCKET_SUFFIX}",
        std::process::id()
    ))
}

fn token_path(socket: &Path) -> PathBuf {
    let mut name = socket.as_os_str().to_os_string();
    name.push(TOKEN_SUFFIX);
    PathBuf::from(name)
}

/// The process a socket or token file is named for.
fn socket_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(SOCKET_PREFIX)?;
    let pid = rest
        .strip_suffix(SOCKET_SUFFIX)
        .or_else(|| rest.strip_suffix(&format!("{SOCKET_SUFFIX}{TOKEN_SUFFIX}")))?;
    pid.parse().ok()
}

fn write_token(path: &Path, token: &str) -> Result<()> {
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("could not create {}", path.display()))?;
    file.write_all(token.as_bytes())
        .with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

/// Constant-time comparison so the token is not leaked by timing.
pub(crate) fn token_matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `kill` with no signal only asks; a process of another user (EPERM) runs.
fn is_running(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return true;
    };
    // SAFETY: signal 0 sends nothing.
    let answered = unsafe { libc::kill(pid, 0) } == 0;
    answered || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Removes the sockets of BenCodes that crashed (one that quits removes
/// its own).
fn sweep_stale() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .file_name()
            .to_str()
            .and_then(socket_pid)
            .is_some_and(|pid| pid != std::process::id() && !is_running(pid));
        if stale {
            remove(&entry.path());
        }
    }
}

/// Starts listening; calls arrive on `calls`. Returns the socket's path.
pub fn listen(calls: mpsc::UnboundedSender<ToolCall>) -> Result<PathBuf> {
    let path = socket_path();
    // A socket of an earlier process with this pid.
    if path.exists()
        && let Err(err) = std::fs::remove_file(&path)
    {
        log::warn!("MCP bridge: could not remove {}: {err}", path.display());
    }
    let token = crate::app_cli::random_hex(32);
    let token_file = token_path(&path);
    write_token(&token_file, &token)?;
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("could not listen on {}", path.display()))?;
    if let Err(err) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
        log::warn!("MCP bridge: could not restrict {}: {err}", path.display());
    }
    std::thread::Builder::new()
        .name("mcp-bridge".into())
        .spawn(move || {
            sweep_stale();
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let calls = calls.clone();
                        let token = token.clone();
                        let spawned =
                            std::thread::Builder::new()
                                .name("mcp-call".into())
                                .spawn(move || {
                                    if let Err(err) = serve(stream, &calls, &token) {
                                        log::warn!("MCP bridge call failed: {err:#}");
                                    }
                                });
                        if let Err(err) = spawned {
                            log::error!("MCP bridge: no thread for a tool call: {err}");
                        }
                    }
                    Err(err) => log::warn!("MCP bridge: connection refused: {err}"),
                }
            }
        })
        .context("could not start the MCP bridge")?;
    Ok(path)
}

/// Removes the socket and its token file on the way out.
pub fn remove(path: &Path) {
    for target in [path.to_path_buf(), token_path(path)] {
        if let Err(err) = std::fs::remove_file(&target)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            log::debug!("MCP bridge: could not remove {}: {err}", target.display());
        }
    }
}

fn serve(
    stream: UnixStream,
    calls: &mpsc::UnboundedSender<ToolCall>,
    expected_token: &str,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut line = String::new();
    BufReader::new(&stream)
        .take(MAX_REQUEST as u64 + 1)
        .read_line(&mut line)?;
    let reply = if line.len() > MAX_REQUEST {
        ToolReply::error("Request exceeds 256 KiB.")
    } else {
        let request: ToolRequest = serde_json::from_str(&line).context("unreadable tool call")?;
        if !token_matches(expected_token, &request.token) {
            ToolReply::error("Unauthorized MCP tool call.")
        } else {
            let (tx, rx) = std::sync::mpsc::channel();
            if calls.send(ToolCall { request, reply: tx }).is_err() {
                ToolReply::error("BenCode is closing.")
            } else {
                rx.recv_timeout(CALL_TIMEOUT)
                    .unwrap_or_else(|err| match err {
                        RecvTimeoutError::Timeout => {
                            ToolReply::error("BenCode did not answer the tool call in time.")
                        }
                        RecvTimeoutError::Disconnected => {
                            ToolReply::error("BenCode dropped the call.")
                        }
                    })
            }
        }
    };
    let mut out = serde_json::to_vec(&reply)?;
    out.push(b'\n');
    (&stream).write_all(&out)?;
    Ok(())
}

/// The MCP server's side: one call over a fresh connection. Blocking.
pub fn call(socket: &Path, request: &ToolRequest) -> Result<ToolReply> {
    let stream = UnixStream::connect(socket)
        .context("BenCode's MCP bridge is not reachable. Is BenCode still open?")?;
    stream.set_read_timeout(Some(CALL_TIMEOUT))?;
    let mut authed = ToolRequest {
        token: request.token.clone(),
        computer_turn: request.computer_turn.clone(),
        tool: request.tool.clone(),
        args: request.args.clone(),
    };
    if authed.token.is_empty() {
        authed.token = std::fs::read_to_string(token_path(socket))
            .context("could not read the BenCode MCP bridge token")?
            .trim()
            .to_string();
    }
    let mut out = serde_json::to_vec(&authed)?;
    out.push(b'\n');
    (&stream).write_all(&out)?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    serde_json::from_str(&line).context("unreadable reply from BenCode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sockets_are_named_for_their_process() {
        let name = socket_path().file_name().unwrap().to_owned();
        assert_eq!(socket_pid(name.to_str().unwrap()), Some(std::process::id()));
        assert_eq!(socket_pid("bencode-tools-41.sock"), Some(41));
        assert_eq!(socket_pid("bencode-tools-41.sock.token"), Some(41));
        assert_eq!(socket_pid("bencode-tools-x.sock"), None);
        assert_eq!(socket_pid("pty.sock"), None);
        assert!(is_running(std::process::id()));
    }

    #[test]
    fn a_call_round_trips_through_the_socket_and_rejects_bad_requests() {
        let (tx, mut rx) = mpsc::unbounded_channel::<ToolCall>();
        let path = listen(tx).unwrap();
        let sock_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let tok_mode = std::fs::metadata(token_path(&path))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(sock_mode, 0o600);
        assert_eq!(tok_mode, 0o600);

        // Wrong token is rejected before reaching the app channel.
        let unauth = call(
            &path,
            &ToolRequest {
                token: "wrong-token".into(),
                computer_turn: None,
                tool: "browser_snapshot".into(),
                args: Value::Null,
            },
        )
        .unwrap();
        assert!(!unauth.ok);
        assert!(unauth.text.contains("Unauthorized"), "{}", unauth.text);

        // Oversized request is rejected.
        let big = call(
            &path,
            &ToolRequest {
                token: String::new(),
                computer_turn: None,
                tool: "browser_evaluate".into(),
                args: Value::String("x".repeat(MAX_REQUEST + 10)),
            },
        )
        .unwrap();
        assert!(!big.ok);
        assert!(big.text.contains("256 KiB"), "{}", big.text);

        // Valid request (auto-reads token from the 0600 token file) succeeds.
        std::thread::spawn(move || {
            let call = rx.blocking_recv().unwrap();
            assert_eq!(call.request.tool, "browser_snapshot");
            call.answer(ToolReply::text("page"));
        });
        let reply = call(
            &path,
            &ToolRequest {
                token: String::new(),
                computer_turn: None,
                tool: "browser_snapshot".into(),
                args: Value::Null,
            },
        )
        .unwrap();
        assert!(reply.ok);
        assert_eq!(reply.text, "page");
        remove(&path);
        assert!(!token_path(&path).exists());
    }
}
