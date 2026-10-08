//! Claude Code's 5-hour and weekly usage (MonoCode `fetch_claude_usage` in
//! `src-tauri/src/rate_limits.rs`): the CLI's own OAuth token, read from the
//! Keychain or `~/.claude/.credentials.json`, asks Anthropic's usage
//! endpoint. The token is only read, never refreshed or stored.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::{Fetched, http, parse_claude_oauth_usage};
use crate::harness::accounts::AccountProfile;

const OAUTH_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const USER_AGENT: &str = "claude-code/2.1.0";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

struct Credentials {
    access_token: String,
    expires_at_ms: Option<i64>,
    has_refresh_token: bool,
}

pub fn fetch(account: Option<&AccountProfile>, now: i64) -> Fetched {
    let Some(creds) = read_credentials(account.map(|account| account.dir.as_path())) else {
        return Fetched::Unavailable("Claude not signed in".into());
    };
    // Claude Code owns this credential and rotates its refresh token. The
    // usage footer must remain read-only: independently refreshing here can
    // race a live CLI (or MonoCode) and leave one process with a spent
    // refresh token, which forces the user through sign-in again.
    if token_expired(creds.expires_at_ms, now) {
        return Fetched::Error(expired_token_error(creds.has_refresh_token));
    }
    let response = http::get(
        OAUTH_USAGE_URL,
        &[
            ("Authorization", &format!("Bearer {}", creds.access_token)),
            ("anthropic-beta", OAUTH_BETA),
            ("User-Agent", USER_AGENT),
        ],
        HTTP_TIMEOUT,
    );
    match response {
        Ok(response) if (200..300).contains(&response.status) => {
            match parse_claude_oauth_usage(&response.body, now) {
                Ok(limits) => Fetched::Limits(limits),
                Err(error) => Fetched::Error(error),
            }
        }
        Ok(response) => Fetched::Error(status_error(response.status)),
        Err(error) => Fetched::Error(format!("Claude usage request failed: {error:#}")),
    }
}

fn status_error(status: u16) -> String {
    match status {
        401 => "Claude sign-in expired".into(),
        403 => "Claude usage is unavailable for this account".into(),
        status => format!("Claude usage request failed ({status})"),
    }
}

/// An expired access token the CLI can still refresh is not a lost sign-in:
/// the account's next turn renews it, so the footer must not offer to sign
/// in (the text avoids the words `needs_provider_login` looks for).
fn expired_token_error(has_refresh_token: bool) -> String {
    if has_refresh_token {
        "Claude token renews on this account's next turn".into()
    } else {
        status_error(401)
    }
}

/// `config_dir` is an account profile's `CLAUDE_CONFIG_DIR`; None reads the
/// default profile.
fn read_credentials(config_dir: Option<&Path>) -> Option<Credentials> {
    #[cfg(target_os = "macos")]
    if let Some(creds) = keychain::read(config_dir) {
        return Some(creds);
    }
    let dir = match config_dir {
        Some(dir) => dir.to_path_buf(),
        None => std::env::var_os("HOME").map(PathBuf::from)?.join(".claude"),
    };
    let raw = std::fs::read_to_string(dir.join(".credentials.json")).ok()?;
    credentials_from_blob(&raw)
}

fn credentials_from_blob(raw: &str) -> Option<Credentials> {
    let blob: Value = serde_json::from_str(raw.trim()).ok()?;
    let oauth = blob.get("claudeAiOauth");
    let field = |key: &str| oauth.and_then(|oauth| oauth.get(key)).or_else(|| blob.get(key));
    let access_token = field("accessToken")?.as_str()?.trim();
    if access_token.is_empty() {
        return None;
    }
    let expires_at_ms = field("expiresAt").and_then(|value| match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().filter(|f| f.is_finite()).map(|f| f as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    });
    let has_refresh_token = field("refreshToken")
        .and_then(Value::as_str)
        .is_some_and(|token| !token.trim().is_empty());
    Some(Credentials {
        access_token: access_token.to_string(),
        expires_at_ms,
        has_refresh_token,
    })
}

/// An unknown expiry is treated as usable: the usage request itself will 401
/// if it is not, which produces the same user-facing result without mutating
/// credentials owned by another process.
fn token_expired(expires_at_ms: Option<i64>, now_ms: i64) -> bool {
    expires_at_ms.is_some_and(|expires| now_ms >= expires)
}

#[cfg(target_os = "macos")]
mod keychain {
    use std::path::Path;

    use sha2::{Digest, Sha256};

    use super::{Credentials, credentials_from_blob};

    const SERVICE: &str = "Claude Code-credentials";
    const FALLBACK_USER: &str = "claude-code-user";

    /// Claude Code has stored the item under no account, the login name and
    /// a fixed fallback over time; the first that parses wins.
    pub fn read(config_dir: Option<&Path>) -> Option<Credentials> {
        let service = service(config_dir);
        let accounts = [None, Some(user()), Some(FALLBACK_USER.to_string())];
        accounts.iter().find_map(|account| {
            let mut args = vec!["find-generic-password", "-s", service.as_str()];
            if let Some(account) = account {
                args.extend(["-a", account]);
            }
            args.push("-w");
            credentials_from_blob(&security(&args)?)
        })
    }

    /// The Keychain item of a profile. Claude Code hashes the exact config
    /// dir string (NFC-normalized; BenCode's profile paths are ASCII under
    /// an ASCII home, where that changes nothing) and suffixes the first
    /// eight hex characters.
    pub(super) fn service(config_dir: Option<&Path>) -> String {
        let Some(config_dir) = config_dir else {
            return SERVICE.into();
        };
        let digest = Sha256::digest(config_dir.to_string_lossy().as_bytes());
        let suffix: String = digest.iter().take(4).map(|byte| format!("{byte:02x}")).collect();
        format!("{SERVICE}-{suffix}")
    }

    fn user() -> String {
        let user = std::env::var("USER").unwrap_or_default();
        let plain = user
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
        if plain && !user.is_empty() {
            user
        } else {
            FALLBACK_USER.into()
        }
    }

    /// Deletes a profile's Keychain item (MonoCode
    /// `delete_claude_keychain_credentials`); one already gone is fine.
    pub fn delete(config_dir: &Path) -> Result<(), String> {
        let service = service(Some(config_dir));
        let failed = |detail: String| {
            format!("Could not remove the Claude credentials from Keychain ({service}): {detail}")
        };
        let output =
            crate::keychain::run(&["delete-generic-password", "-s", service.as_str()]).map_err(failed)?;
        if output.ok || crate::keychain::not_found(&output) {
            Ok(())
        } else {
            Err(failed(output.stderr))
        }
    }

    /// `security`'s stdout, or None when it fails or hangs on a locked
    /// Keychain past the timeout.
    fn security(args: &[&str]) -> Option<String> {
        let output = crate::keychain::run(args).ok()?;
        (output.ok && !output.stdout.is_empty()).then_some(output.stdout)
    }
}

/// Deletes the Claude sign-in an account profile keeps in the Keychain;
/// elsewhere it is `.credentials.json`, which goes with the directory.
/// Blocking.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn delete_credentials(config_dir: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    keychain::delete(config_dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rate_limits::{ProviderRateLimits, needs_provider_login};

    #[test]
    fn credentials_read_nested_and_flat_blobs() {
        let nested = credentials_from_blob(
            r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat-abc","expiresAt":1800000000000}}"#,
        )
        .unwrap();
        assert_eq!(nested.access_token, "sk-ant-oat-abc");
        assert_eq!(nested.expires_at_ms, Some(1_800_000_000_000));

        let flat = credentials_from_blob(r#"{"accessToken":"token-1","expiresAt":"42"}"#).unwrap();
        assert_eq!(flat.access_token, "token-1");
        assert_eq!(flat.expires_at_ms, Some(42));

        assert!(credentials_from_blob(r#"{"claudeAiOauth":{"accessToken":"  "}}"#).is_none());
        assert!(credentials_from_blob("not json").is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn custom_config_dir_selects_claudes_hashed_keychain_service() {
        assert_eq!(
            keychain::service(Some(Path::new("/tmp/profile"))),
            "Claude Code-credentials-902e721c"
        );
        assert_eq!(keychain::service(None), "Claude Code-credentials");
    }

    #[test]
    fn token_expired_uses_actual_expiry() {
        let now = 1_000_000;
        assert!(!token_expired(Some(now + 1), now));
        assert!(token_expired(Some(now), now));
        assert!(token_expired(Some(now - 1), now));
        assert!(!token_expired(None, now));
    }

    /// Reads the real Keychain item and calls Anthropic; run by hand.
    #[test]
    #[ignore]
    fn live_usage_round_trip() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let fetched = fetch(None, now);
        println!("{fetched:?}");
        assert!(matches!(fetched, Fetched::Limits(_)), "{fetched:?}");
    }

    #[test]
    fn expired_token_with_refresh_token_does_not_ask_to_sign_in() {
        let creds = credentials_from_blob(
            r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"r","expiresAt":1}}"#,
        )
        .unwrap();
        assert!(creds.has_refresh_token);
        let pending = ProviderRateLimits::error(expired_token_error(true), None, 0);
        assert!(!needs_provider_login(&pending));

        let creds = credentials_from_blob(r#"{"accessToken":"a","expiresAt":1}"#).unwrap();
        assert!(!creds.has_refresh_token);
        assert_eq!(expired_token_error(false), "Claude sign-in expired");
    }

    #[test]
    fn status_errors_name_the_cause() {
        assert_eq!(status_error(401), "Claude sign-in expired");
        assert_eq!(status_error(403), "Claude usage is unavailable for this account");
        assert_eq!(status_error(500), "Claude usage request failed (500)");
    }
}
