//! The terminal host: the dock's shells run in `bencode --pty-host`, a
//! process of their own, so an update's restart or a crash does not end
//! them (BenCode's own, after Orca's terminal daemon; MonoCode's PTYs live
//! in its Tauri process). Each dock tab runs `bencode --pty-attach`, which
//! relays its keys, output and size to its session over a Unix socket and
//! starts the host when none runs. ⌘Q ends the app's sessions; the app
//! does that, the host only ends with its last one.
//!
//! One host serves a data folder: it holds `pty-host.lock` while it runs,
//! so only it ever touches the socket's name.

mod attach;
mod host;
mod protocol;

use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

pub use protocol::{ControlReply, ControlRequest, SessionInfo};
use protocol::{Frame, PROTOCOL_VERSION, read_frame, write_frame};

const SOCKET: &str = "pty.sock";
const LOCK: &str = "pty-host.lock";
const HOST_ARG: &str = "--pty-host";
const ATTACH_ARG: &str = "--pty-attach";

/// How long a control request may take: it only reads the host's table.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);

fn socket_path() -> Option<PathBuf> {
    crate::storage::data_dir().map(|dir| dir.join(SOCKET))
}

fn lock_path() -> Option<PathBuf> {
    crate::storage::data_dir().map(|dir| dir.join(LOCK))
}

/// Runs the host or an attach client when `main` was started as one;
/// returns the exit code. `None`: this is the app.
pub fn run_from_args() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some(HOST_ARG) => Some(host::run()),
        Some(ATTACH_ARG) => {
            let id = args.next()?;
            let cwd = args.next().unwrap_or_default();
            Some(attach::run(&id, &cwd))
        }
        _ => None,
    }
}

/// What a dock tab runs to show session `id`, started in `cwd` when new.
pub fn attach_command(id: &str, cwd: &str) -> Result<(String, Vec<String>)> {
    let exe = std::env::current_exe().context("cannot find BenCode's executable")?;
    Ok((
        exe.to_string_lossy().into_owned(),
        vec![ATTACH_ARG.to_string(), id.to_string(), cwd.to_string()],
    ))
}

/// A new session id, unique in this data folder.
pub fn new_session_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!(
        "t{}-{}-{n}",
        jiff::Timestamp::now().as_millisecond(),
        std::process::id()
    )
}

/// Opens a connection and says hello; `NotFound` / `ConnectionRefused`
/// mean no host runs.
fn connect() -> io::Result<UnixStream> {
    let path =
        socket_path().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))?;
    let mut stream = UnixStream::connect(path)?;
    write_frame(&mut stream, &Frame::Hello(PROTOCOL_VERSION))?;
    Ok(stream)
}

/// Asks the running host; `Ok(None)` when none runs. Blocking: call it
/// off the UI thread, or on the way out.
pub fn control(request: &ControlRequest) -> Result<Option<ControlReply>> {
    let mut stream = match connect() {
        Ok(stream) => stream,
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(None);
        }
        Err(err) => return Err(err).context("could not reach the terminal host"),
    };
    stream.set_read_timeout(Some(CONTROL_TIMEOUT))?;
    stream.set_write_timeout(Some(CONTROL_TIMEOUT))?;
    write_frame(&mut stream, &Frame::Control(serde_json::to_vec(request)?))?;
    loop {
        match read_frame(&mut stream)? {
            Some(Frame::Hello(version)) if version == PROTOCOL_VERSION => continue,
            Some(Frame::Hello(version)) => bail!("the terminal host speaks version {version}"),
            Some(Frame::ControlReply(body)) => return Ok(Some(serde_json::from_slice(&body)?)),
            Some(Frame::Error(message)) => bail!("terminal host: {message}"),
            Some(other) => bail!("unexpected reply from the terminal host: {other:?}"),
            None => bail!("the terminal host closed the connection"),
        }
    }
}

/// The sessions the host has; empty when none runs.
pub fn list_sessions() -> Result<Vec<SessionInfo>> {
    match control(&ControlRequest::List)? {
        Some(ControlReply::Sessions(sessions)) => Ok(sessions),
        Some(other) => bail!("unexpected reply to List: {other:?}"),
        None => Ok(Vec::new()),
    }
}

/// Ends these sessions' shells and forgets them.
pub fn kill_sessions(ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    if let Err(err) = control(&ControlRequest::Kill { ids }) {
        log::warn!("could not end terminal sessions: {err:#}");
    }
}
