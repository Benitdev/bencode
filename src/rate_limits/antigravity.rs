//! Antigravity's 5-hour and weekly usage (BenCode's own; MonoCode reads
//! none). Asked of the endpoint `agy` itself uses, with the access token
//! of its Keychain sign-in. Models share limits in groups (Gemini; Claude
//! and GPT), so a snapshot is one group's. The token is `agy`'s to
//! refresh: once it has expired, usage waits for the next Antigravity turn.

use std::time::Duration;

use serde_json::Value;

use super::{
    Fetched, ProviderRateLimits, RateLimitWindow, SESSION_WINDOW_MINUTES, WEEKLY_WINDOW_MINUTES,
    clamp_used_percent, http, number_field, ok_limits, parse_reset_timestamp,
};

const API: &str = "https://cloudcode-pa.googleapis.com/v1internal";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// `agy` refreshes its token when it runs; nothing here does.
const TOKEN_EXPIRED: &str = "Antigravity usage updates after its next turn";

/// The limit groups, as the usage key's account id.
pub const GEMINI_GROUP: &str = "gemini";
pub const THIRD_PARTY_GROUP: &str = "3p";
pub const GROUPS: [&str; 2] = [GEMINI_GROUP, THIRD_PARTY_GROUP];

/// The group whose limits `model` (`antigravity:claude-sonnet-4-6`) draws on.
pub fn group_of(model: &str) -> &'static str {
    let model = model.to_ascii_lowercase();
    if model.contains("claude") || model.contains("gpt") {
        THIRD_PARTY_GROUP
    } else {
        GEMINI_GROUP
    }
}

/// What the details popover calls a group.
pub fn group_title(group: &str) -> &'static str {
    if group == THIRD_PARTY_GROUP {
        "Claude and GPT models"
    } else {
        "Gemini models"
    }
}

pub fn fetch(group: &str, now: i64) -> Fetched {
    let token = match crate::harness::agy_accounts::live_access_token() {
        Ok(Some(token)) => token,
        Ok(None) => return Fetched::Unavailable("Antigravity is signed out".into()),
        Err(error) => return Fetched::Error(error),
    };
    if token.expires_at.is_some_and(|at| at <= now) {
        return Fetched::Error(TOKEN_EXPIRED.into());
    }
    let project = match post(
        &token.access_token,
        "loadCodeAssist",
        r#"{"metadata":{"ideType":"ANTIGRAVITY"}}"#,
    ) {
        Ok(body) => body
            .get("cloudaicompanionProject")
            .and_then(Value::as_str)
            .map(String::from),
        Err(error) => return Fetched::Error(error),
    };
    let request = match project {
        Some(project) => serde_json::json!({ "project": project }),
        None => serde_json::json!({}),
    };
    match post(
        &token.access_token,
        "retrieveUserQuotaSummary",
        &request.to_string(),
    ) {
        Ok(body) => {
            let limits = parse_quota_summary(&body, group, now);
            if limits.has_windows() {
                Fetched::Limits(limits)
            } else {
                Fetched::Error("Antigravity usage response was unexpected".into())
            }
        }
        Err(error) => Fetched::Error(error),
    }
}

/// One JSON call; the failure is what the footer shows.
fn post(token: &str, method: &str, body: &str) -> Result<Value, String> {
    let response = http::send(
        &format!("{API}:{method}"),
        &[
            ("Authorization", &format!("Bearer {token}")),
            ("Content-Type", "application/json"),
            // curl's own user agent is turned away (403).
            ("User-Agent", "antigravity"),
        ],
        http::Send {
            body: Some(body),
            ..Default::default()
        },
        HTTP_TIMEOUT,
    );
    match response {
        Ok(response) if (200..300).contains(&response.status) => {
            serde_json::from_str(&response.body)
                .map_err(|_| "Antigravity usage response was not JSON".to_string())
        }
        Ok(response) if response.status == 401 => Err(TOKEN_EXPIRED.into()),
        Ok(response) => Err(format!(
            "Antigravity usage request failed ({})",
            response.status
        )),
        Err(error) => Err(format!("Antigravity usage request failed: {error:#}")),
    }
}

/// `group`'s 5-hour and weekly buckets (`gemini-5h`, `gemini-weekly`) out
/// of every group's.
fn parse_quota_summary(body: &Value, group: &str, now: i64) -> ProviderRateLimits {
    let prefix = format!("{group}-");
    let buckets: Vec<&Value> = body
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|group| group.get("buckets")?.as_array())
        .flatten()
        .filter(|bucket| {
            bucket
                .get("bucketId")
                .and_then(Value::as_str)
                .is_some_and(|id| id.starts_with(&prefix))
        })
        .collect();
    let window = |name: &str, window_minutes: u32| {
        let bucket = buckets
            .iter()
            .find(|bucket| bucket.get("window").and_then(Value::as_str) == Some(name))?;
        Some(RateLimitWindow {
            used_percent: clamp_used_percent(
                (1.0 - number_field(bucket, "remainingFraction")?) * 100.0,
            ),
            window_minutes,
            resets_at: bucket.get("resetTime").and_then(parse_reset_timestamp),
        })
    };
    ok_limits(
        window("5h", SESSION_WINDOW_MINUTES),
        window("weekly", WEEKLY_WINDOW_MINUTES),
        None,
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_reads_its_own_buckets() {
        let body = serde_json::json!({
            "groups": [
                { "displayName": "Gemini Models", "buckets": [
                    { "bucketId": "gemini-weekly", "window": "weekly",
                      "resetTime": "2026-10-12T01:43:38Z", "remainingFraction": 0.75 },
                    { "bucketId": "gemini-5h", "window": "5h",
                      "resetTime": "2026-10-08T12:23:14Z", "remainingFraction": 0.9 },
                ]},
                { "displayName": "Claude and GPT models", "buckets": [
                    { "bucketId": "3p-weekly", "window": "weekly", "remainingFraction": 1 },
                    { "bucketId": "3p-5h", "window": "5h", "remainingFraction": 0 },
                ]},
            ],
        });
        let gemini = parse_quota_summary(&body, GEMINI_GROUP, 7);
        let session = gemini.session.unwrap();
        assert!((session.used_percent - 10.0).abs() < 1e-6);
        assert_eq!(session.window_minutes, SESSION_WINDOW_MINUTES);
        assert_eq!(session.resets_at, Some(1_791_462_194_000));
        assert!((gemini.weekly.unwrap().used_percent - 25.0).abs() < 1e-6);
        assert_eq!(gemini.updated_at, 7);

        let third = parse_quota_summary(&body, THIRD_PARTY_GROUP, 7);
        assert_eq!(third.session.unwrap().used_percent, 100.0);
        let weekly = third.weekly.unwrap();
        assert_eq!((weekly.used_percent, weekly.resets_at), (0.0, None));

        assert!(!parse_quota_summary(&serde_json::json!({}), GEMINI_GROUP, 7).has_windows());
    }

    /// Live: reads the Keychain and Google's endpoint.
    #[test]
    #[ignore]
    fn live_usage_round_trip() {
        let now = jiff::Timestamp::now().as_millisecond();
        for group in GROUPS {
            match fetch(group, now) {
                Fetched::Limits(limits) => {
                    assert!(
                        limits.session.is_some() && limits.weekly.is_some(),
                        "{limits:?}"
                    );
                    println!("{group}: {limits:?}");
                }
                other => panic!("{group}: {other:?}"),
            }
        }
    }

    #[test]
    fn models_fall_into_their_group() {
        assert_eq!(group_of("antigravity:gemini-3.8-flash-high"), GEMINI_GROUP);
        assert_eq!(group_of("antigravity:claude-sonnet-4-6"), THIRD_PARTY_GROUP);
        assert_eq!(group_of("antigravity:GPT-OSS-120b"), THIRD_PARTY_GROUP);
        assert_eq!(group_of(""), GEMINI_GROUP);
    }
}
