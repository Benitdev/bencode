//! OpenCode Go's 5-hour, weekly and monthly usage (MonoCode
//! `fetch_opencode_go_usage` in `src-tauri/src/rate_limits.rs`), asked of
//! the official API with the local Go key.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use super::{Fetched, http, parse_opencode_go_usage};

const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

pub fn fetch(now: i64) -> Fetched {
    let Some(api_key) = read_go_api_key() else {
        return Fetched::Unavailable("OpenCode Go not connected".into());
    };
    let response = http::get(
        USAGE_URL,
        &[("Authorization", &format!("Bearer {api_key}"))],
        HTTP_TIMEOUT,
    );
    match response {
        Ok(response) if (200..300).contains(&response.status) => {
            let Ok(body) = serde_json::from_str::<Value>(&response.body) else {
                return Fetched::Error("OpenCode Go response was not JSON".into());
            };
            let limits = parse_opencode_go_usage(&body, now);
            if limits.has_windows() {
                Fetched::Limits(limits)
            } else {
                // A 200 with no usable windows is malformed: report an error
                // instead of sticking in "unavailable" forever.
                Fetched::Error("OpenCode Go usage response was unexpected".into())
            }
        }
        Ok(response) => status_failure(response.status),
        Err(error) => Fetched::Error(format!("OpenCode Go usage request failed: {error:#}")),
    }
}

fn status_failure(status: u16) -> Fetched {
    match status {
        // 403 means a valid key without a Go subscription — not a failure,
        // so the footer can hide the chip instead of showing an error.
        403 => Fetched::Unavailable("No OpenCode Go subscription".into()),
        401 => Fetched::Error("OpenCode Go sign-in expired".into()),
        status => Fetched::Error(format!("OpenCode Go usage request failed ({status})")),
    }
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn home() -> Option<PathBuf> {
    env_var("HOME").map(PathBuf::from)
}

/// Resolve the OpenCode data directory the same way OpenCode does:
/// `OPENCODE_DATA_DIR`, then `$XDG_DATA_HOME/<app>`, then the default
/// `~/.local/share/<app>`, where `<app>` is `OPENCODE_APPNAME` or "opencode".
fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = env_var("OPENCODE_DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    let app = env_var("OPENCODE_APPNAME").unwrap_or_else(|| "opencode".into());
    if let Some(xdg) = env_var("XDG_DATA_HOME") {
        return Some(PathBuf::from(xdg).join(app));
    }
    Some(home()?.join(".local/share").join(app))
}

/// The Go key lives at `auth.json -> "opencode-go" -> "key"` inside the
/// OpenCode data directory. Resolution mirrors OpenCode's own precedence:
/// the `OPENCODE_AUTH_CONTENT` blob, then an explicit provider key in
/// opencode config, then stored credentials on disk.
fn read_go_api_key() -> Option<String> {
    // Env-injected auth blob is authoritative when it parses: a valid blob
    // without opencode-go means "no key", not "look elsewhere".
    if let Some(blob) = env_var("OPENCODE_AUTH_CONTENT")
        && let Ok(value) = serde_json::from_str::<Value>(&blob)
        && value.is_object()
    {
        return auth_go_key(&value);
    }
    if let Some(key) = read_config_api_key() {
        return Some(key);
    }
    let raw = std::fs::read_to_string(data_dir()?.join("auth.json"))
        .ok()
        .or_else(|| {
            // Legacy macOS location.
            std::fs::read_to_string(home()?.join("Library/Application Support/opencode/auth.json")).ok()
        })?;
    auth_go_key(&serde_json::from_str(raw.trim()).ok()?)
}

fn auth_go_key(auth: &Value) -> Option<String> {
    let key = auth.get("opencode-go")?.get("key")?.as_str()?.trim();
    (!key.is_empty()).then(|| key.to_string())
}

/// Explicit `provider.options.apiKey` for the Go provider in opencode
/// config: `OPENCODE_CONFIG_CONTENT`, then `OPENCODE_CONFIG`, then the
/// global `opencode.json`. Only the Go provider IDs are considered so keys
/// for unrelated providers are never picked up.
fn read_config_api_key() -> Option<String> {
    if let Some(key) = env_var("OPENCODE_CONFIG_CONTENT")
        .and_then(|content| parse_config(&content))
        .and_then(|value| config_go_api_key(&value, &env_var))
    {
        return Some(key);
    }
    config_paths()
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|raw| parse_config(&raw))
        .find_map(|value| config_go_api_key(&value, &env_var))
}

fn config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(custom) = env_var("OPENCODE_CONFIG") {
        paths.push(PathBuf::from(custom));
    }
    let roots = [
        env_var("XDG_CONFIG_HOME").map(|xdg| PathBuf::from(xdg).join("opencode")),
        home().map(|home| home.join(".config/opencode")),
    ];
    for root in roots.into_iter().flatten() {
        paths.push(root.join("opencode.jsonc"));
        paths.push(root.join("opencode.json"));
    }
    paths
}

/// Parse OpenCode configuration using JSONC semantics: comments and trailing
/// commas are accepted, while ordinary JSON stays on serde_json's fast path.
fn parse_config(raw: &str) -> Option<Value> {
    serde_json::from_str(raw.trim()).ok().or_else(|| {
        let normalized = strip_jsonc_trailing_commas(&strip_jsonc_comments(raw)?);
        serde_json::from_str(&normalized).ok()
    })
}

/// Copies `raw` through `outside`, which sees each character that is not
/// part of a string literal; string literals pass through untouched.
fn map_outside_strings(
    raw: &str,
    mut outside: impl FnMut(char, &mut std::iter::Peekable<std::str::Chars<'_>>, &mut String) -> Option<()>,
) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let (mut in_string, mut escaped) = (false, false);
    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
            out.push(ch);
        } else {
            outside(ch, &mut chars, &mut out)?;
        }
    }
    Some(out)
}

/// None when a block comment never closes.
fn strip_jsonc_comments(raw: &str) -> Option<String> {
    map_outside_strings(raw, |ch, chars, out| {
        match (ch, chars.peek().copied()) {
            ('/', Some('/')) => {
                out.push(' ');
                if chars.any(|next| next == '\n') {
                    out.push('\n');
                }
            }
            ('/', Some('*')) => {
                chars.next();
                out.push(' ');
                loop {
                    match chars.next()? {
                        '\n' => out.push('\n'),
                        '*' if chars.next_if_eq(&'/').is_some() => break,
                        _ => {}
                    }
                }
            }
            _ => out.push(ch),
        }
        Some(())
    })
}

fn strip_jsonc_trailing_commas(raw: &str) -> String {
    map_outside_strings(raw, |ch, chars, out| {
        let closes = || matches!(chars.clone().find(|next| !next.is_whitespace()), Some('}' | ']'));
        if ch != ',' || !closes() {
            out.push(ch);
        }
        Some(())
    })
    .unwrap_or_default()
}

/// `env` resolves `{env:NAME}` references, as OpenCode does.
fn config_go_api_key(value: &Value, env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let providers = value.get("provider")?.as_object()?;
    for id in ["opencode-go", "opencode"] {
        let Some(api_key) = providers
            .get(id)
            .and_then(|entry| entry.get("options"))
            .and_then(|options| options.get("apiKey"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|key| !key.is_empty())
        else {
            continue;
        };
        let Some(var) = api_key
            .strip_prefix("{env:")
            .and_then(|rest| rest.strip_suffix('}'))
        else {
            return Some(api_key.to_string());
        };
        if let Some(resolved) = env(var) {
            return Some(resolved);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn auth_json_holds_the_go_key() {
        let auth = |raw: &str| auth_go_key(&serde_json::from_str(raw).unwrap());
        assert_eq!(
            auth(r#"{"openai":{"type":"oauth"},"opencode-go":{"type":"api","key":"sk-go-abc"}}"#).as_deref(),
            Some("sk-go-abc")
        );
        assert_eq!(auth(r#"{"openai":{"type":"oauth"}}"#), None);
        assert_eq!(auth(r#"{"opencode-go":{"key":"  "}}"#), None);
    }

    #[test]
    fn forbidden_maps_to_unavailable() {
        // A valid key without a Go subscription hides the chip instead of
        // rendering an error.
        assert_eq!(
            status_failure(403),
            Fetched::Unavailable("No OpenCode Go subscription".into())
        );
        assert!(matches!(status_failure(500), Fetched::Error(_)));
        assert_eq!(
            status_failure(401),
            Fetched::Error("OpenCode Go sign-in expired".into())
        );
    }

    #[test]
    fn config_accepts_jsonc() {
        let value = parse_config(
            r#"{
              // URLs inside strings must not be treated as comments.
              "provider": {
                /* OpenCode Go
                   credentials */
                "opencode-go": {
                  "options": {
                    "apiKey": "sk-go-jsonc",
                    "baseURL": "https://opencode.ai/v1",
                  },
                },
              },
            }"#,
        )
        .unwrap();
        assert_eq!(config_go_api_key(&value, &no_env).as_deref(), Some("sk-go-jsonc"));
        assert_eq!(
            value["provider"]["opencode-go"]["options"]["baseURL"],
            "https://opencode.ai/v1"
        );
        assert!(parse_config("{ /* never closed").is_none());
    }

    #[test]
    fn config_key_reads_only_the_go_providers() {
        let parse = |raw: &str| serde_json::from_str::<Value>(raw).unwrap();
        let both = parse(
            r#"{"provider":{"anthropic":{"options":{"apiKey":"sk-ant-x"}},"opencode-go":{"options":{"apiKey":"sk-go-cfg"}}}}"#,
        );
        assert_eq!(config_go_api_key(&both, &no_env).as_deref(), Some("sk-go-cfg"));

        let fallthrough = parse(r#"{"provider":{"opencode":{"options":{"apiKey":"sk-go-opencode"}}}}"#);
        assert_eq!(
            config_go_api_key(&fallthrough, &no_env).as_deref(),
            Some("sk-go-opencode")
        );

        let other = parse(r#"{"provider":{"anthropic":{"options":{"apiKey":"sk-ant-x"}}}}"#);
        assert_eq!(config_go_api_key(&other, &no_env), None);
    }

    #[test]
    fn config_key_resolves_env_references() {
        let value: Value = serde_json::from_str(
            r#"{"provider":{"opencode-go":{"options":{"apiKey":"{env:GO_KEY}"}},"opencode":{"options":{"apiKey":"sk-fallback"}}}}"#,
        )
        .unwrap();
        let set = |name: &str| (name == "GO_KEY").then(|| "sk-go-env".to_string());
        assert_eq!(config_go_api_key(&value, &set).as_deref(), Some("sk-go-env"));
        // An unset variable moves on to the next Go provider id.
        assert_eq!(config_go_api_key(&value, &no_env).as_deref(), Some("sk-fallback"));
    }
}
