//! Translations kept on disk (`inbox-translations.json` in the data
//! folder), so a refresh or a relaunch does not pay a model turn again for
//! the same text. Each is kept by item key or comment key and language,
//! with a hash of the exact text it was made from, the title apart from
//! the body: a translation applies only while its source is unchanged, and
//! a list row can use a title whose description has since changed.
//!
//! Bounded: what is older than `MAX_AGE_DAYS`, then all but the newest
//! `MAX_ENTRIES`, is dropped as the file is written. Pure but for
//! `load` and `save`, which touch the disk: use the background executor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::Language;

const FILE_NAME: &str = "inbox-translations.json";
const VERSION: u32 = 1;
/// A translation unused for this long has likely left the Inbox.
pub const MAX_AGE_DAYS: i64 = 90;
/// Items and comments together; a few megabytes at the most.
pub const MAX_ENTRIES: usize = 2_000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Where the cache lives; `None` without a home directory.
pub fn path() -> Option<PathBuf> {
    crate::storage::data_dir().map(|dir| dir.join(FILE_NAME))
}

/// The hash a translation names its source by: SHA-256 of the exact
/// text, its first 16 hex digits (enough to tell one edit from another).
pub fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One translation and what it was made from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// An item's key, or a comment's (`comment_key`).
    pub key: String,
    pub language: Language,
    /// `hash` of the source title; empty for a comment.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// `hash` of the source body (an item's description, a comment's text).
    pub body_hash: String,
    pub body: String,
    /// When it was made (ms).
    pub made_at: i64,
}

impl Entry {
    /// Made from `title` (`None` for a comment) and `body` as they are now.
    pub fn matches(&self, title: Option<&str>, body: &str) -> bool {
        self.title_matches(title.unwrap_or_default()) && self.body_hash == hash(body)
    }

    fn title_matches(&self, title: &str) -> bool {
        if self.title_hash.is_empty() {
            title.is_empty()
        } else {
            self.title_hash == hash(title)
        }
    }
}

/// The cache in memory. Entries are shared, so a save takes a snapshot
/// without copying their text on the UI thread.
#[derive(Default)]
pub struct Cache {
    entries: HashMap<(String, Language), Arc<Entry>>,
}

#[derive(Serialize)]
struct FileOut<'a> {
    version: u32,
    entries: Vec<&'a Entry>,
}

#[derive(Deserialize)]
struct FileIn {
    version: u32,
    #[serde(default)]
    entries: Vec<Entry>,
}

impl Cache {
    pub fn get(&self, key: &str, language: Language) -> Option<&Entry> {
        self.entries
            .get(&(key.to_string(), language))
            .map(Arc::as_ref)
    }

    /// The translation of `key` into `language` made from `title` and
    /// `body` as they are now.
    pub fn valid(
        &self,
        key: &str,
        language: Language,
        title: Option<&str>,
        body: &str,
    ) -> Option<&Entry> {
        self.get(key, language)
            .filter(|entry| entry.matches(title, body))
    }

    /// Item `key`'s title in `language`, when made from a title hashing to
    /// `title_hash`, whatever became of its description.
    pub fn title(&self, key: &str, language: Language, title_hash: &str) -> Option<&str> {
        self.get(key, language)
            .filter(|entry| !entry.title.is_empty() && entry.title_hash == title_hash)
            .map(|entry| entry.title.as_str())
    }

    /// Keeps `entry`, over what `key` had in its language.
    pub fn insert(&mut self, entry: Entry) {
        let at = (entry.key.clone(), entry.language);
        self.entries.insert(at, Arc::new(entry));
    }

    /// What was read from disk, under what was made meanwhile: the newer
    /// of two for the same key and language stays.
    pub fn merge(&mut self, read: Cache) {
        for (at, entry) in read.entries {
            let older = (self.entries.get(&at)).is_some_and(|kept| kept.made_at >= entry.made_at);
            if !older {
                self.entries.insert(at, entry);
            }
        }
    }

    /// Drops what is older than `MAX_AGE_DAYS` at `now_ms`, then all but
    /// the newest `MAX_ENTRIES`.
    pub fn prune(&mut self, now_ms: i64) {
        let oldest = now_ms - MAX_AGE_DAYS * DAY_MS;
        self.entries.retain(|_, entry| entry.made_at >= oldest);
        if self.entries.len() > MAX_ENTRIES {
            // Newest first; ties go by key, so the result does not depend
            // on the map's order.
            let mut order: Vec<_> = self.entries.iter().collect();
            order.sort_by(|(a_at, a), (b_at, b)| {
                (b.made_at.cmp(&a.made_at))
                    .then_with(|| a_at.0.cmp(&b_at.0))
                    .then_with(|| a_at.1.label().cmp(b_at.1.label()))
            });
            let dropped: Vec<_> = (order.into_iter().skip(MAX_ENTRIES))
                .map(|(at, _)| at.clone())
                .collect();
            for at in dropped {
                self.entries.remove(&at);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    /// The entries as they are now, for `save`.
    pub fn snapshot(&self) -> Vec<Arc<Entry>> {
        self.entries.values().cloned().collect()
    }

    /// The cache at `path`; empty when there is none yet. A file from a
    /// newer BenCode's format is left alone and read as empty.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
        };
        let file: FileIn =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let mut cache = Self::default();
        if file.version == VERSION {
            for entry in file.entries {
                cache.insert(entry);
            }
        }
        Ok(cache)
    }
}

/// The last save written, by its sequence number: saves run on the
/// background executor, and an older one must not land over a newer.
static WRITTEN: Mutex<u64> = Mutex::new(0);

/// Writes `entries` (pruned at `now_ms`) to `path` through a temporary
/// file and a rename, unless a save numbered after `seq` was written
/// already.
pub fn save(path: &Path, entries: Vec<Arc<Entry>>, seq: u64, now_ms: i64) -> Result<()> {
    let mut written = WRITTEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if *written > seq {
        return Ok(());
    }
    let mut cache = Cache {
        entries: (entries.into_iter())
            .map(|e| ((e.key.clone(), e.language), e))
            .collect(),
    };
    cache.prune(now_ms);
    let mut entries: Vec<&Entry> = cache.entries.values().map(Arc::as_ref).collect();
    // A stable order: the file diffs and compresses well.
    entries.sort_by(|a, b| (&a.key, a.language.label()).cmp(&(&b.key, b.language.label())));
    let text = serde_json::to_string(&FileOut {
        version: VERSION,
        entries,
    })?;
    write_atomic(path, &text)?;
    *written = seq;
    Ok(())
}

/// Beside the target, then renamed over it: a crash mid-write leaves the
/// old file whole.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text).with_context(|| format!("writing {}", temp.display()))?;
    std::fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, language: Language, title: &str, body: &str, made_at: i64) -> Entry {
        Entry {
            key: key.into(),
            language,
            title_hash: if title.is_empty() {
                String::new()
            } else {
                hash(title)
            },
            title: format!("{title} (translated)"),
            body_hash: hash(body),
            body: format!("{body} (translated)"),
            made_at,
        }
    }

    #[test]
    fn hashes_are_short_stable_and_tell_edits_apart() {
        assert_eq!(hash("ログイン不可").len(), 16);
        assert_eq!(hash("ログイン不可"), hash("ログイン不可"));
        assert_ne!(hash("ログイン不可"), hash("ログイン不可 "));
        // SHA-256's own digits: what a file written today still names.
        assert_eq!(hash(""), "e3b0c44298fc1c14");
    }

    #[test]
    fn entries_are_kept_by_key_and_language() {
        let mut cache = Cache::default();
        cache.insert(entry("A-1", Language::Vietnamese, "件名", "本文", 1));
        cache.insert(entry("A-1", Language::English, "件名", "本文", 2));
        cache.insert(entry("A-1#comment-7", Language::Vietnamese, "", "了解", 3));
        assert_eq!(cache.len(), 3);
        let vi = cache.get("A-1", Language::Vietnamese).unwrap();
        assert_eq!(vi.made_at, 1);
        assert!(cache.get("A-1", Language::Japanese).is_none());
        // A newer translation replaces the one in its language only.
        cache.insert(entry("A-1", Language::Vietnamese, "件名", "本文", 4));
        assert_eq!(cache.len(), 3);
        assert_eq!(cache.get("A-1", Language::Vietnamese).unwrap().made_at, 4);
    }

    #[test]
    fn a_changed_source_invalidates_only_its_part() {
        let mut cache = Cache::default();
        cache.insert(entry("A-1", Language::Vietnamese, "件名", "本文", 1));
        let vi = Language::Vietnamese;
        assert!(cache.valid("A-1", vi, Some("件名"), "本文").is_some());
        assert!(cache.valid("A-1", vi, Some("件名"), "本文を更新").is_none());
        assert!(cache.valid("A-1", vi, Some("新しい件名"), "本文").is_none());
        // The list's title survives a changed description, not a changed title.
        assert_eq!(
            cache.title("A-1", vi, &hash("件名")),
            Some("件名 (translated)")
        );
        assert_eq!(cache.title("A-1", vi, &hash("新しい件名")), None);
        // A comment has no title, and an item's title is no comment's.
        cache.insert(entry("A-1#comment-7", vi, "", "了解", 1));
        assert!(cache.valid("A-1#comment-7", vi, None, "了解").is_some());
        assert!(cache.valid("A-1#comment-7", vi, None, "了解です").is_none());
        assert!(cache.valid("A-1", vi, None, "本文").is_none());
        assert_eq!(cache.title("A-1#comment-7", vi, &hash("")), None);
    }

    #[test]
    fn pruning_drops_the_old_then_the_least_recent() {
        let now = 1_000 * DAY_MS;
        let mut cache = Cache::default();
        cache.insert(entry("old", Language::English, "t", "b", now - 91 * DAY_MS));
        cache.insert(entry(
            "kept",
            Language::English,
            "t",
            "b",
            now - 89 * DAY_MS,
        ));
        cache.prune(now);
        assert!(cache.get("old", Language::English).is_none());
        assert!(cache.get("kept", Language::English).is_some());

        let mut cache = Cache::default();
        for ix in 0..MAX_ENTRIES + 5 {
            let made = now - (ix as i64) * 1000;
            cache.insert(entry(
                &format!("K-{ix}"),
                Language::Vietnamese,
                "t",
                "b",
                made,
            ));
        }
        cache.prune(now);
        assert_eq!(cache.len(), MAX_ENTRIES);
        assert!(
            cache.get("K-0", Language::Vietnamese).is_some(),
            "newest kept"
        );
        let last = format!("K-{}", MAX_ENTRIES + 4);
        assert!(
            cache.get(&last, Language::Vietnamese).is_none(),
            "oldest gone"
        );

        // Ties at the limit still leave exactly the limit.
        let mut cache = Cache::default();
        for ix in 0..MAX_ENTRIES + 3 {
            cache.insert(entry(&format!("T-{ix}"), Language::English, "t", "b", now));
        }
        cache.prune(now);
        assert_eq!(cache.len(), MAX_ENTRIES);
    }

    #[test]
    fn what_was_read_merges_under_what_was_made_meanwhile() {
        let mut made = Cache::default();
        made.insert(entry("A-1", Language::Vietnamese, "new", "b", 10));
        let mut read = Cache::default();
        read.insert(entry("A-1", Language::Vietnamese, "old", "b", 5));
        read.insert(entry("A-2", Language::Vietnamese, "t", "b", 5));
        made.merge(read);
        assert_eq!(made.len(), 2);
        let kept = made.get("A-1", Language::Vietnamese).unwrap();
        assert_eq!(kept.title, "new (translated)");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bencode-translations-{name}-{}",
            crate::app_cli::random_hex(6)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_save_loads_back_and_leaves_no_temporary_file() {
        let dir = scratch("roundtrip");
        let path = dir.join(FILE_NAME);
        assert_eq!(Cache::load(&path).unwrap().len(), 0, "no file yet");
        let now = 100 * DAY_MS;
        let mut cache = Cache::default();
        let item = entry("A-1", Language::Vietnamese, "件名", "本文", now);
        let comment = entry("A-1#comment-7", Language::English, "", "了解", now);
        cache.insert(item.clone());
        cache.insert(comment.clone());
        cache.insert(entry("stale", Language::English, "t", "b", 0));
        save(&path, cache.snapshot(), 1, now).unwrap();
        let read = Cache::load(&path).unwrap();
        assert_eq!(read.len(), 2, "pruned as written");
        assert_eq!(read.get("A-1", Language::Vietnamese), Some(&item));
        assert_eq!(read.get("A-1#comment-7", Language::English), Some(&comment));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("titleHash\":\"\""),
            "a comment writes no title"
        );
        assert!(!path.with_extension("json.tmp").exists());

        // An older save landing late is dropped.
        save(&path, Vec::new(), 0, now).unwrap();
        assert_eq!(Cache::load(&path).unwrap().len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_newer_format_or_a_broken_file_is_not_taken() {
        let dir = scratch("format");
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, r#"{"version": 2, "entries": []}"#).unwrap();
        assert_eq!(Cache::load(&path).unwrap().len(), 0);
        std::fs::write(&path, "{").unwrap();
        assert!(Cache::load(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
