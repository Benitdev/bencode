//! The transcript selection's app side: which text joins it, ⌘C / ⌘A /
//! Esc on it, and MonoCode's `TranscriptSelectionMenu` (Add to chat, Add
//! to notes) over a selection inside one answer or prompt.

use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::prelude::*;
use gpui::{
    AnyElement, ClipboardItem, Context, Div, MouseMoveEvent, SharedString, Stateful, canvas,
    deferred, div, point, px,
};

use super::selection::SegCtx;
use super::turns::{self, Item, Row};
use crate::app::BenCodeApp;
use crate::ui::icons::ExtraIcon;
use crate::ui::sidebar_popovers::popover_frame;

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
    /// Where block `block` of `session_id` draws its text. `response`: a
    /// settled answer or a prompt, whose selections get the menu.
    pub fn seg_ctx(&self, session_id: &str, block: usize, response: bool) -> SegCtx {
        SegCtx {
            selection: self.transcript_selection.clone(),
            focus: self.transcript_focus.clone(),
            list: self
                .transcripts
                .get(session_id)
                .map(|view| view.list.clone()),
            session: SharedString::from(session_id.to_string()),
            block,
            response,
        }
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
                        Item::Activity(_) => None,
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
        let Some(session) = self
            .transcript_selection
            .read(cx)
            .session()
            .map(str::to_string)
        else {
            return;
        };
        let shown = self.shown_block_texts(&session);
        match self.transcript_selection.read(cx).selected_text(&shown) {
            Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            None => log::debug!("transcript copy with nothing selected"),
        }
    }

    /// ⌘A: the whole thread the transcript focus is in.
    pub fn select_all_transcript(&mut self, cx: &mut Context<Self>) {
        let session = self
            .transcript_selection
            .read(cx)
            .session()
            .map(str::to_string)
            .or_else(|| self.selected_session_id.clone());
        let Some(session) = session else {
            return;
        };
        self.transcript_selection.update(cx, |selection, cx| {
            selection.select_all(&SharedString::from(session));
            cx.notify();
        });
    }

    /// Esc: drops the selection; `false` when there was none.
    pub fn clear_transcript_selection(&mut self, cx: &mut Context<Self>) -> bool {
        self.transcript_selection.update(cx, |selection, cx| {
            let cleared = selection.clear();
            if cleared {
                cx.notify();
            }
            cleared
        })
    }

    /// MonoCode `saveSelectionNote`: a note titled from the text, linked to
    /// the thread, left closed.
    fn save_selection_note(&mut self, text: &str, session_id: &str) -> anyhow::Result<()> {
        let cwd = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.cwd.clone())
            .filter(|cwd| !cwd.is_empty());
        let body = text.replace("\r\n", "\n").replace('\r', "\n");
        let upsert = crate::db::NoteUpsert {
            id: format!("note-{}", crate::app::now_ms()),
            title: note_title(&body),
            body,
            tags: Vec::new(),
            source_session_id: Some(session_id.to_string()),
            source_cwd: cwd,
        };
        let note = self.db.upsert_note(&upsert)?;
        self.notes.insert(0, note);
        Ok(())
    }

    /// Gives the pane of `session_id` the transcript's focus and keys, and
    /// keeps a drag selecting past the list's edges by scrolling it.
    pub(crate) fn with_transcript_selection(
        &self,
        body: Stateful<Div>,
        session_id: &str,
        focused: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let selection = self.transcript_selection.read(cx);
        // The pane the selection was made in owns the focus; with none, the
        // focused pane does.
        let owns = selection.session().map_or(focused, |s| s == session_id);
        let list = self
            .transcripts
            .get(session_id)
            .map(|view| view.list.clone());
        let entity = self.transcript_selection.clone();
        let sid = session_id.to_string();
        let scroll = list.map(|list| {
            let app = cx.entity().downgrade();
            canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    let (entity, list, app, sid) =
                        (entity.clone(), list.clone(), app.clone(), sid.clone());
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase != gpui::DispatchPhase::Bubble
                            || !entity.read(cx).is_dragging_in(&sid)
                        {
                            return;
                        }
                        let view = list.viewport_bounds();
                        let y = event.position.y;
                        let past = if y < view.top() {
                            y - view.top()
                        } else if y > view.bottom() {
                            y - view.bottom()
                        } else {
                            return;
                        };
                        list.scroll_by(past / 2.0);
                        if let Err(err) = app.update(cx, |_, cx| cx.notify()) {
                            log::debug!("selection scroll after app drop: {err:#}");
                        }
                    });
                },
            )
            .absolute()
            .size_full()
        });
        let session = session_id.to_string();
        body.when(owns, |el| {
            el.track_focus(&self.transcript_focus)
                .key_context("Transcript")
        })
        // MonoCode closes the menu on any scroll; the selection stays.
        .on_scroll_wheel(cx.listener(move |this, _, _, cx| {
            this.transcript_selection.update(cx, |selection, cx| {
                if selection.session() == Some(session.as_str()) && selection.dismiss_menu() {
                    cx.notify();
                }
            });
        }))
        .children(scroll)
    }

    /// MonoCode `TranscriptSelectionMenu`, over the selection's first line.
    pub(crate) fn render_selection_menu(
        &self,
        session_id: &str,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let selection = self.transcript_selection.read(cx);
        let (text, line) = selection.menu(session_id)?;
        let error = selection.note_error().map(str::to_string);
        let painted = selection.menu_bounds();
        let fg = cx.theme().colors.fg;
        let action = |id: &'static str, icon: ExtraIcon, label: &'static str| {
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
                .text_size(px(13.0))
                .line_height(px(13.0))
                .text_color(fg)
                .hover(move |s| s.bg(fg.opacity(0.05)))
                .child(icon.icon().size(IconSize::Sm).color(fg))
                .child(label)
        };
        let (chat_text, note_text, sid) = (text.clone(), text, session_id.to_string());
        let menu = popover_frame(cx)
            .id("transcript-selection-menu")
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
                    ExtraIcon::CommentAdd,
                    "Add to chat",
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.add_quote_to_chat(&chat_text, cx);
                    this.clear_transcript_selection(cx);
                })),
            )
            .child(
                action(
                    "selection-add-to-notes",
                    ExtraIcon::FilePlusCorner,
                    "Add to notes",
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    match this.save_selection_note(&note_text, &sid) {
                        Ok(()) => {
                            this.clear_transcript_selection(cx);
                        }
                        Err(err) => {
                            log::error!("failed to save selection as note: {err:#}");
                            this.transcript_selection.update(cx, |selection, cx| {
                                selection.set_note_error(Some(format!("{err:#}")));
                                cx.notify();
                            });
                        }
                    }
                })),
            )
            .children(error.map(|error| {
                div()
                    .max_w(px(320.0))
                    .px(px(10.0))
                    .py_1()
                    .text_xs()
                    .text_color(fg.opacity(0.7))
                    .child(format!("Could not save note. {error}"))
            }))
            .child(
                canvas(
                    move |bounds, _, _| painted.set(Some(bounds)),
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
