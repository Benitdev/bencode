//! The PATH the user's own terminal has. An app started from Finder, and a
//! daemon SSH starts, inherit a bare one (`/usr/bin:/bin:…`), so a git hook
//! that runs `php`, `node` or `npx` from Homebrew or an installer's folder
//! fails there though it works in the terminal.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

static LOGIN: OnceLock<OsString> = OnceLock::new();

/// The login shell's PATH, then this process's folders it lacks. `None`
/// when the shell cannot say. Runs the shell: never on the UI thread.
pub fn read() -> Option<OsString> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let output = std::process::Command::new(&shell)
        .args(["-lc", "printf '%s' \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let login = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Ok(out) => {
            log::warn!("{shell} -l exited with {}; keeping PATH", out.status);
            return None;
        }
        Err(err) => {
            log::warn!("cannot run {shell}: {err}; keeping PATH");
            return None;
        }
    };
    if login.is_empty() || login.contains('\n') {
        return None;
    }
    let current = std::env::var_os("PATH").unwrap_or_default();
    join(std::env::split_paths(&login).chain(std::env::split_paths(&current)))
}

/// Reads the login shell's PATH on a thread of its own, so the launches
/// after it (`path`) get it without waiting for the shell.
pub fn resolve_in_background() {
    let spawned = std::thread::Builder::new()
        .name("login-path".into())
        .spawn(|| {
            LOGIN.get_or_init(|| read().unwrap_or_else(fallback));
        });
    if let Err(err) = spawned {
        log::warn!("cannot read the login shell's PATH: {err}");
    }
}

/// The PATH for a child that may run the user's tools (git's hooks): the
/// login shell's once read, until then this process's with the usual
/// install folders added. Never waits.
pub fn path() -> OsString {
    LOGIN.get().cloned().unwrap_or_else(fallback)
}

/// This process's PATH, then `~/.local/bin`, Homebrew's and the system's.
fn fallback() -> OsString {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let local = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/bin"));
    let usual = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from);
    let dirs = std::env::split_paths(&current).chain(local).chain(usual);
    join(dirs).unwrap_or(current)
}

/// The folders in order, each once.
fn join(dirs: impl Iterator<Item = PathBuf>) -> Option<OsString> {
    let mut unique: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if !dir.as_os_str().is_empty() && !unique.contains(&dir) {
            unique.push(dir);
        }
    }
    std::env::join_paths(unique).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_keeps_the_first_of_each_folder() {
        let dirs = ["/a", "/b", "/a", "", "/c", "/b"].map(PathBuf::from);
        assert_eq!(join(dirs.into_iter()), Some(OsString::from("/a:/b:/c")));
    }

    #[test]
    fn fallback_adds_homebrew_after_the_process_path() {
        let path = fallback();
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
        let current = std::env::var_os("PATH").unwrap_or_default();
        if let Some(first) = std::env::split_paths(&current).find(|d| !d.as_os_str().is_empty()) {
            assert_eq!(dirs.first(), Some(&first));
        }
    }
}
