//! `git` as BenCode runs it, for what core runs itself (the file index and
//! search). The app's `git/` module builds every other launch on these.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The git BenCode runs: the first on PATH, except that macOS's
/// `/usr/bin/git` is a stub that looks the developer tools' git up again on
/// every launch, which doubles what a git call costs (about 6ms of each,
/// more while BenCode starts). The git it would run is found once instead.
pub fn program() -> &'static std::ffi::OsStr {
    static PROGRAM: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    PROGRAM
        .get_or_init(|| {
            let on_path = std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|dir| dir.join("git"))
                    .find(|git| git.is_file())
            });
            let stub = on_path
                .as_deref()
                .is_some_and(|git| git == Path::new("/usr/bin/git"));
            let tools = if stub { developer_git() } else { None };
            tools.unwrap_or_else(|| PathBuf::from("git"))
        })
        .as_os_str()
}

/// The git inside the developer folder `xcode-select` points at, as the
/// stub finds it: `DEVELOPER_DIR`, else the folder chosen with
/// `xcode-select -s`, else Xcode's or the Command Line Tools'. Read from
/// disk, never through `xcrun`, which offers to install the tools when
/// they are missing.
fn developer_git() -> Option<PathBuf> {
    let chosen = std::env::var_os("DEVELOPER_DIR")
        .map(PathBuf::from)
        .or_else(|| std::fs::read_link("/var/db/xcode_select_link").ok());
    let folders = chosen.into_iter().chain([
        PathBuf::from("/Applications/Xcode.app/Contents/Developer"),
        PathBuf::from("/Library/Developer/CommandLineTools"),
    ]);
    folders
        .map(|folder| folder.join("usr/bin/git"))
        .find(|git| git.is_file())
}

/// `git` with BenCode's options, run where the caller says. For a launch
/// outside any repository (`diff --no-index`); the others use `command`.
pub fn base_command() -> Command {
    let mut cmd = Command::new(program());
    cmd.args(["-c", "core.quotepath=false", "-c", "color.ui=never"])
        // Status polling from the UI must not fight other git processes for index.lock.
        .env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

/// `git -C cwd` with BenCode's options.
pub fn command(cwd: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = base_command();
    cmd.arg("-C").arg(cwd);
    cmd
}
