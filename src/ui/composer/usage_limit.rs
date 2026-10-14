//! The provider's usage limit (MonoCode `UsageLimitNotice` +
//! `usageLimit.ts`): an amber tab on the composer — "Usage limit reached",
//! when it resets, and Resume (once reset) or Resume at reset, which sends
//! "Continue from where you left off." on its own once the limit lifts.
//! A Mono's chat has its own card instead (`MonoUsageLimitNotice`): the
//! model and account to keep going on, and the same resume.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*,
};

use crate::ui::scale::px;
use jiff::{Timestamp, tz::TimeZone};

use super::menus::{over_composer, popover_anchor};
use super::model_picker::ModelsAnchor;
use crate::app::{BenCodeApp, now_ms};
use crate::db::SessionRow;
use crate::harness::HarnessKind;
use crate::harness::accounts::supports_accounts;
use crate::ui::HarnessIcon;
use crate::ui::app_callback::app_callback;
use ely_gpui_component::buttons::ButtonVariant;
use ely_gpui_component::menus::{DropdownMenu, Menu, MenuItem};

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
    /// MonoCode `ResumeControls`: Resume once the limit has reset, else
    /// Resume at reset, or Resuming at reset while that is armed.
    fn resume_control(
        &self,
        session_id: &str,
        limit: UsageLimit,
        waiting: bool,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = &cx.theme().colors;
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
        let sid = session_id.to_string();
        if !waiting {
            button("usage-resume", IconName::Play, "Resume")
                .on_click(cx.listener(move |this, _, _, cx| this.resume_after_limit(&sid, cx)))
        } else if limit.resume_at_reset {
            button("usage-armed", IconName::Clock, "Resuming at reset")
                .text_color(colors.warning)
                .tooltip(Tooltip::text("Cancel the automatic resume"))
                .on_click(cx.listener(move |this, _, _, cx| this.arm_usage_resume(&sid, false, cx)))
        } else {
            button("usage-arm", IconName::Clock, "Resume at reset")
                .tooltip(Tooltip::text("Continue this session once the limit resets"))
                .on_click(cx.listener(move |this, _, _, cx| this.arm_usage_resume(&sid, true, cx)))
        }
    }

    /// MonoCode `MonoUsageLimitNotice` (`UsageLimitNotice variant="mono"`):
    /// a card over a Mono's composer, `mx-1.5 mb-2 rounded-lg border
    /// border-amber-400/20 bg-amber-400/6 px-3 py-2.5 text-[12px]`, saying
    /// the conversation is kept, when the limit lifts, and offering another
    /// model, another account and the resume.
    pub(super) fn render_mono_usage_limit(
        &self,
        session: &SessionRow,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let limit = self.thread(&session.id)?.usage_limit?;
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let now = now_ms();
        let waiting = limit.waiting(now);
        let provider = HarnessKind::from_id(&session.harness)
            .map_or("Usage limit reached".to_string(), |kind| {
                format!("{} usage limit reached", kind.label())
            });
        let resets = limit.resets_at.map(|at| {
            if waiting {
                format!("Resets {}", format_reset(at, now, &TimeZone::system()))
            } else {
                "Limit has reset".to_string()
            }
        });
        // MonoCode `ModelPicker triggerLabel="Choose another model"`.
        let open = self.composer_menus.models_only == Some(ModelsAnchor::UsageLimit);
        let model = super::composer_chip(
            "mono-limit-model",
            HarnessIcon::new(&session.harness)
                .size(px(16.0))
                .into_any_element(),
            "Choose another model".to_string(),
            None,
            open,
            240.0,
            cx,
        )
        .on_click(
            cx.listener(|this, _, _, cx| this.toggle_models_only(ModelsAnchor::UsageLimit, cx)),
        );
        let model = div()
            .relative()
            .flex_none()
            .child(popover_anchor(model, cx))
            .children(
                self.render_models_only(ModelsAnchor::UsageLimit, false, cx)
                    .map(over_composer),
            );
        Some(
            div()
                .mx_1p5()
                .mb_2()
                .rounded(px(8.0))
                .border_1()
                .border_color(colors.warning.opacity(0.2))
                .bg(colors.warning.opacity(0.06))
                .px_3()
                .py(px(10.0))
                .text_size(px(12.0))
                .text_color(fg.opacity(0.6))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(IconName::Gauge)
                                .size(IconSize::Xs)
                                .color(colors.warning),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(fg.opacity(0.85))
                                .child(provider),
                        ),
                )
                .child(
                    // `pl-5.5`: under the words, past the gauge.
                    div()
                        .pl(px(22.0))
                        .child(div().mt_1().line_height(px(19.5)).child(
                            "Your conversation is saved. Choose another model or account to \
                                 keep going.",
                        ))
                        .children(
                            resets
                                .map(|line| div().mt_1().text_color(fg.opacity(0.45)).child(line)),
                        )
                        .child(
                            div()
                                .mt_2()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_x_3()
                                .gap_y_1p5()
                                .child(model)
                                .children(self.render_mono_account_picker(session, cx))
                                .child(self.resume_control(&session.id, limit, waiting, cx)),
                        ),
                )
                .into_any_element(),
        )
    }

    /// MonoCode `AccountPicker`: the provider's other accounts, the pick
    /// moving the conversation onto it and resuming.
    fn render_mono_account_picker(
        &self,
        session: &SessionRow,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if !supports_accounts(&session.harness) {
            return None;
        }
        let current = self.session_account_id(session);
        let others: Vec<_> = self
            .provider_accounts(&session.harness)
            .into_iter()
            .filter(|account| account.id != current)
            .collect();
        if others.is_empty() {
            return None;
        }
        let menu = others.into_iter().fold(Menu::new(), |menu, account| {
            let (sid, id) = (session.id.clone(), account.id.clone());
            menu.item(
                MenuItem::new(account.label.clone()).on_click(app_callback(cx, move |this, cx| {
                    this.switch_mono_usage_limit_account(&sid, &id, cx)
                })),
            )
        });
        Some(
            DropdownMenu::new("mono-limit-account", "Choose another account", menu)
                .variant(ButtonVariant::Secondary)
                .into_any_element(),
        )
    }

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
    pub(crate) fn resume_after_limit(&mut self, session_id: &str, cx: &mut Context<Self>) {
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
        let action = self.resume_control(session_id, limit, waiting, cx);
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
