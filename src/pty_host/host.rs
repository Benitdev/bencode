//! `bencode --pty-host`: holds the dock's shells. Each session is a PTY,
//! the shell on it (started as Alacritty starts it, which is what Ely's
//! terminal did before), the last output for showing it again, and the one
//! client showing it now. The host leaves once nothing has run and nobody
//! has been attached for a while.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use super::protocol::{
    AttachRequest, Attached, ControlReply, ControlRequest, Frame, PROTOCOL_VERSION, SessionInfo,
    read_frame, write_frame,
};

/// The output kept to show a session again.
const HISTORY: usize = 1024 * 1024;
/// How long the host stays with nothing running and nobody attached.
const IDLE_EXIT: Duration = Duration::from_secs(60);
/// A client that stops reading for this long is let go, not waited on.
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

pub fn run() -> i32 {
    crate::logging::init_file("pty-host.log");
    let (Some(socket), Some(lock)) = (super::socket_path(), super::lock_path()) else {
        log::error!("pty host: no data folder");
        return 1;
    };
    // Another host serves this folder; the client that started this one
    // reaches that one instead.
    let Some(_lock) = claim(&lock) else {
        return 0;
    };
    match serve(&socket, IDLE_EXIT) {
        Ok(()) => 0,
        Err(err) => {
            log::error!("pty host: {err:#}");
            1
        }
    }
}

/// Locks `path`; `None` while another process holds it.
fn claim(path: &Path) -> Option<File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .inspect_err(|err| log::error!("pty host: cannot open {}: {err}", path.display()))
        .ok()?;
    // SAFETY: `flock` on a descriptor this function owns.
    (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(file)
}

/// Serves `socket` until the host has been idle for `idle`. The caller
/// holds the lock, so the name is this host's to replace and remove.
pub(super) fn serve(socket: &Path, idle: Duration) -> Result<()> {
    match std::fs::remove_file(socket) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| format!("cannot replace {}", socket.display()));
        }
    }
    // Only this user may reach the shells.
    // SAFETY: `umask` only swaps the process's file mode mask.
    let mask = unsafe { libc::umask(0o077) };
    let listener = UnixListener::bind(socket);
    unsafe { libc::umask(mask) };
    let listener = listener.with_context(|| format!("cannot listen on {}", socket.display()))?;
    listener.set_nonblocking(true)?;
    let host = Arc::new(Host::default());
    let mut busy_at = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                busy_at = Instant::now();
                let host = host.clone();
                std::thread::spawn(move || {
                    if let Err(err) = host.serve_connection(stream) {
                        log::warn!("pty host: a connection failed: {err:#}");
                    }
                });
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                if host.busy() {
                    busy_at = Instant::now();
                } else if busy_at.elapsed() >= idle {
                    if let Err(err) = std::fs::remove_file(socket) {
                        log::warn!("pty host: could not remove {}: {err}", socket.display());
                    }
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(err).context("accept"),
        }
    }
}

#[derive(Default)]
struct Host {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    next_client: AtomicU64,
}

struct Session {
    id: String,
    pid: u32,
    master: OwnedFd,
    child: Mutex<Option<Child>>,
    state: Mutex<SessionState>,
}

#[derive(Default)]
struct SessionState {
    history: VecDeque<u8>,
    /// The client showing the session, by its number.
    client: Option<(u64, UnixStream)>,
    exited: Option<i32>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Host {
    /// A session still runs, or a client is attached to one.
    fn busy(&self) -> bool {
        lock(&self.sessions).values().any(|session| {
            let state = lock(&session.state);
            state.exited.is_none() || state.client.is_some()
        })
    }

    fn serve_connection(self: &Arc<Self>, mut stream: UnixStream) -> Result<()> {
        stream.set_nonblocking(false)?;
        match read_frame(&mut stream)? {
            Some(Frame::Hello(PROTOCOL_VERSION)) => {
                write_frame(&mut stream, &Frame::Hello(PROTOCOL_VERSION))?
            }
            Some(Frame::Hello(version)) => {
                let message =
                    format!("this terminal host speaks version {PROTOCOL_VERSION}, not {version}");
                write_frame(&mut stream, &Frame::Error(message))?;
                return Ok(());
            }
            _ => bail!("a connection that did not say hello"),
        }
        match read_frame(&mut stream)? {
            Some(Frame::Attach(body)) => self.attach(stream, serde_json::from_slice(&body)?),
            Some(Frame::Control(body)) => {
                let reply = self.control(serde_json::from_slice(&body)?);
                write_frame(
                    &mut stream,
                    &Frame::ControlReply(serde_json::to_vec(&reply)?),
                )?;
                Ok(())
            }
            None => Ok(()),
            Some(other) => bail!("unexpected {other:?}"),
        }
    }

    fn control(&self, request: ControlRequest) -> ControlReply {
        match request {
            ControlRequest::List => {
                let sessions = lock(&self.sessions);
                let mut list: Vec<SessionInfo> = sessions
                    .values()
                    .map(|s| SessionInfo {
                        id: s.id.clone(),
                        pid: s.pid,
                        alive: lock(&s.state).exited.is_none(),
                    })
                    .collect();
                list.sort_by(|a, b| a.id.cmp(&b.id));
                ControlReply::Sessions(list)
            }
            ControlRequest::Kill { ids } => {
                let mut sessions = lock(&self.sessions);
                for id in ids {
                    if let Some(session) = sessions.remove(&id) {
                        session.hang_up();
                    }
                }
                ControlReply::Done
            }
        }
    }

    fn attach(self: &Arc<Self>, mut stream: UnixStream, request: AttachRequest) -> Result<()> {
        let existing = lock(&self.sessions).get(&request.id).cloned();
        let (session, created) = match existing {
            Some(session) => (session, false),
            None => match Session::spawn(&request) {
                Ok(session) => {
                    let session = Arc::new(session);
                    lock(&self.sessions).insert(request.id.clone(), session.clone());
                    let reader = session.clone();
                    std::thread::spawn(move || reader.pump());
                    (session, true)
                }
                Err(err) => {
                    write_frame(&mut stream, &Frame::Error(format!("{err:#}")))?;
                    return Ok(());
                }
            },
        };
        let client = self.next_client.fetch_add(1, Ordering::Relaxed);
        stream.set_write_timeout(Some(CLIENT_WRITE_TIMEOUT))?;
        let mut writer = stream.try_clone()?;
        {
            // The history and the hand-over in one lock: output the shell
            // writes meanwhile goes after it, once.
            let mut state = lock(&session.state);
            let attached = Attached {
                pid: session.pid,
                created,
            };
            write_frame(
                &mut writer,
                &Frame::Attached(serde_json::to_vec(&attached)?),
            )?;
            let (front, back) = state.history.as_slices();
            for part in [front, back].into_iter().filter(|p| !p.is_empty()) {
                write_frame(&mut writer, &Frame::Output(part.to_vec()))?;
            }
            if let Some(code) = state.exited {
                write_frame(&mut writer, &Frame::Exited(code))?;
            }
            if let Some((_, previous)) = state.client.replace((client, writer)) {
                // One client at a time: a window shown again takes it over.
                if let Err(err) = previous.shutdown(std::net::Shutdown::Both) {
                    log::debug!("pty host: previous client of {}: {err}", session.id);
                }
            }
        }
        if !created {
            // A full-screen program draws itself again on a size change.
            session.nudge(request.cols, request.rows);
        }
        loop {
            match read_frame(&mut stream) {
                Ok(Some(Frame::Input(bytes))) => session.write(&bytes),
                Ok(Some(Frame::Resize(cols, rows))) => session.resize(cols, rows),
                Ok(Some(other)) => log::debug!("pty host: ignored {other:?}"),
                Ok(None) => break,
                Err(err) => {
                    log::debug!("pty host: client of {} left: {err}", session.id);
                    break;
                }
            }
        }
        let mut state = lock(&session.state);
        if state.client.as_ref().is_some_and(|(id, _)| *id == client) {
            state.client = None;
        }
        Ok(())
    }
}

impl Session {
    fn spawn(request: &AttachRequest) -> Result<Self> {
        let mut size = libc::winsize {
            ws_row: request.rows.max(1),
            ws_col: request.cols.max(1),
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let (mut master, mut slave): (RawFd, RawFd) = (-1, -1);
        // SAFETY: `openpty` fills the two descriptors; no name buffer.
        let opened = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if opened != 0 {
            return Err(io::Error::last_os_error()).context("openpty");
        }
        // SAFETY: both were just opened and are owned here.
        let (master, slave) =
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        // No other shell may inherit them: a session ends when its master
        // reads end of file, which another holder of the slave would stop.
        for fd in [&master, &slave] {
            // SAFETY: `fcntl` on descriptors owned here.
            unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        set_utf8(&master);

        let user = ShellUser::from_env();
        let mut command = shell_command(&user);
        command
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave))
            .env("USER", &user.name)
            .env("HOME", &user.home)
            .envs(request.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let cwd = Path::new(&request.cwd);
        command.current_dir(if cwd.is_dir() {
            cwd
        } else {
            Path::new(&user.home)
        });
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                for signal in [
                    libc::SIGCHLD,
                    libc::SIGHUP,
                    libc::SIGINT,
                    libc::SIGQUIT,
                    libc::SIGTERM,
                    libc::SIGALRM,
                    libc::SIGPIPE,
                ] {
                    libc::signal(signal, libc::SIG_DFL);
                }
                Ok(())
            });
        }
        let child = command.spawn().context("could not start the shell")?;
        log::info!(
            "pty host: session {} started (pid {})",
            request.id,
            child.id()
        );
        Ok(Self {
            id: request.id.clone(),
            pid: child.id(),
            master,
            child: Mutex::new(Some(child)),
            state: Mutex::new(SessionState::default()),
        })
    }

    /// Reads the shell's output until it ends, keeping it and passing it on.
    fn pump(self: Arc<Self>) {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            // SAFETY: reads into `buf`, at most its length.
            let read =
                unsafe { libc::read(self.master.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            if read < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                // EIO: every holder of the slave is gone.
                break;
            }
            if read == 0 {
                break;
            }
            let bytes = &buf[..read as usize];
            let mut state = lock(&self.state);
            push_history(&mut state.history, bytes);
            send_to_client(&mut state, &Frame::Output(bytes.to_vec()));
        }
        // Taken before the wait, not held through it: from here on the pid
        // is not signalled (`hang_up`), as it is another process's once
        // the shell is reaped.
        let child = lock(&self.child).take();
        let code = child
            .and_then(|mut child| child.wait().ok())
            .map_or(-1, |status| status.code().unwrap_or(-1));
        log::info!("pty host: session {} ended ({code})", self.id);
        let mut state = lock(&self.state);
        state.exited = Some(code);
        send_to_client(&mut state, &Frame::Exited(code));
    }

    fn write(&self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            // SAFETY: writes from `bytes`, at most its length.
            let written =
                unsafe { libc::write(self.master.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
            if written < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                log::debug!("pty host: input for {} lost: {err}", self.id);
                return;
            }
            bytes = &bytes[written as usize..];
        }
    }

    fn resize(&self, cols: u16, rows: u16) {
        let size = libc::winsize {
            ws_row: rows.max(1),
            ws_col: cols.max(1),
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: `TIOCSWINSZ` reads the winsize given.
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &size) } == -1 {
            log::debug!(
                "pty host: resize of {}: {}",
                self.id,
                io::Error::last_os_error()
            );
        }
    }

    /// Two size changes, so the shell's program redraws even when the size
    /// it ends on is the one it had.
    fn nudge(&self, cols: u16, rows: u16) {
        if rows > 1 {
            self.resize(cols, rows - 1);
        }
        self.resize(cols, rows);
    }

    /// Ends the shell as closing a terminal window does: `login` (or the
    /// shell) gets SIGHUP, and its session's jobs with it.
    fn hang_up(&self) {
        // A shell that has ended is reaped (`pump`), and its pid may be
        // another process's by now.
        let child = lock(&self.child);
        if child.is_none() {
            return;
        }
        // SAFETY: a signal to the process this session started, not yet
        // waited on.
        if unsafe { libc::kill(self.pid as libc::pid_t, libc::SIGHUP) } == -1 {
            log::debug!(
                "pty host: SIGHUP to {}: {}",
                self.pid,
                io::Error::last_os_error()
            );
        }
    }
}

fn push_history(history: &mut VecDeque<u8>, bytes: &[u8]) {
    history.extend(bytes);
    let over = history.len().saturating_sub(HISTORY);
    history.drain(..over);
}

/// A client that cannot take the frame is let go: its connection is closed
/// both ways, so it sees the end rather than staying on a silent tab.
fn send_to_client(state: &mut SessionState, frame: &Frame) {
    if let Some((_, stream)) = state.client.as_mut()
        && write_frame(stream, frame).is_err()
        && let Some((_, stream)) = state.client.take()
        && let Err(err) = stream.shutdown(std::net::Shutdown::Both)
    {
        log::debug!("pty host: letting a client go: {err}");
    }
}

fn set_utf8(master: &OwnedFd) {
    // SAFETY: `tcgetattr` fills the zeroed termios; `tcsetattr` reads it.
    unsafe {
        let mut termios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(master.as_raw_fd(), &mut termios) == 0 {
            termios.c_iflag |= libc::IUTF8;
            libc::tcsetattr(master.as_raw_fd(), libc::TCSANOW, &termios);
        }
    }
}

struct ShellUser {
    name: String,
    home: String,
    shell: String,
}

impl ShellUser {
    fn from_env() -> Self {
        let var = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
        Self {
            name: var("USER").unwrap_or_else(|| "root".into()),
            home: var("HOME").unwrap_or_else(|| "/".into()),
            shell: var("SHELL").unwrap_or_else(|| "/bin/zsh".into()),
        }
    }
}

/// Alacritty's `default_shell_command`, as Ely's terminal ran it: on macOS
/// `login`, so the shell is a login shell on its own tty session.
#[cfg(target_os = "macos")]
fn shell_command(user: &ShellUser) -> Command {
    let name = user.shell.rsplit('/').next().unwrap_or("zsh");
    let exec = format!("exec -a -{name} {}", user.shell);
    let hushed = Path::new(&user.home).join(".hushlogin").exists();
    let mut command = Command::new("/usr/bin/login");
    command.args([
        if hushed { "-qflp" } else { "-flp" },
        &user.name,
        "/bin/zsh",
        "-fc",
        &exec,
    ]);
    command
}

#[cfg(not(target_os = "macos"))]
fn shell_command(user: &ShellUser) -> Command {
    Command::new(&user.shell)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keeps_the_latest_bytes() {
        let mut history = VecDeque::new();
        push_history(&mut history, &vec![1u8; HISTORY]);
        push_history(&mut history, &[2, 3]);
        assert_eq!(history.len(), HISTORY);
        assert_eq!(
            history.iter().rev().take(2).copied().collect::<Vec<_>>(),
            vec![3, 2]
        );
    }

    /// A host on a temporary socket: a session runs, is shown again with
    /// its output, and ends on Kill.
    #[test]
    fn a_session_outlives_its_client_and_ends_on_kill() {
        let dir = std::env::temp_dir().join(format!("bc-pty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("s.sock");
        let served = socket.clone();
        std::thread::spawn(move || serve(&served, Duration::from_secs(30)));
        let connect = || {
            for _ in 0..100 {
                if let Ok(mut stream) = UnixStream::connect(&socket) {
                    write_frame(&mut stream, &Frame::Hello(PROTOCOL_VERSION)).unwrap();
                    assert_eq!(
                        read_frame(&mut stream).unwrap(),
                        Some(Frame::Hello(PROTOCOL_VERSION))
                    );
                    stream
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .unwrap();
                    return stream;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            panic!("the host did not start");
        };
        let attach = |stream: &mut UnixStream| {
            let request = AttachRequest {
                id: "one".into(),
                cwd: dir.to_string_lossy().into_owned(),
                env: vec![("PS1".into(), "$ ".into())],
                cols: 80,
                rows: 24,
            };
            write_frame(
                stream,
                &Frame::Attach(serde_json::to_vec(&request).unwrap()),
            )
            .unwrap();
            match read_frame(stream).unwrap() {
                Some(Frame::Attached(body)) => serde_json::from_slice::<Attached>(&body).unwrap(),
                other => panic!("expected Attached, got {other:?}"),
            }
        };
        let read_until = |stream: &mut UnixStream, needle: &str| {
            let mut seen = Vec::new();
            while !String::from_utf8_lossy(&seen).contains(needle) {
                match read_frame(stream).unwrap() {
                    Some(Frame::Output(bytes)) => seen.extend(bytes),
                    Some(Frame::Exited(_)) | None => break,
                    Some(_) => {}
                }
            }
            String::from_utf8_lossy(&seen).into_owned()
        };

        let mut first = connect();
        let attached = attach(&mut first);
        assert!(attached.created);
        // The typed line must not hold the text looked for (the tty echoes
        // it), and must mean the same in zsh, bash and fish.
        write_frame(
            &mut first,
            &Frame::Input(b"printf 'bencode-%d\\n' 42\r".to_vec()),
        )
        .unwrap();
        assert!(read_until(&mut first, "bencode-42").contains("bencode-42"));
        drop(first);

        // Shown again: the same shell, its output replayed.
        let mut second = connect();
        let again = attach(&mut second);
        assert!(!again.created);
        assert_eq!(again.pid, attached.pid);
        assert!(read_until(&mut second, "bencode-42").contains("bencode-42"));

        let mut control = connect();
        write_frame(
            &mut control,
            &Frame::Control(
                serde_json::to_vec(&ControlRequest::Kill {
                    ids: vec!["one".into()],
                })
                .unwrap(),
            ),
        )
        .unwrap();
        assert!(matches!(
            read_frame(&mut control).unwrap(),
            Some(Frame::ControlReply(_))
        ));
        let mut ended = false;
        for _ in 0..1000 {
            match read_frame(&mut second) {
                Ok(Some(Frame::Exited(_))) => {
                    ended = true;
                    break;
                }
                Ok(Some(_)) => {}
                _ => break,
            }
        }
        assert!(ended, "the shell did not end on Kill");
        std::fs::remove_dir_all(&dir).ok();
    }
}
