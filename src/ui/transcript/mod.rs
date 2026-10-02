//! Active thread: header, a virtualized transcript and the composer.
//!
//! Only visible rows are laid out (`gpui::list`), and only the streaming tail
//! is re-measured, so a long thread stays cheap while tokens arrive.

pub mod blocks;

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::chat::StreamingCursor;
use ely_gpui_component::feedback::ConfirmationCard;
use ely_gpui_component::theme::{ActiveTheme, ControlSize, TextSize};
use gpui::{
    AnyElement, Context, FollowMode, FontWeight, IntoElement, ListAlignment, ListState,
    ParentElement, Styled, Window, div, px,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;
use crate::harness::catalog;
use blocks::{MESSAGE_MAX_WIDTH, render_block};

/// Rows near the end that may still change height while an agent runs.
const LIVE_TAIL_ROWS: usize = 3;
const LIST_OVERDRAW: gpui::Pixels = px(600.0);

const SUGGESTIONS: [(&str, &str); 2] = [
    (
        "Review recent changes",
        "Review recent git changes in the workspace and explain them.",
    ),
    (
        "Plan the next step",
        "Plan and implement the next feature step cleanly.",
    ),
];

/// Scroll and measurement state for the transcript list.
pub struct TranscriptView {
    pub list: ListState,
    pub session_id: Option<String>,
}

impl Default for TranscriptView {
    fn default() -> Self {
        Self {
            list: ListState::new(0, ListAlignment::Bottom, LIST_OVERDRAW),
            session_id: None,
        }
    }
}

impl BenCodeApp {
    /// Gets or creates the transcript view for a given session.
    pub fn transcript_view_for(&mut self, session_id: &str) -> &mut TranscriptView {
        self.transcripts.entry(session_id.to_string()).or_default()
    }

    /// Keeps a session list's item count in step with the thread.
    pub fn sync_transcript_list_for(&mut self, session_id: &str) {
        let (rows, running) = match self.sessions.iter().find(|s| s.id == session_id) {
            Some(s) => {
                let running = self.is_agent_running_in(&s.id);
                (s.blocks.len() + usize::from(running), running)
            }
            None => (0, false),
        };
        let view = self.transcripts.entry(session_id.to_string()).or_default();
        if view.session_id.as_deref() != Some(session_id) {
            view.list.reset(rows);
            view.list.set_follow_mode(FollowMode::Tail);
            view.list.scroll_to_end();
            view.session_id = Some(session_id.to_string());
            return;
        }
        let old = view.list.item_count();
        if rows > old {
            view.list.splice(old..old, rows - old);
        } else if rows < old {
            view.list.splice(rows..old, 0);
        }
        if running || rows != old {
            view.list
                .remeasure_items(rows.saturating_sub(LIVE_TAIL_ROWS)..rows);
        }
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
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return div().into_any_element();
        };
        let running = self.is_agent_running_in(&session.id);
        let content = match session.blocks.get(ix) {
            Some(block) => {
                let live = running && ix + 1 == session.blocks.len() && block.role == "assistant";
                render_block(self, session, ix, live, cx)
            }
            None => self.render_trailer(session, cx),
        };
        div()
            .w_full()
            .flex()
            .justify_center()
            .px_6()
            .py_1p5()
            .child(
                div()
                    .w_full()
                    .max_w(MESSAGE_MAX_WIDTH)
                    .flex()
                    .flex_col()
                    .child(content),
            )
            .into_any_element()
    }

    /// The row after the last block: a permission prompt, or a working indicator.
    pub fn render_trailer(&self, session: &SessionRow, cx: &Context<Self>) -> AnyElement {
        if let Some(request) = self.pending_permission_for(&session.id) {
            let (allow_id, deny_id) = (session.id.clone(), session.id.clone());
            let approve =
                cx.listener(move |this, _: &(), _, cx| this.answer_permission(&allow_id, true, cx));
            let deny =
                cx.listener(move |this, _: &(), _, cx| this.answer_permission(&deny_id, false, cx));
            return ConfirmationCard::new("permission-request", format!("Allow {}?", request.tool))
                .body(request.description.clone())
                .confirm("Allow")
                .on_confirm(move |window, cx| approve(&(), window, cx))
                .on_cancel(move |window, cx| deny(&(), window, cx))
                .into_any_element();
        }
        let theme = cx.theme();
        div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(theme.text_size(TextSize::Xs))
            .text_color(theme.colors.fg_muted)
            .child(StreamingCursor::new("agent-working"))
            .child("Agent is working…")
            .into_any_element()
    }

    pub fn render_welcome(&self, session: &SessionRow, cx: &Context<Self>) -> AnyElement {
        let chips = SUGGESTIONS.iter().enumerate().map(|(ix, (label, prompt))| {
            Button::new(("suggestion", ix), *label)
                .variant(ButtonVariant::Secondary)
                .size(ControlSize::Sm)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.prompt_input
                        .update(cx, |input, cx| input.set_text(*prompt, cx));
                }))
        });
        let project_name = if !session.cwd.is_empty() {
            std::path::Path::new(&session.cwd)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&session.title)
        } else {
            &session.title
        };
        let heading = format!("What should we work on in {project_name}?");
        let subtitle = format!("{} · {}", catalog::label_for(&session.model), session.cwd);
        let theme = cx.theme();

        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .w_full()
            .min_w_0()
            .px_4()
            .py_12()
            .gap_2p5()
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .truncate()
                    .text_align(gpui::TextAlign::Center)
                    .text_size(px(16.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.colors.fg)
                    .child(heading),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .truncate()
                    .text_align(gpui::TextAlign::Center)
                    .text_size(px(12.0))
                    .text_color(theme.colors.fg_muted)
                    .child(subtitle),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap_2()
                    .pt_2()
                    .children(chips),
            )
            .into_any_element()
    }
}

/// Share of the context window used, when both numbers are known.
pub(crate) fn context_percent(session: &SessionRow) -> Option<u64> {
    let (used, window) = (session.context_used?, session.context_window?);
    (window > 0).then(|| (used.max(0) as f64 / window as f64 * 100.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_percent_needs_both_numbers() {
        let mut s = SessionRow {
            context_used: Some(50_000),
            context_window: Some(200_000),
            ..Default::default()
        };
        assert_eq!(context_percent(&s), Some(25));
        s.context_window = Some(0);
        assert_eq!(context_percent(&s), None);
        s.context_window = None;
        assert_eq!(context_percent(&s), None);
    }
}
