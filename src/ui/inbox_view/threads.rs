//! The threads an item has (`app/item_threads.rs`), as the Inbox shows
//! them: a mark on the row's meta line with the most urgent state (the
//! navigator's Agents section's: working, needs you, finished unseen,
//! idle) and a count, and the detail's Threads section, a row per thread.
//! A click on either opens the thread, leaving the Inbox.

use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::item_threads::{ItemThread, ThreadState, most_urgent};
use crate::app::session_list::format_age;
use crate::harness::{HarnessKind, catalog};
use crate::ui::rows::ListRow;
use crate::ui::scale::px;
use crate::ui::spinner::terminal_spinner;
use crate::ui::tokens::{Fill, Ink, Status, Text, Tone as _};

/// A thread state's glyph: the spinner (a busy mark) while it works.
fn state_glyph(state: ThreadState, cx: &gpui::App) -> AnyElement {
    let fg = cx.theme().colors.fg;
    let icon = |name: IconName, color| {
        Icon::new(name)
            .size(IconSize::Xs)
            .color(color)
            .into_any_element()
    };
    match state {
        ThreadState::Working => terminal_spinner(fg.ink(Ink::Secondary), cx).into_any_element(),
        ThreadState::NeedsYou => icon(IconName::CircleAlert, Status::Attention.color(fg)),
        ThreadState::Done => icon(IconName::Check, Status::Done.color(fg)),
        ThreadState::Idle => icon(IconName::Bot, fg.ink(Ink::Muted)),
    }
}

/// "Claude · Opus 4.1 · 5m".
fn thread_line(thread: &ItemThread, now: i64) -> String {
    let harness =
        HarnessKind::from_id(&thread.harness).map_or(thread.harness.as_str(), |k| k.label());
    let mut parts = vec![harness.to_string()];
    if !thread.model.is_empty() {
        parts.push(catalog::label_for(&thread.model));
    }
    let age = format_age(thread.updated_at, now);
    if !age.is_empty() {
        parts.push(age);
    }
    parts.join(" · ")
}

/// The row mark's tooltip: the threads by state and title.
fn threads_tip(threads: &[ItemThread]) -> String {
    threads
        .iter()
        .map(|thread| format!("{}: {}", thread.state.label(), thread.title))
        .collect::<Vec<_>>()
        .join("\n")
}

impl BenCodeApp {
    /// The row's mark for its threads; `None` when it has none.
    pub(super) fn render_inbox_row_threads(
        &self,
        key: &str,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let threads = self.item_threads(key);
        let state = most_urgent(&threads)?;
        let fg = cx.theme().colors.fg;
        let hover = fg.fill(Fill::Hover);
        let tone = match state {
            ThreadState::NeedsYou => Status::Attention.color(fg),
            ThreadState::Done => Status::Done.color(fg),
            ThreadState::Working | ThreadState::Idle => fg.ink(Ink::Muted),
        };
        // The thread most worth a look: the most urgent, newest first.
        let open = (threads.iter())
            .find(|thread| thread.state == state)
            .map(|thread| thread.id.clone())?;
        let count = threads.len();
        Some(
            div()
                .id(SharedString::from(format!("inbox-row-threads-{key}")))
                .flex_none()
                .flex()
                .items_center()
                .gap_0p5()
                .px(px(3.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .text_color(tone)
                .tooltip(Tooltip::text(threads_tip(&threads)))
                // The row's own click selects the item: this one opens
                // the thread instead.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.select_live_agent(&open, cx);
                }))
                .child(state_glyph(state, cx))
                .when(count > 1, |el| el.child(count.to_string()))
                .into_any_element(),
        )
    }

    /// The detail's Threads section; `None` while the item has none.
    pub(super) fn render_inbox_threads(&self, key: &str, cx: &Context<Self>) -> Option<AnyElement> {
        let threads = self.item_threads(key);
        if threads.is_empty() {
            return None;
        }
        let fg = cx.theme().colors.fg;
        let now = crate::app::now_ms();
        let heading = div()
            .text_size(Text::Sm.px())
            .text_color(fg.ink(Ink::Muted))
            .child(format!("Threads · {}", threads.len()));
        let rows = threads.into_iter().map(|thread| {
            let id = thread.id.clone();
            ListRow::new(format!("inbox-thread-{}", thread.id), thread.title.clone())
                .tile(state_glyph(thread.state, cx))
                .line(thread_line(&thread, now))
                .tooltip(format!("{} · {}", thread.state.label(), thread.title))
                .on_click(cx.listener(move |this, _, _, cx| this.select_live_agent(&id, cx)))
        });
        Some(
            div()
                .child(heading)
                .child(div().mt_1().children(rows))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_reads_as_its_harness_model_and_age() {
        let thread = ItemThread {
            id: "a".into(),
            title: "Fix the rail".into(),
            harness: "claude".into(),
            model: String::new(),
            updated_at: 1_000,
            state: ThreadState::Idle,
        };
        assert_eq!(thread_line(&thread, 1_000 + 5 * 60_000), "Claude Code · 5m");
        assert_eq!(threads_tip(&[thread]), "Idle: Fix the rail");
    }
}
