//! `bencode-host daemon`: the one host of this account. It holds
//! `daemon.lock` while it runs, so only it touches the socket's name, and
//! serves every connection `connect` relays.

use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::net::UnixListener;
use std::sync::Arc;
use std::time::Duration;

use bencode_core::remote::protocol::{HostInfo, PROTOCOL_VERSION};

use crate::dirs::Dirs;
use crate::runs::Runs;

pub struct Host {
    pub dirs: Dirs,
    pub runs: Arc<Runs>,
    environment_id: String,
    name: String,
    os: String,
    arch: String,
    home: String,
    /// Held for the daemon's life.
    _lock: std::fs::File,
}

impl Host {
    pub fn info(&self) -> HostInfo {
        HostInfo {
            protocol: PROTOCOL_VERSION,
            version: env!("CARGO_PKG_VERSION").into(),
            environment_id: self.environment_id.clone(),
            name: self.name.clone(),
            os: self.os.clone(),
            arch: self.arch.clone(),
            home: self.home.clone(),
            running: self.runs.running(),
        }
    }

    /// `Shutdown`: the runs are closed, then the process ends, a moment
    /// after its reply went out.
    pub fn shutdown(&self) {
        log::info!("shutting down on request");
        self.runs.stop_all();
        if let Err(err) = std::fs::remove_file(self.dirs.socket()) {
            log::warn!("could not remove the socket: {err}");
        }
        std::thread::sleep(Duration::from_millis(200));
        std::process::exit(0);
    }
}

pub fn run() -> u8 {
    let dirs = match Dirs::locate() {
        Ok(dirs) => dirs,
        Err(err) => {
            eprintln!("bencode-host: {err}");
            return 1;
        }
    };
    init_logging(&dirs);
    match serve(dirs) {
        Ok(()) => 0,
        Err(err) => {
            log::error!("{err}");
            1
        }
    }
}

fn serve(dirs: Dirs) -> io::Result<()> {
    let Some(lock) = claim_lock(&dirs)? else {
        log::info!("another daemon runs; leaving it be");
        return Ok(());
    };
    adopt_login_path();
    // The lock is ours, so a socket file left is a dead daemon's.
    match std::fs::remove_file(dirs.socket()) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    let listener = UnixListener::bind(dirs.socket())?;
    let runs = Arc::new(Runs::load(dirs.runs(), dirs.attachments()));
    let host = Arc::new(Host {
        environment_id: dirs.environment_id()?,
        name: computer_name(),
        os: uname("-s"),
        arch: uname("-m"),
        home: std::env::var("HOME").unwrap_or_default(),
        runs,
        dirs,
        _lock: lock,
    });
    log::info!(
        "bencode-host {} listening as {}",
        env!("CARGO_PKG_VERSION"),
        host.environment_id
    );
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let host = host.clone();
                std::thread::spawn(move || crate::session::serve(stream, host));
            }
            Err(err) => log::warn!("accept: {err}"),
        }
    }
    Ok(())
}

fn claim_lock(dirs: &Dirs) -> io::Result<Option<std::fs::File>> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dirs.lock())?;
    // SAFETY: `flock` on a descriptor this function owns; the lock lasts as
    // long as the file stays open.
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    Ok(locked.then_some(file))
}

fn init_logging(dirs: &Dirs) {
    let mut builder = env_logger::Builder::from_env(
        env_logger::Env::default().filter_or("BENCODE_HOST_LOG", "info"),
    );
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dirs.log())
    {
        Ok(file) => {
            builder.target(env_logger::Target::Pipe(Box::new(file)));
        }
        Err(err) => eprintln!("bencode-host: cannot open the log: {err}"),
    }
    builder.init();
}

/// SSH starts the daemon with the bare PATH of a non-interactive shell,
/// where npm's and the installers' folders are missing; the login shell's
/// is what the user's own terminal has (as `child_path` fills a Finder
/// app's in the app).
fn adopt_login_path() {
    // Tests run the host with a PATH of their own, fake CLIs first.
    if std::env::var_os("BENCODE_HOST_KEEP_PATH").is_some() {
        return;
    }
    if let Some(path) = bencode_core::login_path::read() {
        // SAFETY: called before any thread of this process starts.
        unsafe { std::env::set_var("PATH", path) };
    }
}

fn uname(flag: &str) -> String {
    std::process::Command::new("uname")
        .arg(flag)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .to_ascii_lowercase()
        })
        .unwrap_or_default()
}

/// The name the app shows for this machine until the user names it.
fn computer_name() -> String {
    let run = |program: &str, args: &[&str]| {
        std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|name| !name.is_empty())
    };
    let name = if cfg!(target_os = "macos") {
        run("scutil", &["--get", "ComputerName"])
    } else {
        std::fs::read_to_string("/etc/hostname")
            .ok()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
    };
    name.or_else(|| run("hostname", &[]))
        .map(|name| name.chars().filter(|c| !c.is_control()).take(80).collect())
        .unwrap_or_else(|| "Remote machine".into())
}
