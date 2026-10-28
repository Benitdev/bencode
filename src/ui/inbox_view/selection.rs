//! Selecting an item's text in the Inbox (MonoCode's detail is a web page,
//! `user-select: text`): a drag across the description and comments, a
//! double or triple click, ⌘C, ⌘A, Esc. It is the window's selection
//! (`bencode_ui::selection`, wired through `ui::selection_area`) with the
//! item as its document: each description block, comment and reply is one
//! block, in reading order.

use std::rc::Rc;

use bencode_ui::selection::{Document, TextSelection};
use gpui::{Context, Div, SharedString};

use crate::app::BenCodeApp;
use crate::ui::region::Region;
use crate::ui::selection_area::{self, SelectionArea};
use crate::ui::transcript::selection::SegCtx;

const PREFIX: &str = "inbox:";
/// Comments come after every description block; a comment's replies
/// follow it (`comment_block`).
const COMMENTS: usize = 1 << 24;
const REPLIES: usize = 1 << 10;

/// Where item `key` registers its text.
fn place(key: &str) -> SharedString {
    SharedString::from(format!("{PREFIX}{key}"))
}

/// The item a place names.
fn parse_place(place: &str) -> Option<&str> {
    place.strip_prefix(PREFIX)
}

/// Comment `comment`'s block, or that of its reply `reply`.
pub(super) fn comment_block(comment: usize, reply: Option<usize>) -> usize {
    COMMENTS + comment * REPLIES + reply.map_or(0, |r| r + 1)
}

impl BenCodeApp {
    /// Where block `block` of item `key` draws its text.
    pub(super) fn inbox_seg_ctx(&self, key: &str, block: usize, cx: &Context<Self>) -> SegCtx {
        let app = cx.weak_entity();
        let doc = Document::new(place(key), &self.text_selection)
            .list(self.inbox(cx).detail_list.clone())
            .focus(self.inbox_text_focus.clone())
            .on_change(move |cx| {
                let redrawn =
                    app.update(cx, |this, cx| this.redraw_regions(&[Region::Surface], cx));
                if let Err(err) = redrawn {
                    log::debug!("inbox selection after app drop: {err:#}");
                }
            });
        SegCtx {
            doc,
            block,
            quotable: false,
        }
    }

    /// Gives item `key`'s detail the keys of its selection while it holds
    /// one; a press beside the text starts at the nearest character.
    pub(super) fn with_inbox_selection(&self, body: Div, key: &str, cx: &Context<Self>) -> Div {
        let owned = key.to_string();
        SelectionArea {
            selection: self.text_selection.clone(),
            focus: self.inbox_text_focus.clone(),
            key_context: "InboxText",
            owns: Rc::new(move |doc| parse_place(doc) == Some(owned.as_str())),
            beside: Some(place(key)),
            idle_owner: false,
        }
        .wrap(body, cx)
    }

    /// The source of each block of item `key`, for a copy that reaches
    /// blocks the list has not drawn.
    fn inbox_block_texts(&self, key: &str, cx: &Context<Self>) -> Vec<(usize, String)> {
        let inbox = self.inbox(cx);
        let mut texts = Vec::new();
        if let Some(Ok(described)) = inbox.details.get(key) {
            texts.extend(
                (inbox.description_blocks(key, described).iter().enumerate())
                    .map(|(ix, block)| (ix, block.trim().to_string())),
            );
        }
        if let Some(Ok(thread)) = inbox.threads.get(key) {
            // As shown: a translated comment copies its translation.
            let shown = |comment: &crate::github::Comment| {
                (inbox.translations.shown_comment(key, &comment.id))
                    .map_or(comment.body.as_str(), |t| t.as_ref())
                    .trim()
                    .to_string()
            };
            for (ix, comment) in thread.comments.iter().enumerate() {
                texts.push((comment_block(ix, None), shown(comment)));
                texts.extend(
                    (comment.replies.iter().enumerate())
                        .map(|(r, reply)| (comment_block(ix, Some(r)), shown(reply))),
                );
            }
        }
        texts
    }

    /// ⌘C: the selected text as shown.
    pub fn copy_inbox_selection(&mut self, cx: &mut Context<Self>) {
        let doc = self.text_selection.read(cx).doc();
        let Some(key) = doc.as_deref().and_then(parse_place).map(str::to_string) else {
            return;
        };
        let shown = self.inbox_block_texts(&key, cx);
        selection_area::copy(&self.text_selection, &shown, cx);
    }

    /// ⌘A: the whole item shown.
    pub fn select_all_inbox(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.inbox(cx).selected.clone() {
            TextSelection::select_all(&self.text_selection, &place(&key), cx);
        }
    }

    /// Esc: drops the selection; `false` when there was none.
    pub fn clear_inbox_selection(&mut self, cx: &mut Context<Self>) -> bool {
        TextSelection::clear(&self.text_selection, cx)
    }

    /// Item `key`'s description or comments loaded again: what was selected
    /// by block no longer names the same text.
    pub(super) fn forget_inbox_selection(&mut self, key: &str, cx: &mut Context<Self>) {
        let gone = |doc: &str| parse_place(doc) == Some(key);
        TextSelection::forget(&self.text_selection, gone, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_name_their_item() {
        assert_eq!(
            parse_place(&place("backlog:DOC-1102")),
            Some("backlog:DOC-1102")
        );
        assert_eq!(parse_place("diff:x:old"), None);
    }

    #[test]
    fn comments_read_after_the_description_and_replies_after_their_comment() {
        assert!(comment_block(0, None) > 100_000);
        assert!(comment_block(0, None) < comment_block(0, Some(0)));
        assert!(comment_block(0, Some(REPLIES - 2)) < comment_block(1, None));
    }
}
