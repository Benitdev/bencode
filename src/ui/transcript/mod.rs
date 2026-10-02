//! Active thread: header, a virtualized transcript and the composer.
//!
//! Only visible rows are laid out (`gpui::list`), and only the streaming tail
//! is re-measured, so a long thread stays cheap while tokens arrive.

mod blocks;

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::chat::StreamingCursor;
use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::feedback::{ConfirmationCard, EmptyState};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, TextSize};
use gpui::{
    AnyElement, App, Context, FollowMode, FontWeight, IntoElement, ListAlignment, ListState,
    ParentElement, Styled, Window, div, list, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;
use crate::harness::catalog;
use blocks::{MESSAGE_MAX_WIDTH, render_block};

/// Rows near the end that may still change height while an agent runs.
const LIVE_TAIL_ROWS: usize = 3;
const LIST_OVERDRAW: gpui::Pixels = px(600.0);

const SUGGESTIONS: [(&str, &str); 2] = [
    ("Review recent changes", "Review recent git changes in the workspace and explain them."),
    ("Plan the next step", "Plan and implement the next feature step cleanly."),
];

/// Scroll and measurement state for the transcript list.
pub struct TranscriptView {
    list: ListState,
    session_id: Option<String>,
}

impl Default for TranscriptView {
    fn default() -> Self {
        Self { list: ListState::new(0, ListAlignment::Bottom, LIST_OVERDRAW), session_id: None }
    }
}

impl BenCodeApp {
    /// Keeps the list's item count in step with the thread without
    /// re-measuring rows that did not change.
    fn sync_transcript_list(&mut self) {
        let (id, rows, running) = match self.selected_session() {
            Some(s) => {
                let running = self.is_agent_running_in(&s.id);
                (Some(s.id.clone()), s.blocks.len() + usize::from(running), running)
            }
            None => (None, 0, false),
        };
        let view = &mut self.transcript;
        if view.session_id != id {
            view.list.reset(rows);
            view.list.set_follow_mode(FollowMode::Tail);
            view.list.scroll_to_end();
            view.session_id = id;
            return;
        }
        let old = view.list.item_count();
        if rows > old {
            view.list.splice(old..old, rows - old);
        } else if rows < old {
            view.list.splice(rows..old, 0);
        }
        if running || rows != old {
            view.list.remeasure_items(rows.saturating_sub(LIVE_TAIL_ROWS)..rows);
        }
    }

    pub fn render_transcript_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_transcript_list();
        let bg = cx.theme().colors.bg;
        let body = match self.selected_session() {
            None => EmptyState::new("no-thread", IconName::MessageSquare, "No thread selected")
                .body("Pick a thread from the sidebar or start a new one.")
                .into_any_element(),
            Some(session) if session.blocks.is_empty() => self.render_welcome(session, cx),
            Some(_) => list(
                self.transcript.list.clone(),
                cx.processor(|this, ix: usize, window, cx| this.render_transcript_row(ix, window, cx)),
            )
            .size_full()
            .into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(bg)
            .child(self.render_transcript_header(cx))
            .child(div().flex().flex_col().flex_1().min_h_0().justify_center().child(body))
            .child(self.render_composer(self.selected_session(), cx))
    }

    fn render_transcript_row(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(session) = self.selected_session() else { return div().into_any_element() };
        let running = self.is_agent_running_in(&session.id);
        let content = match session.blocks.get(ix) {
            Some(block) => {
                let live = running && ix + 1 == session.blocks.len() && block.role == "assistant";
                render_block(session, ix, live, cx)
            }
            None => self.render_trailer(session, cx),
        };
        div()
            .flex()
            .justify_center()
            .px_6()
            .py_2()
            .child(div().w_full().max_w(MESSAGE_MAX_WIDTH).child(content))
            .into_any_element()
    }

    /// The row after the last block: a permission prompt, or a working indicator.
    fn render_trailer(&self, session: &SessionRow, cx: &Context<Self>) -> AnyElement {
        if let Some(request) = self.pending_permission_for(&session.id) {
            let approve = cx.listener(|this, _: &(), _, cx| this.approve_permission(cx));
            let deny = cx.listener(|this, _: &(), _, cx| this.deny_permission(cx));
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

    fn render_welcome(&self, session: &SessionRow, cx: &Context<Self>) -> AnyElement {
        let chips = SUGGESTIONS.iter().enumerate().map(|(ix, (label, prompt))| {
            Button::new(("suggestion", ix), *label)
                .variant(ButtonVariant::Secondary)
                .size(ControlSize::Sm)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.prompt_input.update(cx, |input, cx| input.set_text(*prompt, cx));
                }))
        });
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_4()
            .child(
                EmptyState::new("empty-thread", IconName::Sparkles, session.title.clone())
                    .body(format!("{} · {}", catalog::label_for(&session.model), session.cwd)),
            )
            .child(div().flex().flex_wrap().justify_center().gap_2().children(chips))
            .into_any_element()
    }

    fn render_transcript_header(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let session = self.selected_session();
        let title = session.map_or("BenCode", |s| s.title.as_str()).to_string();
        let context = session.and_then(context_percent);

        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap_3()
            .px_6()
            .py_2()
            .border_b_1()
            .border_color(theme.colors.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .min_w_0()
                    .child(div().text_size(theme.text_size(TextSize::Sm)).font_weight(FontWeight::SEMIBOLD).truncate().child(title))
                    .when_some(session.map(|s| s.cwd.clone()), |el, cwd| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .min_w_0()
                                .text_size(theme.text_size(TextSize::Xs))
                                .text_color(theme.colors.fg_muted)
                                .child(Icon::new(IconName::Folder).size(IconSize::Xs))
                                .child(div().truncate().child(cwd)),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap_2()
                    .when_some(session.and_then(|s| s.branch.clone()), |el, branch| el.child(Badge::new(branch)))
                    .when_some(session, |el, s| el.child(Badge::new(catalog::label_for(&s.model))))
                    .when_some(context, |el, pct| el.child(Badge::new(format!("Context {pct}%")).tone(Tone::Info))),
            )
    }
}

/// Share of the context window used, when both numbers are known.
fn context_percent(session: &SessionRow) -> Option<u64> {
    let (used, window) = (session.context_used?, session.context_window?);
    (window > 0).then(|| (used.max(0) as f64 / window as f64 * 100.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_percent_needs_both_numbers() {
        let mut s = SessionRow { context_used: Some(50_000), context_window: Some(200_000), ..Default::default() };
        assert_eq!(context_percent(&s), Some(25));
        s.context_window = Some(0);
        assert_eq!(context_percent(&s), None);
        s.context_window = None;
        assert_eq!(context_percent(&s), None);
    }
}
