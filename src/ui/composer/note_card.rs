//! MonoCode's note card (`NoteMiniCard`, `notes.ts` `composeNoteMessage`):
//! a note's "Add to chat" opens a new thread whose composer carries the
//! note as a card. The person may add a message or send it as is; the agent
//! gets the message with the note appended, the transcript keeps the
//! message and a small card naming the note.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use std::rc::Rc;

use crate::app::BenCodeApp;
use crate::db::Note;
use crate::ui::attachment_chip::OnRemove;

/// MonoCode's composer placeholder while a note card is attached.
const NOTE_PLACEHOLDER: &str = "Add a message, or send…";

/// MonoCode `NoteComposerCard`: the note as it was added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteCard {
    pub meta: NoteCardMeta,
    pub body: String,
}

/// MonoCode `NoteCardMeta`, kept on the sent user block as `noteCard`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteCardMeta {
    pub id: String,
    pub slug: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_cwd: Option<String>,
}

impl NoteCardMeta {
    /// The card on a sent user block, if it has one.
    pub fn from_block(extra: &serde_json::Map<String, Value>) -> Option<Self> {
        serde_json::from_value(extra.get("noteCard")?.clone()).ok()
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    fn heading(&self) -> &str {
        match self.title.trim() {
            "" => "Untitled",
            title => title,
        }
    }
}

impl From<&Note> for NoteCard {
    fn from(note: &Note) -> Self {
        Self {
            meta: NoteCardMeta {
                id: note.id.clone(),
                slug: note.slug.clone(),
                title: note.title.clone(),
                source_cwd: note.source_cwd.clone().filter(|cwd| !cwd.is_empty()),
            },
            body: note.body.clone(),
        }
    }
}

/// MonoCode `composeNoteMessage` / `injectNotePrompt`: what the agent reads.
pub fn compose_note_message(card: Option<&NoteCard>, text: &str) -> String {
    let Some(card) = card else {
        return text.to_string();
    };
    let lead = match text.trim() {
        "" => "Use this note.",
        typed => typed,
    };
    format!(
        "{lead}\n\n---\nReferenced note \"{}\":\n\n{}",
        card.meta.heading(),
        card.body.trim()
    )
}

/// MonoCode `NoteMiniCard`: "Note · slug", the title, and (in the composer)
/// the project it came from. `on_dismiss` adds the corner ×.
pub fn note_mini_card(
    meta: &NoteCardMeta,
    embedded: bool,
    on_dismiss: Option<OnRemove>,
    cx: &Context<BenCodeApp>,
) -> AnyElement {
    let colors = &cx.theme().colors;
    let fg = colors.fg;
    let kind = match (embedded, meta.slug.is_empty()) {
        (false, false) => format!("Note · {}", meta.slug),
        _ => "Note".to_string(),
    };
    let project = meta
        .source_cwd
        .as_deref()
        .filter(|_| !embedded)
        .and_then(|cwd| std::path::Path::new(cwd).file_name())
        .map(|name| name.to_string_lossy().into_owned());
    div()
        .relative()
        .rounded(px(6.0))
        .border_1()
        .border_color(fg.opacity(0.1))
        .bg(fg.opacity(0.06))
        .px(px(10.0))
        .py_2()
        .when(on_dismiss.is_some(), |el| el.pr_8())
        .child(
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap_1p5()
                .child(
                    Icon::new(IconName::File)
                        .size(IconSize::Xs)
                        .color(fg.opacity(0.45)),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.0))
                        .text_color(fg.opacity(0.5))
                        .child(kind),
                ),
        )
        .child(
            div()
                .mt_1()
                .truncate()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg)
                .child(SharedString::from(meta.heading().to_string())),
        )
        .when_some(project, |el, project| {
            el.child(
                div()
                    .mt_1()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_1p5()
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.45))
                    .child(
                        Icon::new(IconName::Folder)
                            .size(IconSize::Xs)
                            .color(fg.opacity(0.45)),
                    )
                    .child(div().min_w_0().truncate().child(project)),
            )
        })
        .when_some(on_dismiss, |el, dismiss| {
            let hover = fg.opacity(0.1);
            el.child(
                div()
                    .id(SharedString::from(format!("note-card-remove-{}", meta.id)))
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
        .into_any_element()
}

impl BenCodeApp {
    /// MonoCode `onAddNoteToChat`: a new thread (in the note's project when
    /// it still is one), titled after the note, its composer holding the
    /// card. Empty notes are not added.
    pub fn add_note_to_chat(&mut self, note: &Note, cx: &mut Context<Self>) {
        if note.body.trim().is_empty() {
            return;
        }
        let card = NoteCard::from(note);
        let cwd = card
            .meta
            .source_cwd
            .clone()
            .filter(|cwd| std::path::Path::new(cwd).is_dir())
            .unwrap_or_else(|| self.current_cwd.clone());
        self.close_notes(cx);
        let id = self.create_session_row(&cwd);
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id)
            && !card.meta.title.trim().is_empty()
        {
            session.title = card.meta.title.trim().to_string();
            self.persist_session(&id);
        }
        self.tabs.open(&id);
        self.sync_selection(cx);
        self.note_cards.insert(id, card);
        self.sync_prompt_placeholder(cx);
        self.refocus_prompt(cx);
        cx.notify();
    }

    fn remove_note_card(&mut self, session_id: &str, cx: &mut Context<Self>) {
        self.note_cards.remove(session_id);
        self.sync_prompt_placeholder(cx);
        cx.notify();
    }

    /// MonoCode's placeholder follows the card: "Add a message, or send…"
    /// while a note waits to be sent.
    pub fn sync_prompt_placeholder(&mut self, cx: &mut Context<Self>) {
        let carded = self
            .selected_session_id
            .as_ref()
            .is_some_and(|id| self.note_cards.contains_key(id));
        let placeholder = if carded {
            NOTE_PLACEHOLDER
        } else {
            super::PROMPT_PLACEHOLDER
        };
        self.prompt_input.update(cx, |input, cx| {
            if input.placeholder_text().as_ref() != placeholder {
                input.set_placeholder(placeholder, cx);
            }
        });
    }

    /// The focused thread's note card above the prompt.
    pub(super) fn render_note_card(
        &self,
        session_id: &str,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let card = self.note_cards.get(session_id)?;
        let sid = session_id.to_string();
        Some(
            div()
                .px_3()
                .pt_2()
                .child(note_mini_card(
                    &card.meta,
                    false,
                    Some(Rc::new(move |this, cx| this.remove_note_card(&sid, cx))),
                    cx,
                ))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(title: &str) -> NoteCard {
        NoteCard {
            meta: NoteCardMeta {
                id: "n1".into(),
                slug: "release-plan".into(),
                title: title.into(),
                source_cwd: None,
            },
            body: "  Ship on Friday.\n".into(),
        }
    }

    #[test]
    fn the_note_follows_the_message() {
        assert_eq!(
            compose_note_message(Some(&card("Release")), " check this "),
            "check this\n\n---\nReferenced note \"Release\":\n\nShip on Friday."
        );
        assert_eq!(
            compose_note_message(Some(&card(" ")), ""),
            "Use this note.\n\n---\nReferenced note \"Untitled\":\n\nShip on Friday."
        );
        assert_eq!(compose_note_message(None, "plain"), "plain");
    }

    #[test]
    fn the_block_keeps_only_the_card_meta() {
        let meta = card("Release").meta;
        let mut extra = serde_json::Map::new();
        extra.insert("noteCard".into(), meta.to_json());
        assert_eq!(extra["noteCard"]["slug"], "release-plan");
        assert!(extra["noteCard"].get("sourceCwd").is_none());
        assert_eq!(NoteCardMeta::from_block(&extra), Some(meta));
    }
}
