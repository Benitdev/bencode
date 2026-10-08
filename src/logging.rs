//! Where `log::` output goes. Run from a terminal (`cargo run`), it is the
//! terminal, as before; launched from Finder or the Dock, where stderr goes
//! nowhere, it is `bencode.log` in `storage::logs_dir()`, so a bug report can
//! carry it (Help › Show Logs). Panics are logged too, with a backtrace.

use std::fs::{self, File, OpenOptions};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

const LOG_FILE: &str = "bencode.log";
const OLD_LOG_FILE: &str = "bencode.old.log";
/// A log past this size at startup is kept as the previous one and a fresh
/// file is started, so the folder holds at most two.
const ROTATE_BYTES: u64 = 5 * 1024 * 1024;

/// Sets up logging and the panic hook. Call first thing in `main`.
pub fn init() {
    // Warnings by default: the file is for bug reports, not tracing.
    // `RUST_LOG` still overrides it.
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"));
    if !std::io::stderr().is_terminal()
        && let Some(file) = crate::storage::logs_dir().and_then(|dir| open_log(&dir))
    {
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }
    builder.init();
    install_panic_hook();
}

/// The log file for appending, after rotating one that grew too large.
fn open_log(dir: &Path) -> Option<File> {
    if let Err(err) = fs::create_dir_all(dir) {
        eprintln!(
            "bencode: cannot create the log folder {}: {err}",
            dir.display()
        );
        return None;
    }
    let path = dir.join(LOG_FILE);
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > ROTATE_BYTES)
        && let Err(err) = fs::rename(&path, dir.join(OLD_LOG_FILE))
    {
        eprintln!("bencode: cannot rotate {}: {err}", path.display());
    }
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => Some(file),
        Err(err) => {
            eprintln!("bencode: cannot open {}: {err}", path.display());
            None
        }
    }
}

/// Logs a panic, with its backtrace, before the default hook prints it.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        log::error!(
            "BenCode {} panicked: {info}\n{backtrace}",
            env!("CARGO_PKG_VERSION")
        );
        default_hook(info);
    }));
}

/// Help › Show Logs: the log file selected in Finder, else its folder.
pub fn reveal(cx: &gpui::App) {
    let Some(dir) = crate::storage::logs_dir() else {
        log::warn!("no home folder, so no log folder to show");
        return;
    };
    let file: PathBuf = dir.join(LOG_FILE);
    cx.reveal_path(if file.exists() { &file } else { &dir });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("bencode-logging-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn creates_the_folder_and_appends() {
        let dir = temp_dir("append");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(LOG_FILE), "before\n").unwrap();
        let mut file = open_log(&dir).expect("log file");
        std::io::Write::write_all(&mut file, b"after\n").unwrap();
        assert_eq!(
            fs::read_to_string(dir.join(LOG_FILE)).unwrap(),
            "before\nafter\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rotates_a_large_log() {
        let dir = temp_dir("rotate");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(LOG_FILE), vec![b'x'; ROTATE_BYTES as usize + 1]).unwrap();
        open_log(&dir).expect("log file");
        assert_eq!(fs::metadata(dir.join(LOG_FILE)).unwrap().len(), 0);
        assert_eq!(
            fs::metadata(dir.join(OLD_LOG_FILE)).unwrap().len(),
            ROTATE_BYTES + 1
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
