//! What runs in a dock terminal, read from the kernel: the shell Ely
//! started, the job in its foreground and the folder the shell is in.
//! MonoCode owns its PTY and asks it (`src-tauri/src/pty.rs`
//! `foreground_label`), and takes the folder from OSC 7; Ely's `Terminal`
//! keeps its PTY to itself, so BenCode finds the shell among its own child
//! processes instead.

/// The job in a terminal's foreground: its process group and the name
/// MonoCode shows for it (`vite`, `npm`, `cargo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Foreground {
    pub pgid: u32,
    pub process: String,
}

/// One reading of a terminal's processes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    pub shell: Option<u32>,
    pub foreground: Option<Foreground>,
    pub cwd: Option<String>,
}

/// Reads the terminal whose process is `root`. `shell` is the shell found
/// by an earlier reading; `last` the foreground it saw, whose name is kept
/// while the same job runs so `ps` runs once per job.
pub fn probe(root: u32, shell: Option<u32>, last: Option<&Foreground>) -> Probe {
    let Some(shell) = shell.or_else(|| sys::shell(root)) else {
        return Probe::default();
    };
    let foreground = sys::foreground(shell).and_then(|pgid| match last {
        Some(last) if last.pgid == pgid => Some(last.clone()),
        _ => sys::args(pgid)
            .and_then(|args| command_label(&args))
            .filter(|name| !is_shell_name(name))
            .map(|process| Foreground { pgid, process }),
    });
    Probe {
        shell: Some(shell),
        foreground,
        cwd: sys::cwd(shell),
    }
}

/// The job in the foreground of `shell`'s terminal now, without running
/// anything: `last`'s name while it is the same job, else the process name.
pub fn foreground_now(shell: u32, last: Option<&Foreground>) -> Option<Foreground> {
    let pgid = sys::foreground(shell)?;
    if let Some(last) = last.filter(|last| last.pgid == pgid) {
        return Some(last.clone());
    }
    let process = sys::name(pgid).filter(|name| !is_shell_name(name))?;
    Some(Foreground { pgid, process })
}

/// BenCode's child processes now.
pub fn children() -> Vec<u32> {
    sys::children(std::process::id())
}

/// The process a terminal just started: a child missing from `before` with
/// a terminal of its own (git and agent CLIs share BenCode's, or have none).
pub fn spawned(before: &[u32]) -> Option<u32> {
    let own = sys::tty(std::process::id());
    children()
        .into_iter()
        .filter(|pid| !before.contains(pid))
        .find(|&pid| sys::tty(pid).is_some_and(|tty| Some(tty) != own))
}

/// MonoCode `command_label`: a command's name, or the script an
/// interpreter runs (`node /usr/local/bin/npm run build` is `npm`).
pub fn command_label(args: &str) -> Option<String> {
    let mut parts = args.split_whitespace();
    let base = file_name(parts.next()?)?;
    if is_interpreter(base) {
        for part in parts.filter(|part| !part.starts_with('-')) {
            let name = file_name(part)?;
            if !name.starts_with('-') {
                return Some(name.to_string());
            }
        }
    }
    Some(base.to_string())
}

fn file_name(path: &str) -> Option<&str> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
}

fn is_interpreter(name: &str) -> bool {
    matches!(
        name,
        "node" | "nodejs" | "python" | "python3" | "ruby" | "deno" | "bun"
    )
}

/// MonoCode `is_shell_name`, plus `login` (the process macOS runs the shell
/// under) and a login shell's leading `-`.
fn is_shell_name(name: &str) -> bool {
    matches!(
        name.trim_start_matches('-'),
        "zsh" | "bash" | "sh" | "fish" | "nu" | "dash" | "ksh" | "tcsh" | "zsh5" | "login"
    )
}

#[cfg(target_os = "macos")]
mod sys {
    use std::ffi::CStr;
    use std::mem::{size_of, zeroed};

    fn bsd_info(pid: u32) -> Option<libc::proc_bsdinfo> {
        // SAFETY: `proc_pidinfo` fills at most `size` bytes of the zeroed
        // struct, the layout its flavor names.
        let mut info: libc::proc_bsdinfo = unsafe { zeroed() };
        let size = size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let read = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        (read == size).then_some(info)
    }

    pub fn children(pid: u32) -> Vec<u32> {
        let mut pids = vec![0 as libc::pid_t; 256];
        // SAFETY: the buffer holds `pids.len()` pids and its size is in bytes.
        let count = unsafe {
            libc::proc_listchildpids(
                pid as libc::pid_t,
                pids.as_mut_ptr().cast(),
                (pids.len() * size_of::<libc::pid_t>()) as libc::c_int,
            )
        };
        if count < 0 {
            log::debug!("proc_listchildpids: {}", std::io::Error::last_os_error());
            return Vec::new();
        }
        pids.truncate(count as usize);
        pids.into_iter()
            .filter(|&pid| pid > 0)
            .map(|pid| pid as u32)
            .collect()
    }

    /// The device of `pid`'s controlling terminal, if it has one.
    pub fn tty(pid: u32) -> Option<u32> {
        let tdev = bsd_info(pid)?.e_tdev;
        (tdev != u32::MAX && tdev != 0).then_some(tdev)
    }

    pub fn name(pid: u32) -> Option<String> {
        let mut buf = [0u8; 2 * libc::MAXCOMLEN + 1];
        // SAFETY: `proc_name` writes at most `buf.len()` bytes.
        let len = unsafe {
            libc::proc_name(
                pid as libc::c_int,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
            )
        };
        (len > 0).then(|| String::from_utf8_lossy(&buf[..len as usize]).into_owned())
    }

    /// The shell under `root`: macOS starts it through `login`, which
    /// waits on it as its one child.
    pub fn shell(root: u32) -> Option<u32> {
        if name(root)? == "login" {
            children(root).first().copied()
        } else {
            Some(root)
        }
    }

    /// The process group in the foreground of `shell`'s terminal, unless it
    /// is the shell's own.
    pub fn foreground(shell: u32) -> Option<u32> {
        let tpgid = bsd_info(shell)?.e_tpgid;
        (tpgid != 0 && tpgid != shell).then_some(tpgid)
    }

    pub fn cwd(pid: u32) -> Option<String> {
        // SAFETY: as in `bsd_info`, for the vnode path flavor.
        let mut info: libc::proc_vnodepathinfo = unsafe { zeroed() };
        let size = size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
        let read = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                (&mut info as *mut libc::proc_vnodepathinfo).cast(),
                size,
            )
        };
        if read != size {
            return None;
        }
        // `vip_path` is one `MAXPATHLEN` C string, split in rows for libc.
        let path = info.pvi_cdir.vip_path.as_flattened();
        // SAFETY: `c_char` and `u8` have the same size and alignment.
        let bytes: &[u8] = unsafe { std::slice::from_raw_parts(path.as_ptr().cast(), path.len()) };
        let path = CStr::from_bytes_until_nul(bytes).ok()?.to_str().ok()?;
        (!path.is_empty()).then(|| path.to_string())
    }

    /// The command line of `pid`, as MonoCode reads it (`ps -o args=`).
    pub fn args(pid: u32) -> Option<String> {
        let output = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "args="])
            .output()
            .inspect_err(|err| log::debug!("ps for terminal job {pid}: {err}"))
            .ok()?;
        let args = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (output.status.success() && !args.is_empty()).then_some(args)
    }
}

/// Elsewhere the dock's terminals report no processes.
#[cfg(not(target_os = "macos"))]
mod sys {
    pub fn children(_: u32) -> Vec<u32> {
        Vec::new()
    }
    pub fn tty(_: u32) -> Option<u32> {
        None
    }
    pub fn name(_: u32) -> Option<String> {
        None
    }
    pub fn shell(_: u32) -> Option<u32> {
        None
    }
    pub fn foreground(_: u32) -> Option<u32> {
        None
    }
    pub fn cwd(_: u32) -> Option<String> {
        None
    }
    pub fn args(_: u32) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_label_prefers_the_script_over_its_interpreter() {
        assert_eq!(
            command_label("node /usr/local/bin/npm run build"),
            Some("npm".into())
        );
        assert_eq!(
            command_label("python3 -u manage.py runserver"),
            Some("manage.py".into())
        );
        assert_eq!(command_label("/usr/bin/cargo build"), Some("cargo".into()));
        assert_eq!(command_label("node"), Some("node".into()));
        assert_eq!(command_label(""), None);
    }

    #[test]
    fn shells_and_login_are_not_jobs() {
        assert!(is_shell_name("zsh"));
        assert!(is_shell_name("-zsh"));
        assert!(is_shell_name("login"));
        assert!(!is_shell_name("npm"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_this_process() {
        let me = std::process::id();
        assert!(sys::name(me).is_some_and(|name| !name.is_empty()));
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(sys::cwd(me).as_deref(), cwd.to_str());
    }
}
