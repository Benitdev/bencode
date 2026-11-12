//! Files BenCode writes on a thread's behalf (pasted images, browser
//! snapshots), kept in `threads/<thread>/attachments/` of the data folder so
//! they last as long as the thread and go with it. MonoCode `thread_files`.
//! What the user attached from elsewhere on disk is only referenced, never
//! copied here and never deleted.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const ROOT: &str = "threads";
const ATTACHMENTS: &str = "attachments";
/// Encoded names start with this, so they never collide with a plain id.
const ENCODED_PREFIX: &str = "x-";
/// A folder this new may belong to a thread whose row is still on its way to
/// the database.
const SWEEP_GRACE: Duration = Duration::from_secs(3600);

/// A folder name for a thread id: plain ids as they are, anything a path
/// could trip on as hex.
fn folder_name(id: &str) -> String {
    let plain = !id.is_empty()
        && !id.starts_with('.')
        && !id.starts_with(ENCODED_PREFIX)
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if plain {
        id.to_owned()
    } else {
        let hex: String = id.bytes().map(|b| format!("{b:02x}")).collect();
        format!("{ENCODED_PREFIX}{hex}")
    }
}

/// The thread id a folder is named for; `None` for a name no id gives.
fn thread_id(folder: &str) -> Option<String> {
    let Some(hex) = folder.strip_prefix(ENCODED_PREFIX) else {
        return Some(folder.to_owned());
    };
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

fn attachments_dir_in(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join(ROOT).join(folder_name(id)).join(ATTACHMENTS)
}

/// Where thread `id`'s own files go; the temporary folder without a data
/// folder.
pub fn attachments_dir(id: &str) -> PathBuf {
    crate::storage::data_dir().map_or_else(
        crate::ui::composer::attachments::paste_temp_dir,
        |data_dir| attachments_dir_in(&data_dir, id),
    )
}

fn remove_in(data_dir: &Path, id: &str) {
    let dir = data_dir.join(ROOT).join(folder_name(id));
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        // The launch's sweep tries again.
        Err(err) => log::error!("could not remove {}: {err}", dir.display()),
    }
}

/// A deleted thread's files go with it. Blocking: for the database writer.
pub fn remove(id: &str) {
    if let Some(data_dir) = crate::storage::data_dir() {
        remove_in(&data_dir, id);
    }
}

/// The ids of the folders present, for asking the database which still have
/// a thread. Empty, cheaply, until a thread has files.
fn folders_in(data_dir: &Path) -> Vec<String> {
    let entries = match std::fs::read_dir(data_dir.join(ROOT)) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            log::error!("could not list thread folders: {err}");
            return Vec::new();
        }
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| thread_id(entry.file_name().to_str()?))
        .collect()
}

/// Whether a folder has been left long enough to count as an orphan.
fn settled(dir: &Path, now: SystemTime) -> bool {
    std::fs::metadata(dir)
        .and_then(|meta| meta.modified())
        .is_ok_and(|modified| {
            now.duration_since(modified)
                .is_ok_and(|age| age >= SWEEP_GRACE)
        })
}

fn sweep_in(data_dir: &Path, known: &HashSet<String>, now: SystemTime) {
    for id in folders_in(data_dir) {
        let dir = data_dir.join(ROOT).join(folder_name(&id));
        if !known.contains(&id) && settled(&dir, now) {
            log::info!("removing the files of deleted thread {id}");
            remove_in(data_dir, &id);
        }
    }
}

/// Removes the folders of threads no longer in the database: a crash between
/// deleting a thread and its files, or a row deleted without going through
/// the app. `known` answers for the listed ids only.
pub fn sweep(db: &crate::db::AppDb) -> anyhow::Result<()> {
    let Some(data_dir) = crate::storage::data_dir() else {
        return Ok(());
    };
    let folders = folders_in(&data_dir);
    if folders.is_empty() {
        return Ok(());
    }
    let mut known = HashSet::new();
    for id in folders {
        if db.session_exists(&id)? {
            known.insert(id);
        }
    }
    sweep_in(&data_dir, &known, SystemTime::now());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bencode-thread-files-{}-{}",
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
    fn folder_names_round_trip_and_stay_inside_the_root() {
        for id in [
            "42",
            "abc-DEF_9",
            "remote:env:7",
            "../up",
            ".hidden",
            "x-41",
            "a/b",
            "",
        ] {
            let name = folder_name(id);
            assert_eq!(thread_id(&name).as_deref(), Some(id));
            assert!(
                !name.contains('/') && !name.starts_with('.') && !name.is_empty(),
                "{id} became {name}"
            );
        }
        assert_eq!(thread_id("x-4"), None);
        assert_eq!(thread_id("x-zz"), None);
    }

    #[test]
    fn deleting_a_thread_takes_only_its_own_folder() {
        let data = temp();
        let (gone, kept) = (
            attachments_dir_in(&data, "gone"),
            attachments_dir_in(&data, "kept"),
        );
        std::fs::create_dir_all(&gone).unwrap();
        std::fs::create_dir_all(&kept).unwrap();
        remove_in(&data, "gone");
        remove_in(&data, "never-had-files");
        assert!(!gone.exists());
        assert!(kept.exists());
        std::fs::remove_dir_all(&data).unwrap();
    }

    #[test]
    fn the_sweep_spares_live_threads_and_new_folders() {
        let data = temp();
        let orphan = attachments_dir_in(&data, "orphan");
        let live = attachments_dir_in(&data, "live");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::create_dir_all(&live).unwrap();
        let known: HashSet<String> = ["live".to_string()].into();

        // Just written: the row may not have landed yet.
        sweep_in(&data, &known, SystemTime::now());
        assert!(orphan.exists());

        sweep_in(&data, &known, SystemTime::now() + SWEEP_GRACE * 2);
        assert!(!orphan.exists());
        assert!(live.exists());
        std::fs::remove_dir_all(&data).unwrap();
    }

    #[test]
    fn a_data_folder_without_thread_files_lists_nothing() {
        let data = temp();
        assert!(folders_in(&data).is_empty());
        std::fs::remove_dir_all(&data).unwrap();
    }
}
