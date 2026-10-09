//! The names MonoCode gave its account profiles, read once by the import
//! so the copied profiles keep them. MonoCode keeps the list
//! (`monocode.providerAccounts.v1`) in its webview's local storage, a
//! SQLite file under `~/Library/WebKit`; it is only ever read. Blocking.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OpenFlags};

use crate::harness::accounts::{ProviderAccount, StoredAccounts};

const STORAGE_ROOT: &str = "Library/WebKit/com.monocode.desktop/WebsiteData/Default";
const ACCOUNTS_KEY: &str = "monocode.providerAccounts.v1";

/// Every account MonoCode lists, across its storage origins. Nothing when
/// MonoCode is not installed or its storage cannot be read.
pub fn load(home: &Path) -> Vec<ProviderAccount> {
    let mut accounts = Vec::new();
    for file in storage_files(&home.join(STORAGE_ROOT)) {
        match read_accounts(&file) {
            Ok(found) => accounts.extend(found),
            Err(err) => log::debug!("MonoCode accounts in {}: {err:#}", file.display()),
        }
    }
    accounts
}

/// `<root>/<origin>/<origin>/LocalStorage/localstorage.sqlite3`, one per
/// origin the webview has served the app from.
fn storage_files(root: &Path) -> Vec<PathBuf> {
    let dirs = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect()
    };
    dirs(root)
        .iter()
        .flat_map(|origin| dirs(origin))
        .map(|origin| origin.join("LocalStorage/localstorage.sqlite3"))
        .filter(|file| file.is_file())
        .collect()
}

fn read_accounts(file: &Path) -> Result<Vec<ProviderAccount>> {
    let conn = Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let value: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [ACCOUNTS_KEY],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            err => Err(err),
        })?;
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    parse_accounts(&decode_utf16le(&value))
}

/// WebKit stores local storage values as UTF-16LE.
fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Entries of the wrong shape are skipped one by one, not the whole list.
fn parse_accounts(json: &str) -> Result<Vec<ProviderAccount>> {
    let stored: std::collections::BTreeMap<String, Vec<serde_json::Value>> =
        serde_json::from_str(json).context("account list is not a provider map")?;
    let mut accounts = StoredAccounts::new();
    for (provider, entries) in stored {
        let parsed = entries
            .into_iter()
            .filter_map(|entry| serde_json::from_value::<ProviderAccount>(entry).ok())
            .map(|account| ProviderAccount {
                provider: provider.clone(),
                ..account
            });
        accounts.entry(provider.clone()).or_default().extend(parsed);
    }
    Ok(accounts.into_values().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn reads_the_account_list_from_a_storage_file() {
        let dir = std::env::temp_dir().join(format!("bencode-ls-{}", std::process::id()));
        let origin = dir.join("origin/origin/LocalStorage");
        std::fs::create_dir_all(&origin).unwrap();
        let file = origin.join("localstorage.sqlite3");
        let conn = Connection::open(&file).unwrap();
        conn.execute_batch(
            "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB NOT NULL ON CONFLICT FAIL);",
        )
        .unwrap();
        let json = r#"{"claude":[{"id":"account-1","provider":"claude","label":"Côngty"},{"id":7}],"codex":[]}"#;
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
            rusqlite::params![ACCOUNTS_KEY, utf16le(json)],
        )
        .unwrap();
        drop(conn);

        assert_eq!(storage_files(&dir), [file.clone()]);
        let accounts = read_accounts(&file).unwrap();
        assert_eq!(
            accounts,
            [ProviderAccount {
                id: "account-1".into(),
                provider: "claude".into(),
                label: "Côngty".into()
            }]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_storage_file_without_accounts_is_empty() {
        assert!(parse_accounts("{}").unwrap().is_empty());
        assert!(parse_accounts("[]").is_err());
        assert!(storage_files(Path::new("/nonexistent/bencode")).is_empty());
    }
}
