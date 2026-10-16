//! `bencode-host connect`, what the app runs over SSH: it relays the
//! connection's stdio to the daemon's socket, starting the daemon when none
//! answers (as `bencode --pty-attach` starts the terminal host). Also the
//! `status` and `stop` commands, for whoever is at the host's shell.

use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use bencode_core::remote::protocol::{
    ClientFrame, HostInfo, PROTOCOL_VERSION, Request, ServerFrame,
};

use crate::dirs::Dirs;

/// How long a daemon just started may take to listen.
const DAEMON_START: Duration = Duration::from_secs(10);

pub fn run() -> u8 {
    let stream = match Dirs::locate().and_then(|dirs| connect_or_start(&dirs)) {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("bencode-host: {err}");
            return 1;
        }
    };
    relay(stream)
}

/// stdin to the socket and the socket to stdout, until either side ends.
fn relay(stream: UnixStream) -> u8 {
    let Ok(mut to_daemon) = stream.try_clone() else {
        eprintln!("bencode-host: could not share the daemon's socket");
        return 1;
    };
    std::thread::spawn(move || {
        let copied = io::copy(&mut io::stdin().lock(), &mut to_daemon);
        if let Err(err) = copied {
            eprintln!("bencode-host: stdin: {err}");
        }
        // The app went away: the daemon drops this connection, never a run.
        let _ = to_daemon.shutdown(std::net::Shutdown::Write);
    });
    let mut from_daemon = stream;
    let mut stdout = io::stdout().lock();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        match from_daemon.read(&mut buffer) {
            Ok(0) => return 0,
            Ok(n) => {
                if stdout.write_all(&buffer[..n]).and_then(|()| stdout.flush()).is_err() {
                    return 0;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => {
                eprintln!("bencode-host: daemon: {err}");
                return 1;
            }
        }
    }
}

fn connect_or_start(dirs: &Dirs) -> io::Result<UnixStream> {
    match UnixStream::connect(dirs.socket()) {
        Ok(stream) => return Ok(stream),
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) => {}
        Err(err) => return Err(err),
    }
    start_daemon(dirs)?;
    let started = Instant::now();
    loop {
        match UnixStream::connect(dirs.socket()) {
            Ok(stream) => return Ok(stream),
            Err(err) if started.elapsed() >= DAEMON_START => {
                return Err(io::Error::other(format!(
                    "the host did not start ({err}); see {}",
                    dirs.log().display()
                )));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

/// Starts `bencode-host daemon` away from this SSH session, so neither the
/// session ending nor this process ending stops the agents. Where systemd
/// keeps the account's services past its last login (lingering), the daemon
/// is one of them, as MonoCode's host is a systemd user service.
fn start_daemon(dirs: &Dirs) -> io::Result<()> {
    let exe = std::env::current_exe()?;
    if lingering() {
        let unit = format!("bencode-host-{}", jiff::Timestamp::now().as_millisecond());
        let started = Command::new("systemd-run")
            .args(["--user", "--quiet", "--collect", "--unit", &unit])
            .arg(&exe)
            .arg("daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match started {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => log_line(dirs, &format!("systemd-run failed ({status}); starting it here")),
            Err(err) => log_line(dirs, &format!("systemd-run: {err}; starting it here")),
        }
    }
    let mut command = Command::new(exe);
    command
        .arg("daemon")
        .current_dir(&dirs.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: `setsid` is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // Not waited on: it outlives this process, and init reaps it.
    command.spawn()?;
    Ok(())
}

/// Whether systemd keeps this account's user services after it logs out.
fn lingering() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME"));
    user.is_ok_and(|user| {
        std::path::Path::new("/var/lib/systemd/linger")
            .join(user)
            .exists()
    }) && which("systemd-run")
}

fn which(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
    })
}

fn log_line(dirs: &Dirs, line: &str) {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dirs.log());
    if let Ok(mut file) = file {
        let _ = writeln!(file, "[connect] {line}");
    }
}

/// One request to a running daemon, for `status` and `stop`.
fn ask(request: Request) -> io::Result<Option<serde_json::Value>> {
    let dirs = Dirs::locate()?;
    let mut stream = UnixStream::connect(dirs.socket())?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let frames = [
        ClientFrame {
            id: 1,
            request: Request::Hello {
                protocol: PROTOCOL_VERSION,
                client: "bencode-host".into(),
            },
        },
        ClientFrame { id: 2, request },
    ];
    for frame in &frames {
        let line = serde_json::to_string(frame).map_err(io::Error::other)?;
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
    }
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if let Ok(ServerFrame::Reply { id: 2, result, error }) = serde_json::from_str(&line) {
            return match error {
                Some(error) => Err(io::Error::other(error)),
                None => Ok(result),
            };
        }
    }
}

pub fn status() -> u8 {
    let hello = ask(Request::Hello {
        protocol: PROTOCOL_VERSION,
        client: "bencode-host".into(),
    });
    match hello {
        Ok(Some(value)) => match serde_json::from_value::<HostInfo>(value) {
            Ok(info) => {
                println!(
                    "running: version {}, id {}, {} run(s) going",
                    info.version, info.environment_id, info.running
                );
                0
            }
            Err(err) => {
                println!("running, but its answer is unreadable: {err}");
                1
            }
        },
        Ok(None) => {
            println!("running, but it did not answer");
            1
        }
        Err(err) => {
            println!("not running ({err})");
            3
        }
    }
}

pub fn stop() -> u8 {
    match ask(Request::Shutdown) {
        Ok(_) => {
            println!("stopped");
            0
        }
        Err(err) => {
            println!("not running ({err})");
            3
        }
    }
}
