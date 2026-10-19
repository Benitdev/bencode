//! The Overview's Board tab (BenCode's own; MonoCode lists Sessions): the
//! Mono's tasks under To do, Running, Review and Done, then the sessions
//! it started off the board. A task with a session opens it; Mark done,
//! Reopen and Remove show on hover. Done is folded until opened.

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::*,
};

use super::empty_state;
use crate::app::BenCodeApp;
use crate::app::mono_overview::OverviewTab;
use crate::app::mono_sessions::{LaunchedState, SpawnedSession};
use crate::monos::Mono;
use crate::monos::board::{self, Column, Task};
use crate::ui::HarnessIcon;
use crate::ui::rows::{ListRow, row_button};
use crate::ui::scale::px;
use crate::ui::tokens::Status;
use crate::ui::tokens::{Ink, Text, Tone as _};

/// A column's title and count; Done's folds the column.
fn column_title(title: &str, count: usize, fg: Hsla) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .pb(px(6.0))
        .text_size(Text::Sm.px())
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg.ink(Ink::Muted))
        .child(title.to_uppercase())
        .child(
            div()
                .text_color(fg.ink(Ink::Faint))
                .child(count.to_string()),
        )
}

impl BenCodeApp {
    pub(super) fn render_mono_board(&self, mono: &Mono, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let tasks = self.monos.overview.board.as_deref().unwrap_or(&[]);
        let others = self.off_board_launches(mono, tasks);
        if tasks.is_empty() && others.is_empty() {
            return empty_state(
                OverviewTab::Board,
                format!(
                    "Give {} something bigger, like a feature or a release. It splits the work into tasks here and runs each in a session of its own.",
                    mono.display_name()
                ),
                fg,
            );
        }
        let state = |id: &str| self.launched_state(id);
        let mut sections: Vec<AnyElement> = Vec::new();
        if let Some(error) = &self.monos.overview.board_error {
            sections.push(
                div()
                    .px_2()
                    .pb_2()
                    .text_size(Text::Sm.px())
                    .text_color(cx.theme().colors.danger)
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        for column in Column::ALL {
            let list = board::in_column(tasks, column, state);
            if list.is_empty() {
                continue;
            }
            let title = column_title(column.label(), list.len(), fg);
            let body = if column == Column::Done {
                let open = self.monos.overview.done_open;
                let header = title
                    .id("mono-board-done")
                    .cursor_pointer()
                    .rounded(px(6.0))
                    .hover(move |s| s.text_color(fg.ink(Ink::Body)))
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(IconSize::Xs)
                        .color(fg.ink(Ink::Faint)),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_board_done(cx)));
                div().child(header).when(open, |el| {
                    el.children(list.iter().map(|task| self.render_task(task, column, cx)))
                })
            } else {
                div()
                    .child(title)
                    .children(list.iter().map(|task| self.render_task(task, column, cx)))
            };
            sections.push(body.into_any_element());
        }
        if !others.is_empty() {
            sections.push(
                div()
                    .child(column_title("Other sessions", others.len(), fg))
                    .children(
                        others
                            .into_iter()
                            .rev()
                            .map(|launch| self.render_launch(launch, cx)),
                    )
                    .into_any_element(),
            );
        }
        div()
            .children(
                sections
                    .into_iter()
                    .enumerate()
                    .map(|(i, section)| div().when(i > 0, |el| el.mt_4()).child(section)),
            )
            .into_any_element()
    }

    /// The sessions it started that no task names.
    fn off_board_launches(&self, mono: &Mono, tasks: &[Task]) -> Vec<SpawnedSession> {
        self.mono_launches(mono)
            .into_iter()
            .filter(|launch| {
                !tasks
                    .iter()
                    .any(|t| t.session_id.as_deref() == Some(&launch.session_id))
            })
            .collect()
    }

    /// The line under a task: its project, then its model and where its
    /// session is.
    fn task_line(&self, task: &Task, column: Column) -> String {
        let session = task
            .session_id
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| s.id == id));
        let status = match column {
            Column::Todo if task.session_id.is_some() => "Draft".to_string(),
            Column::Todo => "Not started".to_string(),
            Column::Done => task
                .done_at
                .map(|at| {
                    format!(
                        "Done {}",
                        crate::app::session_list::format_relative(at, crate::app::now_ms())
                    )
                })
                .unwrap_or_default(),
            _ => task
                .session_id
                .as_deref()
                .and_then(|id| self.launched_state(id))
                .map(|state| state.label().to_string())
                .unwrap_or_default(),
        };
        let model = session
            .map(|s| crate::harness::catalog::display_label(&s.harness, &s.model))
            .unwrap_or_default();
        [self.rail_project_label(&task.project), model, status]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    }

    fn render_task(&self, task: &Task, column: Column, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let session = task
            .session_id
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| s.id == id));
        let state = task
            .session_id
            .as_deref()
            .and_then(|id| self.launched_state(id));
        let tile = match (column, session) {
            (Column::Running, _) => {
                let color = state.map_or(Status::Working, Status::from).color(fg);
                crate::ui::marks::mark(
                    move |cx| {
                        div()
                            .size(px(8.0))
                            .rounded_full()
                            .bg(color)
                            .opacity(crate::ui::spinner::pulse(cx))
                            .into_any_element()
                    },
                    cx,
                )
            }
            (Column::Done, _) => Icon::new(IconName::CircleCheck)
                .size(IconSize::Sm)
                .color(fg.ink(Ink::Faint))
                .into_any_element(),
            (Column::Review, _) if state == Some(LaunchedState::Failed) => {
                Icon::new(IconName::CircleAlert)
                    .size(IconSize::Sm)
                    .color(Status::Failed.color(fg))
                    .into_any_element()
            }
            (_, Some(session)) => HarnessIcon::new(&session.harness)
                .size(px(14.0))
                .into_any_element(),
            (_, None) => Icon::new(IconName::CircleDashed)
                .size(IconSize::Sm)
                .color(fg.ink(Ink::Muted))
                .into_any_element(),
        };
        let (done_id, remove_id) = (task.id.clone(), task.id.clone());
        let done = match column {
            Column::Done => row_button(
                format!("mono-task-reopen-{}", task.id),
                IconName::RotateCcw,
                "Reopen",
            )
            .on_click(
                cx.listener(move |this, _, _, cx| this.set_board_task_done(&done_id, false, cx)),
            ),
            _ => row_button(
                format!("mono-task-done-{}", task.id),
                IconName::Check,
                "Mark done",
            )
            .on_click(
                cx.listener(move |this, _, _, cx| this.set_board_task_done(&done_id, true, cx)),
            ),
        };
        let row = ListRow::new(format!("mono-task-{}", task.id), task.title.clone())
            .tile(tile)
            .line(self.task_line(task, column))
            .action(done)
            .action(
                row_button(
                    format!("mono-task-remove-{}", task.id),
                    IconName::Trash2,
                    "Remove from board",
                )
                .on_click(
                    cx.listener(move |this, _, _, cx| this.remove_board_task(&remove_id, cx)),
                ),
            );
        match task.session_id.clone() {
            Some(session) => row.on_click(cx.listener(move |this, _, _, cx| this.show_mono_launch(&session, cx))),
            None => row.tooltip(truncate(task.brief(), 400)),
        }
        .into_any_element()
    }

    /// A session it started off the board, as the sidebar's card would say
    /// it (model, title, project, when, status).
    fn render_launch(&self, launch: SpawnedSession, cx: &Context<Self>) -> AnyElement {
        let now = crate::app::now_ms();
        let id = launch.session_id.clone();
        let loaded = self.sessions.iter().find(|s| s.id == id);
        let title = self.launched_title(&launch);
        let project = self.rail_project_label(&launch.cwd);
        let status = match self.launched_state(&id) {
            Some(LaunchedState::Done) | None => loaded
                .map(|s| crate::app::session_list::format_relative(s.updated_at, now))
                .unwrap_or_default(),
            Some(state) => state.label().to_string(),
        };
        let model = crate::harness::catalog::display_label(&launch.harness, &launch.model);
        let line = [project, model, status]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        ListRow::new(format!("mono-session-{id}"), title)
            .tile(HarnessIcon::new(&launch.harness).size(px(14.0)))
            .line(line)
            .on_click(cx.listener(move |this, _, _, cx| this.show_mono_launch(&id, cx)))
            .into_any_element()
    }
}

fn truncate(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}
