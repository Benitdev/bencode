//! The extras on a MonoCode session card: the linked GitHub item's badge
//! and "updated" dot, and an orchestrator's subagents (the Share icon's
//! hover card, and the inline list while the card is open or busy).

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, App, ClickEvent, Context, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, SharedString, Styled, Window, div, prelude::*, rgb,
};

use crate::app::BenCodeApp;
use crate::app::session_list::LiveStates;
use crate::db::{OrchestrationSummary, OrchestrationTask, SessionRow, TaskTone};
use crate::harness::{HarnessKind, catalog};
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;

const AMBER: u32 = 0xfbbf24;
const EMERALD: u32 = 0x34d399;
/// Rows the subagents card lists before "+N more".
const TIP_ROWS: usize = 12;
/// MonoCode's `text-fuchsia-300/65`, `hover:text-fuchsia-200/90`.
const FUCHSIA: u32 = 0xf0abfc;
const FUCHSIA_HOVER: u32 = 0xf5d0fe;

fn tone_color(tone: TaskTone, fg: Hsla) -> Hsla {
    match tone {
        TaskTone::Attention => rgb(AMBER).into(),
        TaskTone::Done => rgb(EMERALD).into(),
        TaskTone::Quiet => fg.opacity(0.45),
    }
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {one}s")
    }
}

/// MonoCode's subagents hover card (`Popover side="right"`, 248px).
struct OrchestrationTip {
    summary: OrchestrationSummary,
    needs_input: Vec<bool>,
}

impl Render for OrchestrationTip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let total = self.summary.tasks.len();
        // A tooltip cannot scroll; the rest is counted.
        let hidden = total.saturating_sub(TIP_ROWS);
        let rows = self.summary.tasks.iter().zip(&self.needs_input).take(TIP_ROWS).map(|(task, needs)| {
            let label = task.label(&self.summary.status, *needs);
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap_1p5()
                .px_1()
                .py_1()
                .child(div().opacity(0.75).child(HarnessIcon::new(&task.harness).size(px(14.0))))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.0))
                        .text_color(fg.opacity(0.75))
                        .child(task.title.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(10.0))
                        .text_color(tone_color(task.tone(*needs), fg))
                        .child(label),
                )
        });
        crate::ui::sidebar_popovers::popover_frame(cx)
            .w(px(248.0))
            .max_h(px(320.0))
            .overflow_hidden()
            .p(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(fg.opacity(0.85))
                            .child("Subagents"),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(10.0))
                            .text_color(fg.opacity(0.45))
                            .child(format!("{}/{total} done", self.summary.done())),
                    ),
            )
            .child(div().mt_1p5().flex().flex_col().gap(px(2.0)).children(rows))
            .when(hidden > 0, |el| {
                el.child(
                    div()
                        .px_1()
                        .pt_1()
                        .text_size(px(10.0))
                        .text_color(fg.opacity(0.45))
                        .child(format!("+{hidden} more")),
                )
            })
    }
}

impl BenCodeApp {
    /// A worker thread's pending approval or question (MonoCode
    /// `sessionNeedsInput`).
    fn task_needs_input(&self, task: &OrchestrationTask, states: &LiveStates) -> bool {
        task.session_id.as_deref().is_some_and(|id| states.approval.contains(id))
    }

    /// MonoCode's "Linked PR updated since this session" dot: the Inbox
    /// saw the item change after the thread last moved.
    pub(crate) fn linked_update_dot(&self, session: &SessionRow, cx: &Context<Self>) -> Option<AnyElement> {
        let item = session.linked_work_item.as_ref()?;
        let key = format!(
            "{}:{}:{}",
            item.repo.to_lowercase(),
            if item.noun() == "PR" { "pr" } else { "issue" },
            item.number
        );
        let listed = self.inbox.item(&key)?;
        if crate::ui::inbox_view::updated_ms(listed) <= session.updated_at {
            return None;
        }
        Some(
            div()
                .id(SharedString::from(format!("linked-dot-{}", session.id)))
                .size(px(6.0))
                .flex_none()
                .rounded_full()
                .bg(cx.theme().colors.accent)
                .tooltip(Tooltip::text(format!(
                    "Linked {} updated since this session",
                    item.noun()
                )))
                .into_any_element(),
        )
    }

    /// MonoCode's work item badge: opens the item here, ⌘-click or a
    /// middle click opens GitHub.
    pub(crate) fn work_item_badge(&self, session: &SessionRow, cx: &Context<Self>) -> Option<AnyElement> {
        let item = session.linked_work_item.clone()?;
        let accent = cx.theme().colors.accent;
        let icon = if item.noun() == "PR" {
            IconName::GitPullRequest
        } else {
            IconName::CircleDot
        };
        let tip = format!(
            "Open {} #{} beside this session (⌘-click for GitHub)",
            item.noun(),
            item.number
        );
        let (url, middle_url, sid) = (item.url.clone(), item.url.clone(), session.id.clone());
        let number = item.number;
        Some(
            div()
                .id(SharedString::from(format!("work-item-{}", session.id)))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(2.0))
                .px(px(2.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .text_size(px(11.0))
                .text_color(accent)
                .hover(|s| s.underline())
                .tooltip(Tooltip::text(tip))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Middle, move |_, _, cx| {
                    cx.stop_propagation();
                    cx.open_url(&middle_url);
                })
                .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    let mods = event.modifiers();
                    if mods.platform || mods.control {
                        cx.open_url(&url);
                    } else {
                        this.open_linked_work_item(&sid, &item, cx);
                    }
                }))
                .child(Icon::new(icon).size(IconSize::Xs).color(accent))
                .child(format!("#{number}"))
                .into_any_element(),
        )
    }

    /// MonoCode's fuchsia Share icon with the subagents card on hover; a
    /// click opens the orchestrator.
    pub(crate) fn orchestration_button(
        &self,
        session: &SessionRow,
        states: &LiveStates,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let summary = session.orchestration.clone()?;
        let fg = cx.theme().colors.fg;
        let group = SharedString::from(format!("orchestration-{}", session.id));
        let needs_input: Vec<bool> = summary
            .tasks
            .iter()
            .map(|t| self.task_needs_input(t, states))
            .collect();
        let sid = session.id.clone();
        Some(
            div()
                .id(group.clone())
                .group(group.clone())
                .size(px(20.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(move |s| s.bg(fg.opacity(0.10)))
                .tooltip(move |_, cx: &mut App| {
                    let tip = OrchestrationTip {
                        summary: summary.clone(),
                        needs_input: needs_input.clone(),
                    };
                    cx.new(|_| tip).into()
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.open_session(sid.clone(), cx);
                }))
                .child(
                    Icon::new(IconName::Share2)
                        .size(IconSize::Xs)
                        .color(Hsla::from(rgb(FUCHSIA)).opacity(0.65))
                        .group_hover_color(group, Hsla::from(rgb(FUCHSIA_HOVER)).opacity(0.9)),
                )
                .into_any_element(),
        )
    }

    /// MonoCode `OrchestrationSidebarAgents`: the agent count, then one
    /// row per task (a task needing input opens itself).
    pub(crate) fn orchestration_agents(
        &self,
        session: &SessionRow,
        states: &LiveStates,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let summary = session.orchestration.as_ref()?;
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let total = summary.tasks.len();
        let rows = summary.tasks.iter().enumerate().map(|(ix, task)| {
            let needs = self.task_needs_input(task, states);
            let key = format!("{}:{ix}", session.id);
            let open = needs || self.sessions_ui.open_agents.contains(&key);
            let label = task.label(&summary.status, needs);
            let tone = tone_color(task.tone(needs), fg);
            let model = catalog::find(&task.model).map_or_else(|| task.model.clone(), |m| m.label);
            let harness_title = HarnessKind::from_id(&task.harness).map_or(task.harness.as_str(), |k| k.label());
            let tip = format!("{} · {harness_title} · {model} · {label}", task.title);
            let group = SharedString::from(format!("agent-{key}"));
            let toggle_key = key.clone();
            let worker = task.session_id.clone();
            let row = div()
                .id(group.clone())
                .group(group.clone())
                .flex()
                .w_full()
                .items_center()
                .gap_1p5()
                .px_2()
                .py(px(6.0))
                .rounded(px(6.0))
                .when(!open, move |el| el.hover(move |s| s.bg(fg.opacity(0.10))))
                .tooltip(Tooltip::text(tip))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if !this.sessions_ui.open_agents.remove(&toggle_key) {
                        this.sessions_ui.open_agents.insert(toggle_key.clone());
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .relative()
                        .size(px(14.0))
                        .flex_none()
                        .map(|el| {
                            if open {
                                el.child(Icon::new(IconName::ChevronDown).size(IconSize::Xs).color(fg.opacity(0.45)))
                            } else {
                                el.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .opacity(0.75)
                                        .group_hover(group.clone(), |s| s.invisible())
                                        .child(HarnessIcon::new(&task.harness).size(px(14.0))),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .invisible()
                                        .group_hover(group.clone(), |s| s.visible())
                                        .child(
                                            Icon::new(IconName::ChevronRight)
                                                .size(IconSize::Xs)
                                                .color(fg.opacity(0.45)),
                                        ),
                                )
                            }
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.0))
                        .line_height(px(12.0 * 1.375))
                        .text_color(fg.opacity(0.8))
                        .child(task.title.clone()),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .text_size(px(11.0))
                        .text_color(tone)
                        .map(|el| match task.tone(needs) {
                            TaskTone::Attention => el.child(Icon::new(IconName::CircleAlert).size(IconSize::Xs).color(tone)),
                            TaskTone::Done => el.child(Icon::new(IconName::Check).size(IconSize::Xs).color(tone)),
                            TaskTone::Quiet => el,
                        })
                        .child(label),
                );
            let detail = open.then(|| {
                div()
                    .pl(px(26.0))
                    .pr_2()
                    .pb_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(11.0))
                            .text_color(fg.opacity(0.45))
                            .child(HarnessIcon::new(&task.harness).size(px(14.0)))
                            .child(model.clone()),
                    )
                    .when_some(worker.clone(), |el, worker| {
                        el.child(
                            div()
                                .id(SharedString::from(format!("agent-open-{key}")))
                                .w_auto()
                                .self_start()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(fg.opacity(0.15))
                                .hover(move |s| s.bg(fg.opacity(0.25)).text_color(fg))
                                .cursor_pointer()
                                .text_size(px(11.0))
                                .text_color(fg.opacity(0.75))
                                .tooltip(Tooltip::text("Open this agent beside the orchestrator"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.open_worker_session(&worker, cx);
                                }))
                                .child("See details"),
                        )
                    })
            });
            div()
                .rounded(px(6.0))
                .when(open, |el| el.bg(colors.active))
                .child(row)
                .children(detail)
        });
        Some(
            div()
                .relative()
                .mt_1p5()
                .child(
                    div()
                        .mb(px(2.0))
                        .px(px(2.0))
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(px(11.0))
                        .text_color(fg.opacity(0.45))
                        .child(plural(total, "agent"))
                        .child(format!("{}/{total} done", summary.done())),
                )
                .child(div().mx(px(-8.0)).flex().flex_col().gap(px(1.0)).children(rows))
                .into_any_element(),
        )
    }

    /// "See details": the worker thread (hidden from the list) opens.
    fn open_worker_session(&mut self, worker: &str, cx: &mut Context<Self>) {
        if self.sessions.iter().any(|s| s.id == worker) {
            return self.open_session(worker.to_string(), cx);
        }
        let (id, row) = (worker.to_string(), worker.to_string());
        self.db_then(
            cx,
            move |db| db.get_session(&row),
            move |this, loaded, cx| match loaded {
                Ok(Some(row)) => {
                    if !this.sessions.iter().any(|s| s.id == id) {
                        this.sessions.push(row);
                    }
                    this.open_session(id, cx);
                }
                Ok(None) => {}
                Err(err) => log::error!("could not load agent thread {id}: {err:#}"),
            },
        );
    }
}
