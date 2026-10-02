//! Session checkpoint & rollback engine.
//!
//! Ported from MonoCode (`src-tauri/src/checkpoint.rs`).
//! Stores file snapshots before agent tools (write_to_file, replace_file_content)
//! modify them, allowing developers to revert changes and undo turns safely.
//!
//! Safety invariants (mirroring MonoCode):
//! - Session ids are `[A-Za-z0-9_-]+`; relative paths contain only normal
//!   components, so neither can escape the store root or the project.
//! - No path that traverses a symbolic link is read, written, or deleted.
//! - Blobs are named by SHA-256 and verified on every read and reuse.
//! - Restores target the manifest's recorded project root, and refuse to
//!   overwrite a file the user changed after the agent unless forced.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MANIFEST_FILE: &str = "manifest.json";
const BLOB_DIR: &str = "files";
/// Length of a lowercase hex SHA-256 digest.
const HASH_HEX_LEN: usize = 64;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    /// Canonical project root the snapshots belong to.
    pub cwd: String,
    /// relative_path -> SHA-256 of the pre-edit content (`None` if the file
    /// did not exist before the session touched it).
    pub files: BTreeMap<String, Option<String>>,
    /// relative_path -> Unix permission bits of the pre-edit file.
    #[serde(default)]
    pub modes: BTreeMap<String, u32>,
    /// relative_path -> SHA-256 of the content right after the agent's last
    /// edit (`None` if the agent deleted it). Used to detect foreign edits.
    #[serde(default)]
    pub after: BTreeMap<String, Option<String>>,
    /// Paths changed by someone else between two of the agent's edits.
    #[serde(default)]
    pub diverged: BTreeSet<String>,
}

/// Outcome of restoring every file in a session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub restored: Vec<String>,
    /// `(relative_path, error message)` for each file that was not restored.
    pub failed: Vec<(String, String)>,
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

    pub fn default_dir() -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        home.map_or_else(
            || PathBuf::from(".bencode/checkpoints"),
            |h| h.join(".bencode/checkpoints"),
        )
    }

    fn lock(&self) -> Result<MutexGuard<'_, ()>> {
        self.gate
            .lock()
            .map_err(|_| anyhow!("Checkpoint store lock poisoned"))
    }

    fn session_dir(&self, session_id: &str) -> Result<PathBuf> {
        validate_id(session_id, "session")?;
        Ok(self.root.join(session_id))
    }

    /// Prepares a checkpoint before a file is modified by an agent tool.
    /// If the file was not already snapshotted for this session, saves its current content.
    pub fn prepare_file(&self, session_id: &str, cwd: &str, relative_path: &str) -> Result<()> {
        let dir = self.session_dir(session_id)?;
        let relative = validate_relative(relative_path)?;
        let root = project_root(cwd)?;
        let _guard = self.lock()?;

        let mut manifest = read_manifest(&dir)?.unwrap_or_else(|| Manifest {
            cwd: root.to_string_lossy().into_owned(),
            ..Manifest::default()
        });
        ensure_same_root(&manifest, &root)?;
        reject_symlink(&root, &relative)?;

        // Only the first touch owns the undo baseline. A later prepare only
        // checks whether someone else edited the file since the agent did.
        if manifest.files.contains_key(&relative) {
            if let Some(expected) = manifest.after.get(&relative)
                && current_hash(&root, &relative)? != *expected
                && manifest.diverged.insert(relative)
            {
                write_manifest(&dir, &manifest)?;
            }
            return Ok(());
        }

        let target = root.join(&relative);
        let before = match fs::symlink_metadata(&target) {
            Ok(meta) if meta.is_file() => {
                let bytes = fs::read(&target).with_context(|| format!("Cannot read {relative}"))?;
                if let Some(mode) = file_mode(&meta) {
                    manifest.modes.insert(relative.clone(), mode);
                }
                Some(store_blob(&dir, &bytes)?)
            }
            Ok(_) => bail!("Cannot checkpoint {relative}: not a regular file"),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(err).with_context(|| format!("Cannot inspect {relative}")),
        };
        manifest.files.insert(relative, before);
        write_manifest(&dir, &manifest)
    }

    /// Records the file's content right after an agent edit, so a later
    /// restore can tell whether the user changed it afterwards.
    pub fn capture_file(&self, session_id: &str, cwd: &str, relative_path: &str) -> Result<()> {
        let dir = self.session_dir(session_id)?;
        let relative = validate_relative(relative_path)?;
        let root = project_root(cwd)?;
        let _guard = self.lock()?;

        let mut manifest = read_manifest(&dir)?
            .ok_or_else(|| anyhow!("No checkpoints found for session {session_id}"))?;
        ensure_same_root(&manifest, &root)?;
        if !manifest.files.contains_key(&relative) {
            bail!("No checkpoint was prepared for {relative}");
        }
        reject_symlink(&root, &relative)?;
        let hash = current_hash(&root, &relative)?;
        manifest.after.insert(relative, hash);
        write_manifest(&dir, &manifest)
    }

    /// Restores a single file back to its pre-session snapshot state,
    /// refusing if the user changed it after the agent.
    pub fn restore_file(&self, session_id: &str, cwd: &str, relative_path: &str) -> Result<()> {
        self.restore_file_with(session_id, cwd, relative_path, false)
    }

    /// Like [`Self::restore_file`]; `force` overwrites foreign changes.
    pub fn restore_file_with(
        &self,
        session_id: &str,
        cwd: &str,
        relative_path: &str,
        force: bool,
    ) -> Result<()> {
        let dir = self.session_dir(session_id)?;
        let relative = validate_relative(relative_path)?;
        let _guard = self.lock()?;

        let mut manifest = read_manifest(&dir)?
            .ok_or_else(|| anyhow!("No checkpoints found for session {session_id}"))?;
        let root = manifest_root(&manifest, cwd)?;
        if !manifest.files.contains_key(&relative) {
            bail!("No checkpoint found for file {relative}");
        }
        restore_one(&dir, &root, &manifest, &relative, force)?;
        release_path(&mut manifest, &relative);
        write_manifest(&dir, &manifest)
    }

    /// Restores all files changed during the session back to their initial state.
    ///
    /// Returns the restored paths, or an error naming every file that failed
    /// (the others are still restored). Use [`Self::restore_all_with`] for a
    /// structured report.
    pub fn restore_all(&self, session_id: &str, cwd: &str) -> Result<Vec<String>> {
        let report = self.restore_all_with(session_id, cwd, false)?;
        if report.failed.is_empty() {
            return Ok(report.restored);
        }
        let details = report
            .failed
            .iter()
            .map(|(path, err)| format!("{path}: {err}"))
            .collect::<Vec<_>>()
            .join("; ");
        bail!(
            "Restored {} file(s); {} failed: {details}",
            report.restored.len(),
            report.failed.len()
        )
    }

    /// Restores every checkpointed file, collecting per-file failures.
    /// `force` overwrites files the user changed after the agent.
    pub fn restore_all_with(
        &self,
        session_id: &str,
        cwd: &str,
        force: bool,
    ) -> Result<RestoreReport> {
        let dir = self.session_dir(session_id)?;
        let _guard = self.lock()?;

        let mut manifest = read_manifest(&dir)?
            .ok_or_else(|| anyhow!("No checkpoints found for session {session_id}"))?;
        let root = manifest_root(&manifest, cwd)?;

        let mut report = RestoreReport::default();
        let paths: Vec<String> = manifest.files.keys().cloned().collect();
        for path in paths {
            let result = validate_relative(&path)
                .and_then(|relative| restore_one(&dir, &root, &manifest, &relative, force));
            match result {
                Ok(()) => {
                    release_path(&mut manifest, &path);
                    report.restored.push(path);
                }
                Err(err) => report.failed.push((path, format!("{err:#}"))),
            }
        }
        if !report.restored.is_empty() {
            write_manifest(&dir, &manifest)?;
        }
        Ok(report)
    }

    /// Discards snapshot data for a completed or dismissed session.
    pub fn discard(&self, session_id: &str) -> Result<()> {
        let dir = self.session_dir(session_id)?;
        let _guard = self.lock()?;
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_symlink() => fs::remove_file(&dir)?,
            Ok(_) => fs::remove_dir_all(&dir)?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
        Ok(())
    }
}

fn restore_one(
    dir: &Path,
    root: &Path,
    manifest: &Manifest,
    relative: &str,
    force: bool,
) -> Result<()> {
    reject_symlink(root, relative)?;
    if !force {
        if manifest.diverged.contains(relative) {
            bail!("{relative} changed outside this session between agent edits");
        }
        if let Some(expected) = manifest.after.get(relative)
            && current_hash(root, relative)? != *expected
        {
            bail!("{relative} was changed after the agent's edit; refusing to overwrite");
        }
    }

    let target = root.join(relative);
    match manifest.files.get(relative) {
        Some(Some(hash)) => {
            let bytes = read_blob(dir, hash)?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Cannot create parent of {relative}"))?;
            }
            atomic_write(&target, &bytes)?;
            if let Some(mode) = manifest.modes.get(relative) {
                set_file_mode(&target, *mode)?;
            }
            Ok(())
        }
        Some(None) => match fs::symlink_metadata(&target) {
            Ok(_) => fs::remove_file(&target).with_context(|| format!("Cannot remove {relative}")),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err).with_context(|| format!("Cannot inspect {relative}")),
        },
        None => bail!("No checkpoint found for file {relative}"),
    }
}

fn release_path(manifest: &mut Manifest, relative: &str) {
    manifest.files.remove(relative);
    manifest.modes.remove(relative);
    manifest.after.remove(relative);
    manifest.diverged.remove(relative);
}

/// Accepts ids made only of ASCII letters, digits, `-` and `_`.
fn validate_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("Invalid {label} id");
    }
    Ok(())
}

/// Returns a normalized `a/b/c` key for a relative path made only of normal
/// components. Absolute paths, `..`, `.`, and empty paths are rejected.
fn validate_relative(relative: &str) -> Result<String> {
    let mut parts = Vec::new();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or_else(|| anyhow!("Invalid path {relative}"))?,
            ),
            _ => bail!("Invalid path {relative}"),
        }
    }
    if parts.is_empty() {
        bail!("Invalid path {relative:?}");
    }
    Ok(parts.join("/"))
}

/// True if any existing component of `root/relative` is a symlink (or cannot
/// be inspected). Missing trailing components are fine.
fn path_contains_symlink(root: &Path, relative: &str) -> bool {
    let mut current = root.to_path_buf();
    for part in relative.split('/') {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return false,
            Err(_) => return true,
        }
    }
    false
}

fn reject_symlink(root: &Path, relative: &str) -> Result<()> {
    if path_contains_symlink(root, relative) {
        bail!("Refusing to touch {relative}: the path contains a symbolic link");
    }
    Ok(())
}

fn project_root(cwd: &str) -> Result<PathBuf> {
    let trimmed = cwd.trim();
    if trimmed.is_empty() {
        bail!("cwd is required");
    }
    let root = Path::new(trimmed)
        .canonicalize()
        .with_context(|| format!("Cannot resolve {trimmed}"))?;
    if !root.is_dir() {
        bail!("{}: Not a directory", root.display());
    }
    Ok(root)
}

fn ensure_same_root(manifest: &Manifest, root: &Path) -> Result<()> {
    if Path::new(&manifest.cwd) != root {
        bail!(
            "Checkpoint belongs to {}, not {}",
            manifest.cwd,
            root.display()
        );
    }
    Ok(())
}

/// The project root recorded in the manifest; the caller's `cwd` must match it.
fn manifest_root(manifest: &Manifest, cwd: &str) -> Result<PathBuf> {
    let recorded = project_root(&manifest.cwd)?;
    ensure_same_root(manifest, &recorded)?;
    ensure_same_root(manifest, &project_root(cwd)?)?;
    Ok(recorded)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_valid_hash(hash: &str) -> bool {
    hash.len() == HASH_HEX_LEN
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// SHA-256 of the file's current content, or `None` if it does not exist.
fn current_hash(root: &Path, relative: &str) -> Result<Option<String>> {
    let path = root.join(relative);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() => {
            let bytes = fs::read(&path).with_context(|| format!("Cannot read {relative}"))?;
            Ok(Some(sha256_hex(&bytes)))
        }
        Ok(_) => bail!("{relative} is not a regular file"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("Cannot inspect {relative}")),
    }
}

fn blob_path(dir: &Path, hash: &str) -> Result<PathBuf> {
    if !is_valid_hash(hash) {
        bail!("Invalid snapshot hash {hash:?}");
    }
    Ok(dir.join(BLOB_DIR).join(hash))
}

/// Stores `bytes` under its SHA-256. An existing blob is reused only if its
/// content is byte-identical.
fn store_blob(dir: &Path, bytes: &[u8]) -> Result<String> {
    let hash = sha256_hex(bytes);
    let path = blob_path(dir, &hash)?;
    match fs::read(&path) {
        Ok(existing) if existing == bytes => return Ok(hash),
        Ok(_) => bail!("Snapshot blob {hash} exists with different content"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("Cannot read blob {hash}")),
    }
    fs::create_dir_all(dir.join(BLOB_DIR))?;
    atomic_write(&path, bytes)?;
    Ok(hash)
}

/// Reads a blob and verifies its content still matches its name.
fn read_blob(dir: &Path, hash: &str) -> Result<Vec<u8>> {
    let path = blob_path(dir, hash)?;
    let bytes = fs::read(&path).with_context(|| format!("Snapshot content {hash} missing"))?;
    if sha256_hex(&bytes) != hash {
        bail!("Snapshot content {hash} is corrupt");
    }
    Ok(bytes)
}

fn read_manifest(dir: &Path) -> Result<Option<Manifest>> {
    let path = dir.join(MANIFEST_FILE);
    let data = match fs::read(&path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("Cannot read {}", path.display())),
    };
    let manifest = serde_json::from_slice(&data)
        .with_context(|| format!("Corrupt checkpoint manifest {}", path.display()))?;
    Ok(Some(manifest))
}

fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<()> {
    fs::create_dir_all(dir)?;
    let data = serde_json::to_vec_pretty(manifest)?;
    atomic_write(&dir.join(MANIFEST_FILE), &data)
}

/// Writes via a temp file in the destination directory, then renames it into
/// place, so readers never observe a partially written file.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| anyhow!("{} has no file name", path.display()))?
        .to_string_lossy();
    let tmp = parent.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = write_and_sync(&tmp, bytes).and_then(|()| {
        fs::rename(&tmp, path).with_context(|| format!("Cannot replace {}", path.display()))
    });
    if result.is_err()
        && let Err(err) = fs::remove_file(&tmp)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("failed to clean temp file {}: {err}", tmp.display());
    }
    result
}

fn write_and_sync(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let mut file =
        fs::File::create(path).with_context(|| format!("Cannot create {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn file_mode(meta: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn file_mode(_meta: &fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
fn set_file_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Cannot set mode on {}", path.display()))
}

#[cfg(not(unix))]
fn set_file_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique temp dir with `store/` and `project/`, removed on drop.
    struct Fixture {
        base: PathBuf,
        store: CheckpointStore,
        project: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let base = std::env::temp_dir().join(format!(
                "bencode-cp-{}-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let project = base.join("project");
            fs::create_dir_all(&project).unwrap();
            let base = base.canonicalize().unwrap();
            let project = base.join("project");
            let store = CheckpointStore::new(base.join("store"));
            Self {
                base,
                store,
                project,
            }
        }

        fn cwd(&self) -> &str {
            self.project.to_str().unwrap()
        }

        fn write(&self, rel: &str, content: &str) {
            let path = self.project.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }

        fn read(&self, rel: &str) -> String {
            fs::read_to_string(self.project.join(rel)).unwrap()
        }

        fn session_dir(&self, id: &str) -> PathBuf {
            self.store.session_dir(id).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Err(err) = fs::remove_dir_all(&self.base) {
                eprintln!("failed to clean {}: {err}", self.base.display());
            }
        }
    }

    #[test]
    fn checkpoint_prepare_and_restore_cycle() {
        let fx = Fixture::new();
        fx.write("hello.txt", "original content");

        fx.store.prepare_file("s1", fx.cwd(), "hello.txt").unwrap();
        fx.write("hello.txt", "modified content by agent");
        // A second prepare must not overwrite the original snapshot.
        fx.store.prepare_file("s1", fx.cwd(), "hello.txt").unwrap();
        fx.store.restore_file("s1", fx.cwd(), "hello.txt").unwrap();

        assert_eq!(fx.read("hello.txt"), "original content");
        fx.store.discard("s1").unwrap();
        assert!(!fx.session_dir("s1").exists());
    }

    #[test]
    fn checkpoint_handles_newly_created_files() {
        let fx = Fixture::new();

        fx.store
            .prepare_file("s1", fx.cwd(), "new_file.rs")
            .unwrap();
        fx.write("new_file.rs", "fn main() {}");
        fx.store
            .restore_file("s1", fx.cwd(), "new_file.rs")
            .unwrap();

        assert!(!fx.project.join("new_file.rs").exists());
    }

    #[test]
    fn rejects_invalid_session_ids() {
        let fx = Fixture::new();
        fx.write("a.txt", "a");
        fs::create_dir_all(fx.base.join("store")).unwrap();
        fs::write(fx.base.join("store/keep"), "x").unwrap();

        for id in ["", "..", "../x", "/tmp/abs", "a/b", "a.b", "a b"] {
            assert!(
                fx.store.prepare_file(id, fx.cwd(), "a.txt").is_err(),
                "{id:?}"
            );
            assert!(fx.store.discard(id).is_err(), "{id:?}");
        }

        assert!(fx.base.join("store/keep").exists());
    }

    #[test]
    fn rejects_unsafe_relative_paths() {
        let fx = Fixture::new();
        fs::write(fx.base.join("outside.txt"), "secret").unwrap();

        for rel in ["", "/etc/passwd", "../outside.txt", "a/../b", "./a", "a/.."] {
            assert!(
                fx.store.prepare_file("s1", fx.cwd(), rel).is_err(),
                "{rel:?}"
            );
        }

        // Interior `.` and repeated separators normalize to the same key.
        assert_eq!(validate_relative("a//b").unwrap(), "a/b");
        assert_eq!(validate_relative("a/./b").unwrap(), "a/b");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_snapshot_through_symlinks() {
        let fx = Fixture::new();
        let outside = fx.base.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("f.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(&outside, fx.project.join("link")).unwrap();

        let error = fx
            .store
            .prepare_file("s1", fx.cwd(), "link/f.txt")
            .unwrap_err();

        assert!(error.to_string().contains("symbolic link"));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_restore_or_delete_through_symlinks() {
        let fx = Fixture::new();
        fx.write("sub/f.txt", "original");
        fx.store.prepare_file("s1", fx.cwd(), "sub/f.txt").unwrap();
        fx.store
            .prepare_file("s1", fx.cwd(), "sub/new.txt")
            .unwrap();
        let outside = fx.base.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("f.txt"), "victim").unwrap();
        fs::write(outside.join("new.txt"), "victim").unwrap();
        fs::remove_dir_all(fx.project.join("sub")).unwrap();
        std::os::unix::fs::symlink(&outside, fx.project.join("sub")).unwrap();

        let report = fx.store.restore_all_with("s1", fx.cwd(), true).unwrap();

        assert_eq!(report.failed.len(), 2);
        assert_eq!(fs::read_to_string(outside.join("f.txt")).unwrap(), "victim");
        assert!(outside.join("new.txt").exists());
    }

    #[test]
    fn blobs_are_named_by_sha256() {
        let fx = Fixture::new();
        fx.write("abc.txt", "abc");

        fx.store.prepare_file("s1", fx.cwd(), "abc.txt").unwrap();

        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(fx.session_dir("s1").join(BLOB_DIR).join(expected).is_file());
    }

    #[test]
    fn existing_blob_with_wrong_content_is_rejected() {
        let fx = Fixture::new();
        fx.write("abc.txt", "abc");
        let blobs = fx.session_dir("s1").join(BLOB_DIR);
        fs::create_dir_all(&blobs).unwrap();
        let hash = sha256_hex(b"abc");
        fs::write(blobs.join(&hash), "not abc").unwrap();

        let error = fx
            .store
            .prepare_file("s1", fx.cwd(), "abc.txt")
            .unwrap_err();

        assert!(error.to_string().contains("different content"));
    }

    #[test]
    fn corrupt_blob_is_detected_on_restore() {
        let fx = Fixture::new();
        fx.write("a.txt", "original");
        fx.store.prepare_file("s1", fx.cwd(), "a.txt").unwrap();
        fx.write("a.txt", "agent");
        let hash = sha256_hex(b"original");
        fs::write(fx.session_dir("s1").join(BLOB_DIR).join(hash), "tampered").unwrap();

        let error = fx.store.restore_file("s1", fx.cwd(), "a.txt").unwrap_err();

        assert!(error.to_string().contains("corrupt"));
        assert_eq!(fx.read("a.txt"), "agent");
    }

    #[test]
    fn restore_all_collects_per_file_errors() {
        let fx = Fixture::new();
        fx.write("good.txt", "good");
        fx.write("bad.txt", "bad");
        fx.store.prepare_file("s1", fx.cwd(), "good.txt").unwrap();
        fx.store.prepare_file("s1", fx.cwd(), "bad.txt").unwrap();
        fx.write("good.txt", "changed");
        fx.write("bad.txt", "changed");
        fs::remove_file(fx.session_dir("s1").join(BLOB_DIR).join(sha256_hex(b"bad"))).unwrap();

        let report = fx.store.restore_all_with("s1", fx.cwd(), false).unwrap();

        assert_eq!(report.restored, vec!["good.txt".to_string()]);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, "bad.txt");
        assert_eq!(fx.read("good.txt"), "good");
        let error = fx.store.restore_all("s1", fx.cwd()).unwrap_err();
        assert!(error.to_string().contains("bad.txt"));
    }

    #[test]
    fn restore_refuses_a_different_cwd() {
        let fx = Fixture::new();
        let other = fx.base.join("other");
        fs::create_dir_all(&other).unwrap();
        fx.write("a.txt", "original");
        fx.store.prepare_file("s1", fx.cwd(), "a.txt").unwrap();
        fs::write(other.join("a.txt"), "other").unwrap();
        let other_cwd = other.to_str().unwrap();

        assert!(fx.store.restore_file("s1", other_cwd, "a.txt").is_err());
        assert!(fx.store.restore_all("s1", other_cwd).is_err());
        assert!(fx.store.prepare_file("s1", other_cwd, "a.txt").is_err());
        assert_eq!(fs::read_to_string(other.join("a.txt")).unwrap(), "other");
    }

    #[test]
    fn manifest_write_leaves_no_temp_files() {
        let fx = Fixture::new();
        fx.write("a.txt", "a");

        fx.store.prepare_file("s1", fx.cwd(), "a.txt").unwrap();

        let dir = fx.session_dir("s1");
        let stray: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(stray.is_empty(), "{stray:?}");
        assert!(read_manifest(&dir).unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn restore_preserves_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let fx = Fixture::new();
        fx.write("run.sh", "#!/bin/sh\n");
        let path = fx.project.join("run.sh");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        fx.store.prepare_file("s1", fx.cwd(), "run.sh").unwrap();
        fs::remove_file(&path).unwrap();
        fx.write("run.sh", "echo agent\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        fx.store.restore_file("s1", fx.cwd(), "run.sh").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        assert_eq!(fx.read("run.sh"), "#!/bin/sh\n");
    }

    #[test]
    fn refuses_to_overwrite_user_edits_after_the_agent_unless_forced() {
        let fx = Fixture::new();
        fx.write("a.txt", "original");
        fx.store.prepare_file("s1", fx.cwd(), "a.txt").unwrap();
        fx.write("a.txt", "agent");
        fx.store.capture_file("s1", fx.cwd(), "a.txt").unwrap();
        fx.write("a.txt", "user");

        let error = fx.store.restore_file("s1", fx.cwd(), "a.txt").unwrap_err();
        assert!(error.to_string().contains("changed after the agent"));
        assert_eq!(fx.read("a.txt"), "user");

        fx.store
            .restore_file_with("s1", fx.cwd(), "a.txt", true)
            .unwrap();
        assert_eq!(fx.read("a.txt"), "original");
    }

    #[test]
    fn restore_succeeds_when_file_still_matches_agent_edit() {
        let fx = Fixture::new();
        fx.write("a.txt", "original");
        fx.store.prepare_file("s1", fx.cwd(), "a.txt").unwrap();
        fx.write("a.txt", "agent");
        fx.store.capture_file("s1", fx.cwd(), "a.txt").unwrap();

        let restored = fx.store.restore_all("s1", fx.cwd()).unwrap();

        assert_eq!(restored, vec!["a.txt".to_string()]);
        assert_eq!(fx.read("a.txt"), "original");
    }

    #[test]
    fn user_edit_between_agent_edits_marks_file_diverged() {
        let fx = Fixture::new();
        fx.write("a.txt", "original");
        fx.store.prepare_file("s1", fx.cwd(), "a.txt").unwrap();
        fx.write("a.txt", "agent 1");
        fx.store.capture_file("s1", fx.cwd(), "a.txt").unwrap();
        fx.write("a.txt", "user");
        fx.store.prepare_file("s1", fx.cwd(), "a.txt").unwrap();
        fx.write("a.txt", "agent 2");
        fx.store.capture_file("s1", fx.cwd(), "a.txt").unwrap();

        let error = fx.store.restore_file("s1", fx.cwd(), "a.txt").unwrap_err();

        assert!(error.to_string().contains("between agent edits"));
        assert_eq!(fx.read("a.txt"), "agent 2");
    }

    #[test]
    fn default_dir_lives_under_bencode() {
        assert!(CheckpointStore::default_dir().ends_with(".bencode/checkpoints"));
    }

    #[test]
    fn capture_requires_a_prepared_file() {
        let fx = Fixture::new();
        fx.write("a.txt", "a");
        fx.write("b.txt", "b");
        fx.store.prepare_file("s1", fx.cwd(), "a.txt").unwrap();

        assert!(fx.store.capture_file("s1", fx.cwd(), "b.txt").is_err());
    }
}
