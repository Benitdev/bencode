//! MonoCode `AutomationEditor`'s Settings page: triggers, instructions
//! with model and access, session, advanced. Event triggers are shown but
//! cannot be added: BenCode runs time triggers only.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::forms::{Choice, Select};
use ely_gpui_component::menus::{DropdownMenu, Menu, MenuItem};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, Div, FontWeight, IntoElement, ParentElement, Styled, div, prelude::*,
};
use jiff::tz::TimeZone;
use serde_json::Value;

use super::format::{gmt_offset_at, next_run_preview};
use crate::app::automations::{MAX_TRIGGERS, WorkingCopy, access_mode, grace_minutes};
use crate::app::{BenCodeApp, now_ms};
use crate::db::{AutomationRow, DEFAULT_GRACE_MINUTES};
use crate::schedule::{self, ScheduleKind, TimeTrigger, Trigger, WEEKDAYS};
use crate::ui::app_callback::{app_callback, on_value};
use crate::ui::region::Region;
use crate::ui::scale::px;
use crate::ui::settings_parts::{SettingsRow, panel, rule, section_title};
use crate::ui::sidebar_popovers::pretty_path;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// The width a row's control gets in an automation's settings panel.
const SELECT_WIDTH: f32 = 168.0;

/// MonoCode `GRACE_OPTIONS`.
const GRACE_OPTIONS: [(i64, &str); 5] = [
    (0, "Do not catch up"),
    (30, "30 minutes"),
    (120, "2 hours"),
    (720, "12 hours"),
    (1440, "24 hours"),
];
/// The Session folder select's value for no folder (a `Select` value
/// cannot be empty).
const NO_FOLDER: &str = "none";

/// MonoCode `TimeTriggerSentence`'s opening words. A kind this version
/// does not know reads as a weekly one, as its label does.
fn schedule_prefix(kind: Option<ScheduleKind>) -> &'static str {
    match kind {
        Some(ScheduleKind::Hourly) => "Every hour at",
        Some(ScheduleKind::Daily) => "Every day at",
        Some(ScheduleKind::Weekdays) => "Every weekday at",
        Some(ScheduleKind::Weekly) | None => "Every week on",
    }
}

/// "GitHub · pull request opened" for a trigger BenCode cannot run.
fn event_trigger_label(trigger: &Trigger) -> String {
    let text = |key: &str| trigger.get(key).and_then(Value::as_str).unwrap_or("");
    let source = match text("kind") {
        "github" => "GitHub",
        "linear" => "Linear",
        "jira" => "Jira",
        "gitlab" => "GitLab",
        "azuredevops" => "Azure DevOps",
        other => other,
    };
    match text("event") {
        "" => source.to_string(),
        event => format!("{source} · {}", event.replace('_', " ")),
    }
}

impl BenCodeApp {
    pub(super) fn render_automation_settings(
        &self,
        draft: &AutomationRow,
        cx: &Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_8()
            .child(self.render_automation_triggers(draft, cx))
            .child(self.render_automation_instructions(draft, cx))
            .child(self.render_automation_session(draft, cx))
            .child(self.render_automation_advanced(draft, cx))
            .into_any_element()
    }

    fn render_automation_triggers(
        &self,
        draft: &AutomationRow,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let (fg, muted) = (colors.fg, colors.fg_muted);
        let triggers = schedule::triggers_of(draft);
        let full = triggers.len() >= MAX_TRIGGERS;
        let none = triggers.is_empty();
        let rows: Vec<AnyElement> = triggers
            .iter()
            .enumerate()
            .map(|(ix, trigger)| self.render_automation_trigger(ix, trigger, cx))
            .collect();
        let add = ScheduleKind::ALL
            .into_iter()
            .fold(Menu::new(), |menu, kind| {
                let pick = app_callback(cx, move |this, cx| this.add_automation_trigger(kind, cx));
                menu.item(
                    MenuItem::new(kind.label())
                        .icon(IconName::Clock)
                        .disabled(full)
                        .on_click(pick),
                )
            });
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(section_title("Triggers", muted))
            .child(
                panel(fg)
                    .when(!none, |el| {
                        el.child(div().flex().flex_col().px_3().py(px(6.0)).children(rows))
                            .child(rule(fg))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .h(px(48.0))
                            .px_1()
                            .when(none, |el| {
                                el.pl_4()
                                    .child(Icon::new(IconName::Clock).size(IconSize::Sm).color(muted))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(Text::Base.px())
                                            .text_color(muted)
                                            .child("No schedule yet: it runs only when started by hand."),
                                    )
                            })
                            .child(
                                DropdownMenu::new("automation-add-trigger", "Add Trigger", add)
                                    .icon(IconName::Plus)
                                    .variant(ButtonVariant::Ghost),
                            ),
                    ),
            )
    }

    /// MonoCode `TriggerRow`.
    fn render_automation_trigger(
        &self,
        ix: usize,
        trigger: &Trigger,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let (fg, muted) = (colors.fg, colors.fg_muted);
        let sentence = div()
            .flex()
            .flex_wrap()
            .flex_1()
            .items_center()
            .gap_x(px(6.0))
            .gap_y(px(4.0))
            .min_w_0()
            .text_size(Text::Md.px())
            .text_color(fg.ink(Ink::Body));
        let (icon, sentence) = match schedule::time_trigger(trigger) {
            Some(time) => (
                IconName::Clock,
                self.render_time_sentence(sentence, ix, &time, cx),
            ),
            None => (
                IconName::Zap,
                sentence.child(event_trigger_label(trigger)).child(
                    div()
                        .text_color(fg.ink(Ink::Faint))
                        .child("Event triggers do not run in BenCode"),
                ),
            ),
        };
        div()
            .group("automation-trigger")
            .flex()
            .items_center()
            .gap(px(10.0))
            .min_h(px(40.0))
            .py_1()
            .child(Icon::new(icon).size(IconSize::Sm).color(muted))
            .child(sentence)
            .child(
                div()
                    .id(("automation-trigger-remove", ix))
                    .flex_none()
                    .invisible()
                    .group_hover("automation-trigger", |style| style.visible())
                    .child(
                        IconButton::new(("automation-trigger-remove-button", ix), IconName::X)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("Remove trigger")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_automation_trigger(ix, cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    /// MonoCode `TimeTriggerSentence`: "Every week on [Monday] at [09:00]
    /// GMT+7  Next run …".
    fn render_time_sentence(
        &self,
        sentence: Div,
        ix: usize,
        time: &TimeTrigger,
        cx: &Context<Self>,
    ) -> Div {
        let fg = cx.theme().colors.fg;
        let tz = TimeZone::system();
        let now = now_ms();
        let next = time.next_run(now, &tz);
        let pill = |width: f32, select: Select| div().flex_none().w(px(width)).child(select);
        let weekly = matches!(time.kind, Some(ScheduleKind::Weekly) | None);
        let hourly = time.kind == Some(ScheduleKind::Hourly);
        sentence
            .child(schedule_prefix(time.kind))
            .when(weekly, |el| {
                let days = WEEKDAYS
                    .into_iter()
                    .enumerate()
                    .map(|(day, label)| Choice::new(day.to_string(), label));
                el.child(pill(
                    128.0,
                    Select::new(("automation-trigger-day", ix), days)
                        .label("Day")
                        .size(ControlSize::Sm)
                        .selected(time.day_of_week.clamp(0, 6).to_string())
                        .on_change(on_value(cx, move |this, day, cx| {
                            let day = day.parse::<i64>().unwrap_or(1);
                            this.set_automation_trigger(ix, "dayOfWeek", Value::from(day), cx)
                        })),
                ))
                .child("at")
            })
            .map(|el| {
                if hourly {
                    let minutes = [0, 15, 30, 45]
                        .into_iter()
                        .chain(Some(time.minute).filter(|minute| minute % 15 != 0))
                        .map(|minute| Choice::new(minute.to_string(), format!(":{minute:02}")));
                    el.child(pill(
                        84.0,
                        Select::new(("automation-trigger-minute", ix), minutes)
                            .label("Minute")
                            .size(ControlSize::Sm)
                            .selected(time.minute.to_string())
                            .on_change(on_value(cx, move |this, minute, cx| {
                                let minute = minute.parse::<i64>().unwrap_or(0);
                                this.set_automation_trigger(ix, "minute", Value::from(minute), cx)
                            })),
                    ))
                } else {
                    let times = schedule::time_options(&time.time)
                        .into_iter()
                        .map(|option| Choice::new(option.clone(), option));
                    el.child(pill(
                        92.0,
                        Select::new(("automation-trigger-time", ix), times)
                            .label("Time")
                            .size(ControlSize::Sm)
                            .selected(time.time.clone())
                            .on_change(on_value(cx, move |this, time, cx| {
                                this.set_automation_trigger(ix, "time", Value::from(time), cx)
                            })),
                    ))
                }
            })
            .child(
                div()
                    .text_color(fg.ink(Ink::Muted))
                    .child(gmt_offset_at(next.unwrap_or(now), &tz)),
            )
            .when_some(next, |el, at| {
                el.child(
                    div()
                        .ml_1()
                        .text_color(fg.ink(Ink::Faint))
                        .child(next_run_preview(at, &tz)),
                )
            })
    }

    fn render_automation_instructions(
        &self,
        draft: &AutomationRow,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let (fg, muted) = (colors.fg, colors.fg_muted);
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(section_title("Instructions", muted))
            .child(
                panel(fg)
                    .bg(fg.fill(Fill::Faint))
                    .child(
                        div()
                            .px_3()
                            .pt_3()
                            .pb_2()
                            .text_size(Text::Md.px())
                            .text_color(fg)
                            .child(self.automations.prompt_input.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .pb_2()
                            .child(self.render_automation_model(&draft.harness, &draft.model, cx))
                            .child(self.render_automation_access(access_mode(draft), cx)),
                    ),
            )
            .child(
                div()
                    .px_1()
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Faint))
                    .child("Skills work here: start a line with /skill-name."),
            )
    }

    fn render_automation_session(
        &self,
        draft: &AutomationRow,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let (fg, muted) = (colors.fg, colors.fg_muted);
        let copy = WorkingCopy::of(draft);
        let reuse = draft.reuse_session == Some(true);
        let folder = draft.session_folder_id.clone().unwrap_or_default();
        let folders = self
            .session_folders
            .get(&draft.cwd)
            .map_or(&[][..], Vec::as_slice);
        let mut folder_choices = vec![Choice::new(NO_FOLDER, "None")];
        folder_choices.extend(
            folders
                .iter()
                .map(|f| Choice::new(f.id.clone(), f.name.clone())),
        );
        if !folder.is_empty() && !folders.iter().any(|f| f.id == folder) {
            folder_choices.push(Choice::new(folder.clone(), "Removed folder"));
        }
        // MonoCode `SearchableProjectPicker`: the project runs happen in.
        let mut paths = self.worktree_project_choices();
        if !draft.cwd.is_empty() && !paths.contains(&draft.cwd) {
            paths.insert(0, draft.cwd.clone());
        }
        let projects = paths.into_iter().map(|path| {
            Choice::new(path.clone(), self.rail_project_label(&path))
                .icon(IconName::Folder)
                .note(pretty_path(&path))
        });
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(section_title("Session", muted))
            .child(
                panel(fg)
                    .child(
                        SettingsRow::new("Project")
                            .description("Where runs happen")
                            .control_fill(SELECT_WIDTH)
                            .control(
                                Select::new("automation-project", projects)
                                    .label("Project")
                                    .size(ControlSize::Sm)
                                    .placeholder("Choose a project…")
                                    .selected(draft.cwd.clone())
                                    .on_change(on_value(cx, |this, path, cx| {
                                        let path = path.to_string();
                                        // Folders belong to a project.
                                        this.edit_automation(cx, |draft| {
                                            draft.cwd = path;
                                            draft.session_folder_id = Some(String::new());
                                        });
                                    })),
                            ),
                    )
                    .child(rule(fg))
                    .child(
                        SettingsRow::new("Working copy")
                            .description("This repo, or a fresh worktree")
                            .control_fill(SELECT_WIDTH)
                            .control(
                                Select::new(
                                    "automation-working-copy",
                                    WorkingCopy::ALL
                                        .map(|copy| Choice::new(copy.id(), copy.label())),
                                )
                                .label("Working copy")
                                .size(ControlSize::Sm)
                                .selected(copy.id())
                                .on_change(on_value(
                                    cx,
                                    |this, mode, cx| {
                                        let mode = mode.to_string();
                                        this.edit_automation(cx, |draft| {
                                            // A fresh worktree has no earlier chat to continue.
                                            if mode == WorkingCopy::Worktree.id() {
                                                draft.reuse_session = Some(false);
                                            }
                                            draft.workspace_mode = Some(mode);
                                        });
                                    },
                                )),
                            ),
                    )
                    .child(rule(fg))
                    .child(
                        SettingsRow::new("Conversation")
                            .description("New chat, or continue the last run")
                            .control_fill(SELECT_WIDTH)
                            .control(
                                Select::new(
                                    "automation-conversation",
                                    [
                                        Choice::new("fresh", "Start fresh"),
                                        Choice::new("reuse", "Continue last"),
                                    ],
                                )
                                .label("Conversation")
                                .size(ControlSize::Sm)
                                .disabled(copy == WorkingCopy::Worktree)
                                .selected(if reuse { "reuse" } else { "fresh" })
                                .on_change(on_value(
                                    cx,
                                    |this, value, cx| {
                                        let reuse = value == "reuse";
                                        this.edit_automation(cx, |draft| {
                                            draft.reuse_session = Some(reuse)
                                        });
                                    },
                                )),
                            ),
                    )
                    .child(rule(fg))
                    .child(
                        SettingsRow::new("Session folder")
                            .description("Where runs appear in the sidebar")
                            .control_fill(SELECT_WIDTH)
                            .control(
                                Select::new("automation-folder", folder_choices)
                                    .label("Session folder")
                                    .size(ControlSize::Sm)
                                    .selected(if folder.is_empty() {
                                        NO_FOLDER.to_string()
                                    } else {
                                        folder
                                    })
                                    .on_change(on_value(cx, |this, value, cx| {
                                        let id = Some(value)
                                            .filter(|id| *id != NO_FOLDER)
                                            .unwrap_or("")
                                            .to_string();
                                        this.edit_automation(cx, |draft| {
                                            draft.session_folder_id = Some(id)
                                        });
                                    })),
                            ),
                    ),
            )
    }

    /// MonoCode's `<details>`: the catch-up window for missed runs.
    fn render_automation_advanced(
        &self,
        draft: &AutomationRow,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let open = self.automations.advanced_open;
        let grace = grace_minutes(draft);
        let options = GRACE_OPTIONS
            .into_iter()
            .map(|(minutes, label)| Choice::new(minutes.to_string(), label))
            .chain(
                (!GRACE_OPTIONS.iter().any(|(minutes, _)| *minutes == grace))
                    .then(|| Choice::new(grace.to_string(), format!("{grace} minutes"))),
            );
        panel(fg)
            .child(
                div()
                    .id("automation-advanced")
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .min_h(px(56.0))
                    .px_4()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.automations.advanced_open = !this.automations.advanced_open;
                        this.redraw_regions(&[Region::Surface], cx);
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(Text::Md.px())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(fg.ink(Ink::Strong))
                                    .child("Advanced"),
                            )
                            .child(
                                div()
                                    .text_size(Text::Sm.px())
                                    .text_color(fg.ink(Ink::Muted))
                                    .child("Catch-up window for missed runs"),
                            ),
                    )
                    .child(
                        Icon::new(if open {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        })
                        .size(IconSize::Sm)
                        .color(fg.ink(Ink::Muted)),
                    ),
            )
            .when(open, |el| {
                el.child(rule(fg)).child(
                    SettingsRow::new("Missed-run grace")
                        .description("Catch up if a scheduled run was missed")
                        .control_fill(SELECT_WIDTH)
                        .control(
                            Select::new("automation-grace", options)
                                .label("Missed-run grace")
                                .size(ControlSize::Sm)
                                .selected(grace.to_string())
                                .on_change(on_value(cx, |this, value, cx| {
                                    let minutes =
                                        value.parse::<i64>().unwrap_or(DEFAULT_GRACE_MINUTES);
                                    this.edit_automation(cx, |draft| {
                                        draft.missed_run_grace_minutes = Some(minutes)
                                    });
                                })),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_offer_every_half_hour_and_an_odd_current_one() {
        let options = schedule::time_options("09:00");
        assert_eq!(options.len(), 48);
        assert_eq!(
            (options[0].as_str(), options[47].as_str()),
            ("00:00", "23:30")
        );
        assert_eq!(schedule::time_options("09:15")[0], "09:15");
        assert_eq!(schedule::time_options("09:15").len(), 49);
    }

    #[test]
    fn event_triggers_read_as_source_and_event() {
        let trigger = serde_json::json!({"kind": "github", "event": "pull_request_opened"});
        assert_eq!(
            event_trigger_label(trigger.as_object().unwrap()),
            "GitHub · pull request opened"
        );
        let bare = serde_json::json!({"kind": "linear"});
        assert_eq!(event_trigger_label(bare.as_object().unwrap()), "Linear");
    }
}
