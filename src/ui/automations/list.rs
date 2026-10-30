//! MonoCode `AutomationsContent`'s `<aside>` and `AutomationCard`: the
//! filter, Pause all, New automation, the Overview entry, and one card per
//! automation: its name (a dot marks unsaved edits), what it is doing or
//! does next, and its last runs, project and model.

use bencode_ui::buttons::{Button, ButtonVariant, IconButton};
use bencode_ui::forms::{DateTimePicker, Switch};
use bencode_ui::menus::{DropdownMenu, Menu, MenuItem};
use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, IntoElement, ParentElement, Styled, div, prelude::*,
};

use jiff::tz::TimeZone;

use super::editor::unsaved_dot;
use super::format::{ago_text, clock, day_and_time, elapsed, next_text, short_day};
use crate::app::automation_overview::{CardStatus, card_status, strip_summary};
use crate::app::automation_pauses::{PAUSE_LENGTHS, local_datetime};
use crate::app::automations::{AutomationPage, automation_matches};
use crate::app::{BenCodeApp, now_ms};
use crate::db::{AutomationRow, RunStatus};
use crate::harness::catalog;
use crate::schedule::schedule_label;
use crate::ui::app_callback::{app_callback, app_callback_with};
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::scrollbar::Scrolled;
use crate::ui::spinner::ring;
use crate::ui::tokens::{Fill, Ink, Status, Text, Tone as _};

impl BenCodeApp {
    pub(super) fn render_automation_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let query = self
            .automations
            .filter_input
            .read(cx)
            .text()
            .trim()
            .to_lowercase();
        let now = now_ms();
        let cards: Vec<AnyElement> = self
            .automations
            .items
            .iter()
            .enumerate()
            .filter(|(_, auto)| {
                automation_matches(auto, &self.rail_project_label(&auto.cwd), &query)
            })
            .map(|(ix, auto)| self.render_automation_card(ix, auto, now, cx))
            .collect();
        let list = if cards.is_empty() {
            div()
                .px_3()
                .py_8()
                .text_center()
                .text_size(Text::Base.px())
                .text_color(colors.fg_muted)
                .child(if query.is_empty() {
                    "No automations yet"
                } else {
                    "No matching automations"
                })
                .into_any_element()
        } else {
            let cards = div()
                .id("automation-cards")
                .flex()
                .flex_col()
                .gap(px(2.0))
                .size_full()
                .p(px(6.0))
                .overflow_y_scroll()
                .children(cards);
            Scrolled::new("automation-cards-scrollbar", cards).into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .h(px(36.0))
                    .pl(px(10.0))
                    .pr_2()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        Icon::new(IconName::Search)
                            .size(IconSize::Xs)
                            .color(colors.fg_muted),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(Text::Base.px())
                            .child(self.automations.filter_input.clone()),
                    )
                    .child(self.render_pause_menu(cx))
                    .child(
                        IconButton::new("automation-new", IconName::Plus)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("New automation")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.show_automation_picker(cx)),
                            ),
                    ),
            )
            .children(self.render_pause_strip(cx))
            .children(self.render_pause_pick(cx))
            .child(
                div()
                    .flex_none()
                    .px(px(6.0))
                    .pt(px(6.0))
                    .child(self.render_overview_entry(cx)),
            )
            .child(div().flex_1().min_h_0().child(list))
    }

    /// Pause all: for a day, three, a week, until a date, or Resume all.
    fn render_pause_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let paused = self.automations_paused_until().is_some();
        let menu = PAUSE_LENGTHS
            .into_iter()
            .fold(Menu::new(), |menu, (label, days)| {
                menu.item(
                    MenuItem::new(format!("Pause for {label}"))
                        .icon(IconName::Pause)
                        .on_click(app_callback(cx, move |this, cx| {
                            this.pause_automations_for(days, cx)
                        })),
                )
            })
            .item(
                MenuItem::new("Pause until…")
                    .icon(IconName::CalendarDays)
                    .on_click(app_callback(cx, |this, cx| this.open_pause_pick(cx))),
            )
            .separator()
            .item(
                MenuItem::new("Resume all")
                    .icon(IconName::Play)
                    .disabled(!paused)
                    .on_click(app_callback(cx, |this, cx| {
                        this.pause_automations_until(None, cx)
                    })),
            );
        DropdownMenu::new("automation-pause-all", "Pause all", menu).trigger(
            IconButton::new("automation-pause-all-button", IconName::Pause)
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .tooltip("Pause all automations"),
        )
    }

    /// "Paused until Mon 2 Nov, 09:00 · Resume" while Pause all holds.
    fn render_pause_strip(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let until = self.automations_paused_until()?;
        let colors = &cx.theme().colors;
        let tz = jiff::tz::TimeZone::system();
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .min_h(px(32.0))
                .px(px(10.0))
                .border_b_1()
                .border_color(colors.border)
                .text_size(Text::Sm.px())
                .child(
                    Icon::new(IconName::Pause)
                        .size(IconSize::Xs)
                        .color(Status::Attention.color(colors.fg)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(colors.fg.ink(Ink::Body))
                        .child(format!("Paused until {}", day_and_time(until, &tz))),
                )
                .child(
                    Button::new("automation-resume-all", "Resume")
                        .variant(ButtonVariant::Link)
                        .size(ControlSize::Sm)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.pause_automations_until(None, cx)),
                        ),
                ),
        )
    }

    /// Pause all › Until…: a date and time, then Pause or Cancel.
    fn render_pause_pick(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let pick = self.automations.pause_pick.as_ref()?;
        let colors = &cx.theme().colors;
        let picker = DateTimePicker::new("automation-pause-until", Some(pick.value))
            .size(ControlSize::Sm)
            .on_change(app_callback_with(cx, |this, value, cx| {
                this.set_pause_pick(value, cx)
            }));
        let picker = match local_datetime(now_ms()) {
            Some(now) => picker.today(now.date()),
            None => picker,
        };
        Some(
            div()
                .flex_none()
                .px(px(10.0))
                .py_2()
                .border_b_1()
                .border_color(colors.border)
                .text_size(Text::Sm.px())
                .child(
                    div()
                        .text_color(colors.fg.ink(Ink::Muted))
                        .child("Skip every run until"),
                )
                .child(div().mt(px(6.0)).child(picker))
                .children(pick.error.map(|error| {
                    div()
                        .mt(px(6.0))
                        .text_color(Status::Failed.color(colors.fg))
                        .child(error)
                }))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .mt_2()
                        .child(
                            Button::new("automation-pause-cancel", "Cancel")
                                .variant(ButtonVariant::Ghost)
                                .size(ControlSize::Sm)
                                .on_click(cx.listener(|this, _, _, cx| this.close_pause_pick(cx))),
                        )
                        .child(
                            Button::new("automation-pause-submit", "Pause")
                                .primary()
                                .size(ControlSize::Sm)
                                .on_click(cx.listener(|this, _, _, cx| this.submit_pause_pick(cx))),
                        ),
                ),
        )
    }

    /// The list's first entry: the overview, lit while it shows.
    fn render_overview_entry(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let active = self.automations.page == AutomationPage::Overview;
        let count = self.automations_needing_you().len();
        div()
            .id("automation-overview-entry")
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(32.0))
            .px(px(10.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .map(|el| {
                if active {
                    el.bg(colors.fg.fill(Fill::Selected))
                } else {
                    el.hover(|style| style.bg(colors.fg.fill(Fill::Hover)))
                }
            })
            .on_click(cx.listener(|this, _, _, cx| this.show_automation_overview(cx)))
            .child(
                Icon::new(IconName::LayoutDashboard)
                    .size(IconSize::Sm)
                    .color(fg.ink(if active { Ink::Strong } else { Ink::Secondary })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(Text::Md.px())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg.ink(if active { Ink::Strong } else { Ink::Body }))
                    .child("Overview"),
            )
            .when(count > 0, |el| {
                el.child(
                    div()
                        .flex_none()
                        .px(px(6.0))
                        .rounded_full()
                        .bg(Status::Attention.color(fg).fill(Fill::Bold))
                        .text_size(Text::Xs.px())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(Status::Attention.color(fg))
                        .child(count.to_string()),
                )
            })
    }

    fn render_automation_card(
        &self,
        ix: usize,
        auto: &AutomationRow,
        now: i64,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let active = self.editing_automation(&auto.id);
        let unsaved = self.automation_stashed(&auto.id) || (active && self.automation_dirty(cx));
        let (open_id, toggle_id) = (auto.id.clone(), auto.id.clone());
        let toggle = cx.listener(move |this, on: &bool, _, cx| {
            this.set_automation_enabled(&toggle_id, *on, cx)
        });
        let tz = TimeZone::system();
        // A switched-off automation reads quieter, not hidden.
        let off = !auto.enabled;
        let muted = fg.ink(if off { Ink::Faint } else { Ink::Muted });
        let name = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_w_0()
            // Room for the switch.
            .pr(px(40.0))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(Text::Md.px())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if off { fg.ink(Ink::Muted) } else { fg })
                    .child(auto.name.clone()),
            )
            .when(unsaved, |el| {
                el.child(
                    div()
                        .id(("automation-card-unsaved", ix))
                        .flex_none()
                        .p(px(2.0))
                        .tooltip(Tooltip::text("Unsaved changes"))
                        .child(unsaved_dot(cx)),
                )
            });
        let status = card_status(
            auto,
            self.automation_running_since(&auto.id),
            self.automations_paused_until(),
            now,
        );
        let line = status_line(&status, &schedule_label(auto), now, &tz);
        let (icon, tone) = match status {
            CardStatus::Running(_) => (None, fg.ink(Ink::Body)),
            CardStatus::Off => (Some(IconName::Pause), muted),
            CardStatus::PausedUntil(_) | CardStatus::Skipping { .. } => {
                (Some(IconName::Pause), Status::Attention.color(fg))
            }
            CardStatus::Scheduled(_) => (Some(IconName::Clock), muted),
        };
        let status = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_w_0()
            .text_size(Text::Sm.px())
            .text_color(tone)
            .map(|el| match icon {
                Some(icon) => el.child(Icon::new(icon).size(IconSize::Xs).color(tone)),
                None => el.child(ring(IconSize::Xs, cx)),
            })
            .child(div().min_w_0().truncate().child(line));
        div()
            .id(("automation-card", ix))
            .relative()
            .flex_none()
            .rounded(px(6.0))
            .map(|el| {
                if active {
                    el.bg(colors.fg.fill(Fill::Selected))
                } else {
                    el.hover(|style| style.bg(colors.fg.fill(Fill::Hover)))
                }
            })
            .child(
                div()
                    .id(("automation-card-open", ix))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .px(px(10.0))
                    .py_2()
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.select_automation(&open_id, cx)),
                    )
                    .child(name)
                    .child(status)
                    .child(self.render_card_meta(ix, auto, now, &tz, muted, cx)),
            )
            .child(
                // Above the card, so a press on the switch does not open it.
                div()
                    .absolute()
                    .top(px(6.0))
                    .right(px(8.0))
                    .occlude()
                    .child(
                        Switch::new(("automation-card-enabled", ix), auto.enabled)
                            .on_change(move |on, window, cx| toggle(&on, window, cx)),
                    ),
            )
            .into_any_element()
    }

    /// A card's third line: its last runs, when it last ran, its project,
    /// and its model's icon.
    fn render_card_meta(
        &self,
        ix: usize,
        auto: &AutomationRow,
        now: i64,
        tz: &TimeZone,
        muted: Hsla,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let strip = self.automation_run_strip(&auto.id, now, tz);
        let ran = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .map(|el| {
                if strip.is_empty() {
                    return el.child("No runs yet");
                }
                let summary = strip_summary(&strip);
                let marks = strip.iter().map(|status| {
                    div()
                        .flex_none()
                        .size(px(6.0))
                        .rounded(px(1.5))
                        .bg(run_tone(*status).color(fg))
                });
                el.child(
                    div()
                        .id(("automation-card-runs", ix))
                        .flex()
                        .items_center()
                        .gap(px(2.0))
                        .py(px(2.0))
                        .tooltip(Tooltip::text(summary))
                        .children(marks),
                )
                .when_some(auto.last_run_at, |el, at| {
                    el.child("·").child(ago_text(at, now))
                })
            });
        let model = catalog::display_label(&auto.harness, &auto.model);
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_w_0()
            .text_size(Text::Sm.px())
            .text_color(muted)
            .child(ran)
            .child(div().flex_none().child("·"))
            .child(crate::ui::figure::figure_still(
                self.project_figure(&auto.cwd),
                12.0,
                self.project_color(&auto.cwd),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(self.rail_project_label(&auto.cwd)),
            )
            .child(
                div()
                    .id(("automation-card-model", ix))
                    .flex_none()
                    .tooltip(Tooltip::text(model))
                    .child(HarnessIcon::new(auto.harness.clone()).size(px(14.0))),
            )
    }
}

/// How a run's status is coloured on a strip and in the overview.
pub(super) fn run_tone(status: RunStatus) -> Status {
    match status {
        RunStatus::Succeeded => Status::Done,
        RunStatus::Failed => Status::Failed,
        RunStatus::Skipped => Status::Attention,
        RunStatus::Cancelled => Status::Quiet,
        RunStatus::Pending | RunStatus::Running => Status::Working,
    }
}

/// A card's second line, for its `status` and schedule `label`.
fn status_line(status: &CardStatus, label: &str, now: i64, tz: &TimeZone) -> String {
    let day_time = |at: i64| format!("{} {}", weekday(at, tz), clock(at, tz));
    match *status {
        CardStatus::Running(since) => format!("Running · {}", elapsed(since, now)),
        CardStatus::Off => "Paused".to_string(),
        CardStatus::PausedUntil(until) => format!("Paused until {}", short_day(until, tz)),
        CardStatus::Skipping { skipped, next } => {
            format!("Skipping {} · next {}", day_time(skipped), day_time(next))
        }
        CardStatus::Scheduled(Some(at)) => format!("{label} · {}", next_text(at, now, tz)),
        CardStatus::Scheduled(None) => label.to_string(),
    }
}

/// "Fri".
fn weekday(at: i64, tz: &TimeZone) -> String {
    short_day(at, tz)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_line_reads_its_status() {
        const HOUR: i64 = 3_600_000;
        let tz = TimeZone::UTC;
        // 2026-10-30 12:00 UTC, a Friday.
        let now = "2026-10-30T12:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond();
        let line = |status: CardStatus| status_line(&status, "Weekdays at 08:00", now, &tz);
        assert_eq!(line(CardStatus::Running(now - 4 * 60_000)), "Running · 4m");
        assert_eq!(line(CardStatus::Off), "Paused");
        assert_eq!(
            line(CardStatus::PausedUntil(now + 3 * 24 * HOUR)),
            "Paused until Mon 2 Nov"
        );
        assert_eq!(
            line(CardStatus::Skipping {
                skipped: now + 20 * HOUR,
                next: now + 68 * HOUR
            }),
            "Skipping Sat 08:00 · next Mon 08:00"
        );
        assert_eq!(
            line(CardStatus::Scheduled(Some(now + 18 * HOUR))),
            "Weekdays at 08:00 · next in 18h"
        );
        assert_eq!(
            line(CardStatus::Scheduled(Some(now + 68 * HOUR))),
            "Weekdays at 08:00 · next Mon 08:00"
        );
        assert_eq!(line(CardStatus::Scheduled(None)), "Weekdays at 08:00");
    }
}
