//! MonoCode "Edit and resend" (`editLastTurn.ts`): the last message comes
//! back into the composer (dashed accent border, "Cancel edit"); sending
//! rewinds the provider's thread to before that turn, drops it from the
//! transcript and sends the edited text in its place.
//!
//! Only providers that can rewind their own history take part. MonoCode
//! offers it for Codex, OpenCode, Pi and OMP; of BenCode's harnesses that is
//! Codex (`thread/revert`). Claude cannot rewind, so its threads never offer
//! it.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled, div,
    prelude::*,
};

use crate::app::{BenCodeApp, TurnInput};
use crate::db::{Block, SessionRow};
use crate::harness::{Attachment, HarnessKind, codex};
use crate::ui::scale::px;

/// MonoCode `isLastUserTurnBlock`: a user turn the person wrote (not an
/// internal one) that was sent (not a draft).
fn is_sent_user_turn(block: &Block) -> bool {
    let flag = |key: &str| block.extra.get(key).and_then(|v| v.as_bool()) == Some(true);
    block.role == "user" && !flag("internal") && !flag("draft")
}

/// MonoCode `lastUserTurnBlock`: where the last sent user turn starts.
pub fn last_user_turn(blocks: &[Block]) -> Option<usize> {
    blocks.iter().rposition(is_sent_user_turn)
}

/// MonoCode `canEditLastTurn` for an idle thread (no run, queue or open
/// question): the harness can rewind, the provider thread exists, and the
/// last turn is a plain message (not a second opinion, note or CI repair,
/// and not in a handed-off thread).
pub fn can_edit_last_turn(session: &SessionRow, idle: bool) -> bool {
    let rewinds = session.harness == HarnessKind::Codex.id();
    let Some(ix) = last_user_turn(&session.blocks) else {
        return false;
    };
    let block = &session.blocks[ix];
    let special = block.second_opinion.is_some()
        || ["noteCard", "ciContext"]
            .iter()
            .any(|key| block.extra.get(*key).is_some_and(|v| !v.is_null()));
    idle && rewinds
        && session.provider_session_id.is_some()
        && !special
        && !session.blocks.iter().any(|b| b.role == "handoff")
}

/// MonoCode `lastTurnRecall`: the text and files the composer gets back.
pub fn last_turn_recall(blocks: &[Block]) -> Option<(String, Vec<Attachment>)> {
    let block = &blocks[last_user_turn(blocks)?];
    let files = block
        .extra
        .get("attachments")
        .and_then(|v| v.as_array())
        .map(|files| {
            files
                .iter()
                .filter_map(Attachment::from_block_json)
                .collect()
        })
        .unwrap_or_default();
    Some((block.text.clone().unwrap_or_default(), files))
}

impl BenCodeApp {
    /// An idle thread: nothing running, queued or waiting on an answer.
    fn is_settled(&self, session_id: &str) -> bool {
        !self.is_agent_running_in(session_id)
            && !self.edit_rewinding.contains(session_id)
            && self
                .prompt_queues
                .get(session_id)
                .is_none_or(|queue| queue.is_empty())
            && self.pending_question(session_id).is_none()
    }

    /// Whether `session_id`'s last message can be edited and resent now.
    pub fn can_edit_last_turn(&self, session_id: &str) -> bool {
        self.sessions
            .iter()
            .find(|s| s.id == session_id)
            .is_some_and(|s| can_edit_last_turn(s, self.is_settled(session_id)))
    }

    /// The composer is editing the focused thread's last message.
    pub fn is_editing_last_turn(&self) -> bool {
        self.editing_last_turn.is_some() && self.editing_last_turn == self.selected_session_id
    }

    /// MonoCode `recallLastTurn`: the pencil and ↑ bring the last message
    /// back to edit, or (when already editing) leave edit mode.
    pub fn toggle_edit_last_turn(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if self.editing_last_turn.as_deref() == Some(session_id) {
            self.leave_edit_last_turn(cx);
            return;
        }
        if !self.can_edit_last_turn(session_id) {
            return;
        }
        let Some((text, files)) = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .and_then(|s| last_turn_recall(&s.blocks))
        else {
            return;
        };
        self.editing_last_turn = Some(session_id.to_string());
        self.composer_error = None;
        self.composer_attachments
            .insert(session_id.to_string(), files);
        self.prompt_input
            .update(cx, |input, cx| input.set_text(text, cx));
        self.refocus_prompt(cx);
        cx.notify();
    }

    /// MonoCode `exitEditMode`: the recalled text and files leave the
    /// composer.
    pub fn leave_edit_last_turn(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.editing_last_turn.take() else {
            return;
        };
        self.composer_attachments.remove(&session_id);
        *self.mcp_tags.borrow_mut() = Default::default();
        self.prompt_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.refocus_prompt(cx);
        cx.notify();
    }

    /// MonoCode's "Cancel edit" pill beside Send.
    pub(super) fn render_cancel_edit(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.is_editing_last_turn() {
            return None;
        }
        let colors = &cx.theme().colors;
        let (accent, fg) = (colors.accent, colors.fg);
        Some(
            div()
                .id("cancel-edit")
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .h(px(26.0))
                .px_2()
                .rounded(px(6.0))
                .border_1()
                .border_color(accent.opacity(0.2))
                .bg(accent.opacity(0.1))
                .text_color(accent)
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .hover(move |s| {
                    s.border_color(accent.opacity(0.35))
                        .bg(fg.opacity(0.15))
                        .text_color(fg)
                })
                .tooltip(Tooltip::text("Stop editing last message"))
                .on_click(cx.listener(|this, _, _, cx| this.leave_edit_last_turn(cx)))
                .child(Icon::new(IconName::X).size(IconSize::Xs).color(accent))
                .child("Cancel edit")
                .into_any_element(),
        )
    }

    /// Sends `input` in place of the last turn: the provider rewinds first
    /// (off the UI thread); only then does the transcript drop the old turn
    /// and the new one start. If the provider refuses, the edit comes back
    /// to the composer with the reason.
    pub fn resend_edited(
        &mut self,
        session_id: &str,
        input: TurnInput,
        typed: String,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        let Some(thread) = session.provider_session_id.clone() else {
            return;
        };
        let cwd = std::path::PathBuf::from(session.work_dir());
        let account = crate::harness::accounts::AccountProfile::resolve(
            &session.harness,
            session.provider_account_id.as_deref(),
        );
        let session_id = session_id.to_string();
        self.edit_rewinding.insert(session_id.clone());
        let rewind = cx
            .background_executor()
            .spawn(async move { codex::rewind_last_turn(&thread, &cwd, account.as_ref()) });
        cx.spawn(async move |this, cx| {
            let result = rewind.await;
            let _ = this.update(cx, |app, cx| {
                app.edit_rewinding.remove(&session_id);
                match result {
                    Ok(()) => app.replace_last_turn(&session_id, input, cx),
                    Err(err) => {
                        log::warn!("edit last turn: {err:#}");
                        app.restore_refused_edit(&session_id, input, typed, &err, cx);
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn replace_last_turn(&mut self, session_id: &str, input: TurnInput, cx: &mut Context<Self>) {
        if self.editing_last_turn.as_deref() == Some(session_id) {
            self.editing_last_turn = None;
        }
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id)
            && let Some(ix) = last_user_turn(&session.blocks)
        {
            session.blocks.truncate(ix);
        }
        self.send_turn(session_id, input, cx);
    }

    /// The provider kept its history: the edit goes back in the composer
    /// (unless something new was typed there meanwhile).
    fn restore_refused_edit(
        &mut self,
        session_id: &str,
        input: TurnInput,
        typed: String,
        err: &anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let focused = self.selected_session_id.as_deref() == Some(session_id);
        if focused && self.prompt_input.read(cx).text().is_empty() {
            self.prompt_input
                .update(cx, |input, cx| input.set_text(typed, cx));
            self.composer_attachments
                .insert(session_id.to_string(), input.attachments);
        }
        self.composer_error = Some(format!("Could not edit the last message: {err:#}"));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn block(role: &str, extra: serde_json::Value) -> Block {
        let mut block: Block =
            serde_json::from_value(json!({ "id": "b", "role": role, "text": "hi" })).unwrap();
        if let serde_json::Value::Object(map) = extra {
            block.extra.extend(map);
        }
        block
    }

    fn session(harness: &str, blocks: Vec<Block>) -> SessionRow {
        SessionRow {
            harness: harness.into(),
            provider_session_id: Some("th_1".into()),
            blocks,
            ..Default::default()
        }
    }

    #[test]
    fn the_last_sent_message_is_the_one_edited() {
        let blocks = vec![
            block("user", json!({})),
            block("assistant", json!({})),
            block("user", json!({ "internal": true })),
            block("user", json!({ "draft": true })),
        ];
        assert_eq!(last_user_turn(&blocks), Some(0));
        assert_eq!(last_user_turn(&blocks[1..]), None);
    }

    #[test]
    fn only_idle_rewindable_threads_offer_it() {
        let plain = || vec![block("user", json!({})), block("assistant", json!({}))];
        assert!(can_edit_last_turn(&session("codex", plain()), true));
        assert!(!can_edit_last_turn(&session("codex", plain()), false));
        assert!(!can_edit_last_turn(&session("claude", plain()), true));
        let mut fresh = session("codex", plain());
        fresh.provider_session_id = None;
        assert!(!can_edit_last_turn(&fresh, true));
        let ci = vec![block("user", json!({ "ciContext": { "run": 1 } }))];
        assert!(!can_edit_last_turn(&session("codex", ci), true));
        let mut handed = plain();
        handed.push(block("handoff", json!({})));
        assert!(!can_edit_last_turn(&session("codex", handed), true));
    }

    #[test]
    fn recall_brings_back_text_and_files() {
        let files = json!({ "attachments": [
            { "id": "a1", "name": "a.png", "path": "/tmp/a.png", "mimeType": "image/png", "size": 3 }
        ]});
        let blocks = vec![block("user", files)];
        let (text, attached) = last_turn_recall(&blocks).unwrap();
        assert_eq!(text, "hi");
        assert_eq!(attached.len(), 1);
    }
}
