//! MonoCode `InboxPrChecks` and `CheckRepairForm`: a pull request's CI
//! checks (worst first), each opening to its job's steps and annotations;
//! failed ones can be sent to an agent, in a new chat or one of the
//! project's, with their evidence (`ci_repair`).

use std::collections::HashMap;

use ely_gpui_component::buttons::{Button, ButtonVariant, IconButton};
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, anchored, deferred, div, prelude::*,
};

use crate::ui::scale::px;

use super::ci_repair::{Evidence, build_request};
use crate::app::{BenCodeApp, TurnInput};
use crate::github::{self, Check, CheckState, WorkItem};

/// A CI repair sent to a thread (MonoCode `ciRepairTracking`), kept in
/// `settings.json` so its checks still read "Fix sent" after a restart.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repair {
    pub item_key: String,
    pub head_oid: String,
    pub checks: Vec<String>,
    pub session_id: String,
}

/// MonoCode keeps the latest repairs only.
const REPAIRS_KEPT: usize = 50;

/// The open "Fix with agent" picker.
#[derive(Clone, Debug, Default)]
pub struct RepairForm {
    pub item_key: String,
    /// The failed checks to fix, by name.
    pub checks: Vec<String>,
    /// Opened from "Fix all failed" (shown under the header).
    pub all: bool,
    pub busy: bool,
    pub error: Option<String>,
}

/// MonoCode `checkDuration`: "45s", "3m 2s", "1h 4m".
fn duration(started: Option<&str>, completed: Option<&str>) -> String {
    let parse = |t: Option<&str>| t?.parse::<jiff::Timestamp>().ok();
    let (Some(start), Some(end)) = (parse(started), parse(completed)) else {
        return String::new();
    };
    let secs = (end.as_second() - start.as_second()).max(0);
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, secs % 3600 / 60),
    }
}

/// MonoCode `describeCheckCounts`: "2 failed, 5 passed".
pub fn describe_counts(checks: &[Check]) -> String {
    let mut counts: HashMap<CheckState, usize> = HashMap::new();
    for check in checks {
        *counts.entry(check.state).or_default() += 1;
    }
    CheckState::ORDER
        .iter()
        .filter_map(|state| counts.get(state).map(|n| format!("{n} {}", state.label())))
        .collect::<Vec<_>>()
        .join(", ")
}

fn check_key(item_key: &str, name: &str) -> String {
    format!("{item_key}\0{name}")
}

impl BenCodeApp {
    pub(super) fn load_pr_checks(&mut self, key: &str, force: bool, cx: &mut Context<Self>) {
        let Some(item) = self.inbox.item(key).cloned() else {
            return;
        };
        if force {
            self.inbox.checks.value.remove(key);
        }
        super::load(
            self,
            |inbox| &mut inbox.checks,
            key,
            move || github::pr_checks(std::path::Path::new(&item.project), &item.repo, item.number),
            cx,
        );
    }

    fn toggle_check(&mut self, item: &WorkItem, check: &Check, cx: &mut Context<Self>) {
        let id = check_key(&item.key(), &check.name);
        if !self.inbox.expanded_checks.remove(&id) {
            self.inbox.expanded_checks.insert(id);
            if let Some(job) = check
                .url
                .as_deref()
                .and_then(|url| github::actions_job_id(url, &item.repo))
            {
                let (project, repo) = (item.project.clone(), item.repo.clone());
                let job_id = job.clone();
                super::load(
                    self,
                    |inbox| &mut inbox.jobs,
                    &job,
                    move || github::check_details(std::path::Path::new(&project), &repo, &job_id),
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn open_repair_form(
        &mut self,
        item: &WorkItem,
        checks: Vec<String>,
        all: bool,
        cx: &mut Context<Self>,
    ) {
        self.inbox.repair = Some(RepairForm {
            item_key: item.key(),
            checks,
            all,
            ..RepairForm::default()
        });
        cx.notify();
    }

    /// The project's chats a repair can go to: idle ones, newest first.
    fn repair_targets(&self, item: &WorkItem) -> Vec<(String, String)> {
        let project = self.inbox_start_project(item);
        let mut sessions: Vec<_> = self
            .sessions
            .iter()
            .filter(|s| {
                !s.archived
                    && crate::app::same_project_path(&s.cwd, &project)
                    && !self.is_agent_running_in(&s.id)
                    && s.blocks.iter().any(|b| b.role == "user")
            })
            .collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        sessions
            .into_iter()
            .take(12)
            .map(|s| (s.id.clone(), s.title.clone()))
            .collect()
    }

    /// MonoCode `CheckRepairForm.start`: reads each check's job, builds the
    /// request, and sends it to `target` (or a new chat).
    fn start_repair(&mut self, target: Option<String>, cx: &mut Context<Self>) {
        let Some(form) = self.inbox.repair.as_mut().filter(|f| !f.busy) else {
            return;
        };
        form.busy = true;
        form.error = None;
        let form = form.clone();
        let Some(item) = self.inbox.item(&form.item_key).cloned() else {
            return;
        };
        let Some(Ok(checks)) = self.inbox.checks.get(&form.item_key).cloned() else {
            return;
        };
        let selected: Vec<Check> = checks
            .checks
            .iter()
            .filter(|c| form.checks.contains(&c.name))
            .cloned()
            .collect();
        let head_oid = checks.head_oid.clone();
        let (project, repo, number) = (item.project.clone(), item.repo.clone(), item.number);
        let task = cx.background_executor().spawn(async move {
            let evidence: Vec<Evidence> = selected
                .into_iter()
                .map(|check| {
                    let details = check
                        .url
                        .as_deref()
                        .and_then(|url| github::actions_job_id(url, &repo))
                        .map(|job| {
                            github::check_details(std::path::Path::new(&project), &repo, &job)
                        });
                    Evidence { check, details }
                })
                .collect();
            build_request(&repo, number, &head_oid, &evidence)
        });
        cx.spawn(async move |this, cx| {
            let request = task.await;
            let updated = this.update(cx, |app, cx| {
                app.finish_repair(&item, form, target, request, &checks.head_oid, cx)
            });
            if let Err(err) = updated {
                log::debug!("ci repair after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    fn finish_repair(
        &mut self,
        item: &WorkItem,
        form: RepairForm,
        target: Option<String>,
        request: super::ci_repair::RepairRequest,
        head_oid: &str,
        cx: &mut Context<Self>,
    ) {
        let cwd = self.inbox_start_project(item);
        if let Some(id) = &target
            && self.is_agent_running_in(id)
        {
            if let Some(f) = &mut self.inbox.repair {
                f.busy = false;
                f.error = Some("This chat is busy. Choose another chat or start a new one.".into());
            }
            cx.notify();
            return;
        }
        if !crate::app::same_project_path(&cwd, &self.current_cwd) {
            self.switch_project(cwd.clone(), cx);
        }
        let session_id = match target {
            Some(id) => id,
            None => {
                let id = self.create_session_row(&cwd);
                if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
                    session.title = format!("Fix CI #{}: {}", item.number, item.title.trim());
                }
                self.persist_session(&id);
                id
            }
        };
        self.inbox.repair = None;
        self.inbox.repairs.push(Repair {
            item_key: form.item_key,
            head_oid: head_oid.to_string(),
            checks: form.checks,
            session_id: session_id.clone(),
        });
        let extra = self.inbox.repairs.len().saturating_sub(REPAIRS_KEPT);
        self.inbox.repairs.drain(..extra);
        self.save_settings(cx);
        self.close_surface(cx);
        self.open_session(session_id.clone(), cx);
        self.send_turn(
            &session_id,
            TurnInput {
                text: request.text,
                agent_prompt: Some(request.prompt),
                ..TurnInput::default()
            },
            cx,
        );
        cx.notify();
    }

    /// MonoCode `CheckRepairStatus`: the latest repair covering `name`.
    fn repair_status(&self, item_key: &str, head_oid: &str, name: &str) -> Option<(String, bool)> {
        // A chat deleted since leaves nothing to open.
        let repair = self.inbox.repairs.iter().rev().find(|r| {
            r.item_key == item_key
                && r.head_oid == head_oid
                && r.checks.iter().any(|c| c == name)
                && self.sessions.iter().any(|s| s.id == r.session_id)
        })?;
        Some((
            repair.session_id.clone(),
            self.is_agent_running_in(&repair.session_id),
        ))
    }

    /// The Checks section of a pull request.
    pub(super) fn render_pr_checks(&self, item: &WorkItem, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let key = item.key();
        let loading = self.inbox.checks.is_loading(&key);
        let refresh_key = key.clone();
        let header = |summary: String, failed: Vec<String>| {
            let item_for_fix = item.clone();
            let all_open = self
                .inbox
                .repair
                .as_ref()
                .is_some_and(|f| f.all && f.item_key == item.key());
            div()
                .relative()
                .flex()
                .items_center()
                .gap_2()
                .when(all_open, |el| el.child(self.render_repair_form(item, cx)))
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg)
                        .child("Checks"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.0))
                        .text_color(fg.opacity(0.5))
                        .child(summary),
                )
                .when(!failed.is_empty(), |el| {
                    el.child(
                        Button::new("inbox-fix-all", "Fix all failed")
                            .variant(ButtonVariant::Secondary)
                            .size(ControlSize::Sm)
                            .icon(IconName::Sparkles)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_repair_form(&item_for_fix, failed.clone(), true, cx)
                            })),
                    )
                })
                .child(if loading {
                    super::refreshing("inbox-checks-refreshing", cx)
                } else {
                    IconButton::new("inbox-checks-refresh", IconName::RefreshCw)
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Sm)
                        .tooltip("Refresh checks")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.load_pr_checks(&refresh_key, true, cx)
                        }))
                        .into_any_element()
                })
        };
        let section = div()
            .mt_4()
            .pt_4()
            .border_t_1()
            .border_color(colors.border)
            .flex()
            .flex_col()
            .gap_1();
        let checks = match self.inbox.checks.get(&key) {
            None => {
                return section
                    .child(header("Loading…".into(), Vec::new()))
                    .into_any_element();
            }
            Some(Err(err)) => {
                return section
                    .child(header(String::new(), Vec::new()))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(colors.danger.opacity(0.9))
                            .child(SharedString::from(err.clone())),
                    )
                    .into_any_element();
            }
            Some(Ok(checks)) => checks,
        };
        let mut sorted = checks.checks.clone();
        sorted.sort_by_key(|c| {
            CheckState::ORDER
                .iter()
                .position(|s| *s == c.state)
                .unwrap_or(usize::MAX)
        });
        let failed: Vec<String> = sorted
            .iter()
            .filter(|c| c.state == CheckState::Fail)
            .map(|c| c.name.clone())
            .collect();
        let section = section.child(header(describe_counts(&sorted), failed));
        if sorted.is_empty() {
            return section
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(fg.opacity(0.45))
                        .child("No checks reported"),
                )
                .into_any_element();
        }
        section
            .children(
                sorted.iter().enumerate().map(|(ix, check)| {
                    self.render_check_row(ix, item, check, &checks.head_oid, cx)
                }),
            )
            .into_any_element()
    }

    fn render_check_row(
        &self,
        ix: usize,
        item: &WorkItem,
        check: &Check,
        head_oid: &str,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let (icon, tint) = check_mark(check.state, cx);
        let item_key = item.key();
        let expanded = self
            .inbox
            .expanded_checks
            .contains(&check_key(&item_key, &check.name));
        let job = check
            .url
            .as_deref()
            .and_then(|url| github::actions_job_id(url, &item.repo));
        let repair = self.repair_status(&item_key, head_oid, &check.name);
        let hover = fg.opacity(0.02);
        let (toggle_item, toggle_check) = (item.clone(), check.clone());
        let (fix_item, fix_name) = (item.clone(), check.name.clone());
        let url = check.url.clone();
        let form_open =
            self.inbox.repair.as_ref().is_some_and(|f| {
                !f.all && f.item_key == item_key && f.checks == [check.name.clone()]
            });
        let status = match &repair {
            Some((_, true)) => div()
                .text_color(colors.accent)
                .child("Fixing…")
                .into_any_element(),
            Some((session, false)) => {
                let session = session.clone();
                div()
                    .id(("check-repair-open", ix))
                    .cursor_pointer()
                    .text_color(colors.accent)
                    .tooltip(Tooltip::text("Open the repair chat"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_surface(cx);
                        this.open_session(session.clone(), cx);
                    }))
                    .child("Fix sent")
                    .into_any_element()
            }
            None => div()
                .text_color(tint)
                .child(check.state.label())
                .into_any_element(),
        };
        let row = div()
            // Its own id: GPUI redraws on hover only for an element that
            // keeps state.
            .id(("check-row", ix))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_2()
            .rounded(px(12.0))
            .hover(move |s| s.bg(hover))
            .child(Icon::new(icon).size(IconSize::Sm).color(tint))
            .child(
                div()
                    .id(("check-name", ix))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap(px(10.0))
                    .when(job.is_some(), |el| {
                        el.cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_check(&toggle_item, &toggle_check, cx)
                            }))
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg)
                            .child(SharedString::from(check.name.clone())),
                    )
                    .when(!check.workflow.is_empty(), |el| {
                        el.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.0))
                                .text_color(fg.opacity(0.4))
                                .child(SharedString::from(check.workflow.clone())),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(64.0))
                    .flex_none()
                    .text_size(px(11.0))
                    .child(status),
            )
            .child(
                div()
                    .w(px(44.0))
                    .flex_none()
                    .text_size(px(10.0))
                    .text_color(fg.opacity(0.4))
                    .child(duration(
                        check.started_at.as_deref(),
                        check.completed_at.as_deref(),
                    )),
            )
            .child(if check.state == CheckState::Fail {
                IconButton::new(("check-fix", ix), IconName::Sparkles)
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .tooltip("Fix with AI")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_repair_form(&fix_item, vec![fix_name.clone()], false, cx)
                    }))
                    .into_any_element()
            } else {
                div().size(px(28.0)).flex_none().into_any_element()
            })
            .child(match url {
                Some(url) => IconButton::new(("check-link", ix), IconName::ExternalLink)
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .tooltip("View full log on GitHub")
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .into_any_element(),
                None => div().size(px(28.0)).flex_none().into_any_element(),
            });
        div()
            .relative()
            .child(row)
            .when(form_open, |el| el.child(self.render_repair_form(item, cx)))
            .when(expanded, |el| {
                el.child(self.render_check_details(job.as_deref(), cx))
            })
            .into_any_element()
    }

    /// The job's annotations and steps (MonoCode `CheckEvidence`, steps).
    fn render_check_details(&self, job: Option<&str>, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let wrap = div()
            .pl(px(40.0))
            .pr_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(12.0));
        let Some(job) = job else {
            return wrap.into_any_element();
        };
        let details = match self.inbox.jobs.get(job) {
            None => {
                return wrap
                    .child(div().text_color(fg.opacity(0.5)).child("Loading steps…"))
                    .into_any_element();
            }
            Some(Err(err)) => {
                return wrap
                    .child(
                        div()
                            .text_color(fg.opacity(0.6))
                            .child("Could not load job details."),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(fg.opacity(0.5))
                            .child(SharedString::from(err.clone())),
                    )
                    .into_any_element();
            }
            Some(Ok(details)) => details,
        };
        let annotations = details.annotations.iter().map(|a| {
            let tint = if a.level == "failure" {
                colors.danger
            } else {
                colors.warning
            };
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .bg(tint.opacity(0.06))
                .child(
                    div()
                        .font_family(cx.theme().mono_family.clone())
                        .text_size(px(11.0))
                        .text_color(fg.opacity(0.55))
                        .child(format!("{}:{}", a.path, a.line)),
                )
                .child(
                    div()
                        .text_color(fg.opacity(0.85))
                        .child(SharedString::from(a.message.clone())),
                )
        });
        let steps = details.steps.iter().map(|step| {
            let (icon, tint) = check_mark(step.state, cx);
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .when(step.state == CheckState::Fail, |el| {
                    el.bg(colors.danger.opacity(0.05))
                })
                .child(Icon::new(icon).size(IconSize::Sm).color(tint))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(fg.opacity(0.8))
                        .child(SharedString::from(step.name.clone())),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(fg.opacity(0.45))
                        .child(duration(
                            step.started_at.as_deref(),
                            step.completed_at.as_deref(),
                        )),
                )
        });
        wrap.children(annotations)
            .children(details.notice.clone().map(|notice| {
                div()
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.5))
                    .child(notice)
            }))
            .when(!details.steps.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(fg.opacity(0.5))
                        .child("Run steps"),
                )
                .children(steps)
            })
            .into_any_element()
    }

    /// MonoCode `CheckRepairForm`: send the failed checks to a new chat in
    /// the project or one of its idle chats.
    fn render_repair_form(&self, item: &WorkItem, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let Some(form) = &self.inbox.repair else {
            return div().into_any_element();
        };
        let busy = form.busy;
        let choices = std::iter::once((None, "New project chat".to_string())).chain(
            self.repair_targets(item)
                .into_iter()
                .map(|(id, title)| (Some(id), title)),
        );
        let rows = choices.enumerate().map(|(ix, (target, title))| {
            let hover = fg.opacity(0.08);
            div()
                .id(("repair-target", ix))
                .flex()
                .items_center()
                .gap_2()
                .h(px(30.0))
                .px_2()
                .rounded(px(6.0))
                .text_size(px(12.0))
                .text_color(fg.opacity(0.85))
                .when(!busy, |el| {
                    el.cursor_pointer().hover(move |s| s.bg(hover)).on_click(
                        cx.listener(move |this, _, _, cx| this.start_repair(target.clone(), cx)),
                    )
                })
                .child(
                    Icon::new(if ix == 0 {
                        IconName::Plus
                    } else {
                        IconName::MessageSquare
                    })
                    .size(IconSize::Xs)
                    .color(fg.opacity(0.5)),
                )
                .child(div().min_w_0().truncate().child(if title.is_empty() {
                    "Untitled chat".to_string()
                } else {
                    title
                }))
        });
        let count = form.checks.len();
        let panel = div()
            .id("inbox-repair-form")
            .occlude()
            .w(px(300.0))
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .rounded(px(10.0))
            .border_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .shadow_xl()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.inbox.repair.as_ref().is_some_and(|f| !f.busy) {
                    this.inbox.repair = None;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .px_1()
                    .pb_1()
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.5))
                    .child(if busy {
                        "Reading job logs…".to_string()
                    } else {
                        format!(
                            "Fix {count} failed {} with an agent in",
                            if count == 1 { "check" } else { "checks" }
                        )
                    }),
            )
            .children(rows)
            .children(form.error.clone().map(|error| {
                div()
                    .px_1()
                    .pt_1()
                    .text_size(px(11.0))
                    .text_color(colors.danger.opacity(0.9))
                    .child(error)
            }));
        div()
            .absolute()
            .top_full()
            .right_0()
            .child(
                deferred(
                    anchored()
                        .anchor(gpui::Anchor::TopRight)
                        .snap_to_window()
                        .child(panel),
                )
                .with_priority(3),
            )
            .into_any_element()
    }
}

/// MonoCode `checkMark`: the state's glyph and tint.
fn check_mark(state: CheckState, cx: &gpui::App) -> (IconName, gpui::Hsla) {
    let colors = &cx.theme().colors;
    match state {
        CheckState::Pass => (IconName::CircleCheck, colors.success),
        CheckState::Fail => (IconName::CircleX, colors.danger),
        CheckState::Pending => (IconName::LoaderCircle, colors.warning),
        CheckState::Skipping => (IconName::Ban, colors.fg.opacity(0.4)),
        CheckState::Cancel => (IconName::Ban, colors.fg.opacity(0.5)),
        CheckState::Unknown => (IconName::Circle, colors.fg.opacity(0.4)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(state: CheckState) -> Check {
        Check {
            name: "x".into(),
            workflow: String::new(),
            state,
            url: None,
            started_at: None,
            completed_at: None,
        }
    }

    #[test]
    fn counts_read_worst_first() {
        let checks = [
            check(CheckState::Pass),
            check(CheckState::Fail),
            check(CheckState::Pass),
        ];
        assert_eq!(describe_counts(&checks), "1 failed, 2 passed");
    }

    #[test]
    fn durations_are_compact() {
        assert_eq!(
            duration(Some("2026-01-01T00:00:00Z"), Some("2026-01-01T00:03:02Z")),
            "3m 2s"
        );
        assert_eq!(duration(Some("2026-01-01T00:00:00Z"), None), "");
    }
}
