//! Where BenCode keeps its own data. The app's `storage.rs` builds on this;
//! the host keeps its data apart (`remote::host_dir`).

use std::path::PathBuf;

/// `~/Library/Application Support/BenCode` on macOS, `~/.config/bencode`
/// elsewhere; `None` without a home directory.
pub fn data_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/BenCode")
    } else {
        home.join(".config/bencode")
    })
}

/// One config directory per provider account.
pub fn provider_accounts_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("provider-accounts"))
}
