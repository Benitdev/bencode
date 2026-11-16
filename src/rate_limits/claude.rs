//! Claude Code's 5-hour and weekly usage (MonoCode `fetch_claude_usage` in
//! `src-tauri/src/rate_limits.rs`): the CLI's own OAuth token, read from the
//! Keychain or `~/.claude/.credentials.json`, asks Anthropic's usage
//! endpoint. The token is only read, never refreshed or stored.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::{Fetched, http, parse_claude_oauth_usage};
use crate::harness::accounts::AccountProfile;

const OAUTH_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage?cedar_ember=1&skip_spend=1";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const USER_AGENT: &str = "claude-cli/2.1.325 (external, cli)";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const CLAIM_TIMEOUT: Duration = Duration::from_secs(25);
const CEDAR_EMBER_PROGRAM: &str = "cedar_ember";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimResetResult {
    pub cleared: Vec<String>,
    pub resets_left: Option<u32>,
}

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
    // usage chip must remain read-only: independently refreshing here can
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

/// Claims a free rate-limit reset grant (`cedar_ember`) for `account`.
pub fn claim_reset(
    account: Option<&AccountProfile>,
    grant_id: &str,
    request_id: &str,
    now: i64,
) -> Result<ClaimResetResult, String> {
    if !valid_id(grant_id, 40) || !valid_id(request_id, 64) {
        return Err("Invalid reset grant or request identifier".into());
    }
    let dir = account.map(|account| account.dir.as_path());
    let Some(creds) = read_credentials(dir) else {
        return Err("Claude not signed in".into());
    };
    if token_expired(creds.expires_at_ms, now) {
        return Err(expired_token_error(creds.has_refresh_token));
    }
    let Some(org_uuid) = read_organization_uuid(dir) else {
        return Err("Claude organization ID not found; run a turn or sign in again".into());
    };
    let url = format!("https://api.anthropic.com/api/organizations/{org_uuid}/reset_rate_limits");
    let body = serde_json::json!({
        "program": CEDAR_EMBER_PROGRAM,
        "grant_id": grant_id,
        "request_id": request_id,
    })
    .to_string();
    let response = http::send(
        &url,
        &[
            ("Authorization", &format!("Bearer {}", creds.access_token)),
            ("anthropic-beta", OAUTH_BETA),
            ("Content-Type", "application/json"),
            ("User-Agent", USER_AGENT),
        ],
        http::Send {
            method: Some("POST"),
            body: Some(&body),
            ..Default::default()
        },
        CLAIM_TIMEOUT,
    )
    .map_err(|error| format!("Claude reset request failed: {error:#}"))?;
    if !(200..300).contains(&response.status) {
        return Err(match response.status {
            429 => "Too many reset attempts; try again in a minute".into(),
            status => status_error(status),
        });
    }
    parse_claim_reset_response(&response.body)
}

fn parse_claim_reset_response(body: &str) -> Result<ClaimResetResult, String> {
    let parsed: Value = serde_json::from_str(body)
        .map_err(|_| "Claude reset response was not JSON".to_string())?;
    let result = parsed
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    if result == "reset" {
        let cleared = parsed
            .get("cleared")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect();
        let resets_left = parsed
            .get("resets_left")
            .and_then(Value::as_u64)
            .map(|n| n as u32);
        return Ok(ClaimResetResult {
            cleared,
            resets_left,
        });
    }
    let reason = parsed.get("reason").and_then(Value::as_str);
    Err(match (result, reason) {
        ("already_used", _) => "This reset has already been used".into(),
        ("cooldown", _) | (_, Some("cooldown")) => {
            "Try again in a minute before using another reset".into()
        }
        ("not_limited", _) | (_, Some("needs_limit" | "not_limited")) => {
            "This reset can only be used while you're at a limit it refills".into()
        }
        (_, Some("paused")) => "This reset is currently paused".into(),
        (_, Some("expired")) => "This reset has expired".into(),
        (_, Some("blocked_five_hour")) => {
            "Can't be used while you're at your 5-hour limit".into()
        }
        (_, Some("blocked_seven_day")) => {
            "Can't be used while you're at your weekly limit".into()
        }
        (_, Some("blocked_opus")) => "Can't be used while you're at your Opus limit".into(),
        (_, Some("blocked_sonnet")) => {
            "Can't be used while you're at your Sonnet limit".into()
        }
        (_, Some("blocked_other")) => {
            "Can't be used while you're at another limit it doesn't refill".into()
        }
        (_, Some("cannot_clear")) => "Doesn't refill the limit you've reached".into(),
        _ => "Couldn't reset your limits right now; try again in a moment".into(),
    })
}

fn valid_id(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn read_organization_uuid(config_dir: Option<&Path>) -> Option<String> {
    let path = match config_dir {
        Some(dir) => dir.join(".claude.json"),
        None => std::env::var_os("HOME")
            .map(PathBuf::from)?
            .join(".claude.json"),
    };
    let raw = std::fs::read_to_string(path).ok()?;
    let config: Value = serde_json::from_str(&raw).ok()?;
    let uuid = config
        .get("oauthAccount")?
        .get("organizationUuid")?
        .as_str()?
        .trim();
    valid_id(uuid, 128).then(|| uuid.to_string())
}

fn status_error(status: u16) -> String {
    match status {
        401 => "Claude sign-in expired".into(),
        403 => "Claude usage is unavailable for this account".into(),
        status => format!("Claude usage request failed ({status})"),
    }
}

/// An expired access token the CLI can still refresh is not a lost sign-in:
/// the account's next turn renews it, so the chip must not offer to sign
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
    // A Keychain item there is the sign-in, even blank: Claude Code blanks
    // it when a refresh fails, and an old `.credentials.json` beside it
    // would promise a renewal and hide Sign in (MonoCode `2ec5b66c`).
    #[cfg(target_os = "macos")]
    if let Some(item) = keychain::read(config_dir) {
        return item;
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
    let field = |key: &str| {
        oauth
            .and_then(|oauth| oauth.get(key))
            .or_else(|| blob.get(key))
    };
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

    use super::{Credentials, credentials_from_blob};

    const FALLBACK_USER: &str = "claude-code-user";

    /// Claude Code has stored the item under no account, the login name and
    /// a fixed fallback over time; the first that parses wins. `None` when
    /// there is no item; `Some(None)` when there is one, but blank or
    /// unreadable as a sign-in.
    pub fn read(config_dir: Option<&Path>) -> Option<Option<Credentials>> {
        let service = crate::keychain::claude_service(config_dir);
        let accounts = [None, Some(user()), Some(FALLBACK_USER.to_string())];
        let mut found = false;
        for account in &accounts {
            let mut args = vec!["find-generic-password", "-s", service.as_str()];
            if let Some(account) = account {
                args.extend(["-a", account]);
            }
            args.push("-w");
            let Some(stdout) = security(&args) else {
                continue;
            };
            found = true;
            if let Some(creds) = credentials_from_blob(&stdout) {
                return Some(Some(creds));
            }
        }
        found.then_some(None)
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

    /// `security`'s stdout (empty for a blank item), or None when there is
    /// no such item, or it fails or hangs on a locked Keychain past the
    /// timeout.
    fn security(args: &[&str]) -> Option<String> {
        let output = crate::keychain::run(args).ok()?;
        output.ok.then_some(output.stdout)
    }
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
            crate::keychain::claude_service(Some(Path::new("/tmp/profile"))),
            "Claude Code-credentials-902e721c"
        );
        assert_eq!(
            crate::keychain::claude_service(None),
            "Claude Code-credentials"
        );
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
        assert_eq!(
            status_error(403),
            "Claude usage is unavailable for this account"
        );
        assert_eq!(status_error(500), "Claude usage request failed (500)");
    }

    #[test]
    fn claim_reset_responses_map_success_and_reasons() {
        let ok = parse_claim_reset_response(
            r#"{"result":"reset","resets_left":0,"cleared":["five_hour","seven_day_overage_included"]}"#,
        )
        .unwrap();
        assert_eq!(ok.resets_left, Some(0));
        assert_eq!(ok.cleared, vec!["five_hour", "seven_day_overage_included"]);

        assert_eq!(
            parse_claim_reset_response(r#"{"result":"already_used"}"#).unwrap_err(),
            "This reset has already been used"
        );
        assert_eq!(
            parse_claim_reset_response(r#"{"result":"cooldown"}"#).unwrap_err(),
            "Try again in a minute before using another reset"
        );
        assert_eq!(
            parse_claim_reset_response(r#"{"result":"not_limited","reason":"needs_limit"}"#)
                .unwrap_err(),
            "This reset can only be used while you're at a limit it refills"
        );
    }
}
