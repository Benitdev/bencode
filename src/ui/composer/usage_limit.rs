//! The provider's usage limit (MonoCode `UsageLimitNotice` +
//! `usageLimit.ts`): an amber tab on the composer — "Usage limit reached",
//! when it resets, and Resume (once reset) or Resume at reset, which sends
//! "Continue from where you left off." on its own once the limit lifts.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*,
};

use crate::ui::scale::px;
use jiff::{Timestamp, tz::TimeZone};

use crate::app::{BenCodeApp, now_ms};

/// MonoCode `USAGE_LIMIT_RESUME_GRACE_MS`: providers can still refuse right
/// at the reset.
const RESUME_GRACE_MS: i64 = 30_000;
/// MonoCode `CONTINUE_PROMPT`.
pub const CONTINUE_PROMPT: &str = "Continue from where you left off.";

/// A thread stopped by its provider's usage limit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsageLimit {
    /// When the limit lifts (epoch ms), if the provider said.
    pub resets_at: Option<i64>,
    /// Send the continue turn by itself once it lifts.
    pub resume_at_reset: bool,
}

impl UsageLimit {
    /// Still waiting for the reset.
    pub fn waiting(&self, now: i64) -> bool {
        self.resets_at.is_some_and(|at| at > now)
    }

    /// MonoCode `usageLimitResumeDue`: armed, idle and past the reset.
    pub fn resume_due(&self, now: i64, busy: bool) -> bool {
        self.resume_at_reset
            && !busy
            && self.resets_at.is_some_and(|at| now >= at + RESUME_GRACE_MS)
    }
}

/// MonoCode `formatResetDuration`: "45m", "4h 42m", "1d 4h", "now".
pub fn format_reset_duration(ms: i64) -> String {
    if ms <= 0 {
        return "now".into();
    }
    let minutes = ms / 60_000;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let (hours, mins) = (minutes / 60, minutes % 60);
    if hours >= 24 {
        let (days, rest) = (hours / 24, hours % 24);
        return if rest > 0 {
            format!("{days}d {rest}h")
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

/// MonoCode `formatUsageLimitReset`: "3:16 AM · in 4h 42m" today,
/// "Sep 26, 3:16 AM · in 1d 4h" on another day.
pub fn format_reset(resets_at: i64, now: i64, tz: &TimeZone) -> String {
    let local = |ms: i64| {
        Timestamp::from_millisecond(ms)
            .ok()
            .map(|t| t.to_zoned(tz.clone()))
    };
    let (Some(reset), Some(today)) = (local(resets_at), local(now)) else {
        return format!("in {}", format_reset_duration(resets_at - now));
    };
    let pattern = if reset.date() == today.date() {
        "%-I:%M %p"
    } else {
        "%b %-d, %-I:%M %p"
    };
    format!(
        "{} · in {}",
        reset.strftime(pattern),
        format_reset_duration(resets_at - now)
    )
}

impl BenCodeApp {
    pub fn record_usage_limit(&mut self, session_id: &str, resets_at: Option<i64>) {
        self.thread_mut(session_id).usage_limit = Some(UsageLimit {
            resets_at,
            resume_at_reset: false,
        });
    }

    fn dismiss_usage_limit(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if let Some(thread) = self.threads.get_mut(session_id) {
            thread.usage_limit = None;
        }
        cx.notify();
    }

    fn arm_usage_resume(&mut self, session_id: &str, armed: bool, cx: &mut Context<Self>) {
        if let Some(limit) = self
            .threads
            .get_mut(session_id)
            .and_then(|thread| thread.usage_limit.as_mut())
        {
            limit.resume_at_reset = armed;
        }
        cx.notify();
    }

    /// Resume: the continue turn now.
    fn resume_after_limit(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if self.is_agent_running_in(session_id)
            || self
                .threads
                .get_mut(session_id)
                .and_then(|thread| thread.usage_limit.take())
                .is_none()
        {
            return;
        }
        self.send_prompt(session_id, CONTINUE_PROMPT, cx);
    }

    /// The clock's check: resumes armed threads whose limit has lifted, and
    /// says whether a countdown is on screen (so it should redraw).
    pub fn tick_usage_limits(&mut self, cx: &mut Context<Self>) -> bool {
        let now = now_ms();
        let due: Vec<String> = self
            .threads
            .iter()
            .filter_map(|(id, thread)| Some((id, thread.usage_limit.as_ref()?)))
            .filter(|(id, limit)| limit.resume_due(now, self.is_agent_running_in(id)))
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.resume_after_limit(&id, cx);
        }
        self.threads
            .values()
            .filter_map(|thread| thread.usage_limit.as_ref())
            .any(|limit| limit.waiting(now))
    }

    pub(super) fn render_usage_limit(
        &self,
        session_id: &str,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let limit = self.thread(session_id)?.usage_limit?;
        let colors = &cx.theme().colors;
        let now = now_ms();
        let waiting = limit.waiting(now);
        let status = match limit.resets_at {
            None => String::new(),
            Some(at) if waiting => format!("Resets {}", format_reset(at, now, &TimeZone::system())),
            Some(_) => "Limit has reset".to_string(),
        };
        let fg = colors.fg;
        let hover = fg.opacity(0.10);
        let button = |id: &'static str, icon: IconName, label: &'static str| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .gap_1p5()
                .h(px(24.0))
                .px_1p5()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(move |s| s.bg(hover).text_color(fg))
                .child(Icon::new(icon).size(IconSize::Xs).color(fg.opacity(0.7)))
                .child(label)
        };
        let action = {
            let sid = session_id.to_string();
            if !waiting {
                button("usage-resume", IconName::Play, "Resume")
                    .on_click(cx.listener(move |this, _, _, cx| this.resume_after_limit(&sid, cx)))
            } else if limit.resume_at_reset {
                button("usage-armed", IconName::Clock, "Resuming at reset")
                    .text_color(colors.warning)
                    .tooltip(Tooltip::text("Cancel the automatic resume"))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.arm_usage_resume(&sid, false, cx)),
                    )
            } else {
                button("usage-arm", IconName::Clock, "Resume at reset")
                    .tooltip(Tooltip::text("Continue this session once the limit resets"))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.arm_usage_resume(&sid, true, cx)),
                    )
            }
        };
        let dismiss_sid = session_id.to_string();
        Some(
            div()
                .px_2()
                .text_color(fg.opacity(0.55))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(px(32.0))
                        .px_2()
                        .rounded_t(px(10.0))
                        .border_1()
                        .border_b_0()
                        .border_color(colors.warning.opacity(0.25))
                        .bg(colors.warning.opacity(0.10))
                        .text_size(px(12.0))
                        .child(
                            Icon::new(IconName::Gauge)
                                .size(IconSize::Xs)
                                .color(colors.warning),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(fg.opacity(0.85))
                                .child("Usage limit reached"),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(status))
                        .child(action)
                        .child(
                            div()
                                .id(SharedString::from(format!("usage-dismiss-{session_id}")))
                                .size(px(24.0))
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.0))
                                .cursor_pointer()
                                .hover(move |s| s.bg(hover))
                                .tooltip(Tooltip::text("Dismiss"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.dismiss_usage_limit(&dismiss_sid, cx)
                                }))
                                .child(
                                    Icon::new(IconName::X)
                                        .size(IconSize::Xs)
                                        .color(fg.opacity(0.55)),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_like_monocode() {
        assert_eq!(format_reset_duration(0), "now");
        assert_eq!(format_reset_duration(45 * 60_000), "45m");
        assert_eq!(format_reset_duration((4 * 60 + 42) * 60_000), "4h 42m");
        assert_eq!(format_reset_duration(2 * 3_600_000), "2h");
        assert_eq!(format_reset_duration(28 * 3_600_000), "1d 4h");
    }

    #[test]
    fn resets_show_the_day_only_when_it_differs() {
        let tz = TimeZone::UTC;
        let now = 1_791_000_000_000; // 2026-10-03 04:00 UTC
        let later_today = now + 2 * 3_600_000;
        assert_eq!(format_reset(later_today, now, &tz), "6:00 AM · in 2h");
        let tomorrow = now + 26 * 3_600_000;
        assert_eq!(
            format_reset(tomorrow, now, &tz),
            "Oct 4, 6:00 AM · in 1d 2h"
        );
    }

    #[test]
    fn resume_waits_for_the_grace_and_an_idle_thread() {
        let limit = UsageLimit {
            resets_at: Some(1_000),
            resume_at_reset: true,
        };
        assert!(!limit.resume_due(1_000, false));
        assert!(limit.resume_due(1_000 + RESUME_GRACE_MS, false));
        assert!(!limit.resume_due(1_000 + RESUME_GRACE_MS, true));
        assert!(
            !UsageLimit {
                resume_at_reset: false,
                ..limit
            }
            .resume_due(i64::MAX, false)
        );
    }
}
