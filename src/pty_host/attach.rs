//! `bencode --pty-attach <id> <cwd>`: what a dock tab runs. It puts its
//! own terminal (Ely's PTY) in raw mode and relays it to session `id` on
//! the host: keys and size one way, output the other. The session's end
//! is this process's end, which Ely shows as the tab's `[process exited]`.
//! Its own end (the tab closed, the app gone) leaves the session running.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::protocol::{AttachRequest, Frame, PROTOCOL_VERSION, read_frame, write_frame};

/// How long a host just started has to listen.
const HOST_START: Duration = Duration::from_secs(5);
/// How often the terminal's size is read.
const SIZE_POLL: Duration = Duration::from_millis(100);

/// The variables a new session takes from the tab's terminal.
const PASSED_ENV: [&str; 3] = ["TERM", "COLORTERM", "LANG"];

pub fn run(id: &str, cwd: &str) -> i32 {
    crate::logging::init_file("pty-host.log");
    match attach(id, cwd) {
        Ok(code) => code,
        Err(err) => {
            log::error!("pty attach {id}: {err:#}");
            let _ = write!(
                io::stdout(),
                "\r\n[BenCode could not open this terminal: {err}]\r\n"
            );
            1
        }
    }
}

fn attach(id: &str, cwd: &str) -> anyhow::Result<i32> {
    let mut stream = connect_or_start()?;
    let (cols, rows) = terminal_size().unwrap_or((80, 24));
    let request = AttachRequest {
        id: id.to_string(),
        cwd: cwd.to_string(),
        env: PASSED_ENV
            .iter()
            .filter_map(|key| {
                std::env::var(key)
                    .ok()
                    .map(|value| (key.to_string(), value))
            })
            .collect(),
        cols,
        rows,
    };
    write_frame(&mut stream, &Frame::Attach(serde_json::to_vec(&request)?))?;
    match read_frame(&mut stream)? {
        Some(Frame::Attached(_)) => {}
        Some(Frame::Error(message)) => anyhow::bail!(message),
        other => anyhow::bail!("unexpected answer from the terminal host: {other:?}"),
    }
    let _raw = RawMode::enter();

    // Keys.
    let mut keys = stream.try_clone()?;
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if write_frame(&mut keys, &Frame::Input(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Size.
    let mut sizes = stream.try_clone()?;
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            let size = terminal_size();
            if size.is_some() && size != last {
                last = size;
                if let Some((cols, rows)) = size
                    && write_frame(&mut sizes, &Frame::Resize(cols, rows)).is_err()
                {
                    break;
                }
            }
            std::thread::sleep(SIZE_POLL);
        }
    });

    // Output, until the session ends.
    let mut stdout = io::stdout().lock();
    loop {
        match read_frame(&mut stream) {
            Ok(Some(Frame::Output(bytes))) => {
                stdout.write_all(&bytes)?;
                stdout.flush()?;
            }
            Ok(Some(Frame::Exited(code))) => return Ok(code),
            Ok(Some(Frame::Error(message))) => anyhow::bail!(message),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => {
                stdout.write_all(b"\r\n[BenCode's terminal host stopped]\r\n")?;
                return Ok(1);
            }
        }
    }
}

/// The host's socket, after starting a host when none answers.
fn connect_or_start() -> anyhow::Result<UnixStream> {
    let hello = |mut stream: UnixStream| -> anyhow::Result<UnixStream> {
        match read_frame(&mut stream)? {
            Some(Frame::Hello(PROTOCOL_VERSION)) => Ok(stream),
            Some(Frame::Error(message)) => anyhow::bail!(message),
            other => anyhow::bail!("unexpected greeting from the terminal host: {other:?}"),
        }
    };
    match super::connect() {
        Ok(stream) => return hello(stream),
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) => {}
        Err(err) => return Err(err.into()),
    }
    start_host()?;
    let started = Instant::now();
    loop {
        match super::connect() {
            Ok(stream) => return hello(stream),
            Err(err) if started.elapsed() >= HOST_START => {
                return Err(anyhow::anyhow!("the terminal host did not start: {err}"));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

/// Starts `bencode --pty-host` on its own: a session of its own, away from
/// this terminal, so neither this process nor the app ending ends it.
fn start_host() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let mut command = Command::new(exe);
    command
        .arg(super::HOST_ARG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = crate::storage::data_dir() {
        command.current_dir(dir);
    }
    // SAFETY: `setsid` is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // Not waited on: it outlives this process, and launchd reaps it.
    command.spawn()?;
    Ok(())
}

/// This terminal's columns and rows.
fn terminal_size() -> Option<(u16, u16)> {
    // SAFETY: `TIOCGWINSZ` fills the zeroed winsize.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let read = unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut size) };
    (read == 0 && size.ws_col > 0 && size.ws_row > 0).then_some((size.ws_col, size.ws_row))
}

/// The terminal in raw mode while it lives: keys go to the session as
/// typed, and the session's shell does the echoing.
struct RawMode(Option<libc::termios>);

impl RawMode {
    fn enter() -> Self {
        // SAFETY: `tcgetattr` fills the zeroed termios; `cfmakeraw` and
        // `tcsetattr` take it.
        unsafe {
            let mut saved: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut saved) != 0 {
                return Self(None);
            }
            let mut raw = saved;
            libc::cfmakeraw(&mut raw);
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
            Self(Some(saved))
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if let Some(saved) = &self.0 {
            // SAFETY: restores the termios read in `enter`.
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, saved) };
        }
    }
}
