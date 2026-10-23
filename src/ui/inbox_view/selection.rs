//! Selecting an item's text in the Inbox (MonoCode's detail is a web page,
//! `user-select: text`): a drag across the description and comments, a
//! double or triple click, ⌘C, ⌘A, Esc. It is the transcript's selection
//! (`transcript::selection`, wired through `ui::selection_area`) with the
//! item as its place: each description block, comment and reply is one
//! block, in reading order.

use std::rc::Rc;

use gpui::{Context, Div, SharedString};

use crate::app::BenCodeApp;
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
        SegCtx {
            selection: self.inbox_selection.clone(),
            focus: self.inbox_text_focus.clone(),
            list: Some(self.inbox(cx).detail_list.clone()),
            session: place(key),
            block,
            response: false,
        }
    }

    /// Gives item `key`'s detail the keys of its selection while it holds
    /// one; a press beside the text starts at the nearest character, and a
    /// drag past the list's edges scrolls it.
    pub(super) fn with_inbox_selection(&self, body: Div, key: &str, cx: &Context<Self>) -> Div {
        let app = cx.entity().downgrade();
        let owned = key.to_string();
        SelectionArea {
            selection: self.inbox_selection.clone(),
            focus: self.inbox_text_focus.clone(),
            key_context: "InboxText",
            list: Some(self.inbox(cx).detail_list.clone()),
            owns: Rc::new(move |place| parse_place(place) == Some(owned.as_str())),
            beside: Some(place(key)),
            idle_owner: false,
            redraw: Rc::new(move |cx| {
                let redrawn = app.update(cx, |this, cx| {
                    this.redraw_regions(&[crate::ui::region::Region::Surface], cx)
                });
                if let Err(err) = redrawn {
                    log::debug!("inbox selection scroll after app drop: {err:#}");
                }
            }),
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
                (described.blocks.iter().enumerate())
                    .map(|(ix, block)| (ix, block.trim().to_string())),
            );
        }
        if let Some(Ok(thread)) = inbox.threads.get(key) {
            for (ix, comment) in thread.comments.iter().enumerate() {
                texts.push((comment_block(ix, None), comment.body.trim().to_string()));
                texts.extend(
                    (comment.replies.iter().enumerate()).map(|(r, reply)| {
                        (comment_block(ix, Some(r)), reply.body.trim().to_string())
                    }),
                );
            }
        }
        texts
    }

    /// ⌘C: the selected text as shown.
    pub fn copy_inbox_selection(&mut self, cx: &mut Context<Self>) {
        let Some(key) = (self.inbox_selection.read(cx).session())
            .and_then(parse_place)
            .map(str::to_string)
        else {
            return;
        };
        let shown = self.inbox_block_texts(&key, cx);
        selection_area::copy(&self.inbox_selection, &shown, cx);
    }

    /// ⌘A: the whole item shown.
    pub fn select_all_inbox(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.inbox(cx).selected.clone() {
            selection_area::select_all(&self.inbox_selection, &place(&key), cx);
        }
    }

    /// Esc: drops the selection; `false` when there was none.
    pub fn clear_inbox_selection(&mut self, cx: &mut Context<Self>) -> bool {
        selection_area::clear(&self.inbox_selection, cx)
    }

    /// Item `key`'s description or comments loaded again: what was selected
    /// by block no longer names the same text.
    pub(super) fn forget_inbox_selection(&mut self, key: &str, cx: &mut Context<Self>) {
        let gone = |place: &str| parse_place(place) == Some(key);
        selection_area::forget(&self.inbox_selection, gone, cx);
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
