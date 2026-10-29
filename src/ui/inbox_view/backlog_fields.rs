//! A Backlog issue's fields among its properties (`properties.rs`), each
//! editable where it is shown (Backlog's issue page: assignee, priority,
//! milestones, resolution, start and due dates, estimated and actual
//! hours) in a quiet control. Each change is one `PATCH`
//! (`app/backlog.rs`); the fields wait while one is on its way. The hours
//! inputs serve the Inbox's selected issue: a tab beside the chat shows
//! its issue's hours as text.

use bencode_ui::buttons::{ButtonVariant, IconButton};
use bencode_ui::forms::{Choice, DatePicker, Input, MultiSelect, Select, TextInput};
use bencode_ui::primitives::IconName;
use bencode_ui::theme::{ActiveTheme, ControlSize};
use gpui::{
    AnyElement, Context, Entity, IntoElement, ParentElement, SharedString, Styled, div, prelude::*,
};
use jiff::civil::Date;

use super::DetailView;
use super::properties::{due_note, plain, property, property_with};
use crate::app::BenCodeApp;
use crate::backlog::{Change, FieldChoice, IssueFields};
use crate::github::WorkItem;
use crate::ui::app_callback::{app_callback_with, on_value};
use crate::ui::scale::px;
use crate::ui::tokens::{Ink, Text, Tone as _};

/// The controls' size: their text is the properties' body size.
const SIZE: ControlSize = ControlSize::Md;
/// An hours input's width: "12.5" and its inset.
const HOURS_WIDTH: f32 = 56.0;
/// The `Select` value that clears a field.
const NONE: &str = "none";
/// An empty field's placeholder.
const EMPTY: &str = "—";

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
    /// The fields' rows, once the issue is loaded; its project's choices
    /// may still be on their way, and the fields wait for them.
    pub(super) fn render_backlog_fields(
        &self,
        item: &WorkItem,
        view: DetailView,
        cx: &Context<Self>,
    ) -> Option<Vec<AnyElement>> {
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
            estimated_hours,
            actual_hours,
        } = fields;
        let me = options.me;
        let mine = assignee.as_ref().is_some_and(|a| a.id == me);
        let assign_key = item.key();
        // Backlog's "Assign to myself", as an icon after the field's name.
        let assign_me = (!mine && !busy && me != 0).then(|| {
            IconButton::new("backlog-assign-me", IconName::User)
                .variant(ButtonVariant::Ghost)
                .size(ControlSize::Sm)
                .tooltip("Assign to me")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.update_backlog_issue(&assign_key, Change::Assignee(Some(me)), cx)
                }))
        });
        let assignee_field = property_with(
            "Assignee",
            assign_me.map(IntoElement::into_any_element),
            Select::new(
                "backlog-assignee",
                std::iter::once(Choice::new(NONE, "Unassigned")).chain(options.users.iter().map(
                    |user| {
                        Choice::new(user.id.to_string(), user.name.clone())
                            .avatar(user.name.clone())
                    },
                )),
            )
            .quiet()
            .size(SIZE)
            .disabled(busy)
            .placeholder(match &assignee {
                // Shown by name while the choices load.
                Some(a) if busy => a.name.clone(),
                _ => EMPTY.into(),
            })
            .when_some(assignee, |el, a| el.selected(a.id.to_string()))
            .on_change(change(&key, cx, |value| {
                Some(Change::Assignee(id_of(value)))
            })),
            cx,
        );

        let priority_field = property(
            "Priority",
            Select::new("backlog-priority", choices(&options.priorities))
                .quiet()
                .size(SIZE)
                .disabled(busy)
                .placeholder(priority.as_ref().map_or(EMPTY.into(), |p| p.name.clone()))
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
        // bencode-ui's `MultiSelect` refuses an empty list: before the options
        // load, or in a project without milestones, the issue's own show
        // in a closed `Select`.
        let shown = match milestones.as_slice() {
            [] => EMPTY.to_string(),
            some => (some.iter().map(|m| m.name.as_str()))
                .collect::<Vec<_>>()
                .join(", "),
        };
        let milestone_picker = if options.milestones.is_empty() {
            Select::new("backlog-milestones", Vec::<Choice>::new())
                .quiet()
                .size(SIZE)
                .disabled(true)
                .placeholder(shown)
                .into_any_element()
        } else {
            MultiSelect::new("backlog-milestones", choices(&options.milestones))
                .quiet()
                .size(SIZE)
                .placeholder(EMPTY)
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
        let milestone_field = property("Milestone", milestone_picker, cx);

        let resolution_field = property(
            "Resolution",
            Select::new(
                "backlog-resolution",
                std::iter::once(Choice::new(NONE, "No resolution"))
                    .chain(choices(&options.resolutions)),
            )
            .quiet()
            .size(SIZE)
            .disabled(busy)
            .placeholder(match &resolution {
                Some(r) if busy => r.name.clone(),
                _ => EMPTY.into(),
            })
            .when_some(resolution, |el, r| el.selected(r.id.to_string()))
            .on_change(change(&key, cx, |value| {
                Some(Change::Resolution(id_of(value)))
            })),
            cx,
        );

        let start_field = self.date_field(item, "Start", start_date, busy, Change::StartDate, cx);
        let due_field = self.date_field(item, "Due", due_date, busy, Change::DueDate, cx);

        let hours_field = match view {
            DetailView::Tab => {
                let shown = |hours: Option<f64>| hours.map_or_else(|| EMPTY.into(), format_hours);
                let text = format!("{} / {} h", shown(estimated_hours), shown(actual_hours));
                property("Est. / Act.", plain(cx).child(text), cx)
            }
            DetailView::Inbox => self.hours_inputs(cx),
        };

        Some(vec![
            assignee_field,
            priority_field,
            milestone_field,
            resolution_field,
            start_field,
            due_field,
            hours_field,
        ])
    }

    /// The selected issue's hours, each in an input.
    fn hours_inputs(&self, cx: &Context<Self>) -> AnyElement {
        property(
            "Est. / Act.",
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(self.hours(&self.backlog_estimate_input))
                .child(
                    div()
                        .flex_none()
                        .text_size(Text::Sm.px())
                        .text_color(cx.theme().colors.fg.ink(Ink::Faint))
                        .child("/"),
                )
                .child(self.hours(&self.backlog_actual_input))
                .child(
                    div()
                        .flex_none()
                        .text_size(Text::Sm.px())
                        .text_color(cx.theme().colors.fg.ink(Ink::Muted))
                        .child("h"),
                ),
            cx,
        )
    }

    /// A day picker, with how far the day is for a due date, and a clear
    /// button that shows while the row is hovered.
    fn date_field(
        &self,
        item: &WorkItem,
        label: &'static str,
        value: Option<String>,
        busy: bool,
        make: fn(Option<String>) -> Change,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = SharedString::from(format!("backlog-{}", label.to_lowercase()));
        let set = day(&value);
        let (pick_key, clear_key) = (item.key(), item.key());
        let today = jiff::Zoned::now().date();
        let note = (matches!(make(None), Change::DueDate(_)) && !item.closed)
            .then(|| value.as_deref().and_then(|due| due_note(due, today, cx)))
            .flatten();
        let group = SharedString::from(format!("{id}-row"));
        let clear = set.filter(|_| !busy).map(|_| {
            div()
                .flex_none()
                .invisible()
                .group_hover(group.clone(), |s| s.visible())
                .child(
                    IconButton::new(SharedString::from(format!("{id}-clear")), IconName::X)
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .tooltip(format!("Clear the {} date", label.to_lowercase()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update_backlog_issue(&clear_key, make(None), cx)
                        })),
                )
        });
        property(
            label,
            div()
                .id(group.clone())
                .group(group)
                .flex()
                .items_center()
                .gap_1()
                .child(
                    // Its own width, so how far the day is follows it.
                    div().flex_none().max_w_full().child(
                        DatePicker::new(id.clone(), set)
                            .quiet()
                            .size(SIZE)
                            .placeholder(EMPTY)
                            .today(today)
                            .disabled(busy)
                            .on_change(app_callback_with(cx, move |this, date: Date, cx| {
                                let day = date.strftime("%Y-%m-%d").to_string();
                                this.update_backlog_issue(&pick_key, make(Some(day)), cx)
                            })),
                    ),
                )
                .children(note)
                .child(div().flex_1())
                .children(clear),
            cx,
        )
    }

    /// Hours as typed: plain text, sent on Enter or blur and only when it
    /// changed (`commit_backlog_hours`), never per keystroke or step.
    fn hours(&self, input: &Entity<TextInput>) -> impl IntoElement {
        div()
            .w(px(HOURS_WIDTH))
            .flex_none()
            .child(Input::new(input).quiet().size(SIZE))
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
