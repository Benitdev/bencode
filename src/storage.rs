//! Where BenCode keeps its own data: one folder, beside `settings.json`.
//! Nothing here is shared with MonoCode; what a MonoCode install held is
//! copied in once (`monocode_import`).

use std::path::PathBuf;

const DB_FILE: &str = "bencode.db";

/// `~/Library/Application Support/BenCode` on macOS, `~/.config/bencode`
/// elsewhere; `None` without a home directory.
pub fn data_dir() -> Option<PathBuf> {
    crate::settings::settings_dir()
}

/// Threads, notes, automations and reminders.
pub fn db_path() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join(DB_FILE))
}

/// What each thread's agent changed, for the session review card.
pub fn checkpoints_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("checkpoints"))
}

/// One config directory per provider account.
pub fn provider_accounts_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("provider-accounts"))
}

/// `~/Library/Logs/BenCode` on macOS, where Console looks; beside the data
/// elsewhere.
pub fn logs_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        Some(home.join("Library/Logs/BenCode"))
    } else {
        data_dir().map(|dir| dir.join("logs"))
    }
}
