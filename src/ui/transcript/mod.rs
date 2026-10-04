//! Active thread: header, a virtualized transcript and the composer.
//!
//! Only visible rows are laid out (`gpui::list`), and only the streaming tail
//! is re-measured, so a long thread stays cheap while tokens arrive.

mod activity;
pub mod blocks;
pub mod find;
pub mod turns;

use std::collections::{HashMap, HashSet};

use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AnyElement, Context, FollowMode, FontWeight, InteractiveElement, IntoElement, ListAlignment,
    ListState, ParentElement, SharedString, Styled, Window, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;
use blocks::MESSAGE_MAX_WIDTH;
use turns::{Row, TurnLayout};

/// Rows near the end that may still change height while an agent runs.
const LIVE_TAIL_ROWS: usize = 3;
const LIST_OVERDRAW: gpui::Pixels = px(600.0);

/// Scroll and measurement state for the transcript list.
pub struct TranscriptView {
    pub list: ListState,
    pub session_id: Option<String>,
    /// The thread laid out as turns, and the list rows drawn from them.
    pub turns: Vec<TurnLayout>,
    pub rows: Vec<Row>,
}

impl Default for TranscriptView {
    fn default() -> Self {
        Self {
            list: ListState::new(0, ListAlignment::Bottom, LIST_OVERDRAW),
            session_id: None,
            turns: Vec::new(),
            rows: Vec::new(),
        }
    }
}

/// What the transcript remembers the user opened (MonoCode keeps these in
/// component state): folds, phases, failed tool output, long messages.
#[derive(Default)]
pub struct TranscriptUiState {
    pub open_folds: HashSet<String>,
    pub phase_open: HashMap<String, bool>,
    pub open_tool_errors: HashSet<String>,
    pub expanded_messages: HashSet<String>,
    /// Copy buttons showing their check.
    pub copied: HashSet<String>,
}

/// The fold line's clock ticks once a second while an agent runs.
const CLOCK_TICK: std::time::Duration = std::time::Duration::from_secs(1);

impl BenCodeApp {
    /// Redraws every second while any agent runs, so "working for 12s" ticks.
    pub fn start_clock(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK_TICK).await;
                let ticked = this.update(cx, |app, cx| {
                    let countdown = app.tick_usage_limits(cx);
                    if app.is_agent_running() || countdown {
                        cx.notify();
                    }
                });
                if ticked.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// Gets or creates the transcript view for a given session.
    pub fn transcript_view_for(&mut self, session_id: &str) -> &mut TranscriptView {
        self.transcripts.entry(session_id.to_string()).or_default()
    }

    /// Lays the thread out as turns and keeps the list's rows in step. Rows
    /// before the first change keep their measurements; the live tail is
    /// re-measured while the agent streams.
    pub fn sync_transcript_list_for(&mut self, session_id: &str) {
        let running = self.is_agent_running_in(session_id);
        let waiting = self.pending_permission_for(session_id).is_some();
        let (turns, rows) = match self.sessions.iter().find(|s| s.id == session_id) {
            Some(s) => {
                let turns = turns::layout_turns(&s.blocks, running);
                let rows =
                    turns::build_rows(&s.blocks, &turns, &self.transcript_ui.open_folds, waiting);
                (turns, rows)
            }
            None => (Vec::new(), Vec::new()),
        };
        let view = self.transcripts.entry(session_id.to_string()).or_default();
        let count = rows.len();
        if view.session_id.as_deref() != Some(session_id) {
            view.list.reset(count);
            view.list.set_follow_mode(FollowMode::Tail);
            view.list.scroll_to_end();
            view.session_id = Some(session_id.to_string());
        } else {
            let old = view.rows.len();
            let same = view
                .rows
                .iter()
                .zip(&rows)
                .take_while(|(a, b)| a == b)
                .count();
            if same < old || count != old {
                view.list.splice(same..old, count - same);
            }
            if running {
                view.list
                    .remeasure_items(count.saturating_sub(LIVE_TAIL_ROWS)..count);
            }
        }
        view.turns = turns;
        view.rows = rows;
    }

    pub fn render_transcript_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // Delegate to multi-pane tree layout
        self.render_pane_tree(cx)
    }

    pub fn render_transcript_row_for(
        &mut self,
        session_id: &str,
        ix: usize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(view) = self.transcripts.get(session_id) else {
            return div().into_any_element();
        };
        let (Some(row), Some(session)) = (
            view.rows.get(ix),
            self.sessions.iter().find(|s| s.id == session_id),
        ) else {
            return div().into_any_element();
        };
        let turn_of = |t: usize| &view.turns[t];
        // Markdown cannot paint single words, so the row of the current find
        // match is tinted instead (user messages paint their words).
        let find_hit = self
            .find_current_block(session_id, cx)
            .is_some_and(|block| {
                session.blocks[block].role != "user" && find::row_shows(row, &view.turns, block)
            });
        let content = match row {
            Row::Item { turn, item } => self.render_turn_item(session, turn_of(*turn), *item, cx),
            Row::FoldLine { turn } => self.render_fold_line(session, turn_of(*turn), cx),
            Row::FoldItem { turn, item, .. } => {
                self.render_fold_item(session, turn_of(*turn), *item, cx)
            }
            Row::Footer { turn } => self.render_turn_footer(session, turn_of(*turn), cx),
            Row::Trailer => self.render_trailer(session, cx),
        };
        div()
            .w_full()
            .flex()
            .justify_center()
            .px_2()
            .child(
                div()
                    .w_full()
                    .max_w(MESSAGE_MAX_WIDTH)
                    .flex()
                    .flex_col()
                    .when(find_hit, |el| {
                        el.rounded(px(8.0))
                            .bg(cx.theme().colors.accent.opacity(0.1))
                    })
                    .child(content),
            )
            .into_any_element()
    }

    /// The permission prompt, inline under the work like MonoCode's
    /// `ApprovalControls`: what the agent wants, then Allow / Deny.
    pub fn render_trailer(&self, session: &SessionRow, cx: &Context<Self>) -> AnyElement {
        let Some(request) = self
            .pending_permission_for(&session.id)
            .filter(|r| r.tool != crate::app::QUESTION_TOOL)
        else {
            // A question is answered in the composer's form.
            return div().into_any_element();
        };
        let colors = &cx.theme().colors;
        let (allow_id, deny_id) = (session.id.clone(), session.id.clone());
        let button = |id: &str, label: &'static str, primary: bool| {
            let (bg, fg, hover) = if primary {
                (colors.fg, colors.bg, colors.fg.opacity(0.8))
            } else {
                (
                    colors.fg.opacity(0.1),
                    colors.fg.opacity(0.7),
                    colors.fg.opacity(0.2),
                )
            };
            div()
                .id(SharedString::from(format!("{id}-{}", session.id)))
                .px_2p5()
                .py_0p5()
                .rounded(px(6.0))
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .bg(bg)
                .text_color(fg)
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(label)
        };
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .px_4()
            .py_1()
            .child(
                div()
                    .text_size(px(14.0))
                    .text_color(colors.fg.opacity(0.7))
                    .child(format!("Allow {}?", request.tool)),
            )
            .when(!request.description.is_empty(), |el| {
                el.child(
                    div()
                        .font_family(cx.theme().mono_family.clone())
                        .text_size(px(12.0))
                        .text_color(colors.fg.opacity(0.5))
                        .child(request.description.clone()),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(button("perm-allow", "Allow", true).on_click(cx.listener(
                        move |this, _, _, cx| this.answer_permission(&allow_id, true, cx),
                    )))
                    .child(button("perm-deny", "Deny", false).on_click(cx.listener(
                        move |this, _, _, cx| this.answer_permission(&deny_id, false, cx),
                    ))),
            )
            .into_any_element()
    }
}
