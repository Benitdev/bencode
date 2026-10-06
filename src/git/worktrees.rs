//! Git worktree management: list, create, prune, and remove worktrees.
//!
//! Ported from MonoCode (`src-tauri/src/worktrees.rs`).
//! Enables isolated execution environments for sessions so agent edits
//! and branches do not interfere with the user's primary working directory.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub path: String,
    pub branch: Option<String>,
    pub head: String,
    pub is_main: bool,
    pub locked: bool,
    pub prunable: bool,
    pub missing: bool,
    pub dirty: Option<bool>,
    /// Commits on `HEAD` not reachable from any remote. `None` when the repo
    /// has no remotes (nothing could have been pushed) or the count failed.
    pub unpushed: Option<u64>,
    /// Why `dirty`/`unpushed` could not be computed, if a git call failed.
    pub status_error: Option<String>,
}

fn git_cmd(root: &Path, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    let output = command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(if err.is_empty() {
            format!("git {:?} failed", args)
        } else {
            err
        });
    }
    String::from_utf8(output.stdout).map_err(|_| anyhow!("Git returned invalid UTF-8"))
}

pub fn parse_worktrees(text: &str) -> Vec<Worktree> {
    let mut result = Vec::new();
    let mut current = Worktree::default();
    for field in text.split('\0') {
        if field.is_empty() {
            if !current.path.is_empty() {
                current.is_main = result.is_empty();
                result.push(std::mem::take(&mut current));
            }
        } else if let Some(path) = field.strip_prefix("worktree ") {
            current.path = path.to_string();
        } else if let Some(head) = field.strip_prefix("HEAD ") {
            current.head = head.to_string();
        } else if let Some(branch) = field.strip_prefix("branch refs/heads/") {
            current.branch = Some(branch.to_string());
        } else if field == "locked" || field.starts_with("locked ") {
            current.locked = true;
        } else if field == "prunable" || field.starts_with("prunable ") {
            current.prunable = true;
        }
    }
    if !current.path.is_empty() {
        current.is_main = result.is_empty();
        result.push(current);
    }
    result
}

/// Upper bound on concurrent `git` processes when gathering worktree status.
const MAX_PARALLEL_STATUS: usize = 8;

/// Lists all worktrees for the repository located at `cwd`.
///
/// Per-worktree status (`dirty`, `unpushed`) is gathered in parallel. A git
/// failure for one worktree is reported in its `status_error` field rather
/// than failing the whole listing.
pub fn list_worktrees(cwd: &str) -> Result<Vec<Worktree>> {
    let root = Path::new(cwd);
    let mut worktrees = list_registered(root)?;
    let has_remotes = repo_has_remotes(root)?;

    for chunk in worktrees.chunks_mut(MAX_PARALLEL_STATUS) {
        std::thread::scope(|scope| {
            for tree in chunk.iter_mut() {
                scope.spawn(move || fill_status(tree, has_remotes));
            }
        });
    }

    Ok(worktrees)
}

fn list_registered(root: &Path) -> Result<Vec<Worktree>> {
    let output = git_cmd(root, &["worktree", "list", "--porcelain", "-z"])?;
    Ok(parse_worktrees(&output))
}

fn repo_has_remotes(root: &Path) -> Result<bool> {
    Ok(!git_cmd(root, &["remote"])?.trim().is_empty())
}

fn is_dirty(path: &Path) -> Result<bool> {
    let status = git_cmd(path, &["status", "--porcelain", "--untracked-files=normal"])?;
    Ok(!status.trim().is_empty())
}

fn unpushed_count(path: &Path) -> Result<u64> {
    let count = git_cmd(path, &["rev-list", "--count", "HEAD", "--not", "--remotes"])?;
    count
        .trim()
        .parse()
        .map_err(|_| anyhow!("Unexpected rev-list output: {}", count.trim()))
}

fn fill_status(tree: &mut Worktree, has_remotes: bool) {
    let path = PathBuf::from(&tree.path);
    tree.missing = !path.is_dir();
    if tree.missing {
        return;
    }
    let mut errors = Vec::new();
    match is_dirty(&path) {
        Ok(dirty) => tree.dirty = Some(dirty),
        Err(err) => errors.push(format!("status: {err}")),
    }
    if has_remotes {
        match unpushed_count(&path) {
            Ok(count) => tree.unpushed = Some(count),
            Err(err) => errors.push(format!("unpushed: {err}")),
        }
    }
    if !errors.is_empty() {
        let message = errors.join("; ");
        log::warn!("worktree {}: {message}", tree.path);
        tree.status_error = Some(message);
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    a == b
}

/// Computes the default worktrees directory beside the main repository root.
pub fn default_worktrees_dir(main_root: &Path) -> PathBuf {
    let name = main_root
        .file_name()
        .map_or_else(|| "repo".to_string(), |n| n.to_string_lossy().into_owned());
    main_root.with_file_name(format!("{name}-worktrees"))
}

/// Creates a new worktree for `branch`.
///
/// With `existing_branch`, `branch` must name an existing *local* branch
/// (`refs/heads/{branch}`); tags, remote refs, and SHAs are rejected.
/// Otherwise a new branch is created from `base` (or `HEAD` when empty).
pub fn create_worktree(
    cwd: &str,
    branch: &str,
    base: &str,
    existing_branch: bool,
) -> Result<Worktree> {
    let branch = branch.trim();
    if branch.starts_with('-') || branch.starts_with('@') || branch.is_empty() {
        bail!("Enter a valid branch name");
    }

    let root = Path::new(cwd);
    git_cmd(root, &["check-ref-format", "--branch", branch])?;

    let existing_trees = list_registered(root)?;
    if existing_trees
        .iter()
        .any(|t| t.branch.as_deref() == Some(branch))
    {
        bail!("This branch already has a working copy. Select it from the picker.");
    }

    let main = existing_trees
        .first()
        .ok_or_else(|| anyhow!("No working copies found"))?;
    let parent = default_worktrees_dir(Path::new(&main.path));
    let target_path = parent.join(branch_slug(branch));
    if target_path.exists() {
        bail!(
            "{} already exists. Choose another branch name.",
            target_path.display()
        );
    }

    // Resolve user-supplied refs before `worktree add`. Never let a ref be
    // read as an option, and only accept existing local branches.
    let source = if existing_branch {
        format!("refs/heads/{branch}")
    } else if base.trim().is_empty() {
        "HEAD".to_string()
    } else {
        base.trim().to_string()
    };
    let commit = git_cmd(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{source}^{{commit}}"),
        ],
    )?;

    std::fs::create_dir_all(&parent)?;
    let target_str = target_path.to_string_lossy();
    if existing_branch {
        git_cmd(root, &["worktree", "add", "--", &target_str, branch])?;
    } else {
        git_cmd(
            root,
            &[
                "worktree",
                "add",
                "--no-track",
                "-b",
                branch,
                "--",
                &target_str,
                commit.trim(),
            ],
        )?;
    }

    list_worktrees(cwd)?
        .into_iter()
        .find(|t| same_path(Path::new(&t.path), &target_path))
        .ok_or_else(|| anyhow!("Worktree created, but could not be found. Refresh the working copies."))
}

fn branch_slug(branch: &str) -> String {
    branch
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Resolves `path` to a registered, removable linked worktree of `root`.
fn removal_target(root: &Path, path: &Path) -> Result<Worktree> {
    let tree = list_registered(root)?
        .into_iter()
        .find(|tree| same_path(Path::new(&tree.path), path))
        .ok_or_else(|| anyhow!("This path is not a registered worktree of this repository"))?;
    if tree.is_main {
        bail!("The main working copy cannot be deleted");
    }
    if tree.locked {
        bail!("This worktree is locked. Unlock it in Git before deleting it.");
    }
    if tree.branch.is_none() {
        // No branch would keep its commits once the folder is gone.
        bail!("Create a branch for this detached worktree before deleting it.");
    }
    Ok(tree)
}

/// Refuses to drop work that exists only in this worktree unless `force`.
fn check_removal_safety(root: &Path, tree: &Worktree) -> Result<()> {
    let path = Path::new(&tree.path);
    if !path.is_dir() {
        bail!("This worktree's directory is missing. Prune worktrees instead.");
    }
    if is_dirty(path)? {
        bail!("This worktree has uncommitted or untracked changes.");
    }
    if repo_has_remotes(root)? {
        let unpushed = unpushed_count(path)?;
        if unpushed > 0 {
            bail!("This worktree has {unpushed} unpushed commit(s).");
        }
    }
    Ok(())
}

/// Removes a linked worktree by its filesystem path.
///
/// The path must match a registered worktree (never the main one). Without
/// `force`, a dirty worktree, a detached HEAD, or unpushed commits block removal.
pub fn remove_worktree(cwd: &str, path: &str, force: bool) -> Result<()> {
    let root = Path::new(cwd);
    let tree = removal_target(root, Path::new(path))?;
    if !force {
        check_removal_safety(root, &tree)?;
    }
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.extend(["--", tree.path.as_str()]);
    git_cmd(root, &args)?;
    Ok(())
}

/// Prunes dead worktree references from git metadata.
pub fn prune_worktrees(cwd: &str) -> Result<()> {
    let root = Path::new(cwd);
    git_cmd(root, &["worktree", "prune"])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_porcelain_worktrees_output() {
        let porcelain = "worktree /path/to/main\0HEAD 1234567890abcdef\0branch refs/heads/main\0\0worktree /path/to/feature\0HEAD abcdef1234567890\0branch refs/heads/feat\0locked\0\0";
        let trees = parse_worktrees(porcelain);
        assert_eq!(trees.len(), 2);

        assert_eq!(trees[0].path, "/path/to/main");
        assert_eq!(trees[0].branch.as_deref(), Some("main"));
        assert_eq!(trees[0].head, "1234567890abcdef");
        assert!(trees[0].is_main);
        assert!(!trees[0].locked);

        assert_eq!(trees[1].path, "/path/to/feature");
        assert_eq!(trees[1].branch.as_deref(), Some("feat"));
        assert!(!trees[1].is_main);
        assert!(trees[1].locked);
    }

    #[test]
    fn default_worktrees_dir_calculation() {
        let main = Path::new("/Users/user/project");
        let wt_dir = default_worktrees_dir(main);
        assert_eq!(wt_dir, PathBuf::from("/Users/user/project-worktrees"));
    }

    /// A throwaway repository under a unique temp dir, removed on drop.
    struct Repo {
        base: PathBuf,
        main: PathBuf,
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            if let Err(err) = std::fs::remove_dir_all(&self.base) {
                eprintln!("failed to clean {}: {err}", self.base.display());
            }
        }
    }

    fn run(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn repo() -> Repo {
        let base = std::env::temp_dir().join(format!(
            "bencode-wt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let main = base.join("project");
        std::fs::create_dir_all(&main).unwrap();
        run(&main, &["init", "-q"]);
        std::fs::write(main.join("README.md"), "hello\n").unwrap();
        run(&main, &["add", "README.md"]);
        run(&main, &["commit", "-q", "-m", "init"]);
        Repo { base, main }
    }

    fn add_remote_and_push(repo: &Repo) {
        let remote = repo.base.join("remote.git");
        run(
            &repo.base,
            &["init", "-q", "--bare", remote.to_str().unwrap()],
        );
        run(
            &repo.main,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        run(&repo.main, &["push", "-q", "origin", "main"]);
    }

    #[test]
    fn list_reports_no_unpushed_count_without_remotes() {
        let repo = repo();

        let trees = list_worktrees(repo.main.to_str().unwrap()).unwrap();

        assert_eq!(trees.len(), 1);
        assert!(trees[0].is_main);
        assert_eq!(trees[0].dirty, Some(false));
        assert_eq!(trees[0].unpushed, None);
        assert_eq!(trees[0].status_error, None);
    }

    #[test]
    fn list_counts_unpushed_commits_with_a_remote() {
        let repo = repo();
        add_remote_and_push(&repo);
        std::fs::write(repo.main.join("b.txt"), "b").unwrap();
        run(&repo.main, &["add", "b.txt"]);
        run(&repo.main, &["commit", "-q", "-m", "b"]);

        let trees = list_worktrees(repo.main.to_str().unwrap()).unwrap();

        assert_eq!(trees[0].unpushed, Some(1));
    }

    #[test]
    fn list_surfaces_git_failures() {
        let dir = std::env::temp_dir().join(format!("bencode-wt-norepo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let result = list_worktrees(dir.to_str().unwrap());

        assert!(result.is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn create_existing_branch_accepts_only_local_branches() {
        let repo = repo();
        let cwd = repo.main.to_str().unwrap();
        run(&repo.main, &["branch", "feature"]);
        run(&repo.main, &["tag", "v1"]);

        let missing = create_worktree(cwd, "nope", "", true);
        let tag = create_worktree(cwd, "v1", "", true);
        let created = create_worktree(cwd, "feature", "", true).unwrap();

        assert!(missing.is_err());
        assert!(tag.is_err());
        assert_eq!(created.branch.as_deref(), Some("feature"));
        assert!(!created.is_main);
    }

    #[test]
    fn remove_rejects_main_and_unregistered_paths() {
        let repo = repo();
        let cwd = repo.main.to_str().unwrap();
        let stray = repo.base.join("stray");
        std::fs::create_dir_all(&stray).unwrap();

        let main = remove_worktree(cwd, cwd, true).unwrap_err();
        let unregistered = remove_worktree(cwd, stray.to_str().unwrap(), true).unwrap_err();

        assert!(main.to_string().contains("main working copy"));
        assert!(
            unregistered
                .to_string()
                .contains("not a registered worktree")
        );
        assert!(repo.main.join("README.md").exists());
        assert!(stray.exists());
    }

    #[test]
    fn remove_refuses_dirty_worktree_unless_forced() {
        let repo = repo();
        let cwd = repo.main.to_str().unwrap();
        let tree = create_worktree(cwd, "dirty", "", false).unwrap();
        std::fs::write(Path::new(&tree.path).join("scratch.txt"), "wip").unwrap();

        let refused = remove_worktree(cwd, &tree.path, false).unwrap_err();
        assert!(refused.to_string().contains("uncommitted"));
        assert!(Path::new(&tree.path).exists());

        remove_worktree(cwd, &tree.path, true).unwrap();
        assert!(!Path::new(&tree.path).exists());
    }

    #[test]
    fn remove_refuses_unpushed_commits_unless_forced() {
        let repo = repo();
        add_remote_and_push(&repo);
        let cwd = repo.main.to_str().unwrap();
        let tree = create_worktree(cwd, "ahead", "", false).unwrap();
        let tree_path = Path::new(&tree.path);
        std::fs::write(tree_path.join("new.txt"), "new").unwrap();
        run(tree_path, &["add", "new.txt"]);
        run(tree_path, &["commit", "-q", "-m", "ahead"]);

        let refused = remove_worktree(cwd, &tree.path, false).unwrap_err();
        assert!(refused.to_string().contains("unpushed"));

        remove_worktree(cwd, &tree.path, true).unwrap();
        assert!(!tree_path.exists());
    }

    #[test]
    fn remove_clean_pushed_worktree_without_force() {
        let repo = repo();
        add_remote_and_push(&repo);
        let cwd = repo.main.to_str().unwrap();
        let tree = create_worktree(cwd, "clean", "", false).unwrap();

        remove_worktree(cwd, &tree.path, false).unwrap();

        assert!(!Path::new(&tree.path).exists());
    }

    #[test]
    fn prune_drops_worktrees_whose_directory_vanished() {
        let repo = repo();
        let cwd = repo.main.to_str().unwrap();
        let tree = create_worktree(cwd, "gone", "", false).unwrap();
        std::fs::remove_dir_all(&tree.path).unwrap();

        prune_worktrees(cwd).unwrap();

        let trees = list_worktrees(cwd).unwrap();
        assert_eq!(trees.len(), 1);
    }

    #[test]
    fn remove_refuses_detached_worktree_even_forced() {
        let repo = repo();
        let cwd = repo.main.to_str().unwrap();
        let tree = create_worktree(cwd, "detach", "", false).unwrap();
        run(Path::new(&tree.path), &["checkout", "-q", "--detach"]);

        let refused = remove_worktree(cwd, &tree.path, true).unwrap_err();

        assert!(refused.to_string().contains("detached"));
        assert!(Path::new(&tree.path).exists());
    }
}
