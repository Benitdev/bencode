//! Codex usage (MonoCode `fetchCodexRateLimits`): `account/rateLimits/read`
//! on a short-lived `codex app-server`.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;

use super::{Fetched, parse_codex_rate_limits};
use crate::harness::accounts::AccountProfile;
use crate::harness::probe::AppServer;
use crate::harness::resolver::HarnessResolver;

/// MonoCode `DISCOVERY_TIMEOUT_MS`.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

pub fn fetch(account: Option<&AccountProfile>, now: i64) -> Fetched {
    let Some(program) = HarnessResolver::resolve_codex() else {
        return Fetched::Unavailable("Codex CLI not found".into());
    };
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    let result = AppServer::open(&program, &home, PROBE_TIMEOUT, account)
        .and_then(|mut server| server.call("account/rateLimits/read", json!({})));
    match result {
        Ok(result) => {
            let limits = parse_codex_rate_limits(&result, now);
            if !limits.has_windows() && result.is_object() {
                return Fetched::Unavailable("No Codex usage data".into());
            }
            Fetched::Limits(limits)
        }
        Err(error) => failure(&format!("{error:#}")),
    }
}

fn failure(message: &str) -> Fetched {
    let lower = message.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    if has(&[
        "not signed in",
        "chatgpt authentication required",
        "not authenticated",
    ]) {
        Fetched::Unavailable("Codex not signed in".into())
    } else if has(&["enoent", "not found", "could not run", "could not start"]) {
        Fetched::Unavailable("Codex CLI not found".into())
    } else {
        Fetched::Error(message.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_sort_into_signed_out_missing_and_errors() {
        assert_eq!(
            failure("codex account/rateLimits/read: ChatGPT authentication required"),
            Fetched::Unavailable("Codex not signed in".into())
        );
        assert_eq!(
            failure("could not start the CLI: No such file or directory"),
            Fetched::Unavailable("Codex CLI not found".into())
        );
        assert_eq!(
            failure("the CLI timed out"),
            Fetched::Error("the CLI timed out".into())
        );
    }
}
