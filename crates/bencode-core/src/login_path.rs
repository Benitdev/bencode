//! The PATH the user's own terminal has. An app started from Finder, and a
//! daemon SSH starts, inherit a bare one (`/usr/bin:/bin:…`), so a git hook
//! that runs `php`, `node` or `npx` from Homebrew or an installer's folder
//! fails there though it works in the terminal.

use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::{Duration, Instant};

static LOGIN: OnceLock<OsString> = OnceLock::new();

/// Set while the shell runs, so a slow `.zshrc` can skip what a PATH does
/// not need (`[[ -n $BENCODE_RESOLVING_ENVIRONMENT ]] && return`).
const RESOLVING: &str = "BENCODE_RESOLVING_ENVIRONMENT";
/// Around the PATH in the shell's output: an rc file may print its own.
const MARK: &str = "__BENCODE_PATH__";
/// Longer than any rc file the user would wait for in a terminal.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The login shell's PATH, then this process's folders it lacks. `None`
/// when the shell cannot say. Runs the shell: never on the UI thread.
pub fn read() -> Option<OsString> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    // Interactive as well: nvm, pyenv and conda add their folders in
    // `.zshrc`, which a login shell alone does not read.
    let login = shell_path(&shell, "-ilc").or_else(|| shell_path(&shell, "-lc"))?;
    let current = std::env::var_os("PATH").unwrap_or_default();
    join(std::env::split_paths(&login).chain(std::env::split_paths(&current)))
}

/// The PATH `shell flags` prints, or `None` when it fails, prints none or
/// outlasts `TIMEOUT`.
fn shell_path(shell: &str, flags: &str) -> Option<String> {
    let mut command = Command::new(shell);
    command
        .args([flags, &format!("printf '{MARK}%s{MARK}' \"$PATH\"")])
        .env(RESOLVING, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // A session of its own: an interactive shell would otherwise take the
    // terminal BenCode was started from, and the timeout ends all it started.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            log::warn!("cannot run {shell}: {err}; keeping PATH");
            return None;
        }
    };
    let mut stdout = child.stdout.take()?;
    // Read apart from the wait: a chatty rc file could fill the pipe.
    let (sent, output) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = sent.send(stdout.read_to_end(&mut out).map(|_| out));
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                log::warn!("{shell} {flags} took over {TIMEOUT:?}; stopped it");
                stop(&mut child);
                return None;
            }
            Err(err) => {
                log::warn!("cannot wait for {shell} {flags}: {err}");
                stop(&mut child);
                return None;
            }
        }
    };
    if !status.success() {
        log::warn!("{shell} {flags} exited with {status}");
        return None;
    }
    // Something the rc file left running may hold the pipe open.
    let wait = deadline.saturating_duration_since(Instant::now());
    let out = match output.recv_timeout(wait) {
        Ok(Ok(out)) => out,
        Ok(Err(err)) => {
            log::warn!("cannot read {shell} {flags}: {err}");
            return None;
        }
        Err(_) => {
            log::warn!("{shell} {flags} left its output open");
            return None;
        }
    };
    let path = between_marks(&String::from_utf8_lossy(&out));
    if path.is_none() {
        log::warn!("{shell} {flags} printed no PATH");
    }
    path
}

/// The text between the first two `MARK`s, when it is one non-empty line.
fn between_marks(output: &str) -> Option<String> {
    let (_, rest) = output.split_once(MARK)?;
    let (path, _) = rest.split_once(MARK)?;
    (!path.is_empty() && !path.contains('\n')).then(|| path.to_string())
}

/// Kills the shell and everything in its session, and reaps it.
fn stop(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(group) = i32::try_from(child.id()) {
        // SAFETY: a plain syscall; the group is the session `setsid` made.
        unsafe { libc::kill(-group, libc::SIGKILL) };
    }
    if let Err(err) = child.kill() {
        log::debug!("shell already gone: {err}");
    }
    if let Err(err) = child.wait() {
        log::warn!("cannot reap the shell: {err}");
    }
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
    fn between_marks_skips_what_the_rc_file_prints() {
        let out = format!("Welcome!\n{MARK}/a:/b{MARK}\nbye");
        assert_eq!(between_marks(&out), Some("/a:/b".into()));
        assert_eq!(between_marks(&format!("{MARK}{MARK}")), None);
        assert_eq!(between_marks(&format!("{MARK}/a\n/b{MARK}")), None);
        assert_eq!(between_marks("/a:/b"), None);
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
