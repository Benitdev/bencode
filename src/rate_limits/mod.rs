//! Provider usage windows (MonoCode `features/providers/model/rateLimits.ts`):
//! the 5-hour, weekly and monthly limits of Claude Code, Codex and OpenCode
//! Go, their parsers and the footer's copy. Pure; the fetchers live in the
//! submodules and block, so they run on a background executor.

mod account_status;
pub mod antigravity;
mod claude;
mod codex;
pub(crate) mod http;
mod opencode;

use serde_json::Value;

pub use account_status::{
    AccountStatus, AccountTone, account_status, best_alternative, needs_provider_login,
};
pub use claude::delete_credentials as delete_claude_credentials;

use crate::harness::accounts::AccountProfile;

pub const SESSION_WINDOW_MINUTES: u32 = 300;
pub const WEEKLY_WINDOW_MINUTES: u32 = 10_080;
pub const MONTHLY_WINDOW_MINUTES: u32 = 43_200;

const WINDOW_DURATION_TOLERANCE_MINUTES: f64 = 1.0;

/// The harnesses whose usage the footer can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateLimitProvider {
    Claude,
    Codex,
    OpenCode,
    /// BenCode's own; its account id is a limit group (`antigravity`).
    Antigravity,
}

impl RateLimitProvider {
    /// From MonoCode's `sessions.harness` column.
    pub fn from_harness(id: &str) -> Option<Self> {
        match id {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "opencode" => Some(Self::OpenCode),
            "antigravity" => Some(Self::Antigravity),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Antigravity => "antigravity",
        }
    }

    /// MonoCode `HARNESS_TITLE`.
    pub fn title(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::Antigravity => "Antigravity",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RateLimitStatus {
    #[default]
    Idle,
    Fetching,
    Ok,
    Error,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitWindow {
    /// Percentage of the window consumed (0–100).
    pub used_percent: f64,
    /// Window duration in minutes: 300 (5h), 10080 (7d) or 43200 (30d).
    pub window_minutes: u32,
    /// Unix ms timestamp when the window resets, if known.
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Session,
    Weekly,
    Monthly,
}

impl WindowKind {
    /// The details card's heading.
    pub fn title(self) -> &'static str {
        match self {
            Self::Session => "5-hour limit",
            Self::Weekly => "Weekly limit",
            Self::Monthly => "Monthly limit",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderRateLimits {
    pub session: Option<RateLimitWindow>,
    pub weekly: Option<RateLimitWindow>,
    pub monthly: Option<RateLimitWindow>,
    pub updated_at: i64,
    pub error: Option<String>,
    pub status: RateLimitStatus,
}

impl ProviderRateLimits {
    /// The windows the provider reported, shortest first.
    pub fn windows(&self) -> Vec<(WindowKind, &RateLimitWindow)> {
        [
            (WindowKind::Session, &self.session),
            (WindowKind::Weekly, &self.weekly),
            (WindowKind::Monthly, &self.monthly),
        ]
        .into_iter()
        .filter_map(|(kind, window)| Some((kind, window.as_ref()?)))
        .collect()
    }

    pub fn has_windows(&self) -> bool {
        self.session.is_some() || self.weekly.is_some() || self.monthly.is_some()
    }

    /// Nothing to show yet: never asked, or the first answer is on its way.
    pub fn is_loading(&self) -> bool {
        match self.status {
            RateLimitStatus::Idle => true,
            RateLimitStatus::Fetching => !self.has_windows(),
            _ => false,
        }
    }

    /// MonoCode `fetchingRateLimits`: the last snapshot stays up while the
    /// next one loads.
    pub fn fetching(previous: Option<&Self>) -> Self {
        match previous {
            Some(previous) if previous.has_windows() => Self {
                status: RateLimitStatus::Fetching,
                ..previous.clone()
            },
            _ => Self {
                updated_at: previous.map_or(0, |p| p.updated_at),
                status: RateLimitStatus::Fetching,
                ..Self::default()
            },
        }
    }

    /// MonoCode `unavailableRateLimits`: not installed or not signed in.
    pub fn unavailable(error: impl Into<String>, now: i64) -> Self {
        Self {
            updated_at: now,
            error: Some(error.into()),
            status: RateLimitStatus::Unavailable,
            ..Self::default()
        }
    }

    /// MonoCode `errorRateLimits`: a failed read keeps the last snapshot.
    pub fn error(error: impl Into<String>, previous: Option<&Self>, now: i64) -> Self {
        let base = previous.filter(|p| p.has_windows()).cloned().unwrap_or_default();
        Self {
            updated_at: now,
            error: Some(error.into()),
            status: RateLimitStatus::Error,
            ..base
        }
    }
}

/// What one read of a provider came back with.
#[derive(Debug, Clone, PartialEq)]
pub enum Fetched {
    Limits(ProviderRateLimits),
    /// Not installed, not signed in, or no subscription.
    Unavailable(String),
    Error(String),
}

impl Fetched {
    /// The snapshot to show, given the one on screen.
    pub fn into_limits(self, previous: Option<&ProviderRateLimits>, now: i64) -> ProviderRateLimits {
        match self {
            Self::Limits(limits) => limits,
            Self::Unavailable(error) => ProviderRateLimits::unavailable(error, now),
            Self::Error(error) => ProviderRateLimits::error(error, previous, now),
        }
    }
}

/// Reads `provider`'s usage for `account` (None is the default profile);
/// `account_id` is the usage key's, which names Antigravity's limit group.
/// Blocking (Keychain, network, or a CLI child): call it on a background
/// executor.
pub fn fetch(
    provider: RateLimitProvider,
    account: Option<&AccountProfile>,
    account_id: &str,
    now: i64,
) -> Fetched {
    match provider {
        RateLimitProvider::Antigravity => antigravity::fetch(account_id, now),
        RateLimitProvider::Claude => claude::fetch(account, now),
        RateLimitProvider::Codex => codex::fetch(account, now),
        RateLimitProvider::OpenCode => opencode::fetch(now),
    }
}

pub fn clamp_used_percent(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        0.0
    }
}

pub fn format_usage_percent(used_percent: f64) -> String {
    format!("{}%", clamp_used_percent(used_percent).round())
}

/// Compact window-size label. 10080 minutes stays "wk" to match the
/// original status-bar copy.
pub fn format_window_label(window_minutes: u32) -> String {
    const DAY: u32 = 60 * 24;
    match window_minutes {
        WEEKLY_WINDOW_MINUTES => "wk".into(),
        MONTHLY_WINDOW_MINUTES => "mo".into(),
        SESSION_WINDOW_MINUTES => "5h".into(),
        60 => "1h".into(),
        m if m < 60 => format!("{m}m"),
        m if m % (DAY * 7) == 0 => format!("{}wk", m / (DAY * 7)),
        m if m % DAY == 0 => format!("{}d", m / DAY),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

/// Compact remaining duration, flooring to whole units: "47m", "3h 54m",
/// "6d 7h". Returns "now" once the window has already reset.
pub fn format_reset_duration(ms: i64) -> String {
    if ms <= 0 {
        return "now".into();
    }
    let total_mins = ms / 60_000;
    if total_mins < 60 {
        return format!("{total_mins}m");
    }
    let (hours, mins) = (total_mins / 60, total_mins % 60);
    if hours >= 24 {
        let (days, rem_hours) = (hours / 24, hours % 24);
        return if rem_hours > 0 {
            format!("{days}d {rem_hours}h")
        } else {
            format!("{days}d")
        };
    }
    if mins > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{hours}h")
    }
}

pub fn format_reset_countdown(ms: i64) -> String {
    match format_reset_duration(ms).as_str() {
        "now" => "Resets now".into(),
        duration => format!("Resets in {duration}"),
    }
}

/// Status-bar chip label. Prefer remaining time when `resets_at` is known;
/// fall back to the fixed window size otherwise.
pub fn chip_label(window: &RateLimitWindow, now: i64) -> String {
    match window.resets_at {
        Some(resets_at) => format_reset_duration(resets_at - now),
        None => format_window_label(window.window_minutes),
    }
}

/// MonoCode `rateLimitWindowTooltip`.
pub fn window_tooltip(window: &RateLimitWindow, now: i64) -> String {
    let usage = format!("{} used", format_usage_percent(window.used_percent));
    match window.resets_at {
        Some(resets_at) => format!("{usage} · {}", format_reset_countdown(resets_at - now)),
        None => format!("{usage} · {} window", format_window_label(window.window_minutes)),
    }
}

/// MonoCode `updatedLabel`: the details popover's subtitle.
pub fn updated_label(limits: &ProviderRateLimits, now: i64) -> String {
    if limits.updated_at <= 0 {
        return "Rate-limit details".into();
    }
    match ((now - limits.updated_at) / 60_000).max(0) {
        0 => "Updated just now".into(),
        mins if mins < 60 => format!("Updated {mins}m ago"),
        mins => format!("Updated {}h ago", mins / 60),
    }
}

/// A reset time as Unix ms, from epoch seconds, epoch ms or an ISO string.
pub fn parse_reset_timestamp(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => normalize_epoch_ms(number.as_f64()?),
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            if let Ok(numeric) = text.parse::<f64>() {
                return normalize_epoch_ms(numeric);
            }
            text.parse::<jiff::Timestamp>().ok().map(|t| t.as_millisecond())
        }
        _ => None,
    }
}

fn normalize_epoch_ms(value: f64) -> Option<i64> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    // 1e10 sits between seconds-epoch (<2286) and millisecond-epoch (>2001).
    Some(if value > 1e10 { value } else { value * 1000.0 } as i64)
}

fn number_field(rec: &Value, key: &str) -> Option<f64> {
    match rec.get(key)? {
        Value::Number(number) => number.as_f64().filter(|n| n.is_finite()),
        Value::String(text) => text.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        _ => None,
    }
}

fn first_number(rec: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| number_field(rec, key))
}

fn reset_field(rec: &Value) -> Option<i64> {
    ["resets_at", "resetsAt"]
        .iter()
        .find_map(|key| parse_reset_timestamp(rec.get(key)?))
}

fn ok_limits(
    session: Option<RateLimitWindow>,
    weekly: Option<RateLimitWindow>,
    monthly: Option<RateLimitWindow>,
    now: i64,
) -> ProviderRateLimits {
    ProviderRateLimits {
        session,
        weekly,
        monthly,
        updated_at: now,
        error: None,
        status: RateLimitStatus::Ok,
    }
}

fn map_usage_window(raw: Option<&Value>, window_minutes: u32) -> Option<RateLimitWindow> {
    let rec = raw.filter(|v| v.is_object())?;
    let used = first_number(rec, &["used_percentage", "usedPercent", "utilization"])?;
    Some(RateLimitWindow {
        used_percent: clamp_used_percent(used),
        window_minutes,
        resets_at: reset_field(rec),
    })
}

/// The body of Anthropic's `/api/oauth/usage`.
pub fn parse_claude_oauth_usage(body: &str, now: i64) -> Result<ProviderRateLimits, String> {
    let parsed: Value =
        serde_json::from_str(body).map_err(|_| "Claude usage response was not JSON".to_string())?;
    if !parsed.is_object() {
        return Err("Claude usage response was empty".into());
    }
    Ok(ok_limits(
        map_usage_window(parsed.get("five_hour"), SESSION_WINDOW_MINUTES),
        map_usage_window(parsed.get("seven_day"), WEEKLY_WINDOW_MINUTES),
        None,
        now,
    ))
}

struct CodexWindow {
    used_percent: f64,
    duration_mins: Option<f64>,
    resets_at: Option<i64>,
}

impl CodexWindow {
    fn from(rec: Option<&Value>) -> Option<Self> {
        let rec = rec.filter(|v| v.is_object())?;
        Some(Self {
            used_percent: first_number(rec, &["usedPercent", "used_percent", "used_percentage"])?,
            duration_mins: first_number(rec, &["windowDurationMins", "window_duration_mins"]),
            resets_at: reset_field(rec),
        })
    }

    fn kind(&self) -> Option<WindowKind> {
        let duration = self.duration_mins?;
        let near = |minutes: u32| (duration - f64::from(minutes)).abs() <= WINDOW_DURATION_TOLERANCE_MINUTES;
        if near(SESSION_WINDOW_MINUTES) {
            Some(WindowKind::Session)
        } else if near(WEEKLY_WINDOW_MINUTES) {
            Some(WindowKind::Weekly)
        } else if near(MONTHLY_WINDOW_MINUTES) {
            // Free plans get a single 30-day window.
            Some(WindowKind::Monthly)
        } else {
            None
        }
    }

    fn window(&self, window_minutes: u32) -> RateLimitWindow {
        RateLimitWindow {
            used_percent: clamp_used_percent(self.used_percent),
            window_minutes,
            resets_at: self.resets_at,
        }
    }
}

/// The result of Codex's `account/rateLimits/read`. Windows are told apart
/// by their duration; one without a known duration is the session window
/// when it is `primary` and the weekly one when it is `secondary`.
pub fn parse_codex_rate_limits(result: &Value, now: i64) -> ProviderRateLimits {
    let wrapper = result
        .get("rateLimits")
        .filter(|v| v.is_object())
        .unwrap_or(result);
    let primary = CodexWindow::from(wrapper.get("primary"));
    let secondary = CodexWindow::from(wrapper.get("secondary"));
    let (mut session, mut weekly, mut monthly) = (None, None, None);
    for window in [&primary, &secondary].into_iter().flatten() {
        let slot = match window.kind() {
            Some(WindowKind::Session) => &mut session,
            Some(WindowKind::Weekly) => &mut weekly,
            Some(WindowKind::Monthly) => &mut monthly,
            None => continue,
        };
        slot.get_or_insert(window);
    }
    if session.is_none() {
        session = primary.as_ref().filter(|w| w.kind().is_none());
    }
    if weekly.is_none() {
        weekly = secondary.as_ref().filter(|w| w.kind().is_none());
    }
    ok_limits(
        session.map(|w| w.window(SESSION_WINDOW_MINUTES)),
        weekly.map(|w| w.window(WEEKLY_WINDOW_MINUTES)),
        monthly.map(|w| w.window(MONTHLY_WINDOW_MINUTES)),
        now,
    )
}

/// The official OpenCode Go usage payload:
/// `{ usage: { rolling: { status, percent, resetsAt }, weekly, monthly } }`.
/// `percent` is percent used, matching the dashboard.
pub fn parse_opencode_go_usage(result: &Value, now: i64) -> ProviderRateLimits {
    let usage = result.get("usage").filter(|v| v.is_object()).unwrap_or(result);
    let window = |key: &str, window_minutes: u32| {
        let rec = usage.get(key).filter(|v| v.is_object())?;
        // Require an explicit valid status; unknown shapes are dropped so the
        // caller can treat a fully empty payload as an error, not a snapshot.
        if !matches!(rec.get("status")?.as_str()?, "ok" | "rate-limited") {
            return None;
        }
        Some(RateLimitWindow {
            used_percent: clamp_used_percent(first_number(rec, &["percent", "usedPercent"])?),
            window_minutes,
            resets_at: reset_field(rec),
        })
    };
    ok_limits(
        window("rolling", SESSION_WINDOW_MINUTES),
        window("weekly", WEEKLY_WINDOW_MINUTES),
        window("monthly", MONTHLY_WINDOW_MINUTES),
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_800_000_000_000;

    fn window(used_percent: f64, resets_in_ms: Option<i64>) -> RateLimitWindow {
        RateLimitWindow {
            used_percent,
            window_minutes: SESSION_WINDOW_MINUTES,
            resets_at: resets_in_ms.map(|ms| NOW + ms),
        }
    }

    #[test]
    fn durations_floor_to_whole_units() {
        assert_eq!(format_reset_duration(0), "now");
        assert_eq!(format_reset_duration(47 * 60_000 + 59_000), "47m");
        assert_eq!(format_reset_duration((3 * 60 + 54) * 60_000), "3h 54m");
        assert_eq!(format_reset_duration(2 * 3_600_000), "2h");
        assert_eq!(format_reset_duration((6 * 24 + 14) * 3_600_000 + 60_000), "6d 14h");
        assert_eq!(format_reset_duration(24 * 3_600_000), "1d");
        assert_eq!(format_reset_countdown(-5), "Resets now");
        assert_eq!(format_reset_countdown(15 * 60_000), "Resets in 15m");
    }

    #[test]
    fn window_labels_match_the_status_bar_copy() {
        assert_eq!(format_window_label(300), "5h");
        assert_eq!(format_window_label(10_080), "wk");
        assert_eq!(format_window_label(43_200), "mo");
        assert_eq!(format_window_label(60), "1h");
        assert_eq!(format_window_label(45), "45m");
        assert_eq!(format_window_label(20_160), "2wk");
        assert_eq!(format_window_label(2_880), "2d");
        assert_eq!(format_window_label(120), "2h");
        assert_eq!(format_window_label(90), "90m");
    }

    #[test]
    fn chip_prefers_the_countdown_over_the_window_size() {
        assert_eq!(chip_label(&window(100.0, Some(15 * 60_000)), NOW), "15m");
        assert_eq!(chip_label(&window(11.0, None), NOW), "5h");
        assert_eq!(format_usage_percent(10.5), "11%");
        assert_eq!(format_usage_percent(250.0), "100%");
        assert_eq!(format_usage_percent(f64::NAN), "0%");
        assert_eq!(
            window_tooltip(&window(100.0, Some(15 * 60_000)), NOW),
            "100% used · Resets in 15m"
        );
        assert_eq!(window_tooltip(&window(11.0, None), NOW), "11% used · 5h window");
    }

    #[test]
    fn reset_timestamps_accept_seconds_ms_and_iso() {
        assert_eq!(parse_reset_timestamp(&json!(1_791_098_400)), Some(1_791_098_400_000));
        assert_eq!(parse_reset_timestamp(&json!(1_791_098_400_000_i64)), Some(1_791_098_400_000));
        assert_eq!(parse_reset_timestamp(&json!("1791098400")), Some(1_791_098_400_000));
        assert_eq!(
            parse_reset_timestamp(&json!("2026-10-07T04:59:59.943648+00:00")),
            Some(1_791_349_199_943)
        );
        assert_eq!(parse_reset_timestamp(&json!("")), None);
        assert_eq!(parse_reset_timestamp(&json!(0)), None);
        assert_eq!(parse_reset_timestamp(&json!(null)), None);
    }

    #[test]
    fn claude_usage_maps_both_windows() {
        let body = r#"{"five_hour":{"utilization":100.0,"resets_at":"2026-10-07T05:00:00+00:00"},
            "seven_day":{"utilization":11,"resets_at":null},"seven_day_opus":null}"#;
        let limits = parse_claude_oauth_usage(body, NOW).unwrap();
        assert_eq!(limits.status, RateLimitStatus::Ok);
        assert_eq!(limits.updated_at, NOW);
        let session = limits.session.as_ref().unwrap();
        assert_eq!(session.used_percent, 100.0);
        assert_eq!(session.resets_at, Some(1_791_349_200_000));
        assert_eq!(limits.weekly.as_ref().unwrap().resets_at, None);
        assert_eq!(limits.weekly.as_ref().unwrap().window_minutes, WEEKLY_WINDOW_MINUTES);
        assert!(limits.monthly.is_none());
        assert_eq!(limits.windows().len(), 2);

        assert!(parse_claude_oauth_usage("nope", NOW).is_err());
        assert!(parse_claude_oauth_usage("[]", NOW).is_err());
        assert!(!parse_claude_oauth_usage("{}", NOW).unwrap().has_windows());
    }

    #[test]
    fn codex_windows_are_classified_by_duration() {
        let limits = parse_codex_rate_limits(
            &json!({"rateLimits":{
                "primary":{"usedPercent":12,"windowDurationMins":10080,"resetsAt":1791098400},
                "secondary":{"usedPercent":40,"windowDurationMins":299,"resetsAt":1791098400}}}),
            NOW,
        );
        assert_eq!(limits.session.unwrap().used_percent, 40.0);
        assert_eq!(limits.weekly.unwrap().used_percent, 12.0);

        // Free plan: one 30-day window.
        let free = parse_codex_rate_limits(
            &json!({"primary":{"used_percent":"7","window_duration_mins":43200}}),
            NOW,
        );
        assert!(free.session.is_none() && free.weekly.is_none());
        assert_eq!(free.monthly.unwrap().used_percent, 7.0);

        // Unknown durations fall back on their slot.
        let bare = parse_codex_rate_limits(
            &json!({"primary":{"usedPercent":1},"secondary":{"usedPercent":2}}),
            NOW,
        );
        assert_eq!(bare.session.unwrap().used_percent, 1.0);
        assert_eq!(bare.weekly.unwrap().used_percent, 2.0);
        assert!(!parse_codex_rate_limits(&json!({}), NOW).has_windows());
    }

    #[test]
    fn opencode_go_needs_a_known_status() {
        let limits = parse_opencode_go_usage(
            &json!({"usage":{
                "rolling":{"status":"ok","percent":33,"resetsAt":"2026-10-07T05:00:00Z"},
                "weekly":{"status":"rate-limited","percent":120},
                "monthly":{"status":"mystery","percent":5}}}),
            NOW,
        );
        assert_eq!(limits.session.unwrap().used_percent, 33.0);
        assert_eq!(limits.weekly.unwrap().used_percent, 100.0);
        assert!(limits.monthly.is_none());
    }

    #[test]
    fn a_failed_read_keeps_the_last_snapshot() {
        let ok = ok_limits(Some(window(40.0, None)), None, None, NOW - 60_000);
        let fetching = ProviderRateLimits::fetching(Some(&ok));
        assert_eq!(fetching.status, RateLimitStatus::Fetching);
        assert!(!fetching.is_loading(), "the old windows stay on screen");
        assert!(ProviderRateLimits::fetching(None).is_loading());
        assert!(ProviderRateLimits::default().is_loading());

        let failed = Fetched::Error("offline".into()).into_limits(Some(&ok), NOW);
        assert_eq!(failed.status, RateLimitStatus::Error);
        assert_eq!(failed.session, ok.session);
        assert_eq!(failed.error.as_deref(), Some("offline"));

        let gone = Fetched::Unavailable("Claude not signed in".into()).into_limits(Some(&ok), NOW);
        assert_eq!(gone.status, RateLimitStatus::Unavailable);
        assert!(!gone.has_windows());
    }

    #[test]
    fn updated_label_counts_minutes_then_hours() {
        let mut limits = ProviderRateLimits::default();
        assert_eq!(updated_label(&limits, NOW), "Rate-limit details");
        limits.updated_at = NOW - 20_000;
        assert_eq!(updated_label(&limits, NOW), "Updated just now");
        limits.updated_at = NOW - 5 * 60_000;
        assert_eq!(updated_label(&limits, NOW), "Updated 5m ago");
        limits.updated_at = NOW - 130 * 60_000;
        assert_eq!(updated_label(&limits, NOW), "Updated 2h ago");
    }
}
