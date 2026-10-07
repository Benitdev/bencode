//! Provider account profiles (MonoCode `providers/model/providerAccounts.ts`
//! and `provider_account_dir` / the account env in `src-tauri/src/harness.rs`):
//! locally named sign-ins of a CLI, each isolated in its own config
//! directory under MonoCode's `provider-accounts`, so both apps reach the
//! same profiles.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const DEFAULT_ACCOUNT_ID: &str = "default";
pub const DEFAULT_ACCOUNT_LABEL: &str = "Default account";

const ROOT_RELATIVE: &str = "Library/Application Support/com.monocode.desktop/provider-accounts";
const MAX_LABEL_CHARS: usize = 48;

/// Providers whose CLIs support isolated, locally named account profiles.
pub fn supports_accounts(harness: &str) -> bool {
    matches!(harness, "claude" | "codex")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccount {
    pub id: String,
    pub provider: String,
    pub label: String,
}

impl ProviderAccount {
    pub fn is_default(&self) -> bool {
        self.id == DEFAULT_ACCOUNT_ID
    }
}

/// MonoCode `monocode.providerAccounts.v1`: accounts by provider.
pub type StoredAccounts = BTreeMap<String, Vec<ProviderAccount>>;
/// MonoCode `monocode.providerAccountSelections.v1`: the account new threads
/// of a project use, by project then provider.
pub type StoredSelections = BTreeMap<String, BTreeMap<String, String>>;

pub fn valid_account_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// A label as stored: whitespace collapsed, at most 48 characters.
pub fn clean_label(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(MAX_LABEL_CHARS).collect()
}

/// `provider`'s accounts: the default profile first, then `stored` (this
/// app's), then any of `shared` (MonoCode's) not already listed. Entries
/// with a bad id or an empty label are skipped; a stored `default` entry
/// only renames the default profile.
pub fn provider_accounts(
    provider: &str,
    stored: &StoredAccounts,
    shared: &[ProviderAccount],
) -> Vec<ProviderAccount> {
    let mut default_label = DEFAULT_ACCOUNT_LABEL.to_string();
    let mut profiles: Vec<ProviderAccount> = Vec::new();
    let own = stored.get(provider).map(Vec::as_slice).unwrap_or_default();
    let shared = shared.iter().filter(|account| account.provider == provider);
    for account in own.iter().chain(shared) {
        let label = clean_label(&account.label);
        if label.is_empty() || !valid_account_id(&account.id) {
            continue;
        }
        if account.is_default() {
            if default_label == DEFAULT_ACCOUNT_LABEL {
                default_label = label;
            }
        } else if !profiles.iter().any(|known| known.id == account.id) {
            profiles.push(ProviderAccount {
                id: account.id.clone(),
                provider: provider.to_string(),
                label,
            });
        }
    }
    profiles.insert(
        0,
        ProviderAccount {
            id: DEFAULT_ACCOUNT_ID.into(),
            provider: provider.to_string(),
            label: default_label,
        },
    );
    profiles
}

/// A new profile named `label`, or "Account N" when the name is blank.
pub fn new_account(provider: &str, label: &str, existing: usize) -> ProviderAccount {
    let label = clean_label(label);
    ProviderAccount {
        id: format!("account-{}", random_uuid()),
        provider: provider.to_string(),
        label: if label.is_empty() {
            format!("Account {}", existing + 1)
        } else {
            label
        },
    }
}

/// A version-4 UUID from the system's random source.
fn random_uuid() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    let filled = std::fs::File::open("/dev/urandom").and_then(|mut source| source.read_exact(&mut bytes));
    if let Err(err) = filled {
        // Unique enough for a local profile id when there is no random device.
        log::warn!("no random source for an account id: {err}");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        bytes = (nanos ^ (u128::from(std::process::id()) << 96)).to_be_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// MonoCode `renameProviderAccount`: names `account` `label` in this app's
/// list. An account only MonoCode lists, or the default profile, gets an
/// entry here, which wins over MonoCode's name. False for a blank label.
pub fn rename_account(stored: &mut StoredAccounts, account: &ProviderAccount, label: &str) -> bool {
    let label = clean_label(label);
    if label.is_empty() || !valid_account_id(&account.id) {
        return false;
    }
    let list = stored.entry(account.provider.clone()).or_default();
    match list.iter_mut().find(|entry| entry.id == account.id) {
        Some(entry) => entry.label = label,
        None => list.push(ProviderAccount {
            id: account.id.clone(),
            provider: account.provider.clone(),
            label,
        }),
    }
    true
}

/// MonoCode `removeProviderAccount`: drops the account from this app's list
/// and from every project's choice. The default profile stays.
pub fn remove_account(
    stored: &mut StoredAccounts,
    selections: &mut StoredSelections,
    provider: &str,
    id: &str,
) {
    if id == DEFAULT_ACCOUNT_ID {
        return;
    }
    if let Some(list) = stored.get_mut(provider) {
        list.retain(|account| account.id != id);
        if list.is_empty() {
            stored.remove(provider);
        }
    }
    selections.retain(|_, selection| {
        if selection.get(provider).is_some_and(|chosen| chosen == id) {
            selection.remove(provider);
        }
        !selection.is_empty()
    });
}

/// The account a new thread of `project` uses; the default profile when the
/// remembered one is gone.
pub fn selected_account_id(
    selections: &StoredSelections,
    project: &str,
    provider: &str,
    accounts: &[ProviderAccount],
) -> String {
    selections
        .get(project)
        .and_then(|selection| selection.get(provider))
        .filter(|id| accounts.iter().any(|account| &account.id == *id))
        .cloned()
        .unwrap_or_else(|| DEFAULT_ACCOUNT_ID.into())
}

/// `(provider, account id)` of every profile directory on disk, so a thread
/// pinned to one MonoCode's list no longer names can still run. Blocking.
pub fn profiles_on_disk() -> Vec<(String, String)> {
    let Some(root) = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(ROOT_RELATIVE)) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for provider in ["claude", "codex"] {
        let Ok(entries) = std::fs::read_dir(root.join(provider)) else {
            continue;
        };
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().into_owned();
            if valid_account_id(&id) && id != DEFAULT_ACCOUNT_ID && entry.path().is_dir() {
                found.push((provider.to_string(), id));
            }
        }
    }
    found
}

/// A non-default account's config directory and the CLI it belongs to. The
/// default profile has none: its CLI runs with the user's own environment.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AccountProfile {
    provider: &'static str,
    pub dir: PathBuf,
}

impl AccountProfile {
    /// None for the default profile, a provider without profiles, or an id
    /// that could not be a directory name.
    pub fn resolve(provider: &str, account_id: Option<&str>) -> Option<Self> {
        let id = account_id.filter(|id| *id != DEFAULT_ACCOUNT_ID && valid_account_id(id))?;
        let provider = match provider {
            "claude" => "claude",
            "codex" => "codex",
            _ => return None,
        };
        let home = PathBuf::from(std::env::var_os("HOME")?);
        Some(Self {
            provider,
            dir: home.join(ROOT_RELATIVE).join(provider).join(id),
        })
    }

    /// The environment that points the CLI at this profile: variables to
    /// set to its directory, and credentials of the default profile to drop.
    fn env(&self) -> (&'static [&'static str], &'static [&'static str]) {
        match self.provider {
            // Claude scopes both its ordinary config and its macOS Keychain
            // credential to these exact strings. Setting both keeps profiles
            // isolated on every supported platform.
            "claude" => (
                &["CLAUDE_CONFIG_DIR", "CLAUDE_SECURESTORAGE_CONFIG_DIR"],
                &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"],
            ),
            _ => (
                &["CODEX_HOME"],
                &["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN"],
            ),
        }
    }

    fn prepare(&self) -> (&'static [&'static str], &'static [&'static str]) {
        if let Err(err) = std::fs::create_dir_all(&self.dir) {
            log::warn!("could not create {}: {err}", self.dir.display());
        }
        self.env()
    }

    /// Points a child CLI at this profile, creating its directory.
    pub fn apply(&self, command: &mut std::process::Command) {
        let (set, remove) = self.prepare();
        for name in set {
            command.env(name, &self.dir);
        }
        for name in remove {
            command.env_remove(name);
        }
    }

    /// Deletes the profile's directory: a sign-in that never finished
    /// leaves nothing worth keeping. Blocking.
    pub fn discard(&self) {
        match std::fs::remove_dir_all(&self.dir) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => log::warn!("could not remove {}: {err}", self.dir.display()),
        }
    }

    /// MonoCode `provider_account_remove`: deletes the profile's sign-in
    /// (Claude's is in the Keychain), then its directory. Blocking.
    pub fn remove(&self) -> Result<(), String> {
        if self.provider == "claude" {
            crate::rate_limits::delete_claude_credentials(&self.dir)?;
        }
        let failed = |err: std::io::Error| {
            format!(
                "Could not remove the {} account directory {}: {err}",
                self.provider,
                self.dir.display()
            )
        };
        let metadata = match std::fs::symlink_metadata(&self.dir) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(failed(err)),
        };
        if metadata.is_dir() {
            std::fs::remove_dir_all(&self.dir)
        } else {
            std::fs::remove_file(&self.dir)
        }
        .map_err(failed)
    }

    /// `apply` for the harness runtime's commands.
    pub fn apply_async(&self, command: &mut tokio::process::Command) {
        self.apply(command.as_std_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, provider: &str, label: &str) -> ProviderAccount {
        ProviderAccount {
            id: id.into(),
            provider: provider.into(),
            label: label.into(),
        }
    }

    #[test]
    fn default_profile_leads_and_duplicates_are_dropped() {
        let mut stored = StoredAccounts::new();
        stored.insert(
            "claude".into(),
            vec![
                account("account-1", "claude", "  Work   laptop "),
                account("bad id", "claude", "Nope"),
                account("account-2", "claude", "   "),
            ],
        );
        let shared = [
            account("account-1", "claude", "MonoCode's name"),
            account("account-3", "claude", "Personal"),
            account("account-9", "codex", "Other provider"),
        ];
        let accounts = provider_accounts("claude", &stored, &shared);
        let labels: Vec<_> = accounts.iter().map(|a| (a.id.as_str(), a.label.as_str())).collect();
        assert_eq!(
            labels,
            [
                ("default", "Default account"),
                ("account-1", "Work laptop"),
                ("account-3", "Personal")
            ]
        );
        assert_eq!(provider_accounts("codex", &stored, &shared).len(), 2);
    }

    #[test]
    fn a_stored_default_entry_renames_the_default_profile() {
        let mut stored = StoredAccounts::new();
        stored.insert("codex".into(), vec![account("default", "codex", "Main")]);
        let accounts = provider_accounts("codex", &stored, &[]);
        assert_eq!(accounts, [account("default", "codex", "Main")]);
    }

    #[test]
    fn new_accounts_get_a_uuid_id_and_a_fallback_name() {
        let named = new_account("claude", " Work ", 1);
        assert_eq!(named.label, "Work");
        assert!(named.id.starts_with("account-") && named.id.len() == 44, "{}", named.id);
        assert!(valid_account_id(&named.id));
        assert_ne!(named.id, new_account("claude", "Work", 1).id);
        assert_eq!(new_account("claude", "  ", 2).label, "Account 3");
        assert_eq!(clean_label(&"x".repeat(60)).len(), 48);
    }

    #[test]
    fn selection_falls_back_to_the_default_profile() {
        let accounts = [account("default", "claude", "Default account"), account("a1", "claude", "Work")];
        let mut selections = StoredSelections::new();
        selections
            .entry("/repo".into())
            .or_default()
            .insert("claude".into(), "a1".into());
        assert_eq!(selected_account_id(&selections, "/repo", "claude", &accounts), "a1");
        assert_eq!(selected_account_id(&selections, "/other", "claude", &accounts), "default");
        assert_eq!(selected_account_id(&selections, "/repo", "codex", &accounts), "default");
        // The remembered account was removed.
        assert_eq!(selected_account_id(&selections, "/repo", "claude", &accounts[..1]), "default");
    }

    #[test]
    fn renaming_names_the_account_in_this_apps_list() {
        let mut stored = StoredAccounts::new();
        stored.insert("claude".into(), vec![account("a1", "claude", "Work")]);
        assert!(rename_account(&mut stored, &account("a1", "claude", "Work"), "  Day   job "));
        // MonoCode's own account and the default profile get an entry.
        assert!(rename_account(&mut stored, &account("m1", "claude", "MonoCode's"), "Mine"));
        assert!(rename_account(&mut stored, &account("default", "claude", "Default account"), "Main"));
        assert!(!rename_account(&mut stored, &account("a1", "claude", "Work"), "   "));
        let shared = [account("m1", "claude", "MonoCode's")];
        let labels: Vec<_> = provider_accounts("claude", &stored, &shared)
            .into_iter()
            .map(|a| a.label)
            .collect();
        assert_eq!(labels, ["Main", "Day job", "Mine"]);
    }

    #[test]
    fn removing_drops_the_account_and_its_selections() {
        let mut stored = StoredAccounts::new();
        stored.insert(
            "claude".into(),
            vec![account("a1", "claude", "Work"), account("a2", "claude", "Home")],
        );
        stored.insert("codex".into(), vec![account("a1", "codex", "Same id")]);
        let mut selections = StoredSelections::new();
        selections.insert("/one".into(), [("claude".to_string(), "a1".to_string())].into());
        selections.insert(
            "/two".into(),
            [("claude".to_string(), "a1".to_string()), ("codex".to_string(), "a1".to_string())].into(),
        );
        selections.insert("/three".into(), [("claude".to_string(), "a2".to_string())].into());
        remove_account(&mut stored, &mut selections, "claude", "a1");
        assert_eq!(stored["claude"], [account("a2", "claude", "Home")]);
        assert_eq!(stored["codex"].len(), 1);
        assert!(!selections.contains_key("/one"));
        assert_eq!(selections["/two"], [("codex".to_string(), "a1".to_string())].into());
        assert_eq!(selections["/three"]["claude"], "a2");
        remove_account(&mut stored, &mut selections, "claude", "a2");
        assert!(!stored.contains_key("claude"));
        remove_account(&mut stored, &mut selections, "codex", "default");
        assert_eq!(stored["codex"].len(), 1);
    }

    #[test]
    fn removing_a_profile_deletes_its_directory() {
        let dir = std::env::temp_dir().join(format!(
            "bencode-account-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("sessions")).unwrap();
        std::fs::write(dir.join("auth.json"), "{}").unwrap();
        let profile = AccountProfile {
            provider: "codex",
            dir: dir.clone(),
        };
        profile.remove().unwrap();
        assert!(!dir.exists());
        // Already gone is fine.
        profile.remove().unwrap();
    }

    #[test]
    fn profiles_resolve_only_for_real_account_ids() {
        assert!(AccountProfile::resolve("claude", None).is_none());
        assert!(AccountProfile::resolve("claude", Some("default")).is_none());
        assert!(AccountProfile::resolve("claude", Some("../escape")).is_none());
        assert!(AccountProfile::resolve("opencode", Some("account-1")).is_none());
        let profile = AccountProfile::resolve("codex", Some("account-1")).unwrap();
        assert!(profile.dir.ends_with("provider-accounts/codex/account-1"));
        assert_eq!(profile.env().0, ["CODEX_HOME"]);
        let claude = AccountProfile::resolve("claude", Some("account-1")).unwrap();
        assert_eq!(claude.env().0, ["CLAUDE_CONFIG_DIR", "CLAUDE_SECURESTORAGE_CONFIG_DIR"]);
        assert!(claude.env().1.contains(&"ANTHROPIC_API_KEY"));
    }
}
