//! Antigravity accounts (BenCode's own; MonoCode has none). `agy` keeps
//! its one sign-in in the macOS Keychain (service `gemini`, account
//! `antigravity`) and has no setting that points it at another, so an
//! account here is a saved copy of that item, and switching writes another
//! copy back. The switch is for the whole machine, not for one thread.
//! Ports the user's `agy-save` / `agy-switch` scripts, whose profiles in
//! `~/.gemini/profiles` are copied in once. Blocking (Keychain and files):
//! call everything here on a background executor.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde_json::Value;

use crate::harness::accounts::{self, ProviderAccount, valid_account_id};

pub const PROVIDER: &str = "antigravity";

const SERVICE: &str = "gemini";
const ACCOUNT: &str = "antigravity";
/// The Keychain item as `go-keyring` stores it, one file per profile.
const TOKEN_FILE: &str = "keychain_token";
const KEYRING_PREFIX: &str = "go-keyring-base64:";
/// What the scripts also kept in step with the Keychain; `agy` itself
/// reads the Keychain, so it is only refreshed when it already exists.
const JETSKI_TOKEN: &str = ".gemini/jetski-standalone-oauth-token";
const SCRIPT_PROFILES: &str = ".gemini/profiles";

/// A saved account: its id (the folder name) and who it signs in as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgyProfile {
    pub id: String,
    pub email: Option<String>,
}

/// What the Accounts page shows: the saved accounts, who `agy` is signed
/// in as now, and the script profiles just copied in (to name them).
#[derive(Debug, Default)]
pub struct Snapshot {
    pub profiles: Vec<AgyProfile>,
    pub live_email: Option<String>,
    pub imported: Vec<ProviderAccount>,
}

/// Where a sign-in started from: the item it took out of the Keychain, to
/// put back if the sign-in is cancelled.
#[derive(Debug, Default)]
pub struct SignInStart {
    pub previous: Option<String>,
}

/// The JSON a Keychain item holds (`go-keyring-base64:` + base64 JSON, or
/// the JSON itself).
fn decode_item(item: &str) -> Option<Value> {
    let item = item.trim();
    let json = match item.strip_prefix(KEYRING_PREFIX) {
        Some(encoded) => String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()?,
        )
        .ok()?,
        None => item.to_string(),
    };
    serde_json::from_str(&json).ok()
}

/// The email in an item's `id_token`.
pub fn email_of(item: &str) -> Option<String> {
    let token = decode_item(item)?;
    let payload = token.get("id_token")?.as_str()?.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("email")?
        .as_str()
        .map(str::trim)
        .filter(|email| !email.is_empty())
        .map(String::from)
}

/// The saved accounts signed in as `email`.
fn matching<'a>(profiles: &'a [AgyProfile], email: &str) -> impl Iterator<Item = &'a AgyProfile> {
    profiles.iter().filter(move |profile| {
        profile
            .email
            .as_deref()
            .is_some_and(|known| known.eq_ignore_ascii_case(email))
    })
}

/// The saved accounts, on disk under `root`.
struct Store {
    root: PathBuf,
}

impl Store {
    fn open() -> Result<Self, String> {
        let root = crate::storage::provider_accounts_dir()
            .ok_or("BenCode has no data folder for accounts.")?
            .join(PROVIDER);
        Ok(Self { root })
    }

    fn token_path(&self, id: &str) -> PathBuf {
        self.root.join(id).join(TOKEN_FILE)
    }

    fn list(&self) -> Vec<AgyProfile> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut profiles: Vec<AgyProfile> = entries
            .flatten()
            .filter_map(|entry| {
                let id = entry.file_name().to_string_lossy().into_owned();
                let item = std::fs::read_to_string(self.token_path(&id)).ok();
                (valid_account_id(&id) && item.is_some()).then(|| AgyProfile {
                    email: item.as_deref().and_then(email_of),
                    id,
                })
            })
            .collect();
        profiles.sort_by(|a, b| a.id.cmp(&b.id));
        profiles
    }

    fn read(&self, id: &str) -> Result<String, String> {
        std::fs::read_to_string(self.token_path(id))
            .map(|item| item.trim().to_string())
            .map_err(|err| format!("Could not read the saved Antigravity account: {err}"))
    }

    fn write(&self, id: &str, item: &str) -> Result<(), String> {
        if !valid_account_id(id) {
            return Err(format!("Not an account id: {id}"));
        }
        write_private(&self.token_path(id), item)
            .map_err(|err| format!("Could not save the Antigravity account: {err}"))
    }

    fn remove(&self, id: &str) -> Result<(), String> {
        if !valid_account_id(id) {
            return Ok(());
        }
        match std::fs::remove_dir_all(self.root.join(id)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("Could not remove the Antigravity account: {err}")),
        }
    }

    /// Saves `item` over every account signed in as the same person, or
    /// as a new account `new_id` when there is none. The account it went
    /// to, and whether it is new.
    fn keep(&self, item: &str, new_id: &str) -> Result<(AgyProfile, bool), String> {
        // Without an email a refreshed token could not be told from another
        // person's, and every read would save one more account.
        let email = email_of(item).ok_or("The Antigravity sign-in does not say whose it is.")?;
        let profiles = self.list();
        let known: Vec<AgyProfile> = matching(&profiles, &email).cloned().collect();
        for profile in &known {
            if self.read(&profile.id).ok().as_deref() != Some(item.trim()) {
                self.write(&profile.id, item)?;
            }
        }
        if let Some(first) = known.into_iter().next() {
            return Ok((first, false));
        }
        self.write(new_id, item)?;
        Ok((
            AgyProfile {
                id: new_id.to_string(),
                email: Some(email),
            },
            true,
        ))
    }

    /// The profiles `agy-save` left in `dir`, copied in under new ids and
    /// named for their folders. One per person: a second copy is skipped.
    fn import_from(&self, dir: &Path) -> Vec<ProviderAccount> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut folders: Vec<(String, String)> = entries
            .flatten()
            .filter_map(|entry| {
                let item = std::fs::read_to_string(entry.path().join(TOKEN_FILE)).ok()?;
                Some((
                    entry.file_name().to_string_lossy().into_owned(),
                    item.trim().to_string(),
                ))
            })
            .collect();
        // `agy-save` users keep a `current` copy of another profile; the
        // named one wins.
        folders.sort_by(|(a, _), (b, _)| (a == "current", a).cmp(&(b == "current", b)));
        let mut seen = HashSet::new();
        let mut imported = Vec::new();
        for (name, item) in folders {
            let Some(email) = email_of(&item) else {
                continue;
            };
            if !seen.insert(email.to_lowercase()) {
                continue;
            }
            let account = accounts::new_account(PROVIDER, &name, imported.len());
            match self.write(&account.id, &item) {
                Ok(()) => imported.push(account),
                Err(err) => log::warn!("importing Antigravity profile {name}: {err}"),
            }
        }
        imported
    }
}

/// Writes `contents` readable by the user only, through a temporary file
/// so a crash cannot leave half a token.
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp, path)
}

/// The item `agy` signs in with now; None when it is signed out.
fn read_live() -> Result<Option<String>, String> {
    let output =
        crate::keychain::run(&["find-generic-password", "-s", SERVICE, "-a", ACCOUNT, "-w"])
            .map_err(|err| format!("Could not read the Antigravity sign-in: {err}"))?;
    if output.ok && !output.stdout.is_empty() {
        Ok(Some(output.stdout))
    } else if crate::keychain::not_found(&output) || output.ok {
        Ok(None)
    } else {
        Err(format!(
            "Could not read the Antigravity sign-in: {}",
            output.stderr
        ))
    }
}

/// The access token of `agy`'s sign-in and when it stops working.
pub struct AccessToken {
    pub access_token: String,
    /// Unix ms; None when the item does not say.
    pub expires_at: Option<i64>,
}

/// The token `agy` calls its API with now; None when it is signed out.
/// `agy` refreshes it as it runs, nothing here does.
pub fn live_access_token() -> Result<Option<AccessToken>, String> {
    let Some(item) = read_live()? else {
        return Ok(None);
    };
    Ok(access_token_of(&item))
}

fn access_token_of(item: &str) -> Option<AccessToken> {
    let json = decode_item(item)?;
    let token = json.get("token")?;
    let access_token = token
        .get("access_token")?
        .as_str()
        .filter(|t| !t.is_empty())?
        .to_string();
    let expires_at = token
        .get("expiry")
        .and_then(Value::as_str)
        .and_then(|expiry| expiry.parse::<jiff::Timestamp>().ok())
        .map(|expiry| expiry.as_millisecond());
    Some(AccessToken {
        access_token,
        expires_at,
    })
}

/// Makes `item` the one `agy` signs in with.
fn write_live(item: &str) -> Result<(), String> {
    let item = item.trim();
    // `security -i` splits its commands on whitespace and quotes; a
    // go-keyring item is base64 and never holds either.
    if item.is_empty()
        || item
            .chars()
            .any(|ch| ch.is_whitespace() || ch == '"' || ch == '\\')
    {
        return Err("The saved Antigravity account is not a Keychain item.".into());
    }
    let command = format!("add-generic-password -U -s {SERVICE} -a {ACCOUNT} -w \"{item}\"\n");
    let output = crate::keychain::run_with_input(&["-i"], Some(&command))
        .map_err(|err| format!("Could not switch the Antigravity account: {err}"))?;
    if !output.ok || !output.stderr.is_empty() {
        return Err(format!(
            "Could not switch the Antigravity account: {}",
            output.stderr
        ));
    }
    sync_jetski_token(item);
    Ok(())
}

/// Signs `agy` out, so it asks for a sign-in on its next start.
fn delete_live() -> Result<(), String> {
    let output = crate::keychain::run(&["delete-generic-password", "-s", SERVICE, "-a", ACCOUNT])
        .map_err(|err| format!("Could not sign Antigravity out: {err}"))?;
    if output.ok || crate::keychain::not_found(&output) {
        Ok(())
    } else {
        Err(format!("Could not sign Antigravity out: {}", output.stderr))
    }
}

/// Keeps `~/.gemini/jetski-standalone-oauth-token` (the decoded item) in
/// step, as `agy-switch` did, when something already keeps one.
fn sync_jetski_token(item: &str) {
    let Some(path) = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(JETSKI_TOKEN))
    else {
        return;
    };
    if !path.exists() {
        return;
    }
    let Some(json) = decode_item(item) else {
        return;
    };
    if let Err(err) = write_private(&path, &json.to_string()) {
        log::warn!("could not update {}: {err}", path.display());
    }
}

/// The saved accounts and who `agy` is signed in as, saving that sign-in
/// (its newest token, or as a new account). The first read also copies in
/// the profiles `agy-save` kept.
pub fn load() -> Result<Snapshot, String> {
    let store = Store::open()?;
    let mut imported = Vec::new();
    if !store.root.exists() {
        if let Some(home) = std::env::var_os("HOME") {
            imported = store.import_from(&PathBuf::from(home).join(SCRIPT_PROFILES));
        }
        if let Err(err) = std::fs::create_dir_all(&store.root) {
            log::warn!("could not create {}: {err}", store.root.display());
        }
    }
    // The account `agy` is signed in as is always saved, so a switch away
    // from it can come back.
    let live_email = match read_live() {
        Ok(Some(live)) => {
            let new_id = accounts::new_account(PROVIDER, "", 0).id;
            if let Err(err) = store.keep(&live, &new_id) {
                log::warn!("{err}");
            }
            email_of(&live)
        }
        Ok(None) => None,
        Err(err) => {
            log::warn!("{err}");
            None
        }
    };
    Ok(Snapshot {
        profiles: store.list(),
        live_email,
        imported,
    })
}

/// `agy-switch`: saves the sign-in `agy` has now back into its account
/// (it refreshes its token as it runs), then signs `agy` in as `id`.
pub fn activate(id: &str) -> Result<(), String> {
    let store = Store::open()?;
    if let Some(live) = read_live()? {
        let profiles = store.list();
        if let Some(email) = email_of(&live) {
            for profile in matching(&profiles, &email) {
                store.write(&profile.id, &live)?;
            }
        }
    }
    // Read after the save above: switching to the live account keeps its
    // newest token.
    write_live(&store.read(id)?)
}

/// Before a sign-in: keeps the sign-in `agy` has now (as `new_id` when no
/// account has it), then signs `agy` out so it asks for another.
pub fn begin_sign_in(new_id: &str) -> Result<SignInStart, String> {
    let store = Store::open()?;
    let mut start = SignInStart::default();
    if let Some(live) = read_live()? {
        store.keep(&live, new_id)?;
        start.previous = Some(live);
    }
    delete_live()?;
    Ok(start)
}

/// During a sign-in: the account it ended in, saved (over the one already
/// signed in as that person, or as `new_id`), once `agy` has one.
pub fn finish_sign_in(new_id: &str) -> Result<Option<(AgyProfile, bool)>, String> {
    let Some(item) = read_live()? else {
        return Ok(None);
    };
    Store::open()?.keep(&item, new_id).map(Some)
}

/// A cancelled sign-in: puts back what `agy` was signed in as, unless it
/// has signed in since.
pub fn cancel_sign_in(previous: Option<&str>) -> Result<(), String> {
    match (read_live()?, previous) {
        (None, Some(previous)) => write_live(previous),
        _ => Ok(()),
    }
}

/// Forgets a saved account. `agy` stays signed in as whoever it is.
pub fn remove(id: &str) -> Result<(), String> {
    Store::open()?.remove(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(email: &str, refresh: &str) -> String {
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::json!({ "email": email }).to_string());
        let json = serde_json::json!({
            "token": { "access_token": "a", "refresh_token": refresh },
            "id_token": format!("header.{claims}.sig"),
        });
        format!(
            "{KEYRING_PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(json.to_string())
        )
    }

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bencode-agy-{tag}-{}", std::process::id()));
        if let Err(err) = std::fs::remove_dir_all(&dir) {
            assert_eq!(err.kind(), std::io::ErrorKind::NotFound, "{err}");
        }
        dir
    }

    #[test]
    fn reads_the_email_of_go_keyring_and_plain_items() {
        let saved = item("me@example.com", "r1");
        assert_eq!(email_of(&saved).as_deref(), Some("me@example.com"));
        let plain = decode_item(&saved).unwrap().to_string();
        assert_eq!(email_of(&plain).as_deref(), Some("me@example.com"));
        assert_eq!(email_of("go-keyring-base64:not base64"), None);
        assert_eq!(email_of(""), None);
    }

    #[test]
    fn reads_the_access_token_and_its_expiry() {
        let json = serde_json::json!({
            "token": { "access_token": "ya29.x", "expiry": "2026-10-08T16:10:48.882+07:00" },
        });
        let token = access_token_of(&json.to_string()).unwrap();
        assert_eq!(token.access_token, "ya29.x");
        assert_eq!(token.expires_at, Some(1_791_450_648_882));
        assert!(access_token_of("{}").is_none());
    }

    #[test]
    fn keeping_an_item_updates_the_same_person_or_adds_one() {
        let store = Store {
            root: temp_root("keep"),
        };
        let (first, created) = store.keep(&item("a@x.com", "r1"), "account-1").unwrap();
        assert!(created);
        assert_eq!(first.email.as_deref(), Some("a@x.com"));
        // A refreshed token for the same person lands on the same account.
        let (again, created) = store.keep(&item("A@x.com", "r2"), "account-2").unwrap();
        assert!(!created);
        assert_eq!(again.id, "account-1");
        assert_eq!(store.read("account-1").unwrap(), item("A@x.com", "r2"));
        let (other, created) = store.keep(&item("b@x.com", "r3"), "account-3").unwrap();
        assert!(created);
        assert_eq!(other.id, "account-3");
        let ids: Vec<_> = store.list().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["account-1", "account-3"]);
        assert!(store.keep("{}", "account-4").is_err());
        store.remove("account-3").unwrap();
        store.remove("account-3").unwrap();
        assert_eq!(store.list().len(), 1);
        std::fs::remove_dir_all(&store.root).unwrap();
    }

    #[test]
    fn script_profiles_import_once_per_person() {
        let scripts = temp_root("scripts");
        for (name, email) in [
            ("work", "w@x.com"),
            ("current", "w@x.com"),
            ("home", "h@x.com"),
            ("broken", ""),
        ] {
            let dir = scripts.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let token = if email.is_empty() {
                "nope".to_string()
            } else {
                item(email, name)
            };
            std::fs::write(dir.join(TOKEN_FILE), format!("{token}\n")).unwrap();
        }
        let store = Store {
            root: temp_root("import"),
        };
        let imported = store.import_from(&scripts);
        let labels: Vec<_> = imported.iter().map(|a| a.label.as_str()).collect();
        // `current` duplicates `work`, which keeps its name.
        assert_eq!(labels, ["home", "work"]);
        assert!(imported.iter().all(|a| a.provider == PROVIDER));
        assert_eq!(store.list().len(), 2);
        std::fs::remove_dir_all(&scripts).unwrap();
        std::fs::remove_dir_all(&store.root).unwrap();
    }
}
