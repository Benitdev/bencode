//! MonoCode `MonoActivityView` over `MonoActivityTrail`: the steps of the
//! turn picked from a Mono's chat, on its Overview's Activity tab. A line
//! saying whether it is still working (and, once settled, what the work
//! came to), then the work the turn folds, drawn with the transcript's own
//! rows (phases, tool calls, subagents, the prose between them).

use bencode_ui::theme::ActiveTheme;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div};

use super::turns::{self, Item, TurnLayout};
use crate::app::BenCodeApp;
use crate::ui::scale::px;
use crate::ui::spinner;
use crate::ui::tokens::{Ink, Text, Tone as _};

impl BenCodeApp {
    /// The turn `turn_id` of `session_id`, laid out as the chat lays it
    /// out, or `None` once it is gone (a reset).
    fn mono_turn(&self, session_id: &str, turn_id: &str) -> Option<TurnLayout> {
        let session = self.sessions.iter().find(|s| s.id == session_id)?;
        let blocks = &session.blocks;
        let cached = self
            .transcripts
            .get(session_id)
            .filter(|view| view.session_id.as_deref() == Some(session_id))
            .and_then(|view| view.turns.iter().find(|t| t.id(blocks) == turn_id).cloned());
        cached.or_else(|| {
            turns::layout_turns(blocks, self.is_agent_running_in(session_id), true)
                .into_iter()
                .find(|turn| turn.range.start < blocks.len() && turn.id(blocks) == turn_id)
        })
    }

    pub(crate) fn render_mono_activity(
        &self,
        session_id: &str,
        turn_id: &str,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let session = self.sessions.iter().find(|s| s.id == session_id)?;
        let turn = self.mono_turn(session_id, turn_id)?;
        let fg = cx.theme().colors.fg;
        let blocks = &session.blocks;
        let live = turn.live;
        // The work: what the chat folds, else every step but the message.
        let items: Vec<usize> = match turn.fold {
            Some((start, end)) => (start..end).collect(),
            None => (0..turn.items.len())
                .filter(|ix| !matches!(turn.items[*ix], Item::Block(b) if blocks[b].role == "user"))
                .collect(),
        };
        let steps: Vec<usize> = items
            .iter()
            .flat_map(|ix| match &turn.items[*ix] {
                Item::Activity(group) | Item::Subagents(group) => group.clone(),
                Item::Block(b) => vec![*b],
            })
            .collect();
        let summary = (!live)
            .then(|| turns::work_summary(blocks, &steps, false))
            .filter(|s| !s.is_empty());
        let color = self
            .mono_for_session(session_id)
            .map(|mono| self.mono_color(mono))
            .unwrap_or(fg);
        let faint = fg.ink(Ink::Faint);
        // Pulsing while live: a busy mark (`ui/marks.rs`).
        let dot = crate::ui::marks::mark(
            move |cx| {
                let dot = if live {
                    color.opacity(0.4 + 0.6 * spinner::pulse(cx))
                } else {
                    faint
                };
                div()
                    .flex_none()
                    .size(px(6.0))
                    .rounded_full()
                    .bg(dot)
                    .into_any_element()
            },
            cx,
        );
        let line = div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .px_2()
            .pb_3()
            .pt_1()
            .text_size(Text::Sm.px())
            .line_height(px(16.0))
            .text_color(fg.ink(Ink::Muted))
            .child(dot)
            .child(
                div()
                    .flex_none()
                    .text_color(fg.ink(Ink::Body))
                    .child(if live { "Working" } else { "Finished" }),
            )
            .children(
                summary.map(|summary| div().min_w_0().truncate().child(format!("· {summary}"))),
            );
        Some(
            div()
                .child(line)
                .child(
                    div().px_1().pb_2().children(
                        items
                            .into_iter()
                            .map(|item| self.render_fold_item(session, &turn, item, cx)),
                    ),
                )
                .into_any_element(),
        )
    }
}
