//! A Backlog issue's fields under its actions, each editable where it is
//! shown (Backlog's issue page: assignee, priority, milestones,
//! resolution, start and due dates, estimated and actual hours). Each
//! change is one `PATCH` (`app/backlog.rs`); the fields wait while one is
//! on its way.

use bencode_ui::buttons::{ButtonVariant, IconButton};
use bencode_ui::forms::{Choice, DatePicker, Input, MultiSelect, Select, TextInput};
use bencode_ui::primitives::IconName;
use bencode_ui::theme::{ActiveTheme, ControlSize};
use gpui::{
    AnyElement, Context, Entity, IntoElement, ParentElement, SharedString, Styled, div, prelude::*,
};
use jiff::civil::Date;

use crate::app::BenCodeApp;
use crate::backlog::{Change, FieldChoice, IssueFields};
use crate::github::WorkItem;
use crate::ui::app_callback::{app_callback, app_callback_with, on_value};
use crate::ui::scale::px;
use crate::ui::tokens::{Ink, Text, Tone as _};

/// A field's width: two or three to a row in the detail's column.
const FIELD_WIDTH: f32 = 208.0;
/// The `Select` value that clears a field.
const NONE: &str = "none";

fn choices<'a>(named: impl IntoIterator<Item = &'a FieldChoice>) -> Vec<Choice> {
    named
        .into_iter()
        .map(|n| Choice::new(n.id.to_string(), n.name.clone()))
        .collect()
}

/// A `Select`'s change to the issue `key`, as `make` reads the value.
fn change(
    key: &str,
    cx: &Context<BenCodeApp>,
    make: fn(&str) -> Option<Change>,
) -> impl Fn(&SharedString, &mut gpui::Window, &mut gpui::App) + 'static {
    let key = key.to_string();
    on_value(cx, move |this, value, cx| {
        if let Some(change) = make(value) {
            this.update_backlog_issue(&key, change, cx);
        }
    })
}

/// A picked choice's id; `NONE` is none.
fn id_of(value: &str) -> Option<i64> {
    value.parse().ok()
}

fn day(date: &Option<String>) -> Option<Date> {
    Date::strptime("%Y-%m-%d", date.as_deref()?).ok()
}

impl BenCodeApp {
    /// The fields, once the issue and its project's choices are loaded.
    pub(super) fn render_backlog_fields(
        &self,
        item: &WorkItem,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let key = item.key();
        let fields = (self.inbox(cx).details.get(&key))?
            .as_ref()
            .ok()?
            .details
            .backlog
            .clone()?;
        let options = match self.backlog.field_options.get(&item.container_id) {
            Some(Ok(options)) => Some(options.clone()),
            _ => None,
        };
        let busy = self.backlog.issue_busy.is_some() || options.is_none();
        let options = options.unwrap_or_default();
        let IssueFields {
            assignee,
            priority,
            milestones,
            resolution,
            start_date,
            due_date,
            ..
        } = fields;
        let me = options.me;
        let mine = assignee.as_ref().is_some_and(|a| a.id == me);
        let assign_key = item.key();
        let assignee_field = field(
            "Assignee",
            (!mine && !busy && me != 0).then(|| {
                action(
                    "backlog-assign-me",
                    "Assign to me",
                    app_callback(cx, move |this, cx| {
                        this.update_backlog_issue(&assign_key, Change::Assignee(Some(me)), cx)
                    }),
                    cx,
                )
            }),
            Select::new(
                "backlog-assignee",
                std::iter::once(Choice::new(NONE, "Unassigned")).chain(options.users.iter().map(
                    |user| {
                        Choice::new(user.id.to_string(), user.name.clone())
                            .avatar(user.name.clone())
                    },
                )),
            )
            .size(ControlSize::Sm)
            .disabled(busy)
            .selected(assignee.map_or(NONE.to_string(), |a| a.id.to_string()))
            .on_change(change(&key, cx, |value| {
                Some(Change::Assignee(id_of(value)))
            })),
            cx,
        );

        let priority_field = field(
            "Priority",
            None,
            Select::new("backlog-priority", choices(&options.priorities))
                .size(ControlSize::Sm)
                .disabled(busy)
                .placeholder(priority.as_ref().map_or("—".into(), |p| p.name.clone()))
                .when_some(priority, |el, p| el.selected(p.id.to_string()))
                .on_change(change(&key, cx, |value| id_of(value).map(Change::Priority))),
            cx,
        );

        let milestone_key = item.key();
        // Milestones the picker cannot show (archived, or newer than its
        // options) stay on the issue: the PATCH replaces the whole list.
        let unlisted: Vec<_> = (milestones.iter())
            .filter(|m| !options.milestones.iter().any(|o| o.id == m.id))
            .map(|m| m.id)
            .collect();
        let entity = cx.entity().downgrade();
        // Ely's `MultiSelect` refuses an empty list: before the options
        // load, or in a project without milestones, the issue's own show
        // in a closed `Select`.
        let shown = match milestones.as_slice() {
            [] => "No milestone".to_string(),
            some => (some.iter().map(|m| m.name.as_str()))
                .collect::<Vec<_>>()
                .join(", "),
        };
        let milestone_picker = if options.milestones.is_empty() {
            Select::new("backlog-milestones", Vec::<Choice>::new())
                .size(ControlSize::Sm)
                .disabled(true)
                .placeholder(shown)
                .into_any_element()
        } else {
            MultiSelect::new("backlog-milestones", choices(&options.milestones))
                .size(ControlSize::Sm)
                .placeholder("No milestone")
                .selected(milestones.iter().map(|m| m.id.to_string()))
                .on_change(move |values, _, cx| {
                    if busy {
                        return;
                    }
                    let ids = (unlisted.iter().copied())
                        .chain(values.iter().filter_map(|v| v.parse().ok()))
                        .collect();
                    let key = milestone_key.clone();
                    if let Err(err) = entity.update(cx, |this, cx| {
                        this.update_backlog_issue(&key, Change::Milestones(ids), cx)
                    }) {
                        log::debug!("milestones after app drop: {err:#}");
                    }
                })
                .into_any_element()
        };
        let milestone_field = field(
            "Milestone",
            None,
            div().w_full().child(milestone_picker),
            cx,
        );

        let resolution_field = field(
            "Resolution",
            None,
            Select::new(
                "backlog-resolution",
                std::iter::once(Choice::new(NONE, "No resolution"))
                    .chain(choices(&options.resolutions)),
            )
            .size(ControlSize::Sm)
            .disabled(busy)
            .selected(resolution.map_or(NONE.to_string(), |r| r.id.to_string()))
            .on_change(change(&key, cx, |value| {
                Some(Change::Resolution(id_of(value)))
            })),
            cx,
        );

        let start_field =
            self.date_field(item, "Start date", start_date, busy, Change::StartDate, cx);
        let due_field = self.date_field(item, "Due date", due_date, busy, Change::DueDate, cx);

        let hours_field = field(
            "Estimate / Actual (h)",
            None,
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(self.hours(&self.backlog_estimate_input))
                .child(
                    div()
                        .text_size(Text::Sm.px())
                        .text_color(cx.theme().colors.fg.ink(Ink::Faint))
                        .child("/"),
                )
                .child(self.hours(&self.backlog_actual_input)),
            cx,
        );

        Some(
            div()
                .flex()
                .flex_wrap()
                .gap_x_4()
                .gap_y_3()
                .pt_1()
                .child(assignee_field)
                .child(priority_field)
                .child(milestone_field)
                .child(resolution_field)
                .child(start_field)
                .child(due_field)
                .child(hours_field)
                .into_any_element(),
        )
    }

    /// A day picker, with a clear button once a day is set.
    fn date_field(
        &self,
        item: &WorkItem,
        label: &'static str,
        value: Option<String>,
        busy: bool,
        make: fn(Option<String>) -> Change,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let id = SharedString::from(format!(
            "backlog-{}",
            label.to_lowercase().replace(' ', "-")
        ));
        let set = day(&value);
        let (pick_key, clear_key) = (item.key(), item.key());
        let clear = set.filter(|_| !busy).map(|_| {
            IconButton::new(SharedString::from(format!("{id}-clear")), IconName::X)
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .tooltip(format!("Clear the {}", label.to_lowercase()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.update_backlog_issue(&clear_key, make(None), cx)
                }))
        });
        field(
            label,
            None,
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div().flex_1().min_w_0().child(
                        DatePicker::new(id.clone(), set)
                            .size(ControlSize::Sm)
                            .placeholder("Not set")
                            .today(jiff::Zoned::now().date())
                            .disabled(busy)
                            .on_change(app_callback_with(cx, move |this, date: Date, cx| {
                                let day = date.strftime("%Y-%m-%d").to_string();
                                this.update_backlog_issue(&pick_key, make(Some(day)), cx)
                            })),
                    ),
                )
                .children(clear),
            cx,
        )
    }

    /// Hours as typed: plain text, sent on Enter or blur and only when it
    /// changed (`commit_backlog_hours`), never per keystroke or step.
    fn hours(&self, input: &Entity<TextInput>) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .child(Input::new(input).size(ControlSize::Sm))
    }

    /// The selected issue's hours, and the inputs' text for them.
    fn shown_hours(&self, cx: &gpui::App) -> Option<(String, Option<f64>, Option<f64>)> {
        let key = self.inbox(cx).selected.clone()?;
        let fields = (self.inbox(cx).details.get(&key))?
            .as_ref()
            .ok()?
            .details
            .backlog
            .as_ref()?;
        Some((key, fields.estimated_hours, fields.actual_hours))
    }

    /// Puts the selected issue's hours in the inputs when the issue or its
    /// hours change; what is typed stays until then.
    pub(crate) fn sync_backlog_hours(&mut self, cx: &mut Context<Self>) {
        let shown = self.shown_hours(cx);
        if shown == self.backlog.hours_shown {
            return;
        }
        let (estimate, actual) = shown
            .as_ref()
            .map_or((None, None), |(_, estimate, actual)| (*estimate, *actual));
        for (input, hours) in [
            (&self.backlog_estimate_input, estimate),
            (&self.backlog_actual_input, actual),
        ] {
            let text = hours.map(format_hours).unwrap_or_default();
            if input.read(cx).text() != text {
                input.update(cx, |input, cx| input.set_text(text, cx));
            }
        }
        self.backlog.hours_shown = shown;
    }

    /// Enter or blur in an hours input: the one that changed is sent; an
    /// unreadable number goes back to the issue's.
    pub(crate) fn commit_backlog_hours(&mut self, cx: &mut Context<Self>) {
        let Some((key, estimate, actual)) = self.shown_hours(cx) else {
            return;
        };
        let typed = |input: &Entity<TextInput>| parse_hours(input.read(cx).text());
        let change = match (
            typed(&self.backlog_estimate_input),
            typed(&self.backlog_actual_input),
        ) {
            (Some(typed), _) if typed != estimate => Some(Change::EstimatedHours(typed)),
            (_, Some(typed)) if typed != actual => Some(Change::ActualHours(typed)),
            (Some(_), Some(_)) => None,
            _ => {
                // Shows the issue's hours again.
                self.backlog.hours_shown = None;
                cx.notify();
                None
            }
        };
        if let Some(change) = change {
            self.update_backlog_issue(&key, change, cx);
        }
    }
}

/// Hours as typed: empty is none, a comma reads as a point; `None` when
/// it is no number of hours.
fn parse_hours(text: &str) -> Option<Option<f64>> {
    let text = text.trim().replace(',', ".");
    if text.is_empty() {
        return Some(None);
    }
    let hours: f64 = text.parse().ok()?;
    (hours.is_finite() && hours >= 0.0).then(|| Some((hours * 100.0).round() / 100.0))
}

/// `1`, `1.5`, `0.25`: no trailing zeros.
fn format_hours(hours: f64) -> String {
    format!("{hours}")
}

/// A field: its name (and a shortcut) over its control.
fn field(
    label: &'static str,
    shortcut: Option<AnyElement>,
    control: impl IntoElement,
    cx: &Context<BenCodeApp>,
) -> impl IntoElement {
    let fg = cx.theme().colors.fg;
    div()
        .w(px(FIELD_WIDTH))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .h(px(18.0))
                .text_size(Text::Sm.px())
                .text_color(fg.ink(Ink::Muted))
                .child(label)
                .children(shortcut),
        )
        .child(control)
}

/// A link-like shortcut beside a field's name, as Backlog's "Assign to
/// myself".
fn action(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
    cx: &Context<BenCodeApp>,
) -> AnyElement {
    let colors = &cx.theme().colors;
    let (link, fg) = (colors.link, colors.fg);
    div()
        .id(id)
        .cursor_pointer()
        .text_color(link)
        .hover(move |s| s.text_color(fg))
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(label)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hours_read_as_typed() {
        assert_eq!(parse_hours(""), Some(None));
        assert_eq!(parse_hours(" 1,5 "), Some(Some(1.5)));
        assert_eq!(parse_hours("0.333"), Some(Some(0.33)));
        assert_eq!(parse_hours("-1"), None);
        assert_eq!(parse_hours("1.2.3"), None);
        assert_eq!(format_hours(2.0), "2");
        assert_eq!(format_hours(0.25), "0.25");
    }
}
