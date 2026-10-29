//! The transcript selection's app side: which text joins it, ⌘C / ⌘A /
//! Esc on it, and MonoCode's `TranscriptSelectionMenu` (Add to chat, Add
//! to notes; BenCode adds Ask, to a bot) over a selection inside one
//! answer or prompt.

use std::rc::Rc;

use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::selection::{Document, TextSelection};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::prelude::*;
use gpui::{AnyElement, Context, Div, SharedString, Stateful, canvas, deferred, div, point};

use crate::ui::scale::px;
use crate::ui::selection_area::{self, SelectionArea};

use super::selection::SegCtx;
use super::turns::{self, Item, Row};
use crate::app::BenCodeApp;
use crate::ui::icons::ExtraIcon;
use crate::ui::sidebar_popovers::popover_frame;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// MonoCode `MAX_TITLE` for notes.
const NOTE_TITLE_CHARS: usize = 200;
/// The gap between the selection and the menu above it.
const MENU_GAP: f32 = 6.0;

/// `[text](target)` at the start of `s`: the text and the length taken.
/// As MonoCode's `\[([^\]]*)]\([^)]*\)`, the text holds no `]` and the
/// target no `)`.
fn bracket_link(s: &str) -> Option<(&str, usize)> {
    let rest = s.strip_prefix('[')?;
    let close = rest.find(']')?;
    let target = rest[close + 1..].strip_prefix('(')?;
    let end = target.find(')')?;
    Some((&rest[..close], 1 + close + 2 + end + 1))
}

/// MonoCode `unwrapMarkdown`: no images, links as their text, no `*_\``.
fn unwrap_markdown(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find('[') {
        let image = rest[..at].ends_with('!');
        match bracket_link(&rest[at..]) {
            Some((_, taken)) if image => {
                out.push_str(&rest[..at - 1]);
                rest = &rest[at + taken..];
            }
            Some((text, taken)) if !text.is_empty() => {
                out.push_str(&rest[..at]);
                out.push_str(text);
                rest = &rest[at + taken..];
            }
            _ => {
                out.push_str(&rest[..=at]);
                rest = &rest[at + 1..];
            }
        }
    }
    out.push_str(rest);
    out.retain(|c| !matches!(c, '*' | '_' | '`'));
    out.trim().to_string()
}

/// MonoCode `noteTitle`: the first heading, or the first line of prose.
pub fn note_title(text: &str) -> String {
    let clip = |title: String| title.chars().take(NOTE_TITLE_CHARS).collect::<String>();
    let heading = text.lines().find_map(|line| {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let rest = line[indent..].trim_start_matches('#');
        let hashes = line.len() - indent - rest.len();
        (indent <= 3 && (1..=6).contains(&hashes) && rest.starts_with(char::is_whitespace))
            .then(|| rest.trim())
            .filter(|rest| !rest.is_empty())
    });
    if let Some(title) = heading
        .map(|h| clip(unwrap_markdown(h)))
        .filter(|t| !t.is_empty())
    {
        return title;
    }
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        let line = line.trim();
        if fenced || line.is_empty() || line == "---" {
            continue;
        }
        let title = clip(unwrap_markdown(line));
        if !title.is_empty() {
            return title;
        }
    }
    "Untitled".to_string()
}

impl BenCodeApp {
    /// Where block `block` of `session_id` draws its text. `quotable`: a
    /// settled answer or a prompt, whose selections get the menu.
    pub fn seg_ctx(
        &self,
        session_id: &str,
        block: usize,
        quotable: bool,
        cx: &Context<Self>,
    ) -> SegCtx {
        let app = cx.weak_entity();
        let sid = session_id.to_string();
        let mut doc = Document::new(sid.clone(), &self.text_selection)
            .focus(self.transcript_focus.clone())
            .on_change(move |cx| {
                if let Err(err) = app.update(cx, |this, cx| this.redraw_thread(&sid, cx)) {
                    log::debug!("transcript selection after app drop: {err:#}");
                }
            });
        if let Some(view) = self.transcripts.get(session_id) {
            doc = doc.list(view.list.clone());
        }
        SegCtx {
            doc,
            block,
            quotable,
        }
    }

    /// The thread the selection is in, if it is in one.
    fn selection_thread(&self, cx: &Context<Self>) -> Option<String> {
        let doc = self.text_selection.read(cx).doc()?;
        (self.sessions.iter().any(|s| s.id == doc.as_ref())).then(|| doc.to_string())
    }

    /// The text of each block the list shows as text, for a copy that
    /// reaches blocks the list has not drawn. Closed folds stay out, as
    /// MonoCode does not render them.
    fn shown_block_texts(&self, session_id: &str) -> Vec<(usize, String)> {
        let (Some(view), Some(session)) = (
            self.transcripts.get(session_id),
            self.sessions.iter().find(|s| s.id == session_id),
        ) else {
            return Vec::new();
        };
        view.rows
            .iter()
            .filter_map(|row| match row {
                Row::Item { turn, item } | Row::FoldItem { turn, item, .. } => {
                    match view.turns.get(*turn)?.items.get(*item)? {
                        Item::Block(ix) => Some(*ix),
                        Item::Activity(_) | Item::Subagents(_) => None,
                    }
                }
                _ => None,
            })
            .filter_map(|ix| {
                let text = turns::text(session.blocks.get(ix)?).trim();
                (!text.is_empty()).then(|| (ix, text.to_string()))
            })
            .collect()
    }

    /// ⌘C: the selected text as shown.
    pub fn copy_transcript_selection(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.selection_thread(cx) else {
            return;
        };
        let shown = self.shown_block_texts(&session);
        selection_area::copy(&self.text_selection, &shown, cx);
    }

    /// ⌘A: the whole thread the transcript focus is in.
    pub fn select_all_transcript(&mut self, cx: &mut Context<Self>) {
        let session = self
            .selection_thread(cx)
            .or_else(|| self.selected_session_id.clone());
        if let Some(session) = session {
            TextSelection::select_all(&self.text_selection, &session, cx);
        }
    }

    /// Esc: drops the selection; `false` when there was none.
    pub fn clear_transcript_selection(&mut self, cx: &mut Context<Self>) -> bool {
        TextSelection::clear(&self.text_selection, cx)
    }

    /// MonoCode `saveSelectionNote`: a note titled from the text, linked to
    /// the thread, left closed. The selection clears once it is saved; a
    /// failure shows on the selection's menu.
    fn save_selection_note(&mut self, text: &str, session_id: &str, cx: &mut Context<Self>) {
        let cwd = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.cwd.clone())
            .filter(|cwd| !cwd.is_empty());
        let body = text.replace("\r\n", "\n").replace('\r', "\n");
        let upsert = crate::db::NoteUpsert {
            id: crate::app::unique_id("note"),
            title: note_title(&body),
            body,
            tags: Vec::new(),
            source_session_id: Some(session_id.to_string()),
            source_cwd: cwd,
        };
        let (sid, quoted) = (session_id.to_string(), text.to_string());
        self.db_then(
            cx,
            move |db| db.upsert_note(&upsert),
            move |this, saved, cx| match saved {
                Ok(note) => {
                    this.notes.items.insert(0, note);
                    this.selection_note_error = None;
                    this.clear_transcript_selection(cx);
                }
                Err(err) => {
                    log::error!("failed to save selection as note: {err:#}");
                    this.selection_note_error = Some((quoted, format!("{err:#}")));
                    this.redraw_thread(&sid, cx);
                }
            },
        );
    }

    /// Gives the pane of `session_id` the transcript's focus and keys. A
    /// press beside the text (margins, gaps between messages) starts at the
    /// nearest character, as in MonoCode's WebKit transcript.
    pub(crate) fn with_transcript_selection(
        &self,
        body: Stateful<Div>,
        session_id: &str,
        focused: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let owned = session_id.to_string();
        let body = SelectionArea {
            selection: self.text_selection.clone(),
            focus: self.transcript_focus.clone(),
            key_context: "Transcript",
            // The pane the selection was made in owns the focus; with none,
            // the focused pane does.
            owns: Rc::new(move |doc| doc == owned),
            beside: Some(SharedString::from(session_id.to_string())),
            idle_owner: focused,
        }
        .wrap(body, cx);
        let session = session_id.to_string();
        // MonoCode closes the menu on any scroll; the selection stays.
        body.on_scroll(cx.listener(move |this, _, _, cx| {
            let here = this
                .text_selection
                .read(cx)
                .doc()
                .is_some_and(|d| d == session.as_str());
            if here {
                TextSelection::dismiss(&this.text_selection, cx);
            }
        }))
    }

    /// MonoCode `TranscriptSelectionMenu`, over the selection's first line.
    pub(crate) fn render_selection_menu(
        &self,
        session_id: &str,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let (text, line) = self.text_selection.read(cx).quote(session_id)?;
        let error = (self.selection_note_error.as_ref())
            .filter(|(quoted, _)| *quoted == text)
            .map(|(_, error)| error.clone());
        let selection = self.text_selection.clone();
        let fg = cx.theme().colors.fg;
        let action = |id: &'static str, icon: Icon, label: SharedString| {
            div()
                .id(id)
                .flex()
                .h(px(32.0))
                .w_full()
                .items_center()
                .gap_2()
                .px(px(10.0))
                .rounded(px(8.0))
                .whitespace_nowrap()
                .text_size(Text::Md.px())
                .line_height(px(13.0))
                .text_color(fg)
                .hover(move |s| s.bg(fg.fill(Fill::Hover)))
                .child(icon.size(IconSize::Sm).color(fg))
                .child(label)
        };
        // A bot's own chat is not a thread to ask a bot about.
        let ask = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .filter(|_| self.bots_enabled() && !self.is_bot_session(session_id))
            .and_then(|session| self.bot_for_project(&session.cwd))
            .map(|bot| bot.display_name());
        let (ask_text, ask_sid) = (text.clone(), session_id.to_string());
        let (chat_text, note_text, sid) = (text.clone(), text, session_id.to_string());
        let menu = popover_frame(cx)
            .id("transcript-selection-menu")
            .occlude()
            .relative()
            .p_1()
            .min_w(px(144.0))
            .flex()
            .flex_col()
            .items_stretch()
            .gap(px(2.0))
            .child(
                action(
                    "selection-add-to-chat",
                    ExtraIcon::CommentAdd.icon(),
                    "Add to chat".into(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.add_quote_to_chat(&chat_text, cx);
                    this.clear_transcript_selection(cx);
                })),
            )
            .child(
                action(
                    "selection-add-to-notes",
                    ExtraIcon::FilePlusCorner.icon(),
                    "Add to notes".into(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.save_selection_note(&note_text, &sid, cx);
                })),
            )
            .children(ask.map(|name| {
                action(
                    "selection-ask-bot",
                    Icon::new(IconName::Bot),
                    format!("Ask {name}").into(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ask_bot_about(&ask_sid, Some(ask_text.clone()), cx);
                    this.clear_transcript_selection(cx);
                }))
            }))
            .children(error.map(|error| {
                div()
                    .max_w(px(320.0))
                    .px(px(10.0))
                    .py_1()
                    .text_xs()
                    .text_color(fg.ink(Ink::Body))
                    .child(format!("Could not save note. {error}"))
            }))
            .child(
                // A press on the menu keeps the selection.
                canvas(
                    move |bounds, _, cx| selection.read(cx).keep_presses_within(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        let at = point(line.center().x, line.top() - px(MENU_GAP));
        Some(
            deferred(
                gpui::anchored()
                    .anchor(gpui::Anchor::BottomCenter)
                    .position(at)
                    .snap_to_window()
                    .child(menu),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_titles_come_from_a_heading_or_the_first_prose() {
        assert_eq!(note_title("intro\n## The **Plan**\nbody"), "The Plan");
        assert_eq!(
            note_title("```\ncode\n```\n\n---\nSee [docs](x) now"),
            "See docs now"
        );
        assert_eq!(note_title("![img](a.png) `cargo` _run_"), "cargo run");
        assert_eq!(note_title("#hashtag line"), "#hashtag line");
        assert_eq!(note_title("  \n"), "Untitled");
    }

    #[test]
    fn link_text_stops_at_its_bracket() {
        assert_eq!(unwrap_markdown("[a] and [b](c)"), "[a] and b");
        assert_eq!(unwrap_markdown("see ![](x.png)[]() [ok](y)"), "see []() ok");
    }
}
