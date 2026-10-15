//! MonoCode `src-tauri/src/mono.rs` and `monoFiles.ts`: each Mono keeps its
//! files in BenCode's data folder, never in a repository it works in.
//!
//! ```text
//! <data folder>/monos/<mono id>/
//! ├── SOUL.md      who it is; edited by the user
//! ├── MEMORY.md    what it knows; loads into every turn, within its budget
//! ├── habits.json  what it does on its own, on a schedule (`habits.rs`)
//! └── memory/      topic notes, and archive.md
//! ```
//!
//! A save names the hash of the version it was made over, so an edit never
//! overwrites a change made since without asking (MonoCode's "Use theirs /
//! Keep mine"). Files are owner-only: memory can hold personal details.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context as _, Result, anyhow};
use sha2::{Digest, Sha256};

pub const SOUL_FILE: &str = "SOUL.md";
pub const MEMORY_FILE: &str = "MEMORY.md";
pub const HABITS_FILE: &str = "habits.json";
const MEMORY_DIR: &str = "memory";
/// MonoCode `FILE_MAX`: far over what loads, it only stops a runaway write.
const FILE_MAX: usize = 256 * 1024;

/// Hash-checked writes, one at a time, so only one caller saves over a
/// given version (the floating chat and the main window share the files).
static FILES_LOCK: Mutex<()> = Mutex::new(());

/// MonoCode `MonoFiles`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MonoFiles {
    pub soul: String,
    pub soul_hash: String,
    pub memory: String,
    pub memory_hash: String,
    /// Topic notes under `memory/`, by name without `.md`; not the archive.
    pub topics: Vec<String>,
}

/// Why a save did not happen.
#[derive(Debug)]
pub enum SaveError {
    /// The file changed since the version the save was made over.
    Conflict,
    Failed(anyhow::Error),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict => write!(f, "The file changed while you were editing"),
            Self::Failed(err) => write!(f, "{err:#}"),
        }
    }
}

/// MonoCode `content_hash`: SHA-256 of the text, in hex.
pub fn content_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// MonoCode `resolve_file`: SOUL.md, MEMORY.md, habits.json or one
/// `memory/<name>.md`. Nothing nested and nothing hidden, so a name can
/// never leave the folder.
fn resolve(dir: &Path, relative: &str) -> Option<PathBuf> {
    if [SOUL_FILE, MEMORY_FILE, HABITS_FILE].contains(&relative) {
        return Some(dir.join(relative));
    }
    let name = relative.strip_prefix("memory/")?.strip_suffix(".md")?;
    let valid = !name.is_empty()
        && name.len() <= 80
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_'));
    valid.then(|| dir.join(MEMORY_DIR).join(format!("{name}.md")))
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
    }
}

/// MonoCode `write_atomic`: beside the target, then renamed over it, so a
/// crash mid-write leaves the old file whole. Owner-only.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    use std::io::Write as _;
    let temp = path.with_extension("md.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .with_context(|| format!("writing {}", temp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        // A temporary file left by an older write may be wider.
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("securing {}", temp.display()))?;
    }
    file.write_all(text.as_bytes())
        .with_context(|| format!("writing {}", temp.display()))?;
    drop(file);
    std::fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))
}

/// MonoCode `topics`.
fn topics(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir.join(MEMORY_DIR))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().is_file())
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let topic = name.strip_suffix(".md")?.to_string();
                    (topic != "archive" && !topic.starts_with('.')).then_some(topic)
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// MonoCode `agent_dir` then `loadMonoFiles`: the folder with an empty
/// memory on first use, and `SOUL.md` seeded with the default the first
/// time (over the missing file only, so a seed never replaces one).
pub fn load(dir: &Path) -> Result<MonoFiles> {
    std::fs::create_dir_all(dir.join(MEMORY_DIR))
        .with_context(|| format!("creating {}", dir.display()))?;
    let _lock = FILES_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let memory_path = dir.join(MEMORY_FILE);
    let memory = match read_optional(&memory_path)? {
        Some(memory) => memory,
        None => {
            write_atomic(&memory_path, "")?;
            String::new()
        }
    };
    let soul_path = dir.join(SOUL_FILE);
    let soul = match read_optional(&soul_path)? {
        Some(soul) => soul,
        None => {
            let soul = super::default_soul();
            write_atomic(&soul_path, &soul)?;
            soul
        }
    };
    Ok(MonoFiles {
        soul_hash: content_hash(&soul),
        soul,
        memory_hash: content_hash(&memory),
        memory,
        topics: topics(dir),
    })
}

/// MonoCode `mono_read`: a file and the hash a save over it must name
/// (of "" for a missing file).
pub fn read(dir: &Path, relative: &str) -> Result<(Option<String>, String)> {
    let path = resolve(dir, relative).ok_or_else(|| anyhow!("not a Mono file: {relative}"))?;
    let text = read_optional(&path)?;
    let hash = content_hash(text.as_deref().unwrap_or(""));
    Ok((text, hash))
}

/// MonoCode `mono_save`: saves over `expected` (no hash overwrites); the
/// new hash.
pub fn save(
    dir: &Path,
    relative: &str,
    text: &str,
    expected: Option<&str>,
) -> std::result::Result<String, SaveError> {
    let path = resolve(dir, relative)
        .ok_or_else(|| SaveError::Failed(anyhow!("not a Mono file: {relative}")))?;
    if text.len() > FILE_MAX {
        return Err(SaveError::Failed(anyhow!(
            "{relative} is over {} KB",
            FILE_MAX / 1024
        )));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))
            .map_err(SaveError::Failed)?;
    }
    let _lock = FILES_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(expected) = expected {
        let current = read_optional(&path).map_err(SaveError::Failed)?;
        if content_hash(current.as_deref().unwrap_or("")) != expected {
            return Err(SaveError::Conflict);
        }
    }
    write_atomic(&path, text).map_err(SaveError::Failed)?;
    Ok(content_hash(text))
}

/// MonoCode `editMonoFile`: a small edit reapplied to the latest file, a
/// few times over if another save lands in between. `edit` may refuse.
pub fn edit(
    dir: &Path,
    relative: &str,
    edit: impl Fn(&str) -> std::result::Result<String, String>,
) -> std::result::Result<String, String> {
    for attempt in 0.. {
        let (current, hash) = read(dir, relative).map_err(|err| format!("{err:#}"))?;
        let current = current.unwrap_or_default();
        let text = edit(&current)?;
        if text == current {
            return Ok(hash);
        }
        match save(dir, relative, &text, Some(&hash)) {
            Ok(hash) => return Ok(hash),
            Err(SaveError::Conflict) if attempt < 2 => continue,
            Err(SaveError::Conflict) => {
                return Err(
                    "Memory kept changing while saving. Your edit was not saved; try again."
                        .to_string(),
                );
            }
            Err(SaveError::Failed(err)) => return Err(format!("{err:#}")),
        }
    }
    unreachable!("the loop returns")
}

/// A file in the Mono's folder, for its Overview's Documents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    /// Its path in the folder: `SOUL.md`, `memory/releases.md`.
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    /// When it last changed, in ms since the epoch.
    pub modified_ms: i64,
}

/// Every file in the Mono's folder and its `memory/`, newest first; the
/// temporary files saves leave behind are not documents.
pub fn documents(dir: &Path) -> Vec<Document> {
    let mut out = Vec::new();
    let mut scan = |folder: &Path, prefix: &str| {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if !meta.is_file() || name.starts_with('.') || name.ends_with(".tmp") {
                continue;
            }
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |at| at.as_millis() as i64);
            out.push(Document {
                name: format!("{prefix}{name}"),
                path: entry.path(),
                bytes: meta.len(),
                modified_ms,
            });
        }
    };
    scan(dir, "");
    scan(&dir.join(MEMORY_DIR), "memory/");
    out.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bencode-mono-files-{name}-{}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn the_folder_starts_with_a_soul_and_an_empty_memory() {
        let dir = temp_dir("seed");
        let files = load(&dir).unwrap();
        assert_eq!(files.soul, super::super::default_soul());
        assert_eq!(files.memory, "");
        assert_eq!(files.memory_hash, content_hash(""));
        assert!(dir.join("memory").is_dir());
        // Seeded once: an edit stays.
        save(&dir, SOUL_FILE, "# Mine\n", None).unwrap();
        assert_eq!(load(&dir).unwrap().soul, "# Mine\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_save_over_a_changed_file_is_refused() {
        let dir = temp_dir("conflict");
        let files = load(&dir).unwrap();
        let hash = save(&dir, MEMORY_FILE, "- a\n", Some(&files.memory_hash)).unwrap();
        assert!(matches!(
            save(&dir, MEMORY_FILE, "- b\n", Some(&files.memory_hash)),
            Err(SaveError::Conflict)
        ));
        save(&dir, MEMORY_FILE, "- c\n", Some(&hash)).unwrap();
        assert_eq!(load(&dir).unwrap().memory, "- c\n");
        // No hash overwrites.
        save(&dir, MEMORY_FILE, "- d\n", None).unwrap();
        assert_eq!(read(&dir, MEMORY_FILE).unwrap().0.as_deref(), Some("- d\n"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_edit_applies_to_the_latest_file() {
        let dir = temp_dir("edit");
        load(&dir).unwrap();
        edit(&dir, MEMORY_FILE, |text| Ok(format!("{text}- one\n"))).unwrap();
        edit(&dir, MEMORY_FILE, |text| Ok(format!("{text}- two\n"))).unwrap();
        assert_eq!(load(&dir).unwrap().memory, "- one\n- two\n");
        assert_eq!(
            edit(&dir, MEMORY_FILE, |_| Err("no".to_string())),
            Err("no".to_string())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_the_monos_own_files_resolve() {
        let dir = temp_dir("topics");
        load(&dir).unwrap();
        save(&dir, "memory/releases.md", "# Releases", None).unwrap();
        save(&dir, "memory/archive.md", "- old", None).unwrap();
        assert_eq!(load(&dir).unwrap().topics, ["releases"]);
        for bad in ["memory/../SOUL.md", "../x.md", "memory/.hidden.md", "notes.txt"] {
            assert!(read(&dir, bad).is_err(), "{bad}");
        }
        let names: Vec<String> = documents(&dir).into_iter().map(|d| d.name).collect();
        for name in ["SOUL.md", "MEMORY.md", "memory/releases.md", "memory/archive.md"] {
            assert!(names.contains(&name.to_string()), "{name}");
        }
        assert!(!names.iter().any(|name| name.ends_with(".tmp")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp_dir("private");
        load(&dir).unwrap();
        let mode = std::fs::metadata(dir.join(MEMORY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_dir_all(&dir).ok();
    }
}
