//! `bencode app ACTION`: the app access a Mono's agent has, ported from
//! MonoCode's `app` CLI (`src-tauri/src/control_cli.rs` `run_app`,
//! `control.rs`, `features/agent-app/model/agentApp.ts`). The agent runs
//! this binary; it sends the call over a Unix socket (`server.rs`) to the
//! app, which answers it on the UI thread (`app/artifacts.rs`).
//!
//! MonoCode's CLI also drives sessions, worktrees, folders, notes, the
//! soul, memory, habits and chat cards. BenCode's answers its artifacts
//! actions only (`artifacts.rs`); the rest are not ported yet.
//!
//! The turn's credential reaches the CLI in `BENCODE_APP_TOKEN`, with the
//! socket in `BENCODE_APP_SOCKET` (`SpawnRequest.app`). A token works only
//! while its Mono's turn runs.

pub mod artifacts;
mod cli;
pub mod server;

use std::io::Read as _;

const ARG: &str = "app";

/// Runs the CLI when `main` was started as `bencode app …`; returns the
/// exit code. `None`: this is the app.
pub fn run_from_args() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(ARG) {
        return None;
    }
    Some(cli::run(args.collect()))
}

/// How an agent runs the CLI: this binary's path, quoted for a POSIX
/// shell when it needs it (MonoCode `quoted`), then `app`.
pub fn command() -> String {
    let exe = std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "bencode".into());
    format!("{} {ARG}", quoted(&exe))
}

fn quoted(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "._/-:".contains(c))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

/// `bytes` from the system's random source, as hex.
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf));
    if let Err(err) = read {
        // Never a predictable credential: the time and pid at least differ.
        log::error!("no random source ({err}); falling back to the clock");
        let seed = format!(
            "{:?}{}",
            std::time::SystemTime::now(),
            std::process::id()
        );
        use sha2::Digest as _;
        let digest = sha2::Sha256::digest(seed.as_bytes());
        for (ix, byte) in buf.iter_mut().enumerate() {
            *byte = digest[ix % digest.len()];
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_quoted_only_when_they_need_it() {
        assert_eq!(quoted("/usr/local/bin/bencode"), "/usr/local/bin/bencode");
        assert_eq!(
            quoted("/Applications/Ben Code.app/Contents/MacOS/bencode"),
            "'/Applications/Ben Code.app/Contents/MacOS/bencode'"
        );
        assert_eq!(quoted("/a/it's"), r"'/a/it'\''s'");
    }

    #[test]
    fn random_hex_is_as_long_as_asked() {
        let one = random_hex(16);
        assert_eq!(one.len(), 32);
        assert_ne!(one, random_hex(16));
    }
}
