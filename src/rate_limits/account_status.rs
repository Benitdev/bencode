//! How much room an account has left (MonoCode
//! `providers/model/accountUsage.ts`): Ready / Running low / Exhausted, and
//! the account worth switching to.

use super::{ProviderRateLimits, RateLimitStatus, clamp_used_percent, format_reset_duration};

/// At or below this much headroom an account reads as "Running low".
pub const LOW_HEADROOM_PERCENT: f64 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountTone {
    Ready,
    Low,
    Exhausted,
    Checking,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountStatus {
    pub tone: AccountTone,
    pub label: String,
    /// Extra context, e.g. "back in 31m" for an exhausted account.
    pub detail: Option<String>,
}

/// Remaining percent of the tightest window, or None without usage data.
/// A window whose reset time has passed counts as fully available.
pub fn account_headroom(limits: Option<&ProviderRateLimits>, now: i64) -> Option<f64> {
    limits?
        .windows()
        .into_iter()
        .map(|(_, window)| match window.resets_at {
            Some(resets_at) if resets_at <= now => 100.0,
            _ => 100.0 - clamp_used_percent(window.used_percent),
        })
        .reduce(f64::min)
}

/// Ready / Running low / Exhausted for an account, shared by every surface.
pub fn account_status(limits: Option<&ProviderRateLimits>, now: i64) -> AccountStatus {
    let status = |tone, label: &str, detail| AccountStatus {
        tone,
        label: label.to_string(),
        detail,
    };
    let (Some(limits), Some(headroom)) = (limits, account_headroom(limits, now)) else {
        return match limits {
            None => status(AccountTone::Checking, "Checking…", None),
            Some(limits) if limits.is_loading() || limits.status == RateLimitStatus::Fetching => {
                status(AccountTone::Checking, "Checking…", None)
            }
            Some(limits) => {
                let fallback = if limits.status == RateLimitStatus::Unavailable {
                    "Not signed in"
                } else {
                    "Usage unavailable"
                };
                status(
                    AccountTone::Unknown,
                    limits
                        .error
                        .as_deref()
                        .filter(|e| !e.is_empty())
                        .unwrap_or(fallback),
                    None,
                )
            }
        };
    };
    if headroom <= 0.0 {
        // "back in 31m" for the used-up window that stays blocked longest.
        let back_in = exhausted_window_reset_at(limits)
            .filter(|reset_at| *reset_at > now)
            .map(|reset_at| format!("back in {}", format_reset_duration(reset_at - now)));
        status(AccountTone::Exhausted, "Exhausted", back_in)
    } else if headroom <= LOW_HEADROOM_PERCENT {
        status(
            AccountTone::Low,
            "Running low",
            Some(format!("{}% left", headroom.round())),
        )
    } else {
        status(AccountTone::Ready, "Ready", None)
    }
}

/// When a used-up window resets; the later one when several are spent.
pub fn exhausted_window_reset_at(limits: &ProviderRateLimits) -> Option<i64> {
    limits
        .windows()
        .into_iter()
        .filter(|(_, window)| window.used_percent >= 100.0)
        .filter_map(|(_, window)| window.resets_at)
        .max()
}

/// The candidate with the most headroom, if it is comfortably above "low".
pub fn best_alternative<'a, T>(
    candidates: impl IntoIterator<Item = (&'a T, Option<&'a ProviderRateLimits>)>,
    now: i64,
) -> Option<&'a T> {
    candidates
        .into_iter()
        .filter_map(|(candidate, limits)| Some((candidate, account_headroom(limits, now)?)))
        .filter(|(_, headroom)| *headroom > LOW_HEADROOM_PERCENT)
        .fold(
            None,
            |best: Option<(&T, f64)>, (candidate, headroom)| match best {
                Some((_, most)) if most >= headroom => best,
                _ => Some((candidate, headroom)),
            },
        )
        .map(|(candidate, _)| candidate)
}

/// MonoCode `needsProviderLogin`: the snapshot says the account must sign
/// in before usage can be read.
pub fn needs_provider_login(limits: &ProviderRateLimits) -> bool {
    match limits.status {
        RateLimitStatus::Unavailable => true,
        RateLimitStatus::Error => {
            let text = limits.error.as_deref().unwrap_or_default().to_lowercase();
            [
                "expired",
                "sign-in",
                "not signed in",
                "not connected",
                "authentication",
            ]
            .iter()
            .any(|needle| text.contains(needle))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rate_limits::RateLimitWindow;

    const NOW: i64 = 1_800_000_000_000;

    fn limits(session: f64, weekly: f64, session_resets_in: Option<i64>) -> ProviderRateLimits {
        let window = |used_percent, window_minutes, resets_in: Option<i64>| {
            Some(RateLimitWindow {
                used_percent,
                window_minutes,
                resets_at: resets_in.map(|ms| NOW + ms),
            })
        };
        ProviderRateLimits {
            session: window(session, 300, session_resets_in),
            weekly: window(weekly, 10_080, Some(6 * 86_400_000)),
            status: RateLimitStatus::Ok,
            updated_at: NOW,
            ..Default::default()
        }
    }

    #[test]
    fn status_follows_the_tightest_window() {
        let ready = account_status(Some(&limits(30.0, 11.0, None)), NOW);
        assert_eq!(
            (ready.tone, ready.label.as_str()),
            (AccountTone::Ready, "Ready")
        );

        let low = account_status(Some(&limits(85.4, 11.0, None)), NOW);
        assert_eq!(low.tone, AccountTone::Low);
        assert_eq!(low.detail.as_deref(), Some("15% left"));

        let spent = account_status(Some(&limits(100.0, 11.0, Some(31 * 60_000))), NOW);
        assert_eq!(spent.tone, AccountTone::Exhausted);
        assert_eq!(spent.detail.as_deref(), Some("back in 31m"));

        // A spent window whose reset has passed is available again.
        let reset = account_status(Some(&limits(100.0, 11.0, Some(-1_000))), NOW);
        assert_eq!(reset.tone, AccountTone::Ready);
    }

    #[test]
    fn accounts_without_windows_say_why() {
        assert_eq!(account_status(None, NOW).tone, AccountTone::Checking);
        assert_eq!(
            account_status(Some(&ProviderRateLimits::fetching(None)), NOW).label,
            "Checking…"
        );
        let signed_out = ProviderRateLimits::unavailable("Claude not signed in", NOW);
        let status = account_status(Some(&signed_out), NOW);
        assert_eq!(
            (status.tone, status.label.as_str()),
            (AccountTone::Unknown, "Claude not signed in")
        );
        assert!(needs_provider_login(&signed_out));
        assert!(needs_provider_login(&ProviderRateLimits::error(
            "Claude sign-in expired",
            None,
            NOW
        )));
        assert!(!needs_provider_login(&ProviderRateLimits::error(
            "request failed (500)",
            None,
            NOW
        )));
        assert!(!needs_provider_login(&limits(10.0, 10.0, None)));
    }

    #[test]
    fn the_suggested_account_has_the_most_room_above_low() {
        let (low, some, most) = (
            limits(90.0, 10.0, None),
            limits(50.0, 10.0, None),
            limits(5.0, 30.0, None),
        );
        let pick = best_alternative(
            [
                (&"low", Some(&low)),
                (&"some", Some(&some)),
                (&"most", Some(&most)),
                (&"unknown", None),
            ],
            NOW,
        );
        assert_eq!(pick, Some(&"most"));
        assert_eq!(
            best_alternative([(&"low", Some(&low)), (&"unknown", None)], NOW),
            None
        );
    }
}
