//! The Automations overview, where the surface lands: a summary and New
//! automation, Pause all's banner, then Running now, Needs you, Upcoming
//! and Recent results (by day, one row per automation a day), each left out while empty; with no
//! automations at all, a short start and the popular templates. What it
//! reads is chosen in `app/automation_overview.rs`. BenCode's own.

use bencode_ui::buttons::{Button, ButtonVariant};
use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, ClickEvent, Context, Div, FontWeight, Hsla, IntoElement, ParentElement, Styled,
    div, prelude::*,
};
use jiff::tz::TimeZone;

use super::PAGE_WIDTH;
use super::format::{clock, day_and_time, elapsed, in_text, run_duration, when_text};
use super::list::run_tone;
use super::templates::{TemplateCategory, templates_for};
use crate::app::automation_overview::{
    Attention, NeedsYou, ResultGroup, Upcoming, finished_at, group_results, recent_results,
};
use crate::app::{BenCodeApp, now_ms};
use crate::db::{AutomationRow, AutomationRunRow, RunStatus, RunTrigger};
use crate::schedule;
use crate::ui::rows::ListRow;
use crate::ui::scale::px;
use crate::ui::scrollbar::Scrolled;
use crate::ui::settings_parts::{panel, section_title};
use crate::ui::spinner::ring;
use crate::ui::tokens::{Fill, Ink, Status, Text, Tone as _};

/// A section: its title, then its rows in a panel.
fn section(title: &'static str, fg: Hsla, rows: Vec<AnyElement>) -> Div {
    div()
        .mt_8()
        .child(section_title(title, fg.ink(Ink::Muted)))
        .child(panel(fg).mt_3().p(px(4.0)).children(rows))
}

/// What a finished run's mark is.
fn status_icon(status: RunStatus) -> IconName {
    match status {
        RunStatus::Succeeded => IconName::CircleCheck,
        RunStatus::Failed => IconName::CircleX,
        RunStatus::Skipped => IconName::Ban,
        RunStatus::Cancelled => IconName::CircleStop,
        RunStatus::Pending | RunStatus::Running => IconName::Clock,
    }
}

/// "Scheduled", "Test run".
fn trigger_name(trigger: RunTrigger) -> &'static str {
    match trigger {
        RunTrigger::Scheduled => "Scheduled",
        RunTrigger::Manual => "Test run",
        RunTrigger::Event => "Event",
    }
}

impl BenCodeApp {
    pub(super) fn render_automation_overview(&self, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let (tz, now) = (TimeZone::system(), now_ms());
        let column = div()
            .mx_auto()
            .w_full()
            .max_w(px(PAGE_WIDTH))
            .px_8()
            .pt_5()
            .pb_10()
            .child(self.render_overview_header(now, &tz, cx))
            .children(self.render_overview_pause(&tz, cx));
        let column = if self.automations.items.is_empty() {
            column.child(self.render_overview_empty(cx))
        } else {
            column
                .children(self.render_running_now(now, cx))
                .children(self.render_needs_you(cx))
                .children(self.render_upcoming(now, &tz, cx))
                .children(self.render_recent_results(now, &tz, cx))
                .when(self.automations.recent.is_empty(), |el| {
                    el.child(
                        div()
                            .mt_8()
                            .text_size(Text::Base.px())
                            .text_color(fg.ink(Ink::Muted))
                            .child("Nothing has run yet. Runs show here as they finish."),
                    )
                })
        };
        let page = div()
            .id("automation-overview")
            .size_full()
            .overflow_y_scroll()
            .child(column);
        Scrolled::new("automation-overview-scrollbar", page).into_any_element()
    }

    fn automation_named(&self, id: &str) -> Option<&AutomationRow> {
        self.automations.items.iter().find(|a| a.id == id)
    }

    /// "Automations", "3 active · 1 paused · next: Morning standup tomorrow
    /// 08:00", and New automation.
    fn render_overview_header(&self, now: i64, tz: &TimeZone, cx: &Context<Self>) -> Div {
        let fg = cx.theme().colors.fg;
        let items = &self.automations.items;
        let active = items.iter().filter(|a| a.enabled).count();
        let mut summary = vec![format!("{active} active")];
        if active < items.len() {
            summary.push(format!("{} paused", items.len() - active));
        }
        // What falls in Pause all is skipped: the next run is after it.
        let paused = self.automations_paused_until();
        let next = items
            .iter()
            .filter(|a| a.enabled && !schedule::time_triggers(a).is_empty())
            .filter_map(|a| {
                let at = match paused {
                    Some(until) => schedule::next_automation_run_at(a, until - 1, tz)?,
                    None => a.next_run_at,
                };
                (at > now).then_some((at, a.name.as_str()))
            })
            .min();
        if let Some((at, name)) = next {
            summary.push(format!("next: {name} {}", when_text(at, now, tz)));
        }
        div()
            .flex()
            .items_start()
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(Text::Title.px())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child("Automations"),
                    )
                    .when(!items.is_empty(), |el| {
                        el.child(
                            div()
                                .mt(px(6.0))
                                .text_size(Text::Md.px())
                                .text_color(fg.ink(Ink::Muted))
                                .child(summary.join(" · ")),
                        )
                    }),
            )
            // With none yet, the empty state offers it.
            .when(!items.is_empty(), |el| {
                el.child(
                    Button::new("automation-overview-new", "New automation")
                        .primary()
                        .size(ControlSize::Sm)
                        .icon(IconName::Plus)
                        .on_click(cx.listener(|this, _, _, cx| this.show_automation_picker(cx))),
                )
            })
    }

    /// "All automations are paused until … · Resume", while Pause all holds.
    fn render_overview_pause(&self, tz: &TimeZone, cx: &Context<Self>) -> Option<Div> {
        let until = self.automations_paused_until()?;
        let fg = cx.theme().colors.fg;
        let amber = Status::Attention.color(fg);
        Some(
            div()
                .flex()
                .items_center()
                .gap_2()
                .mt_5()
                .px_3()
                .py_2()
                .rounded(px(8.0))
                .border_1()
                .border_color(amber.fill(Fill::Bold))
                .bg(amber.fill(Fill::Hover))
                .text_size(Text::Base.px())
                .child(Icon::new(IconName::Pause).size(IconSize::Sm).color(amber))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(fg.ink(Ink::Body))
                        .child(format!(
                            "All automations are paused until {}",
                            day_and_time(until, tz)
                        )),
                )
                .child(
                    Button::new("automation-overview-resume", "Resume")
                        .variant(ButtonVariant::Link)
                        .size(ControlSize::Sm)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.pause_automations_until(None, cx)),
                        ),
                ),
        )
    }

    /// Running now: each run in flight, how long it has run, and Stop.
    fn render_running_now(&self, now: i64, cx: &Context<Self>) -> Option<Div> {
        let fg = cx.theme().colors.fg;
        let running = self.running_automations();
        if running.is_empty() {
            return None;
        }
        let rows = running
            .into_iter()
            .enumerate()
            .map(|(ix, (session, link))| {
                let auto = self.automation_named(&link.automation_id);
                let name = auto.map_or("Automation", |a| a.name.as_str());
                let (open, stop) = (session.to_string(), session.to_string());
                ListRow::new(format!("automation-running-{ix}"), name.to_string())
                    .tile(ring(IconSize::Sm, cx))
                    .tooltip("Open thread")
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_automation_run(&open, cx)
                    }))
                    .trailing(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(Text::Sm.px())
                                    .text_color(fg.ink(Ink::Muted))
                                    .child(format!("Running · {}", elapsed(link.started_at, now))),
                            )
                            .child(
                                Button::new(("automation-running-stop", ix), "Stop")
                                    .variant(ButtonVariant::Outline)
                                    .size(ControlSize::Sm)
                                    .icon(IconName::Square)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.stop_agent(&stop, cx)
                                    })),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        Some(section("Running now", fg, rows))
    }

    /// Needs you: a prompt waiting, a failed or missed last run.
    fn render_needs_you(&self, cx: &Context<Self>) -> Option<Div> {
        let fg = cx.theme().colors.fg;
        let needs = self.automations_needing_you();
        if needs.is_empty() {
            return None;
        }
        let rows = needs
            .iter()
            .enumerate()
            .filter_map(|(ix, need)| self.render_need(ix, need, cx))
            .collect();
        Some(section("Needs you", fg, rows))
    }

    fn render_need(&self, ix: usize, need: &NeedsYou, cx: &Context<Self>) -> Option<AnyElement> {
        let fg = cx.theme().colors.fg;
        let auto = self.automation_named(&need.automation_id)?;
        let (icon, tone) = match need.kind {
            Attention::Approval => (IconName::Shield, Status::Attention),
            Attention::Failed => (IconName::CircleX, Status::Failed),
            Attention::Missed => (IconName::Clock, Status::Attention),
            Attention::Interrupted => (IconName::CircleStop, Status::Attention),
        };
        let open = need.session_id.clone().map(|session| {
            Button::new(("automation-need-open", ix), "Open thread")
                .variant(if need.kind == Attention::Approval {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Ghost
                })
                .size(ControlSize::Sm)
                .on_click(cx.listener(move |this, _, _, cx| this.open_automation_run(&session, cx)))
        });
        let finished = need.run_id.clone().map(|run_id| {
            let id = auto.id.clone();
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    Button::new(("automation-need-run", ix), "Run now")
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Sm)
                        .icon(IconName::Play)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.run_stored_automation(&id, cx)),
                        ),
                )
                .child(
                    Button::new(("automation-need-dismiss", ix), "Dismiss")
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.dismiss_automation_run(&run_id, cx)
                        })),
                )
        });
        let id = auto.id.clone();
        Some(
            ListRow::new(format!("automation-need-{ix}"), auto.name.clone())
                .tile(Icon::new(icon).size(IconSize::Sm).color(tone.color(fg)))
                .line(need.text.clone())
                .tooltip(need.text.clone())
                .dim(!auto.enabled)
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.select_automation(&id, cx)),
                )
                .trailing(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .children(open)
                        .children(finished),
                )
                .into_any_element(),
        )
    }

    /// Recent results: the week's finished runs by day, newest first, one
    /// row per automation per day; a row with a run finished since the
    /// last look is marked New.
    fn render_recent_results(&self, now: i64, tz: &TimeZone, cx: &Context<Self>) -> Option<Div> {
        let fg = cx.theme().colors.fg;
        let runs: Vec<&AutomationRunRow> = recent_results(&self.automations.recent, now)
            .into_iter()
            .filter(|run| self.automation_named(&run.automation_id).is_some())
            .collect();
        if runs.is_empty() {
            return None;
        }
        // Not looked at yet: nothing is new.
        let seen = self.automations.seen_before.unwrap_or(i64::MAX);
        let mut ix = 0;
        let days = group_results(&runs, now, tz)
            .into_iter()
            .map(|(day, groups)| {
                let rows: Vec<AnyElement> = groups
                    .iter()
                    .map(|group| {
                        ix += 1;
                        self.render_result(ix, group, group.new_since(seen), now, tz, cx)
                    })
                    .collect();
                div()
                    .mt_3()
                    .child(
                        div()
                            .px_1()
                            .text_size(Text::Sm.px())
                            .text_color(fg.ink(Ink::Faint))
                            .child(day),
                    )
                    .child(panel(fg).mt(px(6.0)).p(px(4.0)).children(rows))
            });
        Some(
            div()
                .mt_8()
                .child(section_title("Recent results", fg.ink(Ink::Muted)))
                .children(days),
        )
    }

    /// An automation's runs of one day, read from the latest. Its title
    /// line has room for a second line under it (`ListRow::line`), for
    /// what the run reported. One run opens its thread; several open the
    /// automation's Run history.
    fn render_result(
        &self,
        ix: usize,
        group: &ResultGroup,
        new: bool,
        now: i64,
        tz: &TimeZone,
        cx: &Context<Self>,
    ) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let run = group.latest();
        let name = self
            .automation_named(&run.automation_id)
            .map_or_else(String::new, |a| a.name.clone());
        let status = group.status();
        let tone = run_tone(status).color(fg);
        let mut meta = format!(
            "{} · {} · {}",
            trigger_name(run.trigger),
            clock(finished_at(run), tz),
            run_duration(run, now)
        );
        let count = group.count_text();
        if let Some(count) = &count {
            meta.push_str(" · ");
            meta.push_str(count);
        }
        let accent = cx.theme().colors.accent;
        let row = ListRow::new(format!("automation-result-{ix}"), name)
            .tile(
                Icon::new(status_icon(status))
                    .size(IconSize::Sm)
                    .color(tone),
            )
            .trailing(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(new, |el| {
                        el.child(
                            div()
                                .px(px(6.0))
                                .rounded_full()
                                .bg(accent.fill(Fill::Bold))
                                .text_size(Text::Xs.px())
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(accent)
                                .child("New"),
                        )
                    })
                    .child(
                        div()
                            .text_size(Text::Sm.px())
                            .text_color(if group.failed() > 0 {
                                tone
                            } else {
                                fg.ink(Ink::Muted)
                            })
                            .child(meta),
                    ),
            );
        if count.is_some() {
            let id = run.automation_id.clone();
            return row
                .tooltip("Open run history")
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.open_automation_history(&id, cx)
                }))
                .into_any_element();
        }
        match (run.session_id.clone(), &run.error) {
            (Some(session), _) => row.tooltip("Open thread").on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| this.open_automation_run(&session, cx),
            )),
            (None, Some(error)) => row.tooltip(error.clone()),
            (None, None) => row,
        }
        .into_any_element()
    }

    /// Upcoming: the next day's runs, soonest first.
    fn render_upcoming(&self, now: i64, tz: &TimeZone, cx: &Context<Self>) -> Option<Div> {
        let fg = cx.theme().colors.fg;
        let upcoming = self.upcoming_automation_runs(now, tz);
        if upcoming.is_empty() {
            return None;
        }
        let rows = upcoming
            .iter()
            .enumerate()
            .filter_map(|(ix, row)| self.render_upcoming_row(ix, row, now, tz, cx))
            .collect();
        Some(section("Upcoming", fg, rows))
    }

    fn render_upcoming_row(
        &self,
        ix: usize,
        row: &Upcoming,
        now: i64,
        tz: &TimeZone,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let fg = cx.theme().colors.fg;
        let auto = self.automation_named(&row.automation_id)?;
        let mut when = format!("{} · {}", when_text(row.at, now, tz), in_text(row.at, now));
        if row.more > 0 {
            when.push_str(&format!(" · then {} more", row.more));
        }
        let text =
            |text: String, ink: Hsla| div().text_size(Text::Sm.px()).text_color(ink).child(text);
        let amber = Status::Attention.color(fg);
        let trailing = div().flex().items_center().gap_2().map(|el| {
            if row.skipped {
                let id = auto.id.clone();
                el.child(text("Skipped".to_string(), amber))
                    .child(text(when.clone(), fg.ink(Ink::Faint)))
                    .child(
                        Button::new(("automation-upcoming-undo", ix), "Undo")
                            .variant(ButtonVariant::Link)
                            .size(ControlSize::Sm)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.skip_automation_run(&id, true, cx)
                            })),
                    )
            } else if row.paused {
                el.child(text("Paused".to_string(), amber))
                    .child(text(when.clone(), fg.ink(Ink::Faint)))
            } else {
                el.child(text(when.clone(), fg.ink(Ink::Muted)))
            }
        });
        let id = auto.id.clone();
        Some(
            ListRow::new(format!("automation-upcoming-{ix}"), auto.name.clone())
                .tile(
                    Icon::new(IconName::Clock)
                        .size(IconSize::Sm)
                        .color(fg.ink(Ink::Muted)),
                )
                .dim(row.skipped || row.paused)
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.select_automation(&id, cx)),
                )
                .trailing(trailing)
                .into_any_element(),
        )
    }

    /// No automations yet: what they are for, New automation, and the
    /// popular templates to start from.
    fn render_overview_empty(&self, cx: &Context<Self>) -> Div {
        let fg = cx.theme().colors.fg;
        let cards = templates_for(TemplateCategory::Popular)
            .enumerate()
            .map(|(ix, template)| self.render_template_card(ix, template, cx));
        div()
            .mt_8()
            .child(
                panel(fg)
                    .border_dashed()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .px_6()
                    .py_10()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(40.0))
                            .rounded(px(10.0))
                            .bg(fg.fill(Fill::Selected))
                            .child(
                                Icon::new(IconName::Zap)
                                    .size(IconSize::Md)
                                    .color(fg.ink(Ink::Body)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(Text::Lg.px())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg)
                            .child("No automations yet"),
                    )
                    .child(
                        div()
                            .text_size(Text::Md.px())
                            .text_color(fg.ink(Ink::Muted))
                            .child(
                                "An automation runs an agent on a schedule: a standup summary \
                                 each morning, a test sweep each night.",
                            ),
                    )
                    .child(
                        Button::new("automation-empty-new", "New automation")
                            .primary()
                            .size(ControlSize::Sm)
                            .icon(IconName::Plus)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.show_automation_picker(cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .mt_8()
                    .child(section_title("Popular", fg.ink(Ink::Muted))),
            )
            .child(div().grid().grid_cols(2).gap_3().mt_3().children(cards))
    }
}
