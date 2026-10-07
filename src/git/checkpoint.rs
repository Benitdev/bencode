//! What a session changed, and Keep / Undo for it. Ported from MonoCode
//! (`src-tauri/src/checkpoint.rs`), manifest format included; the store is
//! BenCode's own (`storage::checkpoints_dir`).
//!
//! A session's manifest records, per file, the contents before its first
//! structured edit (`files`) and after its latest one (`after`). Review and
//! Undo use only those snapshots:
//! - a file is reviewed only when its pre-edit state was captured at a tool
//!   start (`prepared`); nothing is guessed from the shared working tree;
//! - Undo refuses a file that changed after the session's last edit, or
//!   that another session in the project also edited;
//! - session ids are `[A-Za-z0-9_-]+` and paths have no `..`, so neither
//!   can leave the store or the project.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::diffs::{FileDiff, patch};
use super::{GitFileChange, GitFileStatus, MAX_UNTRACKED_READ_BYTES, run_git};

/// MonoCode `MAX_SNAPSHOT_FILES`.
const MAX_SNAPSHOT_FILES: usize = 500;
/// MonoCode `MAX_TEXT_FILE_BYTES`: larger files are not snapshotted.
const MAX_TEXT_FILE_BYTES: u64 = MAX_UNTRACKED_READ_BYTES;

/// One file a session changed (MonoCode `CheckpointFile`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointFile {
    pub relative: String,
    /// `added`, `modified` or `deleted`.
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
    /// False when the file changed between this session's own edits, so its
    /// lines cannot be attributed exactly.
    pub exact: bool,
    /// False when restoring could overwrite a change made outside the session.
    pub undoable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckpointStatus {
    pub files: Vec<CheckpointFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    cwd: String,
    files: BTreeMap<String, SnapshotKind>,
    #[serde(default)]
    touched: BTreeSet<String>,
    #[serde(default)]
    tracked: BTreeSet<String>,
    /// Paths captured before a structured edit started. Only these are safe
    /// candidates for Undo.
    #[serde(default)]
    prepared: BTreeSet<String>,
    /// Worktree contents immediately after the session's latest edit.
    #[serde(default)]
    after: BTreeMap<String, SnapshotKind>,
    /// Stable line counts for the session-owned before/after pair.
    #[serde(default)]
    stats: BTreeMap<String, ChangeStats>,
    /// Paths whose contents changed between two edits by this session.
    #[serde(default)]
    diverged: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ChangeStats {
    status: String,
    additions: i64,
    deletions: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum SnapshotKind {
    Contents,
    Missing,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FileState {
    Contents(Vec<u8>),
    Missing,
    Skipped,
}

#[derive(Clone)]
pub struct CheckpointStore {
    root: PathBuf,
    gate: Arc<Mutex<()>>,
}

impl CheckpointStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            gate: Arc::new(Mutex::new(())),
        }
    }

    /// BenCode's own store, or a local folder when `$HOME` is unknown.
    pub fn default_dir() -> PathBuf {
        crate::storage::checkpoints_dir().unwrap_or_else(|| PathBuf::from(".bencode/checkpoints"))
    }

    /// One operation at a time, as MonoCode's `exclusive`.
    fn exclusive<T>(
        &self,
        session_id: &str,
        operation: impl FnOnce(&Self) -> Result<T, String>,
    ) -> Result<T, String> {
        validate_id(session_id)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| "Checkpoint store lock poisoned".to_string())?;
        operation(self)
    }

    fn session_dir(&self, session_id: &str) -> PathBuf {
        self.root.join(session_id)
    }

    /// Before a turn: snapshots the files that are already dirty, so they
    /// are not mistaken for the session's work.
    pub fn ensure(&self, session_id: &str, cwd: &str) -> Result<(), String> {
        self.exclusive(session_id, |store| {
            let root = project_root(cwd)?;
            let dir = store.session_dir(session_id);
            if let Some(manifest) = read_manifest(&dir)? {
                if same_cwd(&manifest.cwd, cwd) {
                    return Ok(());
                }
                if let Err(err) = std::fs::remove_dir_all(&dir) {
                    log::warn!("could not reset checkpoint {}: {err}", dir.display());
                }
            }
            std::fs::create_dir_all(dir.join("files")).map_err(|e| e.to_string())?;

            let mut files = BTreeMap::new();
            let mut tracked = BTreeSet::new();
            for file in dirty_files(&root) {
                if files.len() >= MAX_SNAPSHOT_FILES {
                    break;
                }
                let Ok(relative) = resolve_repo_path(&root, &file.path) else {
                    continue;
                };
                if in_head(&root, &relative) {
                    tracked.insert(relative.clone());
                }
                files.insert(relative.clone(), snapshot_file(&dir, &root, &relative)?);
            }
            write_manifest(
                &dir,
                &Manifest {
                    cwd: root.to_string_lossy().into_owned(),
                    files,
                    touched: BTreeSet::new(),
                    tracked,
                    prepared: BTreeSet::new(),
                    after: BTreeMap::new(),
                    stats: BTreeMap::new(),
                    diverged: BTreeSet::new(),
                },
            )
        })
    }

    /// A structured edit of `paths` is starting: keeps their contents.
    pub fn prepare(&self, session_id: &str, cwd: &str, paths: &[String]) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }
        if paths.len() > MAX_SNAPSHOT_FILES {
            return Err("Too many paths".into());
        }
        self.exclusive(session_id, |store| {
            let root = project_root(cwd)?;
            let dir = store.session_dir(session_id);
            let mut manifest = match read_manifest(&dir)? {
                Some(manifest) if same_cwd(&manifest.cwd, cwd) => manifest,
                _ => return Ok(()),
            };

            let mut dirty = false;
            for path in paths {
                let Ok(relative) = relative_to_root(&root, path) else {
                    continue;
                };
                // Keep the original pre-edit snapshot across later edits by
                // this session. The first tool-start event owns the safe
                // undo boundary.
                if manifest.touched.contains(&relative) && manifest.prepared.contains(&relative) {
                    if !after_matches_worktree(&dir, &root, &manifest, &relative)
                        && manifest.diverged.insert(relative)
                    {
                        dirty = true;
                    }
                    continue;
                }
                if manifest.prepared.contains(&relative) {
                    continue;
                }
                if manifest.touched.contains(&relative) {
                    // Upgrade a completion-only claim by dropping its
                    // untrusted state and starting at this tool boundary.
                    release_path(&mut manifest, &relative);
                    dirty = true;
                }
                let before = snapshot_file(&dir, &root, &relative)?;
                if manifest.files.insert(relative.clone(), before) != Some(before) {
                    dirty = true;
                }
                if manifest.prepared.insert(relative.clone()) {
                    dirty = true;
                }
                if in_head(&root, &relative) && manifest.tracked.insert(relative) {
                    dirty = true;
                }
            }
            if dirty {
                write_manifest(&dir, &manifest)?;
            }
            Ok(())
        })
    }

    /// A structured edit of `paths` finished: keeps what they hold now.
    pub fn capture(&self, session_id: &str, cwd: &str, paths: &[String]) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }
        if paths.len() > MAX_SNAPSHOT_FILES {
            return Err("Too many paths".into());
        }
        self.exclusive(session_id, |store| {
            let root = project_root(cwd)?;
            let dir = store.session_dir(session_id);
            let mut manifest = match read_manifest(&dir)? {
                Some(manifest) if same_cwd(&manifest.cwd, cwd) => manifest,
                _ => return Ok(()),
            };

            let mut dirty = false;
            for path in paths {
                if manifest.touched.len() >= MAX_SNAPSHOT_FILES {
                    break;
                }
                let Ok(relative) = relative_to_root(&root, path) else {
                    continue;
                };
                manifest.touched.insert(relative.clone());
                let tracked_in_head = in_head(&root, &relative);
                if tracked_in_head {
                    manifest.tracked.insert(relative.clone());
                }
                if !manifest.files.contains_key(&relative)
                    && !root.join(&relative).exists()
                    && !tracked_in_head
                {
                    // A completion without a matching prepare event is kept
                    // for review but is deliberately not undoable.
                    manifest
                        .files
                        .insert(relative.clone(), snapshot_file(&dir, &root, &relative)?);
                }
                let after = snapshot_after_file(&dir, &root, &relative)?;
                manifest.after.insert(relative.clone(), after);
                if let Some(stats) = calculate_session_stats(&dir, &manifest, &relative) {
                    manifest.stats.insert(relative, stats);
                }
                dirty = true;
            }
            if dirty {
                write_manifest(&dir, &manifest)?;
            }
            Ok(())
        })
    }

    /// The files this session changed and that still differ.
    pub fn status(&self, session_id: &str, cwd: &str) -> Result<CheckpointStatus, String> {
        self.exclusive(session_id, |store| store.status_locked(session_id, cwd))
    }

    fn status_locked(&self, session_id: &str, cwd: &str) -> Result<CheckpointStatus, String> {
        let Some(manifest) = self.load_matching(session_id, cwd)? else {
            return Ok(CheckpointStatus::default());
        };
        let root = project_root(cwd)?;
        let foreign_touched = self.foreign_touched_paths(cwd, session_id);
        Ok(diff_from_manifest(
            &self.session_dir(session_id),
            &root,
            &manifest,
            &foreign_touched,
        ))
    }

    /// Drops everything recorded for a session (its thread was deleted).
    pub fn forget(&self, session_id: &str) -> Result<(), String> {
        self.exclusive(session_id, |store| {
            let dir = store.session_dir(session_id);
            if dir.exists() {
                std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
            }
            Ok(())
        })
    }

    /// The session's before → after change of one file, with full context
    /// (MonoCode `session_checkpoint_file_diff`, diffed here instead of in
    /// the view).
    pub fn file_diff(&self, session_id: &str, cwd: &str, relative: &str) -> Result<FileDiff, String> {
        self.exclusive(session_id, |store| {
            let Some(manifest) = store.load_matching(session_id, cwd)? else {
                return Err("Session changes are no longer available".into());
            };
            let root = project_root(cwd)?;
            let relative = resolve_repo_path(&root, relative)?;
            if !manifest.touched.contains(&relative) || !manifest.prepared.contains(&relative) {
                return Err("This file was not changed by the session".into());
            }
            if manifest.diverged.contains(&relative) {
                return Err(
                    "Exact lines are unavailable because the file changed between this session's edits"
                        .into(),
                );
            }
            let dir = store.session_dir(session_id);
            let before = manifest
                .files
                .get(&relative)
                .copied()
                .ok_or_else(|| "Session baseline is unavailable".to_string())?;
            let after = manifest
                .after
                .get(&relative)
                .copied()
                .ok_or_else(|| "Session result is unavailable".to_string())?;
            let original = read_snapshot(&dir, &relative, before);
            let current = read_after_snapshot(&dir, &relative, after);
            if matches!(original, FileState::Skipped) || matches!(current, FileState::Skipped) {
                return Ok(FileDiff {
                    too_large: true,
                    ..FileDiff::default()
                });
            }
            if state_is_binary(&original) || state_is_binary(&current) {
                return Ok(FileDiff {
                    binary: true,
                    ..FileDiff::default()
                });
            }
            let before_path = state_blob_path(&dir.join("files"), &relative)?;
            let after_path = state_blob_path(&dir.join("after"), &relative)?;
            let text = diff_blobs(&before_path, &after_path, &["-U2147483647"])
                .ok_or_else(|| "Could not diff the session's snapshots".to_string())?;
            Ok(patch(&text))
        })
    }

    /// Restores one file, or every file, to what it held before the session.
    pub fn undo(
        &self,
        session_id: &str,
        cwd: &str,
        relative: Option<&str>,
    ) -> Result<CheckpointStatus, String> {
        self.exclusive(session_id, |store| {
            let Some(mut manifest) = store.load_matching(session_id, cwd)? else {
                return Ok(CheckpointStatus::default());
            };
            let root = project_root(cwd)?;
            let dir = store.session_dir(session_id);
            let foreign_touched = store.foreign_touched_paths(cwd, session_id);
            let changed = diff_from_manifest(&dir, &root, &manifest, &foreign_touched);
            if let Some(relative) = relative {
                let relative = resolve_repo_path(&root, relative)?;
                let Some(file) = changed.files.iter().find(|file| file.relative == relative) else {
                    return store.status_locked(session_id, cwd);
                };
                if !file.undoable {
                    return Err(format!(
                        "Cannot safely undo {relative}: it changed outside this session"
                    ));
                }
                restore_one(&dir, &root, &manifest, &relative)?;
                release_path(&mut manifest, &relative);
                write_manifest(&dir, &manifest)?;
                return store.status_locked(session_id, cwd);
            }
            if changed.files.iter().any(|file| !file.undoable) {
                return Err(
                    "Cannot safely undo all: one or more files changed outside this session".into(),
                );
            }
            for file in &changed.files {
                restore_one(&dir, &root, &manifest, &file.relative)?;
            }
            if let Err(err) = std::fs::remove_dir_all(&dir) {
                log::warn!("could not clear checkpoint {}: {err}", dir.display());
            }
            Ok(CheckpointStatus::default())
        })
    }

    /// Accepts one file's change, or all of them, and stops reviewing it.
    pub fn keep(
        &self,
        session_id: &str,
        cwd: &str,
        relative: Option<&str>,
    ) -> Result<CheckpointStatus, String> {
        self.exclusive(session_id, |store| {
            let Some(mut manifest) = store.load_matching(session_id, cwd)? else {
                return Ok(CheckpointStatus::default());
            };
            let root = project_root(cwd)?;
            let dir = store.session_dir(session_id);
            let Some(relative) = relative else {
                if let Err(err) = std::fs::remove_dir_all(&dir) {
                    log::warn!("could not clear checkpoint {}: {err}", dir.display());
                }
                return Ok(CheckpointStatus::default());
            };
            let relative = resolve_repo_path(&root, relative)?;
            release_path(&mut manifest, &relative);
            write_manifest(&dir, &manifest)?;
            store.status_locked(session_id, cwd)
        })
    }

    fn load_matching(&self, session_id: &str, cwd: &str) -> Result<Option<Manifest>, String> {
        let dir = self.session_dir(session_id);
        let Some(manifest) = read_manifest(&dir)? else {
            return Ok(None);
        };
        if !same_cwd(&manifest.cwd, cwd) {
            return Ok(None);
        }
        Ok(Some(manifest))
    }

    /// Paths already claimed by another session in the same project.
    fn foreign_touched_paths(&self, cwd: &str, except_session_id: &str) -> HashSet<String> {
        let mut paths = HashSet::new();
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return paths;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy() == except_session_id {
                continue;
            }
            let Ok(Some(manifest)) = read_manifest(&entry.path()) else {
                continue;
            };
            if !same_cwd(&manifest.cwd, cwd) {
                continue;
            }
            paths.extend(manifest.touched.intersection(&manifest.prepared).cloned());
        }
        paths
    }
}

/// Files that differ from HEAD, untracked ones included.
fn dirty_files(root: &Path) -> Vec<GitFileChange> {
    super::status_pass::read_changes(&root.to_string_lossy())
}

fn status_name(status: &GitFileStatus) -> &'static str {
    match status {
        GitFileStatus::Added | GitFileStatus::Untracked => "added",
        GitFileStatus::Deleted => "deleted",
        GitFileStatus::Modified | GitFileStatus::Renamed => "modified",
    }
}

fn git_checked(root: &Path, args: &[&str]) -> Result<(), String> {
    run_git(&root.to_string_lossy(), args)
        .map(|_| ())
        .map_err(|err| format!("{err:#}"))
}

fn diff_from_manifest(
    dir: &Path,
    root: &Path,
    manifest: &Manifest,
    foreign_touched: &HashSet<String>,
) -> CheckpointStatus {
    let index = dirty_files(root);
    let by_relative: BTreeMap<&str, &GitFileChange> =
        index.iter().map(|file| (file.path.as_str(), file)).collect();
    let git_dirty: HashSet<&str> = by_relative.keys().copied().collect();
    let mut files = Vec::new();

    for relative in &manifest.touched {
        // Without a tool-start snapshot there is no trustworthy session
        // boundary. Never guess from the shared working tree.
        if !manifest.prepared.contains(relative) {
            continue;
        }
        if session_snapshot_differs(dir, manifest, relative) == Some(false) {
            continue;
        }
        if !file_differs(dir, root, manifest, relative, &git_dirty) {
            continue;
        }
        // Review is scoped to this session's captured before/after
        // snapshots. A foreign claim can make restoring the file unsafe, but
        // it does not make this session's recorded diff or counts inexact.
        let exact = !manifest.diverged.contains(relative);
        let undoable = exact
            && !foreign_touched.contains(relative)
            && after_matches_worktree(dir, root, manifest, relative);
        files.push(describe_change(
            root,
            relative,
            by_relative.get(relative.as_str()).copied(),
            exact,
            undoable,
            manifest.stats.get(relative),
        ));
    }

    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    CheckpointStatus { files }
}

fn session_snapshot_differs(dir: &Path, manifest: &Manifest, relative: &str) -> Option<bool> {
    let before = manifest.files.get(relative).copied()?;
    let after = manifest.after.get(relative).copied()?;
    Some(read_snapshot(dir, relative, before) != read_after_snapshot(dir, relative, after))
}

fn file_differs(
    dir: &Path,
    root: &Path,
    manifest: &Manifest,
    relative: &str,
    git_dirty: &HashSet<&str>,
) -> bool {
    // Once a tracked path is clean against HEAD, its session change was
    // committed (or otherwise resolved) and no longer needs review.
    if !git_dirty.contains(relative)
        && (manifest.tracked.contains(relative) || in_head(root, relative))
    {
        return false;
    }
    match manifest.files.get(relative) {
        Some(SnapshotKind::Skipped) => false,
        Some(kind) => read_worktree(root, relative) != read_snapshot(dir, relative, *kind),
        None => {
            git_dirty.contains(relative)
                || (root.join(relative).is_file() && !in_head(root, relative))
        }
    }
}

fn describe_change(
    root: &Path,
    relative: &str,
    git: Option<&GitFileChange>,
    exact: bool,
    undoable: bool,
    session: Option<&ChangeStats>,
) -> CheckpointFile {
    let count = |n: i64| usize::try_from(n).unwrap_or(0);
    let (status, additions, deletions) = match (session, git) {
        (Some(stats), _) if exact => (
            stats.status.clone(),
            count(stats.additions),
            count(stats.deletions),
        ),
        (Some(stats), _) => (stats.status.clone(), 0, 0),
        (None, Some(file)) => (
            status_name(&file.status).to_string(),
            file.additions,
            file.deletions,
        ),
        (None, None) => {
            let status = if root.join(relative).exists() { "modified" } else { "deleted" };
            (status.to_string(), 0, 0)
        }
    };
    CheckpointFile {
        relative: relative.to_string(),
        status,
        additions,
        deletions,
        exact,
        undoable,
    }
}

fn calculate_session_stats(dir: &Path, manifest: &Manifest, relative: &str) -> Option<ChangeStats> {
    let before = manifest.files.get(relative).copied()?;
    let after = manifest.after.get(relative).copied()?;
    if before == SnapshotKind::Skipped || after == SnapshotKind::Skipped {
        return None;
    }
    let before_path = state_blob_path(&dir.join("files"), relative).ok()?;
    let after_path = state_blob_path(&dir.join("after"), relative).ok()?;
    let text = diff_blobs(&before_path, &after_path, &["--numstat"])?;
    let mut fields = text.lines().next().unwrap_or("0\t0").split('\t');
    let additions = fields.next()?.parse().unwrap_or(0);
    let deletions = fields.next()?.parse().unwrap_or(0);
    let status = match (before, after) {
        (SnapshotKind::Missing, SnapshotKind::Missing) => "modified",
        (SnapshotKind::Missing, _) => "added",
        (_, SnapshotKind::Missing) => "deleted",
        _ => "modified",
    };
    Some(ChangeStats {
        status: status.into(),
        additions,
        deletions,
    })
}

/// `git diff --no-index` of two stored blobs; exit status 1 means they differ.
fn diff_blobs(before: &Path, after: &Path, flags: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(["diff", "--no-index", "--no-ext-diff", "--no-color"])
        .args(flags)
        .arg("--")
        .arg(before)
        .arg(after)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    if !output.status.success() && output.status.code() != Some(1) {
        log::warn!(
            "git diff --no-index failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn after_matches_worktree(dir: &Path, root: &Path, manifest: &Manifest, relative: &str) -> bool {
    let Some(kind) = manifest.after.get(relative).copied() else {
        return false;
    };
    read_worktree(root, relative) == read_after_snapshot(dir, relative, kind)
}

fn release_path(manifest: &mut Manifest, relative: &str) {
    manifest.files.remove(relative);
    manifest.touched.remove(relative);
    manifest.tracked.remove(relative);
    manifest.prepared.remove(relative);
    manifest.after.remove(relative);
    manifest.stats.remove(relative);
    manifest.diverged.remove(relative);
}

fn restore_one(dir: &Path, root: &Path, manifest: &Manifest, relative: &str) -> Result<(), String> {
    let relative = resolve_repo_path(root, relative)?;
    if path_contains_symlink(root, &relative) {
        return Err(format!("Cannot restore through symbolic link {relative}"));
    }
    match manifest.files.get(&relative) {
        Some(SnapshotKind::Skipped) => Ok(()),
        Some(kind) => restore_snapshot(dir, root, &relative, *kind),
        None => revert_new_change(root, &relative),
    }
}

/// Unstages `relative`; a path git does not know is not an error.
fn unstage(root: &Path, relative: &str) {
    if let Err(err) = git_checked(root, &["reset", "-q", "HEAD", "--", relative]) {
        log::debug!("unstaging {relative}: {err}");
    }
}

fn restore_snapshot(
    dir: &Path,
    root: &Path,
    relative: &str,
    kind: SnapshotKind,
) -> Result<(), String> {
    match kind {
        SnapshotKind::Skipped => Ok(()),
        SnapshotKind::Missing => {
            unstage(root, relative);
            remove_worktree(root, relative)
        }
        SnapshotKind::Contents => {
            let FileState::Contents(bytes) = read_snapshot(dir, relative, kind) else {
                return Ok(());
            };
            let path = root.join(relative);
            write_worktree(&path, &bytes)?;
            set_file_mode(&path, snapshot_mode(&dir.join("files"), relative, kind))?;
            unstage(root, relative);
            Ok(())
        }
    }
}

fn revert_new_change(root: &Path, relative: &str) -> Result<(), String> {
    if in_head(root, relative) {
        return git_checked(
            root,
            &["restore", "--source=HEAD", "--staged", "--worktree", "--", relative],
        );
    }
    unstage(root, relative);
    remove_worktree(root, relative)
}

fn in_head(root: &Path, relative: &str) -> bool {
    git_checked(root, &["cat-file", "-e", &format!("HEAD:{relative}")]).is_ok()
}

fn path_contains_symlink(root: &Path, relative: &str) -> bool {
    let mut current = root.to_path_buf();
    for part in relative.split('/') {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
            Err(_) => return true,
        }
    }
    false
}

fn snapshot_file(dir: &Path, root: &Path, relative: &str) -> Result<SnapshotKind, String> {
    snapshot_file_at(&dir.join("files"), root, relative)
}

fn snapshot_after_file(dir: &Path, root: &Path, relative: &str) -> Result<SnapshotKind, String> {
    snapshot_file_at(&dir.join("after"), root, relative)
}

fn snapshot_file_at(blob_root: &Path, root: &Path, relative: &str) -> Result<SnapshotKind, String> {
    let abs = root.join(relative);
    let blob = state_blob_path(blob_root, relative)?;
    let write_blob = |bytes: &[u8]| -> Result<(), String> {
        if let Some(parent) = blob.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&blob, bytes).map_err(|e| e.to_string())
    };
    let meta = match std::fs::symlink_metadata(&abs) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write_blob(&[])?;
            return Ok(SnapshotKind::Missing);
        }
        Err(error) => return Err(error.to_string()),
    };
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > MAX_TEXT_FILE_BYTES {
        return Ok(SnapshotKind::Skipped);
    }
    let bytes = std::fs::read(&abs).map_err(|e| e.to_string())?;
    write_blob(&bytes)?;
    set_file_mode(&blob, file_mode(&abs))?;
    Ok(SnapshotKind::Contents)
}

#[cfg(unix)]
fn file_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|meta| meta.is_file() && !meta.file_type().is_symlink())
        .map(|meta| meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn file_mode(_path: &Path) -> Option<u32> {
    None
}

#[cfg(unix)]
fn set_file_mode(path: &Path, mode: Option<u32>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(mode) = mode {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_file_mode(_path: &Path, _mode: Option<u32>) -> Result<(), String> {
    Ok(())
}

fn snapshot_mode(blob_root: &Path, relative: &str, kind: SnapshotKind) -> Option<u32> {
    if kind != SnapshotKind::Contents {
        return None;
    }
    state_blob_path(blob_root, relative)
        .ok()
        .and_then(|path| file_mode(&path))
}

fn read_snapshot(dir: &Path, relative: &str, kind: SnapshotKind) -> FileState {
    read_snapshot_at(&dir.join("files"), relative, kind)
}

fn read_after_snapshot(dir: &Path, relative: &str, kind: SnapshotKind) -> FileState {
    read_snapshot_at(&dir.join("after"), relative, kind)
}

fn read_snapshot_at(blob_root: &Path, relative: &str, kind: SnapshotKind) -> FileState {
    match kind {
        SnapshotKind::Missing => FileState::Missing,
        SnapshotKind::Skipped => FileState::Skipped,
        SnapshotKind::Contents => match state_blob_path(blob_root, relative)
            .ok()
            .and_then(|path| std::fs::read(path).ok())
        {
            Some(bytes) => FileState::Contents(bytes),
            None => FileState::Missing,
        },
    }
}

fn read_worktree(root: &Path, relative: &str) -> FileState {
    let abs = root.join(relative);
    if !abs.exists() {
        return FileState::Missing;
    }
    if !abs.is_file() {
        return FileState::Skipped;
    }
    match std::fs::metadata(&abs).and_then(|meta| {
        if meta.len() > MAX_TEXT_FILE_BYTES {
            return Ok(FileState::Skipped);
        }
        std::fs::read(&abs).map(FileState::Contents)
    }) {
        Ok(state) => state,
        Err(_) => FileState::Missing,
    }
}

fn state_is_binary(state: &FileState) -> bool {
    matches!(state, FileState::Contents(bytes) if bytes.contains(&0))
}

fn write_worktree(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.is_dir() {
        return Err(format!("{} is a directory", path.display()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

fn remove_worktree(root: &Path, relative: &str) -> Result<(), String> {
    let abs = root.join(relative);
    if abs.is_file() || abs.is_symlink() {
        return std::fs::remove_file(&abs).map_err(|e| e.to_string());
    }
    if abs.is_dir() {
        return Err(format!("{relative} is a directory"));
    }
    Ok(())
}

fn state_blob_path(blob_root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty()
        || relative.starts_with('/')
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("Invalid path".into());
    }
    Ok(blob_root.join(relative))
}

fn read_manifest(dir: &Path) -> Result<Option<Manifest>, String> {
    let path = dir.join("manifest.json");
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let dest = dir.join("manifest.json");
    let tmp = dir.join("manifest.json.tmp");
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, dest).map_err(|e| e.to_string())
}

fn expand_home(path: &str) -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    if path == "~" {
        return home().unwrap_or_else(|| PathBuf::from(path));
    }
    match (path.strip_prefix("~/"), home()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

fn project_root(cwd: &str) -> Result<PathBuf, String> {
    let trimmed = cwd.trim();
    if trimmed.is_empty() || trimmed == "~" {
        return Err("cwd is required".into());
    }
    let root = expand_home(trimmed);
    if !root.is_dir() {
        return Err(format!("{}: Not a directory", root.display()));
    }
    Ok(root)
}

fn same_cwd(saved: &str, cwd: &str) -> bool {
    match (project_root(saved), project_root(cwd)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// A project-relative path with only normal components.
fn resolve_repo_path(root: &Path, relative: &str) -> Result<String, String> {
    let relative = relative.trim();
    if relative.is_empty()
        || relative.starts_with('/')
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("Invalid path".into());
    }
    if !root.join(relative).starts_with(root) {
        return Err("Invalid path".into());
    }
    Ok(relative.to_string())
}

fn relative_to_root(root: &Path, path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Invalid path".into());
    }
    let expanded = expand_home(trimmed);
    if expanded.is_absolute() {
        let relative = expanded
            .strip_prefix(root)
            .map_err(|_| "Path is outside the project".to_string())?;
        return resolve_repo_path(root, &relative.to_string_lossy());
    }
    resolve_repo_path(root, trimmed.trim_start_matches("./"))
}

fn validate_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("Invalid session id".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::DiffLineKind;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        base: PathBuf,
        repo: PathBuf,
        store: CheckpointStore,
    }

    impl Fixture {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!(
                "bencode-checkpoint-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            let repo = base.join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            let fixture = Self {
                store: CheckpointStore::new(base.join("store")),
                repo,
                base,
            };
            fixture.git(&["init", "-q", "-b", "main"]);
            fixture.git(&["config", "user.name", "BenCode Test"]);
            fixture.git(&["config", "user.email", "test@example.invalid"]);
            fixture.git(&["config", "commit.gpgsign", "false"]);
            fixture.write("a.txt", "one\ntwo\n");
            fixture.git(&["add", "."]);
            fixture.git(&["commit", "-q", "-m", "init"]);
            fixture
        }

        fn cwd(&self) -> &str {
            self.repo.to_str().unwrap()
        }

        fn git(&self, args: &[&str]) {
            run_git(self.cwd(), args).expect("test git command");
        }

        fn write(&self, relative: &str, text: &str) {
            std::fs::write(self.repo.join(relative), text).unwrap();
        }

        fn read(&self, relative: &str) -> String {
            std::fs::read_to_string(self.repo.join(relative)).unwrap()
        }

        /// One structured edit by `session`: prepare, write, capture.
        fn edit(&self, session: &str, relative: &str, text: &str) {
            let paths = [relative.to_string()];
            self.store.prepare(session, self.cwd(), &paths).unwrap();
            self.write(relative, text);
            self.store.capture(session, self.cwd(), &paths).unwrap();
        }

        fn files(&self, session: &str) -> Vec<CheckpointFile> {
            self.store.status(session, self.cwd()).unwrap().files
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    #[test]
    fn a_session_edit_is_reviewed_and_undone() {
        let fx = Fixture::new();
        fx.store.ensure("s1", fx.cwd()).unwrap();
        fx.edit("s1", "a.txt", "one\nTWO\nthree\n");
        fx.edit("s1", "new.txt", "hello\n");

        let files = fx.files("s1");
        assert_eq!(files.len(), 2);
        assert_eq!(
            (files[0].relative.as_str(), files[0].status.as_str(), files[0].additions, files[0].deletions),
            ("a.txt", "modified", 2, 1)
        );
        assert_eq!((files[1].status.as_str(), files[1].additions), ("added", 1));
        assert!(files.iter().all(|f| f.exact && f.undoable));

        let diff = fx.store.file_diff("s1", fx.cwd(), "a.txt").unwrap();
        assert!(diff.lines.contains(&DiffLineKind::Context("one".into())));
        assert!(diff.lines.contains(&DiffLineKind::Deletion("two".into())));
        assert!(diff.lines.contains(&DiffLineKind::Addition("three".into())));

        assert!(fx.store.undo("s1", fx.cwd(), None).unwrap().files.is_empty());
        assert_eq!(fx.read("a.txt"), "one\ntwo\n");
        assert!(!fx.repo.join("new.txt").exists());
    }

    #[test]
    fn earlier_dirty_work_is_not_the_sessions() {
        let fx = Fixture::new();
        fx.write("a.txt", "mine\n");
        fx.write("other.txt", "untouched\n");
        fx.store.ensure("s1", fx.cwd()).unwrap();
        assert!(fx.files("s1").is_empty());

        // The session edits the already dirty file; Undo returns to the
        // user's version, not HEAD.
        fx.edit("s1", "a.txt", "agent\n");
        assert_eq!(fx.files("s1").len(), 1);
        fx.store.undo("s1", fx.cwd(), Some("a.txt")).unwrap();
        assert_eq!(fx.read("a.txt"), "mine\n");
        assert_eq!(fx.read("other.txt"), "untouched\n");
    }

    #[test]
    fn a_later_outside_edit_blocks_undo() {
        let fx = Fixture::new();
        fx.store.ensure("s1", fx.cwd()).unwrap();
        fx.edit("s1", "a.txt", "agent\n");
        fx.write("a.txt", "agent\nand me\n");

        let files = fx.files("s1");
        assert!(files[0].exact && !files[0].undoable);
        assert!(fx.store.undo("s1", fx.cwd(), None).is_err());
        assert_eq!(fx.read("a.txt"), "agent\nand me\n");

        // Between two of the session's own edits it is no longer exact.
        fx.edit("s1", "a.txt", "agent again\n");
        let files = fx.files("s1");
        assert!(!files[0].exact && !files[0].undoable);
        assert!(fx.store.file_diff("s1", fx.cwd(), "a.txt").is_err());
    }

    #[test]
    fn another_sessions_claim_blocks_undo() {
        let fx = Fixture::new();
        fx.store.ensure("s1", fx.cwd()).unwrap();
        fx.store.ensure("s2", fx.cwd()).unwrap();
        fx.edit("s1", "a.txt", "first\n");
        fx.edit("s2", "a.txt", "second\n");
        assert!(!fx.files("s1")[0].undoable);
        assert!(!fx.files("s2")[0].undoable);
    }

    #[test]
    fn keep_and_commit_end_the_review() {
        let fx = Fixture::new();
        fx.store.ensure("s1", fx.cwd()).unwrap();
        fx.edit("s1", "a.txt", "agent\n");
        fx.edit("s1", "b.txt", "new\n");
        let left = fx.store.keep("s1", fx.cwd(), Some("b.txt")).unwrap();
        assert_eq!(left.files.len(), 1);
        assert_eq!(fx.read("b.txt"), "new\n");

        fx.git(&["commit", "-qam", "agent work"]);
        assert!(fx.files("s1").is_empty());

        fx.edit("s1", "a.txt", "more\n");
        assert_eq!(fx.files("s1").len(), 1);
        assert!(fx.store.keep("s1", fx.cwd(), None).unwrap().files.is_empty());
        assert_eq!(fx.read("a.txt"), "more\n");
    }

    #[test]
    fn a_change_without_a_tool_start_is_not_claimed() {
        let fx = Fixture::new();
        fx.store.ensure("s1", fx.cwd()).unwrap();
        fx.write("a.txt", "shell edit\n");
        fx.store.capture("s1", fx.cwd(), &["a.txt".to_string()]).unwrap();
        assert!(fx.files("s1").is_empty());
    }

    #[test]
    fn ids_and_paths_cannot_escape() {
        let fx = Fixture::new();
        assert!(fx.store.ensure("../x", fx.cwd()).is_err());
        fx.store.ensure("s1", fx.cwd()).unwrap();
        let outside = ["../outside.txt".to_string(), "/etc/hosts".to_string()];
        fx.store.prepare("s1", fx.cwd(), &outside).unwrap();
        fx.store.capture("s1", fx.cwd(), &outside).unwrap();
        assert!(fx.files("s1").is_empty());
        assert!(resolve_repo_path(&fx.repo, "a/../b").is_err());
        assert_eq!(
            relative_to_root(&fx.repo, &format!("{}/src/a.rs", fx.cwd())).unwrap(),
            "src/a.rs"
        );
    }

    #[test]
    fn a_deleted_file_comes_back() {
        let fx = Fixture::new();
        fx.store.ensure("s1", fx.cwd()).unwrap();
        let paths = ["a.txt".to_string()];
        fx.store.prepare("s1", fx.cwd(), &paths).unwrap();
        std::fs::remove_file(fx.repo.join("a.txt")).unwrap();
        fx.store.capture("s1", fx.cwd(), &paths).unwrap();
        let files = fx.files("s1");
        assert_eq!((files[0].status.as_str(), files[0].deletions), ("deleted", 2));
        fx.store.undo("s1", fx.cwd(), None).unwrap();
        assert_eq!(fx.read("a.txt"), "one\ntwo\n");
    }
}
