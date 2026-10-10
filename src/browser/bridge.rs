//! The app's end of the browser tools: a Unix socket the MCP server
//! (`mcp.rs`) connects to, one connection per tool call. A call is one line
//! of JSON each way; the app answers it on the UI thread (`app/browser.rs`).
//!
//! The socket sits in the user's temporary folder (private to them), named
//! for this process, so two BenCodes never answer each other's agents.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

/// The longest a call may take: a page load waits up to 30s.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ToolRequest {
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
    pub reply: oneshot::Sender<ToolReply>,
}

impl ToolCall {
    pub fn answer(self, reply: ToolReply) {
        if self.reply.send(reply).is_err() {
            log::debug!("browser tool answered after its caller left");
        }
    }
}

const SOCKET_PREFIX: &str = "bencode-browser-";
const SOCKET_SUFFIX: &str = ".sock";

fn socket_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "{SOCKET_PREFIX}{}{SOCKET_SUFFIX}",
        std::process::id()
    ))
}

/// The process a socket file is named for.
fn socket_pid(name: &str) -> Option<u32> {
    name.strip_prefix(SOCKET_PREFIX)?
        .strip_suffix(SOCKET_SUFFIX)?
        .parse()
        .ok()
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
        log::warn!("browser: could not remove {}: {err}", path.display());
    }
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("could not listen on {}", path.display()))?;
    std::thread::Builder::new()
        .name("browser-bridge".into())
        .spawn(move || {
            sweep_stale();
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let calls = calls.clone();
                        let spawned = std::thread::Builder::new()
                            .name("browser-call".into())
                            .spawn(move || {
                                if let Err(err) = serve(stream, &calls) {
                                    log::warn!("browser tool call failed: {err:#}");
                                }
                            });
                        if let Err(err) = spawned {
                            log::error!("browser: no thread for a tool call: {err}");
                        }
                    }
                    Err(err) => log::warn!("browser: connection refused: {err}"),
                }
            }
        })
        .context("could not start the browser bridge")?;
    Ok(path)
}

/// Removes the socket on the way out.
pub fn remove(path: &Path) {
    if let Err(err) = std::fs::remove_file(path)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        log::debug!("browser: could not remove {}: {err}", path.display());
    }
}

fn serve(stream: UnixStream, calls: &mpsc::UnboundedSender<ToolCall>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let request: ToolRequest = serde_json::from_str(&line).context("unreadable tool call")?;
    let (tx, rx) = oneshot::channel();
    let reply = if calls.send(ToolCall { request, reply: tx }).is_err() {
        ToolReply::error("BenCode is closing.")
    } else {
        rx.blocking_recv()
            .unwrap_or_else(|_| ToolReply::error("BenCode dropped the call."))
    };
    let mut out = serde_json::to_vec(&reply)?;
    out.push(b'\n');
    (&stream).write_all(&out)?;
    Ok(())
}

/// The MCP server's side: one call over a fresh connection. Blocking.
pub fn call(socket: &Path, request: &ToolRequest) -> Result<ToolReply> {
    let stream = UnixStream::connect(socket)
        .context("BenCode's browser is not reachable; is BenCode still open?")?;
    stream.set_read_timeout(Some(CALL_TIMEOUT))?;
    let mut out = serde_json::to_vec(request)?;
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
        assert_eq!(socket_pid("bencode-browser-41.sock"), Some(41));
        assert_eq!(socket_pid("bencode-browser-x.sock"), None);
        assert_eq!(socket_pid("pty.sock"), None);
        assert!(is_running(std::process::id()));
    }

    #[test]
    fn a_call_round_trips_through_the_socket() {
        let (tx, mut rx) = mpsc::unbounded_channel::<ToolCall>();
        let path = listen(tx).unwrap();
        std::thread::spawn(move || {
            let call = rx.blocking_recv().unwrap();
            assert_eq!(call.request.tool, "browser_snapshot");
            call.answer(ToolReply::text("page"));
        });
        let reply = call(
            &path,
            &ToolRequest {
                tool: "browser_snapshot".into(),
                args: Value::Null,
            },
        )
        .unwrap();
        assert!(reply.ok);
        assert_eq!(reply.text, "page");
        remove(&path);
    }
}
