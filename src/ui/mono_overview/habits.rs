//! MonoCode `MonoHabits` (`HabitsList`, `HabitRow`, `HabitsEmpty`),
//! `HabitPage` and `NewHabitPage`: the Overview's Habits tab. The list
//! groups the day ("Up next today", "Done today", then "Later"); a row
//! opens the habit's page, and Run now, Pause and Remove show on hover. A
//! page replaces the list until Back, as an artifact does.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::forms::{Choice, Select};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, prelude::*,
};

use super::{STROKE, empty_state};
use crate::app::BenCodeApp;
use crate::app::mono_overview::{HabitRoute, OverviewTab};
use crate::monos::Mono;
use crate::monos::habits::{self, Habit, HabitOutcome, HabitRun, TodayState};
use crate::schedule::{ScheduleKind, WEEKDAYS};
use crate::ui::app_callback::on_value;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// MonoCode `when`: "Today 09:00", "Mon 09:00" within the week, else
/// "Oct 8, 09:00".
pub fn when(at: i64, now: i64) -> String {
    let tz = jiff::tz::TimeZone::system();
    let zoned = |ms: i64| jiff::Timestamp::from_millisecond(ms).ok().map(|t| t.to_zoned(tz.clone()));
    let (Some(then), Some(today)) = (zoned(at), zoned(now)) else {
        return String::new();
    };
    if then.date() == today.date() {
        format!("Today {}", then.strftime("%H:%M"))
    } else if (now - at).abs() < 6 * 24 * 60 * 60 * 1000 {
        then.strftime("%a %H:%M").to_string()
    } else {
        then.strftime("%b %-d, %H:%M").to_string()
    }
}

/// MonoCode `duration`: "42s", "3m", "3m 10s".
pub fn duration(ms: i64) -> String {
    let seconds = ((ms as f64) / 1000.0).round().max(1.0) as i64;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let (minutes, rest) = (seconds / 60, seconds % 60);
    if rest > 0 {
        format!("{minutes}m {rest}s")
    } else {
        format!("{minutes}m")
    }
}

/// MonoCode `OUTCOME`: what a run came to, and its dot.
fn outcome_label(outcome: HabitOutcome) -> &'static str {
    match outcome {
        HabitOutcome::Posted => "Messaged you",
        HabitOutcome::Quiet => "Nothing to report",
        HabitOutcome::Failed => "Couldn't finish",
    }
}

fn outcome_dot(outcome: HabitOutcome, mono: Hsla, fg: Hsla) -> Hsla {
    match outcome {
        HabitOutcome::Posted => mono,
        HabitOutcome::Quiet => fg.ink(Ink::Faint),
        HabitOutcome::Failed => gpui::rgba(0xef4444b3).into(),
    }
}

fn group_title(title: &str, count: usize, fg: Hsla) -> impl IntoElement {
    div()
        .px_2()
        .pb(px(6.0))
        .text_size(Text::Sm.px())
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg.ink(Ink::Muted))
        .child(format!("{} ", title.to_uppercase()))
        .child(
            div()
                .text_color(fg.ink(Ink::Faint))
                .child(count.to_string()),
        )
}

/// MonoCode `PageHeader`: Back, the title, then the page's buttons.
fn page_header(
    title: String,
    buttons: Vec<AnyElement>,
    cx: &Context<BenCodeApp>,
) -> impl IntoElement {
    let fg = cx.theme().colors.fg;
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .h(px(40.0))
        .pl_2()
        .pr_3()
        .border_b_1()
        .border_color(fg.opacity(STROKE))
        .child(
            IconButton::new("habit-page-back", IconName::ChevronLeft)
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .tooltip("Back")
                .on_click(cx.listener(|this, _, _, cx| this.close_mono_habit_page(cx))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(Text::Md.px())
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg)
                .child(title),
        )
        .children(buttons)
}

fn section(title: &str, fg: Hsla) -> gpui::Div {
    div()
        .border_t_1()
        .border_color(fg.opacity(STROKE))
        .px_2()
        .pt_3()
        .pb_4()
        .child(
            div()
                .px_2()
                .pb_2()
                .text_size(Text::Sm.px())
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg.ink(Ink::Muted))
                .child(title.to_uppercase()),
        )
}

fn property(label: &str, value: impl IntoElement, fg: Hsla) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .min_h(px(28.0))
        .text_size(Text::Base.px())
        .child(
            div()
                .flex_none()
                .w(px(88.0))
                .text_color(fg.ink(Ink::Muted))
                .child(label.to_string()),
        )
        .child(div().flex_1().min_w_0().text_color(fg.ink(Ink::Strong)).child(value))
}

impl BenCodeApp {
    /// The Habits tab: the day's groups, or the empty clock.
    pub(super) fn render_mono_habits(&self, mono: &Mono, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let Some(list) = &self.monos.overview.habits else {
            // Blank while it loads, as MonoCode's list.
            return div().into_any_element();
        };
        if list.is_empty() {
            return empty_state(
                OverviewTab::Habits,
                format!(
                    "Ask {} to put recurring work on a schedule, like a morning digest or a weekly report.",
                    mono.display_name()
                ),
                fg,
            );
        }
        let now = crate::app::now_ms();
        let today = habits::habits_today(list, now, |id| self.habit_running_since(id));
        let upcoming: Vec<&Habit> = today
            .iter()
            .filter(|item| item.state != TodayState::Done)
            .map(|item| item.habit)
            .collect();
        let done: Vec<&Habit> = today
            .iter()
            .filter(|item| item.state == TodayState::Done)
            .map(|item| item.habit)
            .collect();
        let later: Vec<&Habit> = habits::ordered(list)
            .into_iter()
            .filter(|habit| !today.iter().any(|item| item.habit.id == habit.id))
            .collect();
        let later_title = if today.is_empty() { "Scheduled" } else { "Later" };
        let group = |title: &str, items: Vec<&Habit>| {
            (!items.is_empty()).then(|| {
                div()
                    .child(group_title(title, items.len(), fg))
                    .children(items.into_iter().map(|habit| self.render_habit_row(mono, habit, cx)))
            })
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .pt_1()
            .children(group("Up next today", upcoming))
            .children(group("Done today", done))
            .children(group(later_title, later))
            .children(self.monos.overview.habit_error.clone().map(|error| {
                div()
                    .px_2()
                    .text_size(Text::Base.px())
                    .text_color(cx.theme().colors.danger)
                    .child(error)
            }))
            .into_any_element()
    }

    /// MonoCode `HabitRow`: its tile (a pulse while it runs), its name and
    /// when it runs; Run now, Pause and Remove on hover.
    fn render_habit_row(&self, mono: &Mono, habit: &Habit, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let now = crate::app::now_ms();
        let running = self.habit_running_since(&habit.id);
        let starting = running.is_none() && habits::habit_starting(habit, now);
        let lit = habit.enabled || running.is_some();
        let group = SharedString::from(format!("mono-habit-{}", habit.id));
        let line = match running {
            Some(since) => format!("Running now · {}", duration(now - since)),
            None if starting => "Starting…".to_string(),
            None if habit.enabled => habit.schedule.label(),
            None => format!("Paused · {}", habit.schedule.label()),
        };
        let tile = match running {
            Some(_) => div()
                .size(px(8.0))
                .rounded_full()
                .bg(self.mono_color(mono))
                .opacity(crate::ui::spinner::pulse(cx))
                .into_any_element(),
            None => Icon::new(IconName::Clock)
                .size(IconSize::Sm)
                .color(fg.opacity(if lit { 0.6 } else { 0.3 }))
                .into_any_element(),
        };
        let (run_id, toggle_id, remove_id, open_id) = (
            habit.id.clone(),
            habit.id.clone(),
            habit.id.clone(),
            habit.id.clone(),
        );
        let button = |id: String, icon: IconName, label: &'static str, disabled: bool| {
            IconButton::new(SharedString::from(id), icon)
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .tooltip(label)
                .disabled(disabled)
        };
        div()
            .id(group.clone())
            .group(group.clone())
            .flex()
            .w_full()
            .min_w_0()
            .items_center()
            .rounded(px(6.0))
            .hover(move |s| s.bg(fg.fill(Fill::Hover)))
            .child(
                div()
                    .id(SharedString::from(format!("mono-habit-open-{}", habit.id)))
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(px(10.0))
                    .px_2()
                    .py_2()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.open_mono_habit(&open_id, cx)))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .size(px(28.0))
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .bg(fg.fill(Fill::Hover))
                            .child(tile),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(Text::Base.px())
                                    .line_height(px(20.0))
                                    .text_color(fg.opacity(if lit { 0.85 } else { 0.45 }))
                                    .child(habit.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(Text::Sm.px())
                                    .line_height(px(16.0))
                                    .text_color(fg.ink(Ink::Muted))
                                    .child(line),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .pr_2()
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .child(
                        button(
                            format!("mono-habit-run-{}", habit.id),
                            IconName::Zap,
                            if running.is_some() {
                                "Already running"
                            } else if starting {
                                "Starting"
                            } else {
                                "Run now"
                            },
                            running.is_some() || starting,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.run_mono_habit_now(&run_id, cx))),
                    )
                    .child(
                        button(
                            format!("mono-habit-toggle-{}", habit.id),
                            if habit.enabled { IconName::Pause } else { IconName::Play },
                            if habit.enabled { "Pause" } else { "Resume" },
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_mono_habit(&toggle_id, cx))),
                    )
                    .child(
                        button(
                            format!("mono-habit-remove-{}", habit.id),
                            IconName::Trash2,
                            "Remove",
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.remove_mono_habit(&remove_id, cx))),
                    ),
            )
            .into_any_element()
    }

    /// The page over the Habits list, if one is open.
    pub(super) fn render_habit_route(&self, mono: &Mono, cx: &Context<Self>) -> Option<AnyElement> {
        match self.monos.overview.habit_route.as_ref()? {
            HabitRoute::New => Some(self.render_new_habit(cx)),
            HabitRoute::Habit(id) => {
                // A habit removed while open closes its page.
                let habit = self.monos.overview.habits.as_ref()?.iter().find(|h| &h.id == id)?;
                Some(self.render_habit_page(mono, habit, cx))
            }
        }
    }

    /// MonoCode `HabitPage`: what it does, when, and how its recent runs
    /// went (a run opens onto its report or error).
    fn render_habit_page(&self, mono: &Mono, habit: &Habit, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let now = crate::app::now_ms();
        let running = self.habit_running_since(&habit.id);
        let starting = running.is_none() && habits::habit_starting(habit, now);
        let color = self.mono_color(mono);
        let (run_id, toggle_id, remove_id) = (habit.id.clone(), habit.id.clone(), habit.id.clone());
        let buttons = vec![
            IconButton::new("habit-page-run", IconName::Zap)
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .tooltip(if running.is_some() {
                    "Already running"
                } else if starting {
                    "Starting"
                } else {
                    "Run now"
                })
                .disabled(running.is_some() || starting)
                .on_click(cx.listener(move |this, _, _, cx| this.run_mono_habit_now(&run_id, cx)))
                .into_any_element(),
            IconButton::new(
                "habit-page-toggle",
                if habit.enabled { IconName::Pause } else { IconName::Play },
            )
            .size(ControlSize::Sm)
            .variant(ButtonVariant::Ghost)
            .tooltip(if habit.enabled { "Pause" } else { "Resume" })
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_mono_habit(&toggle_id, cx)))
            .into_any_element(),
            IconButton::new("habit-page-remove", IconName::Trash2)
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .tooltip("Remove")
                .on_click(cx.listener(move |this, _, _, cx| this.remove_mono_habit(&remove_id, cx)))
                .into_any_element(),
        ];
        let runs = habit.runs.clone().unwrap_or_default();
        let next = if starting {
            Some("Starting…".to_string())
        } else {
            habit.enabled.then(|| when(habit.next_run_at, now))
        };
        let recent = section("Recent runs", fg)
            .children(running.map(|since| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .px_2()
                    .py_2()
                    .text_size(Text::Base.px())
                    .child(
                        div()
                            .size(px(8.0))
                            .rounded_full()
                            .bg(color)
                            .opacity(crate::ui::spinner::pulse(cx)),
                    )
                    .child(div().flex_1().text_color(fg.ink(Ink::Strong)).child("Running now"))
                    .child(div().text_color(fg.ink(Ink::Muted)).child(duration(now - since)))
            }))
            .when(runs.is_empty() && running.is_none(), |el| {
                el.child(
                    div()
                        .px_2()
                        .py_3()
                        .text_size(Text::Base.px())
                        .text_color(fg.ink(Ink::Muted))
                        .child("It hasn't run yet."),
                )
            })
            .children(runs.iter().map(|run| self.render_habit_run(habit, run, color, cx)));
        let body = div()
            .id("habit-page-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .px_4()
                    .py_3()
                    .child(property(
                        "Runs",
                        if habit.enabled {
                            habit.schedule.label()
                        } else {
                            "Paused".to_string()
                        },
                        fg,
                    ))
                    .children(next.map(|next| property("Next", next, fg))),
            )
            .child(
                section("What it does", fg).child(
                    div()
                        .px_2()
                        .text_size(Text::Base.px())
                        .line_height(px(20.0))
                        .text_color(fg.opacity(0.75))
                        .child(habit.instructions.clone()),
                ),
            )
            .child(recent);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(page_header(habit.name.clone(), buttons, cx))
            .child(crate::ui::scrollbar::Scrolled::new("habit-page-scrollbar", body))
            .into_any_element()
    }

    /// MonoCode `RunRow`: one past run, opening onto what it said.
    fn render_habit_run(&self, habit: &Habit, run: &HabitRun, color: Hsla, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let now = crate::app::now_ms();
        let detail = run.report.clone().or_else(|| run.error.clone());
        let open = detail.is_some()
            && self
                .monos
                .overview
                .open_runs
                .contains(&(habit.id.clone(), run.at));
        let (habit_id, at) = (habit.id.clone(), run.at);
        let timing = match run.duration_ms {
            0 => when(run.at, now),
            ms => format!("{} · {}", when(run.at, now), duration(ms)),
        };
        div()
            .rounded(px(6.0))
            .when(open, |el| el.bg(fg.fill(Fill::Hover)))
            .child(
                div()
                    .id(SharedString::from(format!("habit-run-{}-{}", habit.id, run.at)))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .px_2()
                    .py_2()
                    .rounded(px(6.0))
                    .text_size(Text::Base.px())
                    .when(detail.is_some(), |el| {
                        el.cursor_pointer()
                            .when(!open, |el| el.hover(move |s| s.bg(fg.fill(Fill::Hover))))
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_habit_run(&habit_id, at, cx)))
                    })
                    .child(
                        div()
                            .flex_none()
                            .size(px(8.0))
                            .rounded_full()
                            .bg(outcome_dot(run.outcome, color, fg)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(fg.ink(Ink::Strong))
                            .child(outcome_label(run.outcome)),
                    )
                    .child(div().flex_none().text_color(fg.ink(Ink::Muted)).child(timing)),
            )
            .when(open, |el| {
                let content: AnyElement = match (&run.report, &run.error) {
                    (Some(report), _) => crate::ui::transcript::markdown::AgentMarkdown::new(
                        SharedString::from(format!("habit-run-report-{}-{}", habit.id, run.at)),
                        report.clone(),
                    )
                    .into_any_element(),
                    (None, Some(error)) => div()
                        .text_color(gpui::rgba(0xef4444cc))
                        .child(error.clone())
                        .into_any_element(),
                    _ => div().into_any_element(),
                };
                el.child(
                    div()
                        .pb_3()
                        .pl(px(26.0))
                        .pr_3()
                        .text_size(Text::Base.px())
                        .line_height(px(20.0))
                        .text_color(fg.opacity(0.75))
                        .child(content),
                )
            })
            .into_any_element()
    }

    /// MonoCode `NewHabitPage`: its name, when it runs, what it does, and
    /// Create.
    fn render_new_habit(&self, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let Some(form) = &self.monos.overview.habit_form else {
            return div().into_any_element();
        };
        let ready = self.new_habit_ready(cx);
        let create = div()
            .id("new-habit-create")
            .flex()
            .items_center()
            .h(px(24.0))
            .px_2()
            .rounded(px(6.0))
            .text_size(Text::Base.px())
            .bg(fg.fill(Fill::Strong))
            .text_color(fg)
            .map(|el| {
                if ready {
                    el.cursor_pointer()
                        .hover(move |s| s.bg(fg.opacity(0.14)))
                        .on_click(cx.listener(|this, _, _, cx| this.create_mono_habit(cx)))
                } else {
                    el.opacity(0.4)
                }
            })
            .child("Create")
            .into_any_element();
        let schedule = &form.schedule;
        let kind = ScheduleKind::parse(&schedule.schedule_kind).unwrap_or(ScheduleKind::Daily);
        let kinds = [
            ScheduleKind::Daily,
            ScheduleKind::Weekdays,
            ScheduleKind::Weekly,
            ScheduleKind::Hourly,
        ]
        .into_iter()
        .map(|kind| Choice::new(kind.id(), kind.label()));
        let pill = |width: f32, select: Select| div().flex_none().w(px(width)).child(select);
        let mut fields = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .py_0p5()
            .child(pill(
                112.0,
                Select::new("new-habit-kind", kinds)
                    .label("Repeats")
                    .size(ControlSize::Sm)
                    .selected(kind.id())
                    .on_change(on_value(cx, |this, value, cx| {
                        let value = value.to_string();
                        this.set_new_habit_schedule(|s| s.schedule_kind = value, cx);
                    })),
            ));
        if kind == ScheduleKind::Weekly {
            let days = WEEKDAYS
                .into_iter()
                .enumerate()
                .map(|(day, label)| Choice::new(day.to_string(), label));
            fields = fields.child(div().text_color(fg.ink(Ink::Muted)).child("on")).child(pill(
                120.0,
                Select::new("new-habit-day", days)
                    .label("Day")
                    .size(ControlSize::Sm)
                    .selected(schedule.day_of_week.to_string())
                    .on_change(on_value(cx, |this, value, cx| {
                        let day = value.parse::<u8>().unwrap_or(1).min(6);
                        this.set_new_habit_schedule(|s| s.day_of_week = day, cx);
                    })),
            ));
        }
        fields = fields.child(div().text_color(fg.ink(Ink::Muted)).child("at"));
        fields = if kind == ScheduleKind::Hourly {
            let minutes = [0u8, 15, 30, 45]
                .into_iter()
                .map(|minute| Choice::new(minute.to_string(), format!(":{minute:02}")));
            fields.child(pill(
                84.0,
                Select::new("new-habit-minute", minutes)
                    .label("Minute")
                    .size(ControlSize::Sm)
                    .selected(schedule.minute.to_string())
                    .on_change(on_value(cx, |this, value, cx| {
                        let minute = value.parse::<u8>().unwrap_or(0).min(59);
                        this.set_new_habit_schedule(|s| s.minute = minute, cx);
                    })),
            ))
        } else {
            let times = crate::schedule::time_options(&schedule.time)
                .into_iter()
                .map(|time| Choice::new(time.clone(), time));
            fields.child(pill(
                92.0,
                Select::new("new-habit-time", times)
                    .label("Time")
                    .size(ControlSize::Sm)
                    .selected(schedule.time.clone())
                    .on_change(on_value(cx, |this, value, cx| {
                        let time = value.to_string();
                        this.set_new_habit_schedule(|s| s.time = time, cx);
                    })),
            ))
        };
        let next = when(schedule.next_run_at(crate::app::now_ms()), crate::app::now_ms());
        let body = div()
            .id("new-habit-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(div().px_2().pt_3().child(form.name.clone()))
            .child(
                div()
                    .px_4()
                    .py_3()
                    .child(property("Runs", fields, fg))
                    .child(property("Next", next, fg)),
            )
            .child(
                section("What it does", fg)
                    .child(div().px_2().child(form.instructions.clone()))
                    .children(form.error.clone().map(|error| {
                        div()
                            .px_2()
                            .pt_2()
                            .text_size(Text::Base.px())
                            .text_color(gpui::rgba(0xef4444cc))
                            .child(error)
                    })),
            );
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(page_header("New habit".to_string(), vec![create], cx))
            .child(crate::ui::scrollbar::Scrolled::new("new-habit-scrollbar", body))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_short() {
        assert_eq!(duration(400), "1s");
        assert_eq!(duration(42_000), "42s");
        assert_eq!(duration(180_000), "3m");
        assert_eq!(duration(190_000), "3m 10s");
    }
}
