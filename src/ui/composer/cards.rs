//! MonoCode's composer cards (`Composer.tsx` `noteCard` / `handoffCard`):
//! context a new thread starts with, shown above the prompt until the next
//! send. A card can be sent without a message, changes the placeholder,
//! wraps what the agent reads, and leaves its mark on the sent turn.

use std::rc::Rc;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, Div, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    div, prelude::*,
};

use crate::ui::scale::px;

use super::handoff::{HandoffCard, HandoffMeta, handoff_mini_card};
use super::note_card::{NoteCard, note_mini_card};
use crate::app::BenCodeApp;
use crate::db::{Block, SessionRow};
use crate::ui::attachment_chip::OnRemove;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposerCard {
    Note(NoteCard),
    Handoff(HandoffCard),
    Inbox(super::inbox_card::InboxCard),
}

impl ComposerCard {
    /// What the agent reads for the typed `text`.
    pub fn agent_prompt(&self, text: &str) -> String {
        match self {
            Self::Note(card) => super::note_card::compose_note_message(Some(card), text),
            Self::Handoff(card) => card.agent_prompt(text),
            Self::Inbox(card) => super::inbox_card::compose_inbox_message(card, text),
        }
    }

    /// MonoCode's placeholder while the card waits.
    fn placeholder(&self) -> &'static str {
        match self {
            Self::Note(_) => "Add a message, or send…",
            Self::Handoff(_) => "Add context, or send to continue…",
            Self::Inbox(_) => "Add a note, or send to start…",
        }
    }

    /// Marks the sent user block (MonoCode `userTurnCards`).
    pub fn stamp(&self, block: &mut Block) {
        match self {
            Self::Note(card) => {
                block.extra.insert("noteCard".into(), card.meta.to_json());
            }
            Self::Handoff(card) => block.second_opinion = Some(card.turn_meta()),
            // MonoCode sends the issue in the message itself.
            Self::Inbox(_) => {}
        }
    }

    fn render(&self, on_dismiss: OnRemove, cx: &Context<BenCodeApp>) -> AnyElement {
        match self {
            Self::Note(card) => note_mini_card(&card.meta, false, Some(on_dismiss), cx),
            Self::Handoff(card) => {
                handoff_mini_card(&HandoffMeta::from(card), Some(on_dismiss), cx)
            }
            Self::Inbox(card) => super::inbox_card::inbox_mini_card(card, Some(on_dismiss), cx),
        }
    }
}

/// The cards a sent turn shows in its bubble.
pub fn turn_cards(block: &Block, cx: &Context<BenCodeApp>) -> Option<AnyElement> {
    if let Some(meta) = super::note_card::NoteCardMeta::from_block(&block.extra) {
        return Some(note_mini_card(&meta, true, None, cx));
    }
    HandoffMeta::from_block(block).map(|meta| handoff_mini_card(&meta, None, cx))
}

/// MonoCode's mini-card frame (`rounded-md border-content/10 bg-content/6`)
/// with the corner × when it can be removed.
pub fn card_frame(id: &str, on_dismiss: Option<OnRemove>, cx: &Context<BenCodeApp>) -> Div {
    let fg = cx.theme().colors.fg;
    let hover = fg.opacity(0.1);
    div()
        .relative()
        .rounded(px(6.0))
        .border_1()
        .border_color(fg.opacity(0.1))
        .bg(fg.opacity(0.06))
        .px(px(10.0))
        .py_2()
        .when_some(on_dismiss, |el, dismiss| {
            el.pr_8().child(
                div()
                    .id(SharedString::from(format!("card-remove-{id}")))
                    .absolute()
                    .right(px(6.0))
                    .top(px(6.0))
                    .size(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .tooltip(Tooltip::text("Remove"))
                    .on_click(cx.listener(move |this, _, _, cx| dismiss(this, cx)))
                    .child(
                        Icon::new(IconName::X)
                            .size(IconSize::Xs)
                            .color(fg.opacity(0.4)),
                    ),
            )
        })
}

/// A card's leading icon: Ely's, or one Ely lacks.
pub enum CardIcon {
    Named(IconName),
    Extra(crate::ui::icons::ExtraIcon),
}

/// A card's first line: its icon and kind ("Note · slug", "Handoff").
pub fn card_kind(icon: CardIcon, kind: String, cx: &Context<BenCodeApp>) -> Div {
    let tint = cx.theme().colors.fg.opacity(0.45);
    let glyph = match icon {
        CardIcon::Named(name) => Icon::new(name)
            .size(IconSize::Xs)
            .color(tint)
            .into_any_element(),
        CardIcon::Extra(extra) => extra.icon().size(IconSize::Sm).color(tint).into_any_element(),
    };
    let fg = cx.theme().colors.fg;
    div()
        .flex()
        .min_w_0()
        .items_center()
        .gap_1p5()
        .child(glyph)
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(11.0))
                .text_color(fg.opacity(0.5))
                .child(kind),
        )
}

impl BenCodeApp {
    /// A new thread in `cwd`, set up by `configure`, opened in a tab with
    /// `card` in its composer.
    pub fn open_thread_with_card(
        &mut self,
        cwd: &str,
        configure: impl FnOnce(&mut SessionRow),
        card: ComposerCard,
        cx: &mut Context<Self>,
    ) {
        let id = self.create_session_row(cwd);
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
            configure(session);
        }
        self.persist_session(&id);
        self.tabs.open(&id);
        self.sync_selection(cx);
        self.thread_mut(&id).composer_card = Some(card);
        self.sync_prompt_placeholder(cx);
        self.refocus_prompt(cx);
        cx.notify();
    }

    fn remove_composer_card(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if let Some(thread) = self.threads.get_mut(session_id) {
            thread.composer_card = None;
        }
        self.sync_prompt_placeholder(cx);
        cx.notify();
    }

    /// The focused thread carries a card, so it can be sent as is.
    pub fn has_composer_card(&self) -> bool {
        self.selected_session_id
            .as_ref()
            .is_some_and(|id| self.thread(id).is_some_and(|t| t.composer_card.is_some()))
    }

    /// The placeholder follows the focused thread's card.
    pub fn sync_prompt_placeholder(&mut self, cx: &mut Context<Self>) {
        let placeholder = match self.selected_session_id.as_deref() {
            Some(id) if self.worktree_removed(id) => super::removed_worktree::REMOVED_PLACEHOLDER,
            id => id
                .and_then(|id| self.thread(id)?.composer_card.as_ref())
                .map_or(super::PROMPT_PLACEHOLDER, ComposerCard::placeholder),
        };
        self.prompt_input.update(cx, |input, cx| {
            if input.placeholder_text().as_ref() != placeholder {
                input.set_placeholder(placeholder, cx);
            }
        });
    }

    /// The focused thread's card above the prompt.
    pub(super) fn render_composer_card(
        &self,
        session_id: &str,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let card = self.thread(session_id)?.composer_card.as_ref()?;
        let sid = session_id.to_string();
        let dismiss: OnRemove = Rc::new(move |this, cx| this.remove_composer_card(&sid, cx));
        Some(
            div()
                .px_3()
                .pt_2()
                .child(card.render(dismiss, cx))
                .into_any_element(),
        )
    }
}
