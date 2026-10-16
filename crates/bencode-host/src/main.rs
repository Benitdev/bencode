//! `bencode-host`: BenCode on another machine (MonoCode Host, `host/` in
//! MonoCode, which is Node; this is Rust and shares BenCode's harnesses).
//! The app reaches it over SSH, `ssh <machine> bencode-host connect`, and
//! the connection's stdio carries `bencode_core::remote::protocol`.
//!
//! `connect` relays to the daemon, one per account (`daemon.lock`), over a
//! Unix socket in `~/.bencode-host`, and starts it when none answers. The
//! daemon runs the agents and keeps each run's events on disk, so a turn
//! goes on when SSH drops or the app quits, and the app picks it up again.

mod daemon;
mod dirs;
mod ops;
mod proxy;
mod runs;
mod session;

use std::process::ExitCode;

const USAGE: &str = "usage: bencode-host <connect | daemon | status | stop | --version>";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let code = match args.next().as_deref() {
        Some("--version" | "-V" | "version") => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("connect") => proxy::run(),
        Some("daemon") => daemon::run(),
        Some("status") => proxy::status(),
        Some("stop") => proxy::stop(),
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };
    ExitCode::from(code)
}
