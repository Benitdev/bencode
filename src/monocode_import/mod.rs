//! The one-time copy of a MonoCode install's data into BenCode's own
//! folder. BenCode used to work in MonoCode's database, checkpoint store
//! and account profiles; it now keeps its own (`storage`), and brings what
//! was there along the first time it starts without a database.
//!
//! MonoCode's files are only read. The database is copied last, so an
//! import cut short starts over on the next launch.

mod accounts;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OpenFlags};

use crate::harness::accounts::{ProviderAccount, StoredAccounts, valid_account_id};

/// Relative to `$HOME`: MonoCode's app data directory.
const MONOCODE_DIR: &str = "Library/Application Support/com.monocode.desktop";
/// Relative to `$HOME`: the database BenCode used without MonoCode.
const LEGACY_DB: &str = ".bencode/bencode.db";
/// A copy in progress, renamed into place once whole.
const PARTIAL_SUFFIX: &str = "importing";

/// What an import brought that lives in `settings.json`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Imported {
    /// The names MonoCode gave the account profiles.
    pub accounts: Vec<ProviderAccount>,
}

/// How the first-launch import went.
#[derive(Debug)]
pub enum ImportOutcome {
    /// BenCode already had a database, or there was nothing to bring.
    Nothing,
    Imported(Imported),
    /// The copy failed and left no database, so the next launch tries again.
    Failed(String),
}

/// Imports once: nothing happens when BenCode already has a database, or
/// when there is nothing to bring. Blocking; run before the database opens.
pub fn import_once() -> ImportOutcome {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return ImportOutcome::Nothing;
    };
    let Some(db) = crate::storage::db_path() else {
        return ImportOutcome::Nothing;
    };
    match import(&home, &db) {
        Ok(Some(imported)) => ImportOutcome::Imported(imported),
        Ok(None) => ImportOutcome::Nothing,
        Err(err) => {
            log::error!("importing MonoCode's data: {err:#}");
            ImportOutcome::Failed(format!("{err:#}"))
        }
    }
}

fn import(home: &Path, db: &Path) -> Result<Option<Imported>> {
    if db.exists() {
        return Ok(None);
    }
    let data = db.parent().context("the database has no folder")?;
    let monocode = home.join(MONOCODE_DIR);
    let monocode_db = monocode.join("monocode.db");
    let legacy_db = home.join(LEGACY_DB);
    let from_monocode = monocode_db.is_file();
    let source = if from_monocode {
        &monocode_db
    } else if legacy_db.is_file() {
        &legacy_db
    } else {
        return Ok(None);
    };
    std::fs::create_dir_all(data).with_context(|| format!("creating {}", data.display()))?;
    log::info!("importing {} into {}", source.display(), data.display());
    if from_monocode {
        for folder in ["checkpoints", "provider-accounts"] {
            copy_folder_once(&monocode.join(folder), &data.join(folder))?;
        }
    }
    copy_database(source, db)?;
    Ok(Some(Imported {
        accounts: if from_monocode { accounts::load(home) } else { Vec::new() },
    }))
}

/// A consistent copy of a database that may be open elsewhere, write-ahead
/// log included.
fn copy_database(source: &Path, dest: &Path) -> Result<()> {
    let partial = partial_path(dest);
    remove_if_present(&partial)?;
    let conn = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {}", source.display()))?;
    conn.execute("VACUUM INTO ?1", [partial.to_string_lossy()])
        .with_context(|| format!("copying {}", source.display()))?;
    std::fs::rename(&partial, dest).with_context(|| format!("placing {}", dest.display()))
}

/// Copies `source` to `dest` unless `dest` is already there.
fn copy_folder_once(source: &Path, dest: &Path) -> Result<()> {
    if !source.is_dir() || dest.exists() {
        return Ok(());
    }
    let partial = partial_path(dest);
    match std::fs::remove_dir_all(&partial) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("clearing {}", partial.display())),
    }
    copy_tree(source, &partial)?;
    std::fs::rename(&partial, dest).with_context(|| format!("placing {}", dest.display()))
}

/// Files, folders and symbolic links; anything else (a socket) is left.
fn copy_tree(source: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    for entry in std::fs::read_dir(source).with_context(|| format!("reading {}", source.display()))? {
        let entry = entry?;
        let (from, to) = (entry.path(), dest.join(entry.file_name()));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&from, &to)?;
        } else if kind.is_file() {
            std::fs::copy(&from, &to).with_context(|| format!("copying {}", from.display()))?;
        } else if kind.is_symlink() {
            copy_symlink(&from, &to)?;
        } else {
            log::debug!("import skips {}", from.display());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(from: &Path, to: &Path) -> Result<()> {
    let target = std::fs::read_link(from).with_context(|| format!("reading {}", from.display()))?;
    std::os::unix::fs::symlink(target, to).with_context(|| format!("linking {}", to.display()))
}

#[cfg(not(unix))]
fn copy_symlink(from: &Path, _to: &Path) -> Result<()> {
    log::debug!("import skips the link {}", from.display());
    Ok(())
}

fn partial_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(PARTIAL_SUFFIX);
    dest.with_file_name(name)
}

fn remove_if_present(file: &Path) -> Result<()> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("removing {}", file.display())),
    }
}

/// Adds the imported account names to this app's list; an account already
/// named here keeps its name. True when the list changed.
pub fn merge_accounts(stored: &mut StoredAccounts, imported: Vec<ProviderAccount>) -> bool {
    let mut changed = false;
    for account in imported {
        if !valid_account_id(&account.id) || account.label.trim().is_empty() {
            continue;
        }
        let list = stored.entry(account.provider.clone()).or_default();
        if !list.iter().any(|known| known.id == account.id) {
            list.push(account);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bencode-import-{name}-{}", std::process::id()));
        if let Err(err) = std::fs::remove_dir_all(&dir) {
            assert_eq!(err.kind(), std::io::ErrorKind::NotFound, "{err}");
        }
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// Adds a note to the write-ahead database at `path`, creating it.
    fn database(path: &Path, note: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE IF NOT EXISTS notes (body TEXT);")
            .unwrap();
        conn.execute("INSERT INTO notes (body) VALUES (?1)", [note]).unwrap();
    }

    fn notes(path: &Path) -> Vec<String> {
        let conn = Connection::open(path).unwrap();
        let mut stmt = conn.prepare("SELECT body FROM notes").unwrap();
        stmt.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
    }

    #[test]
    fn imports_monocodes_database_and_folders_once() {
        let home = temp_home("monocode");
        let monocode = home.join(MONOCODE_DIR);
        database(&monocode.join("monocode.db"), "from monocode");
        write(&monocode.join("checkpoints/s1/manifest.json"), "{}");
        write(&monocode.join("provider-accounts/claude/account-1/.claude.json"), "{\"a\":1}");
        #[cfg(unix)]
        std::os::unix::fs::symlink("manifest.json", monocode.join("checkpoints/s1/link")).unwrap();
        let data = home.join("data");
        let db = data.join("bencode.db");

        let imported = import(&home, &db).unwrap();

        assert_eq!(imported, Some(Imported::default()), "no account list to read");
        assert_eq!(notes(&db), ["from monocode"]);
        assert_eq!(std::fs::read_to_string(data.join("checkpoints/s1/manifest.json")).unwrap(), "{}");
        assert!(data.join("provider-accounts/claude/account-1/.claude.json").is_file());
        #[cfg(unix)]
        assert_eq!(
            std::fs::read_link(data.join("checkpoints/s1/link")).unwrap(),
            PathBuf::from("manifest.json")
        );
        assert!(!partial_path(&db).exists());
        // MonoCode's own files are untouched.
        assert_eq!(notes(&monocode.join("monocode.db")), ["from monocode"]);

        // A second launch imports nothing, whatever MonoCode has done since.
        database(&monocode.join("monocode.db"), "later");
        assert_eq!(import(&home, &db).unwrap(), None);
        assert_eq!(notes(&db), ["from monocode"]);
    }

    /// What `ImportOutcome::Failed` relies on: no database is left behind,
    /// so the next launch imports again.
    #[test]
    fn a_failed_import_leaves_no_database() {
        let home = temp_home("corrupt");
        write(&home.join(MONOCODE_DIR).join("monocode.db"), "not a database");
        let db = home.join("data/bencode.db");

        assert!(import(&home, &db).is_err());

        assert!(!db.exists());
        assert!(!partial_path(&db).exists());
    }

    #[test]
    fn falls_back_to_the_legacy_database_or_nothing() {
        let home = temp_home("legacy");
        let db = home.join("data/bencode.db");
        assert_eq!(import(&home, &db).unwrap(), None);
        assert!(!db.exists(), "nothing to import leaves a fresh start");

        database(&home.join(LEGACY_DB), "legacy");
        assert_eq!(import(&home, &db).unwrap(), Some(Imported::default()));
        assert_eq!(notes(&db), ["legacy"]);
    }

    #[test]
    fn folders_already_here_are_kept() {
        let home = temp_home("kept");
        let monocode = home.join(MONOCODE_DIR);
        database(&monocode.join("monocode.db"), "x");
        write(&monocode.join("checkpoints/theirs.json"), "theirs");
        let data = home.join("data");
        write(&data.join("checkpoints/mine.json"), "mine");

        import(&home, &data.join("bencode.db")).unwrap();

        assert!(data.join("checkpoints/mine.json").is_file());
        assert!(!data.join("checkpoints/theirs.json").exists());
    }

    #[test]
    fn imported_names_fill_in_without_renaming() {
        let account = |id: &str, provider: &str, label: &str| ProviderAccount {
            id: id.into(),
            provider: provider.into(),
            label: label.into(),
        };
        let mut stored = StoredAccounts::new();
        stored.insert("claude".into(), vec![account("a1", "claude", "Mine")]);
        let changed = merge_accounts(
            &mut stored,
            vec![
                account("a1", "claude", "MonoCode's"),
                account("a2", "claude", "Work"),
                account("bad id", "claude", "Nope"),
                account("c1", "codex", "  "),
            ],
        );
        assert!(changed);
        assert_eq!(stored["claude"], [account("a1", "claude", "Mine"), account("a2", "claude", "Work")]);
        assert!(!stored.contains_key("codex") || stored["codex"].is_empty());
        assert!(!merge_accounts(&mut stored, vec![account("a2", "claude", "Again")]));
    }
}
