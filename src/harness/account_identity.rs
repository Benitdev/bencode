//! Who an account profile is signed in as (MonoCode
//! `src-tauri/src/account_identity.rs`): read from what the provider CLI
//! already cached on disk, so no token is sent anywhere. Blocking file
//! reads; call `read` on a background executor.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde_json::Value;

use crate::harness::accounts::AccountProfile;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountIdentity {
    pub email: Option<String>,
    pub name: Option<String>,
    pub plan: Option<String>,
    pub organization: Option<String>,
}

impl AccountIdentity {
    /// MonoCode `ProviderAccountSubtitle`: "Max · me@example.com".
    pub fn subtitle(&self) -> Option<String> {
        let parts: Vec<&str> = [self.plan.as_deref(), self.email.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// Org chip text: "Personal" for Claude's default "<name>'s Organization".
    pub fn organization_tag(&self) -> Option<&str> {
        let name = self.organization.as_deref()?.trim();
        if name.is_empty() {
            return None;
        }
        let personal = name.ends_with("'s Organization") || name.ends_with("’s Organization");
        Some(if personal { "Personal" } else { name })
    }
}

/// `provider`'s signed-in identity for `account` (None is the default
/// profile); None when the profile is not signed in.
pub fn read(provider: &str, account: Option<&AccountProfile>) -> Option<AccountIdentity> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let dir = account.map(|account| account.dir.clone());
    match provider {
        "claude" => {
            let path = match dir {
                Some(dir) => dir.join(".claude.json"),
                None => home()?.join(".claude.json"),
            };
            parse_claude_identity(&read_json(&path)?)
        }
        "codex" => {
            let dir = dir
                .or_else(|| std::env::var_os("CODEX_HOME").map(PathBuf::from))
                .or_else(|| Some(home()?.join(".codex")))?;
            parse_codex_identity(&read_json(&dir.join("auth.json"))?)
        }
        _ => None,
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Parse the `oauthAccount` block Claude Code writes to `.claude.json`.
fn parse_claude_identity(config: &Value) -> Option<AccountIdentity> {
    let account = config.get("oauthAccount")?;
    // organizationType is e.g. "claude_max", "claude_pro", "claude_team".
    let plan = text(account, "organizationType")
        .map(|kind| capitalize(kind.strip_prefix("claude_").unwrap_or(&kind)));
    Some(AccountIdentity {
        email: text(account, "emailAddress"),
        name: text(account, "displayName").or_else(|| text(account, "fullName")),
        plan,
        organization: text(account, "organizationName"),
    })
}

/// Parse the claims of the `id_token` in Codex's `auth.json`.
fn parse_codex_identity(auth: &Value) -> Option<AccountIdentity> {
    let id_token = auth.get("tokens")?.get("id_token")?.as_str()?;
    let payload = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    let openai = claims.get("https://api.openai.com/auth");
    let organization = openai
        .and_then(|auth| auth.get("organizations"))
        .and_then(Value::as_array)
        .and_then(|orgs| {
            orgs.iter()
                .find(|org| org.get("is_default").and_then(Value::as_bool) == Some(true))
        })
        .and_then(|org| text(org, "title"));
    Some(AccountIdentity {
        email: text(&claims, "email").or_else(|| {
            claims
                .get("https://api.openai.com/profile")
                .and_then(|profile| text(profile, "email"))
        }),
        name: text(&claims, "name"),
        plan: openai
            .and_then(|auth| text(auth, "chatgpt_plan_type"))
            .map(|plan| capitalize(&plan)),
        organization,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn codex_auth(claims: Value) -> Value {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
        json!({ "tokens": { "id_token": format!("header.{payload}.signature") } })
    }

    #[test]
    fn claude_reads_oauth_account() {
        let identity = parse_claude_identity(&json!({ "oauthAccount": {
            "emailAddress": "me@example.com",
            "displayName": "Me",
            "organizationType": "claude_max",
            "organizationName": "Me's Organization",
        }}))
        .unwrap();
        assert_eq!(identity.email.as_deref(), Some("me@example.com"));
        assert_eq!(identity.name.as_deref(), Some("Me"));
        assert_eq!(identity.plan.as_deref(), Some("Max"));
        assert_eq!(identity.subtitle().as_deref(), Some("Max · me@example.com"));
        assert_eq!(identity.organization_tag(), Some("Personal"));
        assert_eq!(parse_claude_identity(&json!({ "numStartups": 3 })), None);
    }

    #[test]
    fn claude_falls_back_to_full_name_and_keeps_missing_email_optional() {
        let identity =
            parse_claude_identity(&json!({ "oauthAccount": { "fullName": "Full Name" } })).unwrap();
        assert_eq!(identity.name.as_deref(), Some("Full Name"));
        assert_eq!(identity.email, None);
        assert_eq!(identity.subtitle(), None);
        assert_eq!(identity.organization_tag(), None);
    }

    #[test]
    fn codex_reads_id_token_claims() {
        let identity = parse_codex_identity(&codex_auth(json!({
            "email": "dev@example.com",
            "name": "Dev",
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": "pro",
                "organizations": [
                    { "title": "Other", "is_default": false },
                    { "title": "Acme", "is_default": true },
                ],
            },
        })))
        .unwrap();
        assert_eq!(identity.email.as_deref(), Some("dev@example.com"));
        assert_eq!(identity.plan.as_deref(), Some("Pro"));
        assert_eq!(identity.organization_tag(), Some("Acme"));

        let namespaced = parse_codex_identity(&codex_auth(json!({
            "https://api.openai.com/profile": { "email": "ns@example.com" },
        })))
        .unwrap();
        assert_eq!(namespaced.email.as_deref(), Some("ns@example.com"));
    }

    #[test]
    fn codex_rejects_malformed_tokens() {
        let token = |id_token: &str| json!({ "tokens": { "id_token": id_token } });
        assert_eq!(parse_codex_identity(&token("no-dots")), None);
        assert_eq!(parse_codex_identity(&token("header.!!!.signature")), None);
        assert_eq!(parse_codex_identity(&json!({})), None);
    }
}
