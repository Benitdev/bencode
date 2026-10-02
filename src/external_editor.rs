//! External editors discovery and launch integration.
//!
//! Ported from MonoCode (`src-tauri/src/external_editor.rs`).
//! Discovers installed IDEs and code editors (VS Code, Cursor, Zed, Windsurf, Sublime Text)
//! and provides functionality to open workspaces or specific files at a line number.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalEditor {
    pub id: &'static str,
    pub name: &'static str,
    /// Launcher resolved during discovery, so callers that cache the editor
    /// list do not rescan `PATH` on every click.
    #[serde(skip)]
    launcher: Option<EditorLauncher>,
}

pub struct EditorDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub commands: &'static [&'static str],
    #[cfg(target_os = "macos")]
    pub mac_apps: &'static [&'static str],
}

pub const EDITORS: &[EditorDefinition] = &[
    EditorDefinition {
        id: "cursor",
        name: "Cursor",
        commands: &["cursor"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Cursor.app"],
    },
    EditorDefinition {
        id: "vscode",
        name: "Visual Studio Code",
        commands: &["code"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Visual Studio Code.app"],
    },
    EditorDefinition {
        id: "zed",
        name: "Zed",
        commands: &["zed"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Zed.app", "Zed Preview.app"],
    },
    EditorDefinition {
        id: "windsurf",
        name: "Windsurf",
        commands: &["windsurf"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Windsurf.app"],
    },
    EditorDefinition {
        id: "sublime-text",
        name: "Sublime Text",
        commands: &["subl", "sublime_text"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Sublime Text.app"],
    },
    EditorDefinition {
        id: "vscodium",
        name: "VSCodium",
        commands: &["codium"],
        #[cfg(target_os = "macos")]
        mac_apps: &["VSCodium.app"],
    },
    EditorDefinition {
        id: "vscode-insiders",
        name: "Visual Studio Code Insiders",
        commands: &["code-insiders"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Visual Studio Code - Insiders.app"],
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorLauncher {
    Command(PathBuf),
    #[cfg(target_os = "macos")]
    MacApp(PathBuf),
}

pub fn definition(id: &str) -> Option<&'static EditorDefinition> {
    EDITORS.iter().find(|editor| editor.id == id)
}

#[cfg(target_os = "macos")]
fn installed_mac_app(editor: &EditorDefinition) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let user_applications = home.map(|h| h.join("Applications"));

    editor.mac_apps.iter().find_map(|name| {
        user_applications
            .as_ref()
            .map(|root| root.join(name))
            .filter(|path| path.is_dir())
            .or_else(|| {
                let path = Path::new("/Applications").join(name);
                path.is_dir().then_some(path)
            })
    })
}

/// True when `path` is a regular file (after following symlinks) that the
/// current platform considers runnable.
fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn find_executable_in<I>(dirs: I, cmd: &str) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    dirs.into_iter()
        .map(|dir| dir.join(cmd))
        .find(|candidate| is_executable_file(candidate))
}

/// Fallback locations for CLIs when the GUI app was launched without a login `PATH`.
const COMMON_BIN_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"];

fn resolve_cli_binary(cmd: &str) -> Option<PathBuf> {
    let path_dirs = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    let common_dirs = COMMON_BIN_DIRS.iter().map(PathBuf::from);
    let local_bin = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/bin"));

    find_executable_in(
        path_dirs.into_iter().chain(common_dirs).chain(local_bin),
        cmd,
    )
}

pub fn resolve_editor(editor: &EditorDefinition) -> Option<EditorLauncher> {
    // Check CLI first for direct line-number support if available
    for command in editor.commands {
        if let Some(path) = resolve_cli_binary(command) {
            return Some(EditorLauncher::Command(path));
        }
    }

    #[cfg(target_os = "macos")]
    if let Some(path) = installed_mac_app(editor) {
        return Some(EditorLauncher::MacApp(path));
    }

    None
}

/// Returns a list of external editors currently installed on the host system.
///
/// This scans `PATH` and application folders; cache the result and launch
/// with [`launch_external_editor_with`] rather than calling it per click.
pub fn list_external_editors() -> Vec<ExternalEditor> {
    EDITORS
        .iter()
        .filter_map(|editor| {
            resolve_editor(editor).map(|launcher| ExternalEditor {
                id: editor.id,
                name: editor.name,
                launcher: Some(launcher),
            })
        })
        .collect()
}

/// Launches the specified external editor with the given directory or file.
pub fn launch_external_editor(
    editor_id: &str,
    cwd: &str,
    file_relative: Option<&str>,
    line: Option<usize>,
) -> Result<()> {
    let editor = definition(editor_id).ok_or_else(|| anyhow!("Unknown external editor."))?;
    let editor = ExternalEditor {
        id: editor.id,
        name: editor.name,
        launcher: None,
    };
    launch_external_editor_with(&editor, cwd, file_relative, line)
}

/// Launches an editor taken from a (possibly cached) [`list_external_editors`]
/// result. Uses the launcher resolved at discovery time when available.
pub fn launch_external_editor_with(
    editor: &ExternalEditor,
    cwd: &str,
    file_relative: Option<&str>,
    line: Option<usize>,
) -> Result<()> {
    let definition = definition(editor.id).ok_or_else(|| anyhow!("Unknown external editor."))?;
    let (cwd_path, target_path) = resolve_target(cwd, file_relative)?;

    let launcher = match &editor.launcher {
        Some(launcher) => launcher.clone(),
        None => resolve_editor(definition)
            .ok_or_else(|| anyhow!("{} is not installed.", definition.name))?,
    };

    let mut cmd = build_command(&launcher, definition.id, &target_path, file_relative, line);
    cmd.current_dir(&cwd_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = cmd
        .spawn()
        .with_context(|| format!("Could not open {}", definition.name))?;
    reap_in_background(child, definition.name);
    Ok(())
}

/// Canonicalizes `cwd` and the optional relative target, refusing any target
/// that resolves (via `..`, an absolute path, or a symlink) outside `cwd`.
fn resolve_target(cwd: &str, file_relative: Option<&str>) -> Result<(PathBuf, PathBuf)> {
    let cwd_path = Path::new(cwd);
    if !cwd_path.is_dir() {
        bail!("{} is not a directory.", cwd_path.display());
    }
    let cwd_canonical = cwd_path
        .canonicalize()
        .with_context(|| format!("Cannot resolve {}", cwd_path.display()))?;
    let Some(rel) = file_relative else {
        return Ok((cwd_canonical.clone(), cwd_canonical));
    };
    let joined = cwd_canonical.join(rel);
    let target = joined
        .canonicalize()
        .with_context(|| format!("Cannot resolve {}", joined.display()))?;
    if !target.starts_with(&cwd_canonical) {
        bail!("{rel} is outside the workspace.");
    }
    Ok((cwd_canonical, target))
}

fn build_command(
    launcher: &EditorLauncher,
    editor_id: &str,
    target_path: &Path,
    file_relative: Option<&str>,
    line: Option<usize>,
) -> Command {
    match launcher {
        EditorLauncher::Command(binary) => {
            let mut command = Command::new(binary);
            match (editor_id, file_relative, line) {
                // VS Code / Cursor / Windsurf support -g file:line
                (
                    "vscode" | "cursor" | "windsurf" | "vscode-insiders" | "vscodium",
                    Some(_),
                    Some(line_num),
                ) => {
                    command
                        .arg("-g")
                        .arg(format!("{}:{line_num}", target_path.display()));
                }
                // Zed supports path:line directly
                ("zed", Some(_), Some(line_num)) => {
                    command.arg(format!("{}:{line_num}", target_path.display()));
                }
                _ => {
                    command.arg(target_path);
                }
            }
            command
        }
        #[cfg(target_os = "macos")]
        EditorLauncher::MacApp(app) => {
            let mut command = Command::new("/usr/bin/open");
            command.arg("-a").arg(app).arg(target_path);
            command
        }
    }
}

/// Waits on the launcher in a detached thread so it never lingers as a zombie.
fn reap_in_background(mut child: Child, editor_name: &'static str) {
    let spawned = std::thread::Builder::new()
        .name("external-editor-reaper".into())
        .spawn(move || match child.wait() {
            Ok(status) if !status.success() => {
                log::warn!("{editor_name} launcher exited with {status}");
            }
            Ok(_) => {}
            Err(err) => log::warn!("failed to wait on {editor_name} launcher: {err}"),
        });
    if let Err(err) = spawned {
        log::warn!("could not start reaper thread for {editor_name}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bencode-editor-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn editor_ids_and_names_are_unique() {
        let mut ids = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for editor in EDITORS {
            assert!(ids.insert(editor.id), "duplicate editor id: {}", editor.id);
            assert!(
                names.insert(editor.name),
                "duplicate editor name: {}",
                editor.name
            );
            assert!(!editor.commands.is_empty());
        }
    }

    #[test]
    fn unknown_editor_is_rejected_before_launch() {
        let error = launch_external_editor("not-an-editor", ".", None, None).unwrap_err();
        assert_eq!(error.to_string(), "Unknown external editor.");
    }

    #[test]
    fn invalid_cwd_is_rejected() {
        let error = launch_external_editor("vscode", "/path/that/does/not/exist/ever", None, None)
            .unwrap_err();
        assert!(error.to_string().contains("is not a directory"));
    }

    #[test]
    fn resolve_target_accepts_files_inside_cwd() {
        let root = temp_dir("inside");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();

        let (cwd, target) = resolve_target(root.to_str().unwrap(), Some("src/main.rs")).unwrap();

        assert_eq!(target, cwd.join("src/main.rs"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_target_rejects_parent_and_absolute_escapes() {
        let root = temp_dir("escape");
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(root.join("secret.txt"), "x").unwrap();
        let cwd = project.to_str().unwrap();
        let absolute = root.join("secret.txt");

        assert!(resolve_target(cwd, Some("../secret.txt")).is_err());
        assert!(resolve_target(cwd, Some(absolute.to_str().unwrap())).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn resolve_target_rejects_symlink_escapes() {
        let root = temp_dir("symlink");
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(root.join("secret.txt"), "x").unwrap();
        std::os::unix::fs::symlink(root.join("secret.txt"), project.join("link.txt")).unwrap();

        let error = resolve_target(project.to_str().unwrap(), Some("link.txt")).unwrap_err();

        assert!(error.to_string().contains("outside the workspace"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cli_lookup_requires_the_executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("exec");
        let plain = dir.join("plain-editor");
        let runnable = dir.join("runnable-editor");
        std::fs::write(&plain, "#!/bin/sh\n").unwrap();
        std::fs::write(&runnable, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&runnable, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(find_executable_in([dir.clone()], "plain-editor"), None);
        assert_eq!(
            find_executable_in([dir.clone()], "runnable-editor"),
            Some(runnable)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn launch_with_cached_launcher_spawns_and_reaps() {
        let root = temp_dir("launch");
        std::fs::write(root.join("a.txt"), "a").unwrap();
        let editor = ExternalEditor {
            id: "zed",
            name: "Zed",
            launcher: Some(EditorLauncher::Command(PathBuf::from("/usr/bin/true"))),
        };

        launch_external_editor_with(&editor, root.to_str().unwrap(), Some("a.txt"), Some(3))
            .unwrap();
        let escape =
            launch_external_editor_with(&editor, root.to_str().unwrap(), Some("../"), None);

        assert!(escape.is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
