use std::collections::HashMap;
use std::path::{Component, Path};
use std::process::{Command, Output};

use anyhow::{Context as _, Result, bail};

mod branches;
mod diffs;
pub mod graph;
mod rows;
mod status_pass;
pub mod sync;
pub mod text;
pub use branches::{
    Branch, SwitchError, create_branch, list_branches, stash_changes, switch_branch,
};
pub use diffs::{DiffSource, commit_files, file_diff};
pub use rows::number_rows;

// Only `list_worktrees` feeds the sidebar switcher so far; create/remove/prune
// are tracked in docs/migration/TODO-100-PERCENT-COVERAGE.md (2.1). `expect`
// (not `allow`) so the attribute errors out once the rest becomes live.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "worktree create/remove/prune are not wired yet")
)]
pub mod worktrees;
pub use status_pass::{StateFingerprint, StatusPass, read_local_state};
pub use worktrees::Worktree;

pub mod checkpoint;

/// Fallback branch name shown when git cannot tell us anything better.
const DEFAULT_BRANCH: &str = "main";
/// Well-known SHA-1 empty tree; used only if `git hash-object` fails.
const EMPTY_TREE_SHA1: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
/// Untracked files larger than this are not read for their diff.
const MAX_UNTRACKED_READ_BYTES: u64 = 8 * 1024 * 1024;
/// MonoCode `MAX_UNTRACKED_BYTES`: larger untracked files count no lines
/// in the +N figures, so the rail and Changes match MonoCode's.
const MAX_UNTRACKED_COUNT_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitFileStatus {
    Modified,
    Added,
    Deleted,
    Untracked,
    Renamed,
}

#[derive(Clone, Debug)]
pub struct GitFileChange {
    pub path: String,
    pub status: GitFileStatus,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Header(String),
    Addition(String),
    Deletion(String),
    Context(String),
}

#[derive(Clone, Debug, Default)]
pub struct GitDetailedStatus {
    pub branch: String,
    pub staged: Vec<GitFileChange>,
    pub unstaged: Vec<GitFileChange>,
}

// ---------------------------------------------------------------------------
// Process helpers
// ---------------------------------------------------------------------------

/// `git` with BenCode's options, run where the caller says. For a launch
/// outside any repository (`diff --no-index`); the others use `git_command`.
pub(crate) fn git_base_command() -> Command {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "core.quotepath=false", "-c", "color.ui=never"])
        // Status polling from the UI must not fight other git processes for index.lock.
        .env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

/// `git -C cwd` with BenCode's options: every git launch starts here.
pub(crate) fn git_command(cwd: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = git_base_command();
    cmd.arg("-C").arg(cwd);
    cmd
}

fn git_output(cwd: &str, args: &[&str]) -> Result<Output> {
    git_command(cwd)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn `git {}`", args.join(" ")))
}

/// Runs git and returns stdout, or an error containing stderr on non-zero exit.
fn run_git(cwd: &str, args: &[&str]) -> Result<Vec<u8>> {
    let out = git_output(cwd, args)?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    bail!(
        "`git {}` failed ({}): {}",
        args.join(" "),
        out.status,
        String::from_utf8_lossy(&out.stderr).trim()
    )
}

fn run_git_string(cwd: &str, args: &[&str]) -> Result<String> {
    run_git(cwd, args).map(|out| String::from_utf8_lossy(&out).into_owned())
}

fn git_succeeds(cwd: &str, args: &[&str]) -> bool {
    git_output(cwd, args)
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn has_head(cwd: &str) -> bool {
    git_succeeds(cwd, &["rev-parse", "--verify", "--quiet", "HEAD"])
}

fn empty_tree(cwd: &str) -> String {
    run_git_string(cwd, &["hash-object", "-t", "tree", "--stdin"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| EMPTY_TREE_SHA1.to_string())
}

fn is_in_index(cwd: &str, file: &str) -> bool {
    git_succeeds(cwd, &["ls-files", "--error-unmatch", "--", file])
}

/// Rejects empty, absolute, or `..`-escaping paths before touching the filesystem.
fn validate_relative_path(file: &str) -> Result<()> {
    if file.trim().is_empty() {
        bail!("file path must not be empty");
    }
    let path = Path::new(file);
    let escapes = path
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir));
    if escapes {
        bail!("refusing to operate on path outside the repository: {file}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Porcelain / numstat parsing
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
struct StatusEntry {
    index: char,
    worktree: char,
    path: String,
    orig_path: Option<String>,
}

impl StatusEntry {
    fn is_untracked(&self) -> bool {
        self.index == '?' && self.worktree == '?'
    }

    fn is_ignored(&self) -> bool {
        self.index == '!'
    }

    fn is_unmerged(&self) -> bool {
        let (x, y) = (self.index, self.worktree);
        x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D')
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Parses `git status --porcelain=v1 -z`. Renames/copies are `XY new\0old\0`.
fn parse_porcelain_z(raw: &[u8]) -> Vec<StatusEntry> {
    let mut records = raw.split(|b| *b == 0);
    let mut entries = Vec::new();
    while let Some(rec) = records.next() {
        if rec.len() < 4 || rec[2] != b' ' {
            continue;
        }
        let index = rec[0] as char;
        let worktree = rec[1] as char;
        let has_origin = matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C');
        let orig_path = if has_origin {
            records.next().map(lossy)
        } else {
            None
        };
        entries.push(StatusEntry {
            index,
            worktree,
            path: lossy(&rec[3..]),
            orig_path,
        });
    }
    entries
}

/// Cheap summary of the repository state, in three parts so a poll can tell
/// what kind of change happened: refs (HEAD, branch, upstream counts, every
/// ref), the set of added / removed paths, and everything status and the
/// changed-line totals show. Equal fingerprints mean a refresh would show
/// nothing new. `None` outside a repository.
pub fn state_fingerprint(cwd: &str) -> Option<StateFingerprint> {
    StatusPass::read(cwd).map(|pass| pass.fingerprint(cwd))
}

fn read_status(cwd: &str) -> Result<Vec<StatusEntry>> {
    if !Path::new(cwd).exists() {
        bail!("workspace path does not exist: {cwd}");
    }
    let raw = run_git(cwd, &["status", "--porcelain=v1", "-z", "-uall"])?;
    Ok(parse_porcelain_z(&raw))
}

type NumstatMap = HashMap<String, (usize, usize)>;

/// Parses `git diff --numstat -z`. Renames are `adds\tdels\t\0old\0new\0`.
/// Binary files report `-` and are counted as 0/0. Keyed by the new path.
fn parse_numstat_z(raw: &[u8]) -> NumstatMap {
    let text = String::from_utf8_lossy(raw);
    let mut records = text.split('\0');
    let mut map = NumstatMap::new();
    while let Some(rec) = records.next() {
        let mut parts = rec.splitn(3, '\t');
        let (Some(adds), Some(dels), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let key = if path.is_empty() {
            let _old = records.next();
            match records.next() {
                Some(new) => new.to_string(),
                None => continue,
            }
        } else {
            path.to_string()
        };
        map.insert(key, (adds.parse().unwrap_or(0), dels.parse().unwrap_or(0)));
    }
    map
}

fn diff_numstat(cwd: &str, extra: &[&str]) -> NumstatMap {
    let args: Vec<&str> = ["diff", "--no-ext-diff", "--numstat", "-z", "-M"]
        .into_iter()
        .chain(extra.iter().copied())
        .collect();
    run_git(cwd, &args)
        .map(|raw| parse_numstat_z(&raw))
        .unwrap_or_default()
}

/// Counts lines of an untracked text file; binary / huge / unreadable files count as 0.
fn count_untracked_lines(cwd: &str, file: &str) -> usize {
    let full = Path::new(cwd).join(file);
    let Ok(meta) = std::fs::metadata(&full) else {
        return 0;
    };
    if !meta.is_file() || meta.len() > MAX_UNTRACKED_COUNT_BYTES {
        return 0;
    }
    let Ok(bytes) = std::fs::read(&full) else {
        return 0;
    };
    if bytes.contains(&0) {
        return 0;
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count();
    let trailing_partial = bytes.last().is_some_and(|b| *b != b'\n');
    newlines + usize::from(trailing_partial)
}

fn index_side_status(c: char) -> Option<GitFileStatus> {
    match c {
        ' ' | '?' | '!' => None,
        'A' | 'C' => Some(GitFileStatus::Added),
        'D' => Some(GitFileStatus::Deleted),
        'R' => Some(GitFileStatus::Renamed),
        _ => Some(GitFileStatus::Modified),
    }
}

fn worktree_side_status(c: char) -> Option<GitFileStatus> {
    match c {
        ' ' | '!' => None,
        '?' => Some(GitFileStatus::Untracked),
        'A' => Some(GitFileStatus::Added),
        'D' => Some(GitFileStatus::Deleted),
        'R' => Some(GitFileStatus::Renamed),
        _ => Some(GitFileStatus::Modified),
    }
}

/// Single status for the combined HEAD -> worktree view.
fn combined_status(entry: &StatusEntry) -> GitFileStatus {
    if entry.is_untracked() {
        return GitFileStatus::Untracked;
    }
    if entry.is_unmerged() {
        return GitFileStatus::Modified;
    }
    let (x, y) = (entry.index, entry.worktree);
    if x == 'R' || y == 'R' {
        GitFileStatus::Renamed
    } else if x == 'D' || y == 'D' {
        GitFileStatus::Deleted
    } else if matches!(x, 'A' | 'C') {
        GitFileStatus::Added
    } else {
        GitFileStatus::Modified
    }
}

fn make_change(path: &str, status: GitFileStatus, cwd: &str, stats: &NumstatMap) -> GitFileChange {
    let (additions, deletions) = if status == GitFileStatus::Untracked {
        (count_untracked_lines(cwd, path), 0)
    } else {
        stats.get(path).copied().unwrap_or((0, 0))
    };
    GitFileChange {
        path: path.to_string(),
        status,
        additions,
        deletions,
    }
}

// ---------------------------------------------------------------------------
// Read-only queries
// ---------------------------------------------------------------------------

/// Modified, added or untracked files; refreshes read them through
/// [`read_local_state`].
#[cfg(test)]
pub fn get_workspace_changes(cwd: &str) -> Vec<GitFileChange> {
    StatusPass::read(cwd).map_or_else(Vec::new, |pass| pass.changes(cwd))
}

/// MonoCode `git_diff_stats_for`: uncommitted lines against `HEAD` (index
/// plus work tree when there is no commit yet), untracked text files
/// counting as additions. `None` outside a repository.
pub fn diff_stats(cwd: &str) -> Option<(usize, usize)> {
    let untracked = run_git(
        cwd,
        &["ls-files", "-o", "--exclude-standard", "-z", "--", "."],
    )
    .ok()?;
    let numstat = |extra: &[&str]| {
        let args: Vec<&str> = ["diff", "--no-ext-diff", "--numstat", "-z"]
            .into_iter()
            .chain(extra.iter().copied())
            .chain(["--", "."])
            .collect();
        run_git(cwd, &args).map(|raw| parse_numstat_z(&raw))
    };
    let mut files = numstat(&["HEAD"]).unwrap_or_else(|_| {
        let mut files = numstat(&[]).unwrap_or_default();
        for (path, (adds, dels)) in numstat(&["--cached"]).unwrap_or_default() {
            let entry = files.entry(path).or_default();
            *entry = (entry.0 + adds, entry.1 + dels);
        }
        files
    });
    for path in String::from_utf8_lossy(&untracked)
        .split('\0')
        .filter(|p| !p.is_empty())
    {
        if !files.contains_key(path) {
            files.insert(path.to_string(), (count_untracked_lines(cwd, path), 0));
        }
    }
    Some(
        files
            .values()
            .fold((0, 0), |(add, del), (a, d)| (add + a, del + d)),
    )
}

fn current_branch(cwd: &str) -> String {
    if let Ok(name) = run_git_string(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        let name = name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    // Detached HEAD: show the short commit hash rather than a fake branch name.
    run_git_string(cwd, &["rev-parse", "--short", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BRANCH.to_string())
}

pub fn get_detailed_status(cwd: &str) -> GitDetailedStatus {
    if !Path::new(cwd).exists() {
        return GitDetailedStatus::default();
    }
    match StatusPass::read(cwd) {
        Some(pass) => pass.detailed_status(cwd),
        // Not a repository: the fallback branch name, nothing else.
        None => GitDetailedStatus {
            branch: current_branch(cwd),
            ..Default::default()
        },
    }
}

/// Parses a single-file unified diff into display lines.
fn parse_unified_diff(text: &str) -> Vec<DiffLineKind> {
    let mut lines = Vec::new();
    let mut in_hunk = false;
    for line in text.lines() {
        if line.starts_with("diff --git ") {
            in_hunk = false;
        } else if line.starts_with("@@") {
            in_hunk = true;
            lines.push(DiffLineKind::Header(line.to_string()));
        } else if !in_hunk {
            continue;
        } else if let Some(rest) = line.strip_prefix('+') {
            // Inside a hunk, "+++"/"---" are real content lines, not file headers.
            lines.push(DiffLineKind::Addition(rest.to_string()));
        } else if let Some(rest) = line.strip_prefix('-') {
            lines.push(DiffLineKind::Deletion(rest.to_string()));
        } else if let Some(rest) = line.strip_prefix(' ') {
            lines.push(DiffLineKind::Context(rest.to_string()));
        } else if line.is_empty() {
            lines.push(DiffLineKind::Context(String::new()));
        }
    }
    lines
}

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

pub fn stage_file(cwd: &str, file: &str) -> Result<()> {
    validate_relative_path(file)?;
    run_git(cwd, &["add", "--", file]).map(|_| ())
}

/// Returns `file` plus the original path when `file` is the target of a staged rename,
/// so unstaging a rename restores both sides.
fn paths_with_rename_origin(cwd: &str, file: &str) -> Vec<String> {
    let origin = read_status(cwd).ok().and_then(|entries| {
        entries
            .into_iter()
            .find(|e| e.path == file && e.index == 'R')
            .and_then(|e| e.orig_path)
    });
    std::iter::once(file.to_string()).chain(origin).collect()
}

pub fn unstage_file(cwd: &str, file: &str) -> Result<()> {
    validate_relative_path(file)?;
    let paths = paths_with_rename_origin(cwd, file);
    let head_args = ["reset", "-q", "HEAD", "--"];
    let unborn_args = ["rm", "--cached", "-r", "-q", "--ignore-unmatch", "--"];
    let prefix: &[&str] = if has_head(cwd) {
        &head_args
    } else {
        &unborn_args
    };
    let args: Vec<&str> = prefix
        .iter()
        .copied()
        .chain(paths.iter().map(String::as_str))
        .collect();
    run_git(cwd, &args).map(|_| ())
}

fn is_reported_untracked(cwd: &str, file: &str) -> Result<bool> {
    let entries = read_status(cwd)?;
    let trimmed = file.trim_end_matches('/');
    Ok(entries
        .iter()
        .any(|e| e.is_untracked() && (e.path == file || e.path.trim_end_matches('/') == trimmed)))
}

/// Discards unstaged changes to `file`.
///
/// - Tracked (in the index): the worktree copy is restored from the index; staged
///   changes are preserved (this is invoked from the "unstaged" list).
/// - Untracked: deleted, but only after both `ls-files --error-unmatch` and
///   `status --porcelain` confirm git does not track it.
/// - Anything else (ignored, missing): error, nothing is deleted.
pub fn discard_file(cwd: &str, file: &str) -> Result<()> {
    validate_relative_path(file)?;

    if is_in_index(cwd, file) {
        return run_git(cwd, &["restore", "--worktree", "--", file]).map(|_| ());
    }

    if !is_reported_untracked(cwd, file)? {
        bail!("refusing to discard {file}: it is neither tracked nor reported as untracked");
    }

    let full = Path::new(cwd).join(file);
    let meta = std::fs::symlink_metadata(&full)
        .with_context(|| format!("cannot stat untracked path {}", full.display()))?;
    let removal = if meta.is_dir() {
        std::fs::remove_dir_all(&full)
    } else {
        std::fs::remove_file(&full)
    };
    removal.with_context(|| format!("failed to delete untracked path {}", full.display()))
}

pub fn stage_all(cwd: &str) -> Result<()> {
    run_git(cwd, &["add", "-A"]).map(|_| ())
}

pub fn unstage_all(cwd: &str) -> Result<()> {
    if has_head(cwd) {
        run_git(cwd, &["reset", "-q"]).map(|_| ())
    } else {
        run_git(
            cwd,
            &["rm", "--cached", "-r", "-q", "--ignore-unmatch", "--", "."],
        )
        .map(|_| ())
    }
}

/// Discards all unstaged changes under `cwd`: restores tracked files from the index and
/// removes untracked (non-ignored) files. Staged changes are preserved.
pub fn discard_all(cwd: &str) -> Result<()> {
    let tracked = run_git(cwd, &["ls-files", "-z", "--", "."])?;
    if !tracked.is_empty() {
        run_git(cwd, &["restore", "--worktree", "--", "."])?;
    }
    run_git(cwd, &["clean", "-f", "-d", "-q", "--", "."]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static REPO_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Temporary git repository that is deleted on drop.
    struct TempRepo {
        root: PathBuf,
    }

    impl TempRepo {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let id = REPO_COUNTER.fetch_add(1, Ordering::SeqCst);
            let root = std::env::temp_dir().join(format!(
                "bencode-git-test-{}-{nanos}-{id}",
                std::process::id()
            ));
            std::fs::create_dir_all(&root).expect("create temp repo dir");
            let repo = Self { root };
            repo.git(&["init", "-q", "-b", "main"]);
            repo.git(&["config", "user.name", "BenCode Test"]);
            repo.git(&["config", "user.email", "test@example.invalid"]);
            repo.git(&["config", "commit.gpgsign", "false"]);
            repo.git(&["config", "core.hooksPath", ".git/no-hooks"]);
            repo
        }

        fn cwd(&self) -> &str {
            self.root.to_str().expect("utf-8 temp path")
        }

        fn git(&self, args: &[&str]) -> String {
            run_git_string(self.cwd(), args).expect("test git command")
        }

        fn write(&self, rel: &str, content: &str) {
            let path = self.root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent dirs");
            }
            std::fs::write(path, content).expect("write file");
        }

        fn read(&self, rel: &str) -> Option<String> {
            std::fs::read_to_string(self.root.join(rel)).ok()
        }

        fn exists(&self, rel: &str) -> bool {
            self.root.join(rel).exists()
        }

        fn commit_all(&self, message: &str) {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "-m", message]);
        }

        fn staged_blob(&self, rel: &str) -> String {
            self.git(&["show", &format!(":{rel}")])
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn find<'a>(changes: &'a [GitFileChange], path: &str) -> &'a GitFileChange {
        changes
            .iter()
            .find(|c| c.path == path)
            .unwrap_or_else(|| panic!("no change for {path} in {changes:?}"))
    }

    #[test]
    fn auto_fetch_reports_new_remote_commits_as_behind() {
        let remote = TempRepo::new();
        remote.write("a.txt", "one\n");
        remote.commit_all("first");
        let local = TempRepo::new();
        local.git(&["remote", "add", "origin", remote.cwd()]);
        local.git(&["fetch", "-q", "origin"]);
        local.git(&["reset", "-q", "--hard", "origin/main"]);
        local.git(&["branch", "-q", "--set-upstream-to=origin/main"]);
        assert_eq!(sync::auto_fetch(local.cwd()), Ok(false));

        remote.write("a.txt", "two\n");
        remote.commit_all("second");
        assert_eq!(sync::sync_info(local.cwd()).behind, 0);
        assert_eq!(sync::auto_fetch(local.cwd()), Ok(true));
        assert_eq!(sync::sync_info(local.cwd()).behind, 1);
    }

    // --- pure parsers -----------------------------------------------------

    #[test]
    fn parses_porcelain_z_renames_and_special_paths() {
        let raw = b"R  new name.txt\0old -> name.txt\0 M caf\xc3\xa9.rs\0?? dir/a b\0";
        let entries = parse_porcelain_z(raw);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path, "new name.txt");
        assert_eq!(entries[0].orig_path.as_deref(), Some("old -> name.txt"));
        assert_eq!(entries[1].path, "café.rs");
        assert_eq!(entries[1].worktree, 'M');
        assert!(entries[2].is_untracked());
        assert_eq!(entries[2].path, "dir/a b");
    }

    #[test]
    fn parses_numstat_z_with_renames_and_binary() {
        let raw = b"3\t1\tsrc/a.rs\0-\t-\timg.png\0" as &[u8];
        let rename = b"0\t2\t\0old.rs\0new.rs\0" as &[u8];
        let map = parse_numstat_z(&[raw, rename].concat());
        assert_eq!(map.get("src/a.rs"), Some(&(3, 1)));
        assert_eq!(map.get("img.png"), Some(&(0, 0)));
        assert_eq!(map.get("new.rs"), Some(&(0, 2)));
        assert!(!map.contains_key("old.rs"));
    }

    #[test]
    fn unified_diff_keeps_content_lines_that_look_like_headers() {
        let diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n---old\n+++new\n ctx\n";
        let lines = parse_unified_diff(diff);
        assert!(matches!(&lines[0], DiffLineKind::Header(_)));
        assert!(matches!(&lines[1], DiffLineKind::Deletion(s) if s == "--old"));
        assert!(matches!(&lines[2], DiffLineKind::Addition(s) if s == "++new"));
        assert!(matches!(&lines[3], DiffLineKind::Context(s) if s == "ctx"));
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn rejects_paths_escaping_repo() {
        assert!(validate_relative_path("../etc/passwd").is_err());
        assert!(validate_relative_path("/etc/passwd").is_err());
        assert!(validate_relative_path("").is_err());
        assert!(validate_relative_path("src/ok.rs").is_ok());
    }

    // --- discard_file -------------------------------------------------------

    #[test]
    fn discard_tracked_file_restores_content_without_deleting() {
        let repo = TempRepo::new();
        repo.write("a.txt", "original\n");
        repo.commit_all("init");
        repo.write("a.txt", "changed\n");

        discard_file(repo.cwd(), "a.txt").expect("discard tracked");

        assert_eq!(repo.read("a.txt").as_deref(), Some("original\n"));
    }

    #[test]
    fn discard_tracked_file_preserves_staged_changes() {
        let repo = TempRepo::new();
        repo.write("a.txt", "v1\n");
        repo.commit_all("init");
        repo.write("a.txt", "v2 staged\n");
        repo.git(&["add", "a.txt"]);
        repo.write("a.txt", "v3 unstaged\n");

        discard_file(repo.cwd(), "a.txt").expect("discard");

        assert_eq!(repo.read("a.txt").as_deref(), Some("v2 staged\n"));
        assert_eq!(repo.staged_blob("a.txt"), "v2 staged\n");
    }

    #[test]
    fn discard_restores_deleted_tracked_file() {
        let repo = TempRepo::new();
        repo.write("gone.txt", "keep me\n");
        repo.commit_all("init");
        std::fs::remove_file(repo.root.join("gone.txt")).expect("rm");

        discard_file(repo.cwd(), "gone.txt").expect("discard deletion");

        assert_eq!(repo.read("gone.txt").as_deref(), Some("keep me\n"));
    }

    #[test]
    fn discard_untracked_file_deletes_it() {
        let repo = TempRepo::new();
        repo.write("keep.txt", "x\n");
        repo.commit_all("init");
        repo.write("new file.txt", "scratch\n");

        discard_file(repo.cwd(), "new file.txt").expect("discard untracked");

        assert!(!repo.exists("new file.txt"));
        assert!(repo.exists("keep.txt"));
    }

    #[test]
    fn discard_refuses_ignored_and_missing_files() {
        let repo = TempRepo::new();
        repo.write(".gitignore", "secret.env\n");
        repo.commit_all("init");
        repo.write("secret.env", "TOKEN=1\n");

        assert!(discard_file(repo.cwd(), "secret.env").is_err());
        assert!(repo.exists("secret.env"));
        assert!(discard_file(repo.cwd(), "does-not-exist.txt").is_err());
        assert!(discard_file(repo.cwd(), "../outside.txt").is_err());
    }

    #[test]
    fn discard_works_in_repo_without_commits() {
        let repo = TempRepo::new();
        repo.write("staged.txt", "staged\n");
        repo.git(&["add", "staged.txt"]);
        repo.write("staged.txt", "edited\n");
        repo.write("untracked.txt", "u\n");

        discard_file(repo.cwd(), "staged.txt").expect("discard staged-new");
        discard_file(repo.cwd(), "untracked.txt").expect("discard untracked");

        assert_eq!(repo.read("staged.txt").as_deref(), Some("staged\n"));
        assert!(!repo.exists("untracked.txt"));
    }

    // --- status -------------------------------------------------------------

    #[test]
    fn workspace_changes_handle_renames_and_special_paths() {
        let repo = TempRepo::new();
        repo.write("old -> name.txt", "a\nb\nc\n");
        repo.write("mod.txt", "1\n");
        repo.commit_all("init");
        repo.git(&["mv", "old -> name.txt", "new name.txt"]);
        repo.write("mod.txt", "1\n2\n");
        repo.write("café \"q\".txt", "x\ny\n");

        let changes = get_workspace_changes(repo.cwd());

        assert_eq!(
            find(&changes, "new name.txt").status,
            GitFileStatus::Renamed
        );
        let modified = find(&changes, "mod.txt");
        assert_eq!(modified.status, GitFileStatus::Modified);
        assert_eq!((modified.additions, modified.deletions), (1, 0));
        let untracked = find(&changes, "café \"q\".txt");
        assert_eq!(untracked.status, GitFileStatus::Untracked);
        assert_eq!(untracked.additions, 2);
        assert_eq!(changes.len(), 3);
    }

    #[test]
    fn diff_stats_count_tracked_and_untracked_lines() {
        let repo = TempRepo::new();
        repo.write("a.txt", "1\n2\n");
        repo.commit_all("init");
        repo.write("a.txt", "1\nchanged\n3\n");
        repo.write("new.txt", "x\ny\nz\n");

        // a.txt: +2 -1; new.txt: +3.
        assert_eq!(diff_stats(repo.cwd()), Some((5, 1)));
    }

    #[test]
    fn untracked_files_over_a_mebibyte_count_no_lines() {
        let repo = TempRepo::new();
        repo.write("a.txt", "1\n");
        repo.commit_all("init");
        repo.write("big.xml", &"line\n".repeat(300_000));
        repo.write("small.txt", "x\ny\n");
        assert_eq!(diff_stats(repo.cwd()), Some((2, 0)));
    }

    #[test]
    fn diff_stats_without_a_commit_or_a_repository() {
        let repo = TempRepo::new();
        repo.write("staged.txt", "1\n");
        repo.git(&["add", "staged.txt"]);
        repo.write("loose.txt", "1\n2\n");
        assert_eq!(diff_stats(repo.cwd()), Some((3, 0)));

        let plain = std::env::temp_dir().join(format!("bencode-no-repo-{}", std::process::id()));
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(diff_stats(plain.to_str().unwrap()), None);
        std::fs::remove_dir_all(&plain).unwrap();
    }

    #[test]
    fn untracked_binary_file_counts_zero_lines() {
        let repo = TempRepo::new();
        std::fs::write(repo.root.join("blob.bin"), [0u8, 1, 2, b'\n', 0]).expect("write");
        repo.write("no-newline.txt", "a\nb");

        let changes = get_workspace_changes(repo.cwd());

        assert_eq!(find(&changes, "blob.bin").additions, 0);
        assert_eq!(find(&changes, "no-newline.txt").additions, 2);
    }

    #[test]
    fn detailed_status_splits_staged_and_unstaged_counts() {
        let repo = TempRepo::new();
        repo.write("a.txt", "1\n");
        repo.write("r.txt", "rename me\n");
        repo.commit_all("init");
        repo.write("a.txt", "1\n2\n");
        repo.git(&["add", "a.txt"]);
        repo.write("a.txt", "1\n2\n3\n4\n");
        repo.git(&["mv", "r.txt", "r2.txt"]);
        repo.write("u.txt", "u\n");

        let status = get_detailed_status(repo.cwd());

        assert_eq!(status.branch, "main");
        let staged_a = find(&status.staged, "a.txt");
        assert_eq!((staged_a.additions, staged_a.deletions), (1, 0));
        let unstaged_a = find(&status.unstaged, "a.txt");
        assert_eq!((unstaged_a.additions, unstaged_a.deletions), (2, 0));
        assert_eq!(
            find(&status.staged, "r2.txt").status,
            GitFileStatus::Renamed
        );
        assert_eq!(
            find(&status.unstaged, "u.txt").status,
            GitFileStatus::Untracked
        );
        assert_eq!(status.staged.len(), 2);
        assert_eq!(status.unstaged.len(), 2);
    }

    #[test]
    fn detailed_status_in_empty_repo() {
        let repo = TempRepo::new();
        repo.write("first.txt", "a\nb\n");
        repo.git(&["add", "first.txt"]);

        let status = get_detailed_status(repo.cwd());

        assert_eq!(status.branch, "main");
        let first = find(&status.staged, "first.txt");
        assert_eq!(first.status, GitFileStatus::Added);
        assert_eq!(first.additions, 2);
        assert!(
            list_branches(repo.cwd()).is_empty(),
            "unborn branch has no ref yet"
        );
        let diff = file_diff(repo.cwd(), "first.txt", &DiffSource::Staged).unwrap();
        assert_eq!(
            diff.lines
                .iter()
                .filter(|l| matches!(l, DiffLineKind::Addition(_)))
                .count(),
            2
        );
    }

    #[test]
    fn file_diff_for_untracked_and_unchanged_files() {
        let repo = TempRepo::new();
        repo.write("same.txt", "same\n");
        repo.commit_all("init");
        repo.write("fresh.txt", "one\ntwo\n");

        let same = file_diff(repo.cwd(), "same.txt", &DiffSource::Unstaged).unwrap();
        assert!(same.lines.is_empty());
        let fresh = file_diff(repo.cwd(), "fresh.txt", &DiffSource::Unstaged).unwrap();
        assert_eq!(fresh.lines.len(), 3);
        repo.write("same.txt", "same\nmore\n");
        let edited = file_diff(repo.cwd(), "same.txt", &DiffSource::Unstaged).unwrap();
        // Full context: the unchanged first line is kept.
        assert_eq!(
            edited.lines[1..],
            [
                DiffLineKind::Context("same".into()),
                DiffLineKind::Addition("more".into())
            ]
        );
    }

    // --- mutations ----------------------------------------------------------

    #[test]
    fn mutations_report_git_errors_with_stderr() {
        let repo = TempRepo::new();
        repo.write("a.txt", "1\n");
        repo.commit_all("init");

        let err = stage_file(repo.cwd(), "missing.txt").expect_err("missing path");
        assert!(err.to_string().contains("missing.txt"), "{err}");
        assert!(stage_all("/definitely/not/a/repo/bencode").is_err());
    }

    #[test]
    fn stage_unstage_and_commit_round_trip() {
        let repo = TempRepo::new();
        repo.write("a.txt", "1\n");
        repo.commit_all("init");
        repo.write("a.txt", "2\n");
        repo.write("b.txt", "b\n");

        stage_file(repo.cwd(), "a.txt").expect("stage");
        assert_eq!(get_detailed_status(repo.cwd()).staged.len(), 1);
        unstage_file(repo.cwd(), "a.txt").expect("unstage");
        assert!(get_detailed_status(repo.cwd()).staged.is_empty());
        stage_all(repo.cwd()).expect("stage all");
        assert_eq!(get_detailed_status(repo.cwd()).staged.len(), 2);
        unstage_all(repo.cwd()).expect("unstage all");
        assert!(get_detailed_status(repo.cwd()).staged.is_empty());
        stage_all(repo.cwd()).expect("stage all again");
        sync::commit(repo.cwd(), "second", false).expect("commit");

        assert!(get_workspace_changes(repo.cwd()).is_empty());
        let subject = run_git_string(repo.cwd(), &["log", "-1", "--format=%s"]).expect("log");
        assert_eq!(subject.trim(), "second");
    }

    #[test]
    fn unstage_rename_restores_both_sides() {
        let repo = TempRepo::new();
        repo.write("old.txt", "content\n");
        repo.commit_all("init");
        repo.git(&["mv", "old.txt", "new.txt"]);

        unstage_file(repo.cwd(), "new.txt").expect("unstage rename");

        assert!(get_detailed_status(repo.cwd()).staged.is_empty());
    }

    #[test]
    fn unstage_works_without_head() {
        let repo = TempRepo::new();
        repo.write("a.txt", "a\n");
        repo.write("b.txt", "b\n");
        repo.git(&["add", "-A"]);

        unstage_file(repo.cwd(), "a.txt").expect("unstage one");
        let status = get_detailed_status(repo.cwd());
        assert_eq!(status.staged.len(), 1);
        assert_eq!(
            find(&status.unstaged, "a.txt").status,
            GitFileStatus::Untracked
        );

        unstage_all(repo.cwd()).expect("unstage all");
        assert!(get_detailed_status(repo.cwd()).staged.is_empty());
        assert!(repo.exists("a.txt") && repo.exists("b.txt"));
    }

    #[test]
    fn discard_all_keeps_staged_and_ignored_files() {
        let repo = TempRepo::new();
        repo.write(".gitignore", "*.log\n");
        repo.write("a.txt", "1\n");
        repo.write("s.txt", "s1\n");
        repo.commit_all("init");
        repo.write("s.txt", "s2\n");
        repo.git(&["add", "s.txt"]);
        repo.write("a.txt", "dirty\n");
        repo.write("tmp/new.txt", "n\n");
        repo.write("debug.log", "log\n");

        discard_all(repo.cwd()).expect("discard all");

        assert_eq!(repo.read("a.txt").as_deref(), Some("1\n"));
        assert_eq!(repo.read("s.txt").as_deref(), Some("s2\n"));
        assert!(!repo.exists("tmp/new.txt"));
        assert!(repo.exists("debug.log"));
    }

    #[test]
    fn discard_all_in_empty_repo_removes_untracked_only() {
        let repo = TempRepo::new();
        repo.write("u.txt", "u\n");

        discard_all(repo.cwd()).expect("discard all empty repo");

        assert!(!repo.exists("u.txt"));
    }

    #[test]
    fn staged_and_unstaged_sides_differ() {
        let repo = TempRepo::new();
        repo.write("a.txt", "one\n");
        repo.git(&["add", "a.txt"]);
        repo.git(&["commit", "-q", "-m", "init"]);
        repo.write("a.txt", "two\n");
        repo.git(&["add", "a.txt"]);
        repo.write("a.txt", "three\n");

        let added = |rows: Vec<DiffLineKind>| -> Vec<String> {
            rows.into_iter()
                .filter_map(|r| match r {
                    DiffLineKind::Addition(t) => Some(t),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            added(
                file_diff(repo.cwd(), "a.txt", &DiffSource::Staged)
                    .unwrap()
                    .lines
            ),
            ["two"]
        );
        assert_eq!(
            added(
                file_diff(repo.cwd(), "a.txt", &DiffSource::Unstaged)
                    .unwrap()
                    .lines
            ),
            ["three"]
        );
    }

    #[test]
    fn commit_files_and_diff_describe_a_past_commit() {
        let repo = TempRepo::new();
        repo.write("a.txt", "one\n");
        repo.git(&["add", "a.txt"]);
        repo.git(&["commit", "-q", "-m", "init"]);
        let sha = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
        repo.write("a.txt", "dirty\n");

        let files = commit_files(repo.cwd(), &sha).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "a.txt");
        assert_eq!(files[0].additions, 1);
        let rows = file_diff(repo.cwd(), "a.txt", &DiffSource::Commit(sha))
            .unwrap()
            .lines;
        assert!(rows.contains(&DiffLineKind::Addition("one".into())));
        assert!(!rows.contains(&DiffLineKind::Addition("dirty".into())));
    }

    #[test]
    fn switching_branches_creates_tracks_and_reports_blocking_changes() {
        let repo = TempRepo::new();
        repo.write("a.txt", "one\n");
        repo.git(&["add", "a.txt"]);
        repo.git(&["commit", "-q", "-m", "init"]);
        assert_eq!(create_branch(repo.cwd(), "feature").unwrap(), "feature");
        repo.write("a.txt", "feature\n");
        repo.git(&["commit", "-qam", "on feature"]);

        repo.write("a.txt", "dirty\n");
        let main = Branch {
            name: "main".into(),
            remote: false,
            current: false,
        };
        assert!(matches!(
            switch_branch(repo.cwd(), &main),
            Err(SwitchError::BlockedByChanges)
        ));

        stash_changes(repo.cwd(), "test").unwrap();
        assert_eq!(switch_branch(repo.cwd(), &main).unwrap(), "main");
        let names: Vec<_> = list_branches(repo.cwd())
            .into_iter()
            .map(|b| (b.name, b.current))
            .collect();
        assert!(names.contains(&("main".to_string(), true)));
        assert!(names.contains(&("feature".to_string(), false)));
        assert!(create_branch(repo.cwd(), "-bad").is_err());
    }

    #[test]
    fn undoing_the_last_commit_keeps_its_changes_staged() {
        let repo = TempRepo::new();
        repo.write("a.txt", "one\n");
        repo.commit_all("first");
        repo.write("a.txt", "two\n");
        repo.commit_all("second");
        sync::undo_last_commit(repo.cwd()).expect("undo");
        assert_eq!(repo.git(&["log", "--format=%s"]).trim(), "first");
        assert_eq!(repo.staged_blob("a.txt"), "two\n");
        // The first commit has nothing under it to step back to.
        assert!(sync::undo_last_commit(repo.cwd()).is_err());
    }

    #[test]
    fn reverting_a_commit_adds_one_that_takes_it_back() {
        let repo = TempRepo::new();
        repo.write("a.txt", "one\n");
        repo.commit_all("first");
        repo.write("b.txt", "new\n");
        repo.commit_all("add b");
        let sha = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
        assert_eq!(
            sync::commit_message(repo.cwd(), &sha).as_deref(),
            Ok("add b")
        );
        sync::revert_commit(repo.cwd(), &sha).expect("revert");
        assert!(!repo.exists("b.txt"));
        let log = repo.git(&["log", "--format=%s"]);
        assert_eq!(log.lines().next(), Some("Revert \"add b\""));
        assert!(sync::revert_commit(repo.cwd(), "--abort").is_err());
    }

    #[test]
    fn a_revert_that_conflicts_leaves_the_tree_as_it_was() {
        let repo = TempRepo::new();
        repo.write("a.txt", "one\n");
        repo.commit_all("first");
        repo.write("a.txt", "two\n");
        repo.commit_all("second");
        let second = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
        repo.write("a.txt", "three\n");
        repo.commit_all("third");
        assert!(sync::revert_commit(repo.cwd(), &second).is_err());
        assert_eq!(repo.read("a.txt").as_deref(), Some("three\n"));
        assert_eq!(repo.git(&["status", "--porcelain"]).trim(), "");
        assert!(
            run_git_string(repo.cwd(), &["rev-parse", "-q", "--verify", "REVERT_HEAD"]).is_err()
        );
    }
}
