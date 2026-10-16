//! The app's end of `bencode app` (MonoCode `control.rs`, its loopback
//! transport): a Unix socket in the user's temporary folder, named for this
//! process, one connection per call. A call is one line of JSON each way;
//! the app answers it on the UI thread.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use anyhow::{Context as _, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// MonoCode's cap on one call's input.
pub const MAX_REQUEST: usize = 262_144;
/// How long the app may take to answer (MonoCode's 35s).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(35);

/// A call as the CLI sends it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub token: String,
    pub action: String,
    pub input: Value,
    pub request_id: String,
}

/// A call waiting for the app.
pub struct AppCall {
    pub request: Request,
    reply: std::sync::mpsc::Sender<Value>,
}

impl AppCall {
    /// `{"ok":true,"result":…}`.
    pub fn answer(self, result: Value) {
        self.send(json!({ "ok": true, "result": result }));
    }

    /// `{"ok":false,"error":…}`; a call that never reached the executor is
    /// not worth retrying (MonoCode's `retryable: false`).
    pub fn fail(self, error: impl Into<String>, retryable: bool) {
        let mut reply = json!({ "ok": false, "error": error.into() });
        if !retryable {
            reply["retryable"] = json!(false);
        }
        self.send(reply);
    }

    fn send(self, reply: Value) {
        if self.reply.send(reply).is_err() {
            log::debug!("app call answered after its caller left");
        }
    }
}

const SOCKET_PREFIX: &str = "bencode-app-";
const SOCKET_SUFFIX: &str = ".sock";

fn socket_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "{SOCKET_PREFIX}{}{SOCKET_SUFFIX}",
        std::process::id()
    ))
}

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
pub fn listen(calls: mpsc::UnboundedSender<AppCall>) -> Result<PathBuf> {
    let path = socket_path();
    if path.exists()
        && let Err(err) = std::fs::remove_file(&path)
    {
        log::warn!("app access: could not remove {}: {err}", path.display());
    }
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("could not listen on {}", path.display()))?;
    std::thread::Builder::new()
        .name("app-access".into())
        .spawn(move || {
            sweep_stale();
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let calls = calls.clone();
                        let spawned = std::thread::Builder::new()
                            .name("app-call".into())
                            .spawn(move || serve(stream, &calls));
                        if let Err(err) = spawned {
                            log::error!("app access: no thread for a call: {err}");
                        }
                    }
                    Err(err) => log::warn!("app access: connection refused: {err}"),
                }
            }
        })
        .context("could not start app access")?;
    Ok(path)
}

/// Removes the socket on the way out.
pub fn remove(path: &Path) {
    if let Err(err) = std::fs::remove_file(path)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        log::debug!("app access: could not remove {}: {err}", path.display());
    }
}

fn serve(stream: UnixStream, calls: &mpsc::UnboundedSender<AppCall>) {
    let reply = read_request(&stream)
        .and_then(|request| {
            let (tx, rx) = std::sync::mpsc::channel();
            calls
                .send(AppCall { request, reply: tx })
                .map_err(|_| "BenCode is closing.".to_string())?;
            rx.recv_timeout(ANSWER_TIMEOUT).map_err(|err| match err {
                RecvTimeoutError::Timeout => {
                    "BenCode did not answer in time. Retry with the same request ID.".to_string()
                }
                RecvTimeoutError::Disconnected => "BenCode dropped the call.".to_string(),
            })
        })
        .unwrap_or_else(|error| json!({ "ok": false, "error": error }));
    let written = serde_json::to_vec(&reply)
        .map_err(std::io::Error::other)
        .and_then(|mut out| {
            out.push(b'\n');
            (&stream).write_all(&out)
        });
    if let Err(err) = written {
        log::debug!("app access: the caller left before its answer: {err}");
    }
}

fn read_request(stream: &UnixStream) -> Result<Request, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .take(MAX_REQUEST as u64 + 1024)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if line.len() > MAX_REQUEST + 1023 {
        return Err("Request exceeds 256 KiB".into());
    }
    let request: Request =
        serde_json::from_str(&line).map_err(|_| "Invalid app request".to_string())?;
    if !request.input.is_object() || request.request_id.is_empty() || request.request_id.len() > 128
    {
        return Err("Invalid input or request ID".into());
    }
    Ok(request)
}

/// The CLI's side: one call over a fresh connection. Blocking.
pub fn call(socket: &Path, line: &Value) -> Result<Value, (String, bool)> {
    let stream = UnixStream::connect(socket).map_err(|err| {
        (
            format!(
                "Cannot reach BenCode at {}: {err}. BenCode may have restarted, or this agent's \
                 sandbox may be blocking it.",
                socket.display()
            ),
            false,
        )
    })?;
    let sent = |err: std::io::Error| (err.to_string(), true);
    stream
        .set_read_timeout(Some(ANSWER_TIMEOUT + Duration::from_secs(5)))
        .map_err(sent)?;
    let mut out = serde_json::to_vec(line).map_err(|err| (err.to_string(), false))?;
    out.push(b'\n');
    (&stream).write_all(&out).map_err(sent)?;
    let mut reply = String::new();
    BufReader::new(&stream)
        .take(4_000_001)
        .read_line(&mut reply)
        .map_err(|err| (format!("No reply from BenCode: {err}"), true))?;
    serde_json::from_str(&reply).map_err(|_| ("BenCode returned an invalid response".into(), true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sockets_are_named_for_their_process() {
        let name = socket_path().file_name().unwrap().to_owned();
        assert_eq!(socket_pid(name.to_str().unwrap()), Some(std::process::id()));
        assert_eq!(socket_pid("bencode-browser-41.sock"), None);
    }

    #[test]
    fn a_call_round_trips_through_the_socket() {
        let (tx, mut rx) = mpsc::unbounded_channel::<AppCall>();
        let path = listen(tx).unwrap();
        std::thread::spawn(move || {
            let call = rx.blocking_recv().unwrap();
            assert_eq!(call.request.action, "artifacts.list");
            assert_eq!(call.request.token, "t");
            call.answer(json!({ "total": 0 }));
        });
        let reply = call(
            &path,
            &json!({ "token": "t", "action": "artifacts.list", "input": {}, "requestId": "r1" }),
        )
        .unwrap();
        assert_eq!(reply, json!({ "ok": true, "result": { "total": 0 } }));
        let bad = call(&path, &json!({ "token": "t" })).unwrap();
        assert_eq!(bad["ok"], json!(false));
        remove(&path);
    }
}
