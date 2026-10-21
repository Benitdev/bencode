//! A project's files for Go to File and search (MonoCode `list_project_files`
//! in `src-tauri/src/fs.rs`). In core so the remote host lists a folder
//! there as the app lists one here.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;

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
/// `.gitignore` honoured, repositories cloned inside it included), else a
/// bounded walk that skips vendored folders. Paths are relative, at most
/// 20,000. Blocking: run off the UI thread.
pub fn list_project_files(root: &Path) -> Vec<String> {
    if !root.is_dir() || !indexable_root(root) {
        return Vec::new();
    }
    git_ls_files(root).unwrap_or_else(|| walk_project_files(root))
}

/// How deep repositories cloned inside repositories are followed.
const MAX_NESTED_DEPTH: usize = 2;

/// `None` when this is not a repository or the listing ran past its cap; a
/// partial listing would quietly hide files, so the walk takes over.
fn git_ls_files(root: &Path) -> Option<Vec<String>> {
    let mut files = Vec::new();
    list_repo(root, "", 0, &mut files)?;
    files.sort();
    files.truncate(MAX_PROJECT_FILES);
    Some(files)
}

/// Adds `dir`'s files to `files` under `prefix`, then those of repositories
/// cloned inside it: a workspace that keeps its repositories side by side
/// usually ignores them, so git alone never lists their files.
fn list_repo(dir: &Path, prefix: &str, depth: usize, files: &mut Vec<String>) -> Option<()> {
    let listed = ls_files(dir, &["-co", "--exclude-standard"])?;
    let mut folders = Vec::new();
    for rel in listed {
        if rel.ends_with('/') {
            // An untracked folder git does not enter: a repository.
            folders.push(rel);
        } else if !rel.split('/').any(skip_dir_name) && !rel.ends_with(".DS_Store") {
            files.push(format!("{prefix}{rel}"));
        }
    }
    if depth >= MAX_NESTED_DEPTH {
        return Some(());
    }
    // Ignored folders come back whole with `--directory`.
    if let Some(ignored) = ls_files(dir, &["-oi", "--exclude-standard", "--directory"]) {
        folders.extend(ignored.into_iter().filter(|rel| rel.ends_with('/')));
    }
    for folder in folders {
        if files.len() >= MAX_PROJECT_FILES {
            break;
        }
        if folder.split('/').any(skip_dir_name) {
            continue;
        }
        // A clone has a `.git` folder; a worktree or submodule's is a file,
        // and a worktree only repeats its repository's files.
        let nested = dir.join(&folder);
        if !nested.join(".git").is_dir() {
            continue;
        }
        let prefix = format!("{prefix}{folder}");
        if list_repo(&nested, &prefix, depth + 1, files).is_none() {
            log::debug!("listing nested repository {}", nested.display());
        }
    }
    Some(())
}

/// `git ls-files -z` with `args` in `dir`; `None` when git fails or the
/// output runs past its cap.
fn ls_files(dir: &Path, args: &[&str]) -> Option<Vec<String>> {
    let mut child = crate::git_cli::command(dir)
        .arg("ls-files")
        .args(args)
        .arg("-z")
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
    Some(
        raw.split(|b| *b == 0)
            .filter(|rel| !rel.is_empty())
            .map(|rel| String::from_utf8_lossy(rel).into_owned())
            .collect(),
    )
}

/// Lines in a text file, for a change list's `+N` of an untracked file:
/// a binary, missing or larger-than-`max_bytes` file counts as 0.
pub fn count_text_lines(path: &Path, max_bytes: u64) -> usize {
    let Ok(meta) = std::fs::metadata(path) else {
        return 0;
    };
    if !meta.is_file() || meta.len() > max_bytes {
        return 0;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return 0;
    };
    if bytes.contains(&0) {
        return 0;
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count();
    let trailing_partial = bytes.last().is_some_and(|b| *b != b'\n');
    newlines + usize::from(trailing_partial)
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
    fn project_files_include_repositories_cloned_inside() {
        let root = std::env::temp_dir().join(format!("bencode-pf-nested-{}", std::process::id()));
        let git = |dir: &Path, args: &[&str]| {
            let ok = crate::git_cli::command(dir)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            assert!(ok, "git {args:?}");
        };
        std::fs::create_dir_all(root.join("app/src")).unwrap();
        std::fs::create_dir_all(root.join("app/node_modules/x")).unwrap();
        std::fs::create_dir_all(root.join("app/.claude/worktrees")).unwrap();
        std::fs::create_dir_all(root.join("loose/src")).unwrap();
        git(&root, &["init", "-q"]);
        git(&root.join("app"), &["init", "-q"]);
        git(&root.join("loose"), &["init", "-q"]);
        std::fs::write(root.join(".gitignore"), "app/\n").unwrap();
        std::fs::write(root.join("README.md"), "").unwrap();
        std::fs::write(root.join("app/.gitignore"), "node_modules/\n").unwrap();
        std::fs::write(root.join("app/src/Form.tsx"), "").unwrap();
        std::fs::write(root.join("app/node_modules/x/y.js"), "").unwrap();
        // A worktree's `.git` is a file: its files repeat the repository's.
        let app = root.join("app");
        git(&app, &["add", "src"]);
        git(
            &app,
            &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "a"],
        );
        git(&app, &["worktree", "add", "-q", ".claude/worktrees/w"]);
        std::fs::write(root.join("loose/src/lib.rs"), "").unwrap();

        assert_eq!(
            git_ls_files(&root).unwrap(),
            [
                ".gitignore",
                "README.md",
                "app/.gitignore",
                "app/src/Form.tsx",
                "loose/src/lib.rs",
            ]
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn project_files_from_this_repo_use_git() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let files = git_ls_files(root).expect("BenCode is a git repository");
        assert!(files.iter().any(|f| f == "src/files.rs"));
        assert!(!files.iter().any(|f| f.starts_with("target/")));
    }
}
