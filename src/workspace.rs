use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn list_workspace_files(root: &Path, max_files: usize) -> Vec<String> {
    let mut results = Vec::new();
    let mut dirs_to_visit = vec![root.to_path_buf()];

    while let Some(current_dir) = dirs_to_visit.pop() {
        if results.len() >= max_files {
            break;
        }

        let entries = match std::fs::read_dir(&current_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            if is_ignored(&name_str) {
                continue;
            }

            if path.is_dir() {
                dirs_to_visit.push(path);
            } else if path.is_file() {
                if let Ok(rel) = path.strip_prefix(root) {
                    results.push(rel.to_string_lossy().to_string());
                }
                if results.len() >= max_files {
                    break;
                }
            }
        }
    }

    results.sort();
    results
}

/// Directories and dotfiles hidden from the file tree and `@` mentions.
const IGNORED_NAMES: &[&str] = &["target", "node_modules", "dist", "build"];

fn is_ignored(name: &str) -> bool {
    name.starts_with('.') || IGNORED_NAMES.contains(&name)
}

/// MonoCode `MAX_PROJECT_FILES`, `MAX_WALK_DIRS`, `MAX_LS_FILES_BYTES`.
const MAX_PROJECT_FILES: usize = 20_000;
const MAX_WALK_DIRS: usize = 4_000;
const MAX_LS_FILES_BYTES: usize = 8 * 1024 * 1024;

/// MonoCode `skip_walk_dir_name`: build output, caches and vendored code.
fn skip_dir_name(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | "out"
            | ".next"
            | ".nuxt"
            | ".output"
            | ".cache"
            | ".turbo"
            | ".parcel-cache"
            | ".vercel"
            | ".svelte-kit"
            | "coverage"
            | "__pycache__"
            | ".venv"
            | "venv"
            | ".tox"
            | ".mypy_cache"
            | ".pytest_cache"
            | ".gradle"
            | ".idea"
            | "Pods"
            | "vendor"
            | "bower_components"
            | ".yarn"
            | ".pnpm-store"
    )
}

/// MonoCode `is_indexable_root`: never index the home folder or a volume root.
fn indexable_root(root: &Path) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let broad = [
        home,
        Some("/Users".into()),
        Some("/Applications".into()),
        Some("/Volumes".into()),
        Some("/home".into()),
    ];
    root.parent().is_some()
        && root.extension().is_none_or(|ext| ext != "app")
        && !broad.iter().flatten().any(|b| b.as_path() == root)
}

/// Every file of the project for Go to File (MonoCode `list_project_files`):
/// `git ls-files` when the folder is a repository (tracked and untracked,
/// `.gitignore` honoured), else a bounded walk that skips vendored folders.
/// Paths are relative, at most 20,000. Blocking: run off the UI thread.
pub fn list_project_files(root: &Path) -> Vec<String> {
    if !root.is_dir() || !indexable_root(root) {
        return Vec::new();
    }
    git_ls_files(root).unwrap_or_else(|| walk_project_files(root))
}

/// `None` when this is not a repository or the listing ran past its cap; a
/// partial listing would quietly hide files, so the walk takes over.
fn git_ls_files(root: &Path) -> Option<Vec<String>> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-co", "--exclude-standard", "-z"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut raw = Vec::new();
    let read = child
        .stdout
        .take()?
        .take(MAX_LS_FILES_BYTES as u64 + 1)
        .read_to_end(&mut raw);
    if read.is_err() || raw.len() > MAX_LS_FILES_BYTES {
        if let Err(err) = child.kill() {
            log::debug!("git ls-files already ended: {err}");
        }
        if let Err(err) = child.wait() {
            log::debug!("git ls-files wait: {err}");
        }
        return None;
    }
    if !child.wait().ok()?.success() {
        return None;
    }
    let mut files: Vec<String> = raw
        .split(|b| *b == 0)
        .filter(|rel| !rel.is_empty())
        .map(|rel| String::from_utf8_lossy(rel).into_owned())
        .filter(|rel| {
            !rel.ends_with('/') && !rel.split('/').any(skip_dir_name) && !rel.ends_with(".DS_Store")
        })
        .take(MAX_PROJECT_FILES)
        .collect();
    files.sort();
    Some(files)
}

/// MonoCode `walk_project_files`: symlinks are not followed, and the walk
/// stops at 4,000 folders or 20,000 files.
fn walk_project_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    let mut visited = 0;
    while let Some(dir) = dirs.pop() {
        visited += 1;
        if visited > MAX_WALK_DIRS || files.len() >= MAX_PROJECT_FILES {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() || name == ".DS_Store" {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                if !skip_dir_name(name) {
                    dirs.push(path);
                }
            } else if let Ok(rel) = path.strip_prefix(root) {
                files.push(rel.to_string_lossy().into_owned());
                if files.len() >= MAX_PROJECT_FILES {
                    break;
                }
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_files_walk_skips_vendored_dirs_and_symlinks() {
        let root = std::env::temp_dir().join(format!("bencode-pf-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src/deep/er")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("src/deep/er/a.rs"), "").unwrap();
        std::fs::write(root.join("node_modules/x/y.js"), "").unwrap();
        std::fs::write(root.join(".env"), "").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("src"), root.join("link")).unwrap();

        assert_eq!(walk_project_files(&root), [".env", "src/deep/er/a.rs"]);
        assert!(!indexable_root(Path::new("/")));
        assert!(!indexable_root(Path::new("/Users")));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn project_files_from_this_repo_use_git() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let files = git_ls_files(root).expect("BenCode is a git repository");
        assert!(files.iter().any(|f| f == "src/workspace.rs"));
        assert!(!files.iter().any(|f| f.starts_with("target/")));
    }

    #[test]
    fn list_workspace_files_skips_ignored_and_dotfiles() {
        let root = std::env::temp_dir().join(format!("bencode-ws-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("b.txt"), "").unwrap();
        std::fs::write(root.join("A.md"), "").unwrap();
        std::fs::write(root.join(".env"), "").unwrap();
        std::fs::write(root.join("src/main.rs"), "").unwrap();

        assert_eq!(
            list_workspace_files(&root, 10),
            ["A.md", "b.txt", "src/main.rs"]
        );

        std::fs::remove_dir_all(&root).unwrap();
    }
}
