//! Running an agent turn: spawning the harness, folding its events into the
//! session transcript, permission prompts and stop/cancel.
//!
//! `apply_event` is a pure reducer over `SessionRow` so transcript behaviour
//! is unit-tested without GPUI; `BenCodeApp` methods are thin glue around it.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{Context, Focusable};
use serde_json::{Value, json};

use crate::app::session_review::edit_paths;
use crate::app::thread_state::ThreadState;
use crate::app::{BenCodeApp, PermissionMode};
use crate::db::{Block, RunStatus, SessionRow, TurnModel};
use crate::harness::Attachment;
use crate::harness::accounts::AccountProfile;
use crate::harness::events::TurnMetrics;
use crate::harness::{
    self, AgentEvent, DoneStatus, HarnessKind, HarnessProcessHandle, PermissionPolicy,
    PermissionRequest, SpawnRequest, catalog, summarize_tool_input,
};
use crate::ui::composer::cards::ComposerCard;
use crate::ui::composer::mode_commands::{self, ModeCommand};

const TITLE_PREVIEW_CHARS: usize = 48;
const MAX_TOOL_OUTPUT_CHARS: usize = 4_000;
const STOPPED_NOTICE: &str = "Agent execution stopped by user.";
/// MonoCode `INTERRUPT_MESSAGE` (inFlight.ts), with BenCode's name.
const INTERRUPT_NOTICE: &str = "Turn interrupted when BenCode quit.";
/// A streaming turn is saved at most this often, so a crash or quit loses
/// seconds of output rather than the whole turn.
const STREAM_SAVE_INTERVAL: Duration = Duration::from_secs(3);
/// A finished tool is worth saving sooner, but not on every call of a fast loop.
const TOOL_SAVE_INTERVAL: Duration = Duration::from_millis(500);
pub const NEW_SESSION_TITLE: &str = "New AI Thread";

/// One in-flight agent turn; each thread runs its own. `id` guards against
/// late events from a run that was already stopped or superseded.
/// What one turn sends: the text, attached files, and MonoCode Plan mode.
#[derive(Clone, Debug, Default)]
pub struct TurnInput {
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub plan: bool,
    /// A composer card (note, handoff) the turn is sent with.
    pub card: Option<Box<ComposerCard>>,
    /// What the agent reads instead of `text`, which the thread shows
    /// (MonoCode `ciRepair`: a one-line ask over the CI evidence).
    pub agent_prompt: Option<String>,
}

/// Claude's clarifying-question tool, answered in the composer.
pub const QUESTION_TOOL: &str = "AskUserQuestion";

pub struct AgentRun {
    pub id: u64,
    pub handle: HarnessProcessHandle,
    pub pending_permission: Option<PermissionRequest>,
    /// How the turn ended, once the harness reports `Done`.
    pub outcome: Option<DoneStatus>,
    /// Automation run row to close when this turn ends.
    pub automation_run_id: Option<String>,
    /// Full access: every tool is allowed at once, questions still ask.
    pub auto_approve: bool,
    /// Whether the harness takes a follow-up in the middle of this turn.
    pub can_steer: bool,
    pub purpose: RunPurpose,
    /// A compaction the harness confirmed, with the context left after it.
    pub compacted: Option<Option<u64>>,
    /// The files of each edit tool still running, by call id.
    pub edit_paths: HashMap<String, Vec<String>>,
    /// When this run last saved its thread (see `save_due`).
    pub last_saved: Instant,
}

/// What a run is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunPurpose {
    /// A user turn.
    #[default]
    Turn,
    /// `/compact`: shown as status lines, not as a turn.
    Compact,
}

/// What `start_run` sends.
struct RunRequest {
    prompt: String,
    attachments: Vec<Attachment>,
    plan: bool,
    purpose: RunPurpose,
}

/// The command a harness compacts its context with, and MonoCode's
/// status lines around it.
const COMPACT_COMMAND: &str = "/compact";
const COMPACTING_NOTICE: &str = "Compacting context…";
const COMPACTED_NOTICE: &str = "Compacted context";

/// MonoCode's "Claude Code did not confirm context compaction", for
/// whichever harness the thread runs.
fn unconfirmed_compact(harness: &str) -> String {
    let label = harness::HarnessKind::from_id(harness).map_or(harness, |kind| kind.label());
    format!("{label} did not confirm context compaction")
}

/// MonoCode `canCompactHarnessContext` for the harnesses BenCode drives.
pub fn can_compact(harness: &str) -> bool {
    matches!(
        harness::HarnessKind::from_id(harness),
        Some(harness::HarnessKind::Claude | harness::HarnessKind::Grok)
    )
}

/// MonoCode's terminal automation-run status for a turn outcome.
fn automation_status(outcome: Option<DoneStatus>) -> RunStatus {
    match outcome {
        Some(DoneStatus::Completed) => RunStatus::Succeeded,
        Some(DoneStatus::Cancelled) => RunStatus::Cancelled,
        Some(DoneStatus::Failed) | None => RunStatus::Failed,
    }
}

/// What `on_agent_event` does with one event of a live run, decided without
/// GPUI so it can be tested.
#[derive(Debug, PartialEq)]
enum EventStep {
    /// The provider refused the turn on its usage limit.
    UsageLimited(Option<i64>),
    /// The harness confirmed a compaction.
    Compacted(Option<u64>),
    /// Full access: allow at once.
    AutoApprove(PermissionRequest),
    /// The user answers; a question for the focused thread takes the keys.
    Ask {
        request: PermissionRequest,
        focus_question: bool,
    },
    /// Fold into the transcript, after the checkpoint call it needs.
    Apply {
        event: AgentEvent,
        /// The edited files, and whether the tool completed (capture) or
        /// is starting (prepare). Never empty.
        checkpoint: Option<(Vec<String>, bool)>,
        done: Option<DoneStatus>,
    },
}

/// MonoCode `trackSessionEdits` and the permission rules, for one event.
/// `running_edits` is the run's map of edit tools still running.
fn plan_event(
    auto_approve: bool,
    running_edits: &mut HashMap<String, Vec<String>>,
    focused: bool,
    event: AgentEvent,
) -> EventStep {
    let event = match event {
        AgentEvent::UsageLimited { resets_at } => return EventStep::UsageLimited(resets_at),
        AgentEvent::Compacted { tokens_after } => return EventStep::Compacted(tokens_after),
        AgentEvent::PermissionRequest(request) => {
            let question = request.tool == QUESTION_TOOL;
            // MonoCode full access answers everything but a question.
            if auto_approve && !question {
                return EventStep::AutoApprove(request);
            }
            return EventStep::Ask {
                request,
                focus_question: question && focused,
            };
        }
        event => event,
    };
    // MonoCode `trackSessionEdits`: snapshot a file when its edit tool
    // starts, and again once it completed.
    let edit = match &event {
        AgentEvent::ToolCallStart { id, name, input } if tool_kind(name) == "edit" => {
            let paths = edit_paths(input);
            running_edits.insert(id.clone(), paths.clone());
            Some((paths, false))
        }
        AgentEvent::ToolCallFinish { id, success, .. } => running_edits
            .remove(id)
            .filter(|_| *success)
            .map(|paths| (paths, true)),
        _ => None,
    };
    let done = match &event {
        AgentEvent::Done(status) => Some(*status),
        _ => None,
    };
    EventStep::Apply {
        event,
        checkpoint: edit.filter(|(paths, _)| !paths.is_empty()),
        done,
    }
}

/// Whether this event should save the running thread now, given how long
/// ago the run last saved.
fn save_due(event: &AgentEvent, since_last_save: Duration) -> bool {
    match event {
        // The resume id and the turn's end must never wait.
        AgentEvent::SessionStarted { .. } | AgentEvent::Done(_) => true,
        AgentEvent::ToolCallFinish { .. } => since_last_save >= TOOL_SAVE_INTERVAL,
        _ => since_last_save >= STREAM_SAVE_INTERVAL,
    }
}

/// What a thread's queue does when its turn ends (MonoCode `canDispatchQueuedHead`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueueDispatch {
    /// Paused or usage-limited: wait for Resume.
    Wait,
    /// The head is being edited: send once the edit ends.
    Hold,
    Send,
}

fn queue_dispatch(paused: bool, usage_limited: bool, editing_head: bool) -> QueueDispatch {
    if paused || usage_limited {
        QueueDispatch::Wait
    } else if editing_head {
        // MonoCode holds the queue while its head is being edited.
        QueueDispatch::Hold
    } else {
        QueueDispatch::Send
    }
}

/// Takes the head of `session_id`'s queue; an edit further down moves up
/// with it.
fn pop_queue_head(
    queue: &mut Vec<TurnInput>,
    editing: &mut Option<(String, usize)>,
    session_id: &str,
) -> Option<TurnInput> {
    if queue.is_empty() {
        return None;
    }
    let next = queue.remove(0);
    if let Some((sid, ix)) = editing
        && sid == session_id
    {
        *ix = ix.saturating_sub(1);
    }
    Some(next)
}

/// Takes the item at `ix`; an emptied queue is unpaused.
fn take_queued(thread: &mut ThreadState, ix: usize) -> Option<TurnInput> {
    if ix >= thread.queue.len() {
        return None;
    }
    let input = thread.queue.remove(ix);
    if thread.queue.is_empty() {
        thread.queue_paused = false;
    }
    Some(input)
}

/// How a `/compact` run ended.
#[derive(Debug, PartialEq, Eq)]
enum CompactEnd {
    /// Confirmed, with the context left when the harness reported it.
    Confirmed(Option<i64>),
    Unconfirmed,
    /// Stopped by the user: no notice.
    Cancelled,
}

fn compact_end(compacted: Option<Option<u64>>, outcome: Option<DoneStatus>) -> CompactEnd {
    match compacted {
        Some(tokens) => CompactEnd::Confirmed(tokens.and_then(|t| i64::try_from(t).ok())),
        None if outcome == Some(DoneStatus::Cancelled) => CompactEnd::Cancelled,
        None => CompactEnd::Unconfirmed,
    }
}

impl PermissionMode {
    pub fn policy(self) -> PermissionPolicy {
        match self {
            Self::Supervised => PermissionPolicy::Ask,
            Self::AutoAcceptEdits => PermissionPolicy::AcceptEdits,
            Self::Auto => PermissionPolicy::Auto,
            Self::FullAccess => PermissionPolicy::AutoApprove,
        }
    }
}

impl BenCodeApp {
    /// Whether any thread has an agent running.
    pub fn is_agent_running(&self) -> bool {
        !self.runs.is_empty()
    }

    /// Whether the agent is running in this particular thread.
    pub fn is_agent_running_in(&self, session_id: &str) -> bool {
        self.runs.contains_key(session_id)
    }

    /// The permission prompt owned by this thread's run, if any. Scoped so an
    /// Approve click in one tab can never answer another thread's agent.
    pub fn pending_permission_for(&self, session_id: &str) -> Option<&PermissionRequest> {
        self.runs.get(session_id)?.pending_permission.as_ref()
    }

    /// Prompts waiting for this thread's current turn to end.
    pub fn queued_prompts(&self, session_id: &str) -> &[TurnInput] {
        self.thread(session_id)
            .map_or(&[], |thread| thread.queue.as_slice())
    }

    /// MonoCode's paused queue: the agent was stopped with messages waiting.
    pub fn queue_paused(&self, session_id: &str) -> bool {
        self.thread(session_id)
            .is_some_and(|thread| thread.queue_paused && !thread.queue.is_empty())
    }

    /// MonoCode `onResumeQueue`: the agent continues where it was stopped,
    /// then the queue drains after that turn.
    pub fn resume_queue(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let was_paused = self
            .threads
            .get_mut(session_id)
            .is_some_and(|thread| std::mem::take(&mut thread.queue_paused));
        if self.is_agent_running_in(session_id) || !was_paused {
            return;
        }
        self.send_prompt(
            session_id,
            crate::ui::composer::usage_limit::CONTINUE_PROMPT,
            cx,
        );
        cx.notify();
    }

    /// Edit on a queued message: its text goes into the inline field.
    pub fn start_queue_edit(&mut self, session_id: &str, ix: usize, cx: &mut Context<Self>) {
        let Some(item) = self.queued_prompts(session_id).get(ix) else {
            return;
        };
        let text = item.text.clone();
        self.queue_editing = Some((session_id.to_string(), ix));
        self.queue_edit_input.update(cx, |input, cx| {
            input.set_text(text.clone(), cx);
            input.select(text.len()..text.len(), cx);
        });
        crate::ui::composer::focus_later(self.queue_edit_input.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    /// Enter / ✓: keeps the edit (an empty message without files is not kept).
    pub fn save_queue_edit(&mut self, cx: &mut Context<Self>) {
        let Some((session_id, ix)) = self.queue_editing.clone() else {
            return;
        };
        let text = self.queue_edit_input.read(cx).text().trim().to_string();
        if let Some(item) = self
            .threads
            .get_mut(&session_id)
            .and_then(|thread| thread.queue.get_mut(ix))
        {
            if text.is_empty() && item.attachments.is_empty() {
                return;
            }
            item.text = text;
        }
        self.end_queue_edit(&session_id, cx);
    }

    /// Esc / ✕: leaves the message as it was.
    pub fn cancel_queue_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((session_id, _)) = self.queue_editing.clone() else {
            return false;
        };
        self.end_queue_edit(&session_id, cx);
        true
    }

    fn end_queue_edit(&mut self, session_id: &str, cx: &mut Context<Self>) {
        self.queue_editing = None;
        self.refocus_prompt(cx);
        let held = self
            .threads
            .get_mut(session_id)
            .is_some_and(|thread| std::mem::take(&mut thread.queue_held));
        if held {
            self.send_next_queued(session_id, cx);
        }
        cx.notify();
    }

    pub fn remove_queued_prompt(&mut self, session_id: &str, ix: usize, cx: &mut Context<Self>) {
        if self
            .queue_editing
            .as_ref()
            .is_some_and(|(sid, edited)| sid == session_id && *edited == ix)
        {
            self.queue_editing = None;
        }
        let taken = self
            .threads
            .get_mut(session_id)
            .and_then(|thread| take_queued(thread, ix));
        if taken.is_some() {
            cx.notify();
        }
    }

    /// MonoCode `canSaveDraft`: the agent is idle, the thread holds no
    /// draft yet, and no card waits in the composer.
    pub fn can_save_draft(&self, session_id: Option<&str>) -> bool {
        let Some(id) = session_id else {
            return true;
        };
        !self.is_agent_running_in(id)
            && !self.thread(id).is_some_and(|t| t.composer_card.is_some())
            && !self.sessions.iter().any(|s| {
                s.id == id
                    && s.blocks
                        .iter()
                        .any(|b| b.extra.get("draft").and_then(Value::as_bool) == Some(true))
            })
    }

    /// MonoCode `hasValue`: text, attached files or a card.
    pub fn composer_has_value(&self, cx: &gpui::App) -> bool {
        !self.prompt_input.read(cx).text().trim().is_empty()
            || self.has_composer_card()
            || self
                .selected_session_id
                .as_ref()
                .is_some_and(|id| self.thread(id).is_some_and(|t| !t.attachments.is_empty()))
    }

    /// The composer's button: Stop when the focused thread is running and
    /// nothing is typed; otherwise Send (which queues while running).
    pub fn handle_send_or_stop(&mut self, cx: &mut Context<Self>) {
        let typed = self.composer_has_value(cx);
        match self.selected_session_id.clone() {
            Some(id) if self.is_agent_running_in(&id) && !typed => self.stop_agent(&id, cx),
            _ => self.submit_prompt(cx),
        }
    }

    /// Sends the composer text to the focused thread. While that thread's
    /// agent is busy the message is queued, as MonoCode's Queue follow-up
    /// behaviour does.
    pub fn submit_prompt(&mut self, cx: &mut Context<Self>) {
        let typed = self.prompt_input.read(cx).text().trim().to_string();
        // MonoCode runs a lone `/compact` and opens the `/mcp` picker
        // instead of sending them.
        match mode_commands::standalone_command(&typed) {
            // The text stays when the thread cannot compact now (MonoCode
            // `if (!onCompactContext?.()) return`).
            Some(mode_commands::Command::Compact) => {
                if let Some(id) = self.selected_session_id.clone()
                    && !self.is_agent_running_in(&id)
                    && !self.worktree_removed(&id)
                {
                    self.prompt_input
                        .update(cx, |input, cx| input.set_text("", cx));
                    self.compact_context(&id, cx);
                }
                return;
            }
            Some(mode_commands::Command::AddToFolder) => {
                self.prompt_input
                    .update(cx, |input, cx| input.set_text("", cx));
                self.open_folder_picker(cx);
                return;
            }
            Some(mode_commands::Command::Mcp) => {
                self.prompt_input
                    .update(cx, |input, cx| input.set_text("", cx));
                self.open_mcp_picker(Some(0), cx);
                return;
            }
            _ => {}
        }
        // MonoCode `consumeSessionFolderCommand`: a leading `/add-to-folder`
        // files the thread first; the rest stays to send after.
        if self.selected_session_id.is_some()
            && let Some(rest) = typed
                .strip_prefix("/add-to-folder")
                .filter(|rest| rest.starts_with(char::is_whitespace))
        {
            let rest = rest.trim_start().to_string();
            let caret = rest.len();
            self.set_prompt(rest, caret, cx);
            self.open_folder_picker(cx);
            return;
        }
        // A leading `/plan` or `/draft` acts as its mode and is not sent;
        // `/draft` only where a draft can be saved (MonoCode `canSaveDraft`).
        let can_draft = self.can_save_draft(self.selected_session_id.as_deref());
        let (command, text) = match mode_commands::strip_leading_mode(&typed) {
            (Some(ModeCommand::Draft), _) if !can_draft => (None, typed.clone()),
            stripped => stripped,
        };
        if self.selected_session_id.is_none() {
            if text.is_empty() {
                return;
            }
            self.create_new_session(cx);
            if let Some(id) = self.selected_session_id.clone() {
                self.adopt_draft_workspace(&id);
            }
        }
        let Some(session_id) = self.selected_session_id.clone() else {
            return;
        };
        if self.defer_send_for_attachments(&session_id) {
            return;
        }
        // An edited resend or a new worktree is still being prepared, or the
        // thread's worktree is gone (MonoCode `worktreeRemoved`).
        if self
            .thread(&session_id)
            .is_some_and(|t| t.edit_rewinding || t.preparing_worktree)
            || self.worktree_removed(&session_id)
        {
            return;
        }
        let centred = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .is_some_and(|s| !s.blocks.iter().any(|b| b.role == "user"));
        let has_files = self
            .thread(&session_id)
            .is_some_and(|t| !t.attachments.is_empty());
        let has_card = self
            .thread(&session_id)
            .is_some_and(|t| t.composer_card.is_some());
        // A card sends even without a message (MonoCode "Use this note.").
        if text.is_empty() && !has_files && !has_card {
            return;
        }
        // MonoCode `composeInboxMessage`: an Inbox card becomes the start of
        // the message the thread shows and the agent reads.
        let card = self
            .threads
            .get_mut(&session_id)
            .and_then(|t| t.composer_card.take());
        let (text, card) = match card {
            Some(ComposerCard::Inbox(card)) => (
                crate::ui::composer::inbox_card::compose_inbox_message(&card, &text),
                None,
            ),
            card => (text, card),
        };
        let input = TurnInput {
            text: self.take_mcp_context(text),
            attachments: self
                .threads
                .get_mut(&session_id)
                .map(|t| std::mem::take(&mut t.attachments))
                .unwrap_or_default(),
            plan: self.thread(&session_id).is_some_and(|t| t.plan_mode)
                || command == Some(ModeCommand::Plan),
            card: card.map(Box::new),
            agent_prompt: None,
        };
        self.sync_prompt_placeholder(cx);
        self.prompt_input
            .update(cx, |input, cx| input.set_text("", cx));
        if let Some(thread) = self.threads.get_mut(&session_id) {
            thread.draft = None;
        }
        // MonoCode records the model on send too, so ⌘. offers threads that
        // never changed model.
        if let Some(model) = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.model.clone())
            .filter(|m| self.recent_models.first() != Some(m))
        {
            self.record_recent_model(&model, cx);
        }
        if centred {
            self.launch_dock_motion(&session_id);
        }
        // MonoCode clears Plan after each send; Draft stays chosen until a
        // draft is saved.
        let drafting = self.threads.get_mut(&session_id).is_some_and(|t| {
            t.plan_mode = false;
            std::mem::take(&mut t.draft_mode)
        }) && can_draft;
        if drafting || command == Some(ModeCommand::Draft) {
            // A draft keeps the card in its text (MonoCode `onSaveDraft`).
            let input = TurnInput {
                text: input
                    .card
                    .as_deref()
                    .map_or_else(|| input.text.clone(), |card| card.agent_prompt(&input.text)),
                card: None,
                ..input
            };
            self.save_draft(&session_id, input, cx);
            return;
        }
        if self.editing_last_turn.as_deref() == Some(session_id.as_str()) {
            self.resend_edited(&session_id, input, typed, cx);
            return;
        }
        if self.new_worktrees.contains_key(&session_id) && !self.is_agent_running_in(&session_id) {
            self.send_in_new_worktree(&session_id, input, typed, cx);
            return;
        }
        if self.is_agent_running_in(&session_id) {
            // MonoCode's default follow-up: steer into the running turn
            // (Plan waits for its own turn); harnesses that cannot, queue.
            if let Err(input) = self.steer(&session_id, input, cx) {
                self.thread_mut(&session_id).queue.push(input);
            }
            cx.notify();
            return;
        }
        self.send_turn(&session_id, input, cx);
    }

    /// Writes `input` into the running turn, showing it in the transcript.
    /// Gives it back when this thread's agent cannot take it mid-turn.
    pub fn steer(
        &mut self,
        session_id: &str,
        input: TurnInput,
        cx: &mut Context<Self>,
    ) -> Result<(), TurnInput> {
        let Some(run) = self.runs.get(session_id).filter(|run| run.can_steer) else {
            return Err(input);
        };
        // Plan and card turns wait for their own turn.
        if input.plan || input.card.is_some() {
            return Err(input);
        }
        let prompt = self.apply_skills(&input.text);
        if !run.handle.steer(&prompt, &input.attachments) {
            return Err(input);
        }
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            start_turn(session, &input.text, now_ms(), &input.attachments);
        }
        self.persist_session(session_id);
        cx.notify();
        Ok(())
    }

    /// The queue row's Steer: that message goes into the running turn now,
    /// or, with the agent idle, starts a turn of its own (MonoCode).
    pub fn steer_queued(&mut self, session_id: &str, ix: usize, cx: &mut Context<Self>) {
        let Some(input) = self
            .threads
            .get_mut(session_id)
            .and_then(|thread| take_queued(thread, ix))
        else {
            return;
        };
        if !self.is_agent_running_in(session_id) {
            self.send_turn(session_id, input, cx);
            cx.notify();
            return;
        }
        if let Err(input) = self.steer(session_id, input, cx) {
            self.thread_mut(session_id).queue.insert(ix, input);
        }
        cx.notify();
    }

    /// Whether this thread's running agent takes follow-ups mid-turn.
    pub fn can_steer(&self, session_id: &str) -> bool {
        self.runs.get(session_id).is_some_and(|run| run.can_steer)
    }

    /// MonoCode Draft: keeps the message in the thread without starting the
    /// agent; it can be sent or removed from the transcript later.
    fn save_draft(&mut self, session_id: &str, input: TurnInput, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else {
            return;
        };
        let now = now_ms();
        start_turn(session, &input.text, now, &input.attachments);
        if let Some(block) = session.blocks.last_mut() {
            block.extra.insert("draft".into(), json!(true));
            if input.plan {
                block.extra.insert("plan".into(), json!(true));
            }
        }
        self.persist_session(session_id);
        cx.notify();
    }

    /// Sends a saved draft: it leaves the transcript and runs as a turn.
    pub fn send_draft(&mut self, session_id: &str, block_id: &str, cx: &mut Context<Self>) {
        let Some(input) = self.take_draft(session_id, block_id) else {
            return;
        };
        if self.is_agent_running_in(session_id) {
            self.thread_mut(session_id).queue.push(input);
            cx.notify();
        } else {
            self.send_turn(session_id, input, cx);
        }
    }

    pub fn remove_draft(&mut self, session_id: &str, block_id: &str, cx: &mut Context<Self>) {
        if self.take_draft(session_id, block_id).is_some() {
            self.persist_session(session_id);
            cx.notify();
        }
    }

    fn take_draft(&mut self, session_id: &str, block_id: &str) -> Option<TurnInput> {
        let session = self.sessions.iter_mut().find(|s| s.id == session_id)?;
        let ix = session.blocks.iter().position(|b| b.id == block_id)?;
        let block = session.blocks.remove(ix);
        let attachments = block
            .extra
            .get("attachments")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(Attachment::from_block_json)
                    .collect()
            })
            .unwrap_or_default();
        Some(TurnInput {
            plan: block.extra.get("plan").and_then(Value::as_bool) == Some(true),
            text: block.text.unwrap_or_default(),
            attachments,
            card: None,
            agent_prompt: None,
        })
    }

    /// Starts a turn of plain `prompt` in `session_id` (automations, retries).
    pub fn send_prompt(&mut self, session_id: &str, prompt: &str, cx: &mut Context<Self>) {
        let input = TurnInput {
            text: prompt.to_string(),
            ..Default::default()
        };
        self.send_turn(session_id, input, cx);
    }

    /// Starts a turn of `input` in `session_id`.
    /// Why a turn cannot start in `session_id`, if it cannot.
    fn turn_blocked(&self, session_id: &str) -> Option<&'static str> {
        if self.worktree_removed(session_id) {
            Some("has no working copy")
        } else if !self.sessions.iter().any(|s| s.id == session_id) {
            Some("is not loaded")
        } else {
            None
        }
    }

    pub fn send_turn(&mut self, session_id: &str, input: TurnInput, cx: &mut Context<Self>) {
        if let Some(why) = self.turn_blocked(session_id) {
            log::warn!("thread {session_id} {why}; turn not sent");
            return;
        }
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else {
            return;
        };
        start_turn(session, &input.text, now_ms(), &input.attachments);
        if let (Some(card), Some(block)) = (&input.card, session.blocks.last_mut()) {
            card.stamp(block);
        }
        if let Some(thread) = self.threads.get_mut(session_id) {
            thread.usage_limit = None;
        }
        // MonoCode `dismissNoticesForContinuedSession`.
        self.dismiss_due_reminder(session_id, cx);
        // Skill bodies are small SKILL.md files; read them as MonoCode does
        // right before the turn starts.
        let typed = match (&input.agent_prompt, &input.card) {
            (Some(prompt), _) => prompt.clone(),
            (None, Some(card)) => card.agent_prompt(&input.text),
            (None, None) => input.text.clone(),
        };
        let agent_prompt = self.apply_skills(&typed);
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        // MonoCode `applyFileMentionsToTurn`, against this thread's folder.
        let agent_prompt = if self.project_files.root == session.work_dir() {
            let index = self.project_files.mentions.borrow().clone();
            crate::ui::composer::mentions::spell_out_mentions(&agent_prompt, &index)
        } else {
            agent_prompt
        };
        // MonoCode `applyNotesToTurn`: `@note/slug` brings the note along.
        let agent_prompt = super::notes::apply_notes_to_turn(&agent_prompt, &self.notes.items);
        self.persist_session(session_id);
        let request = RunRequest {
            prompt: agent_prompt,
            attachments: input.attachments,
            plan: input.plan,
            purpose: RunPurpose::Turn,
        };
        self.start_run(session_id, request, cx);
    }

    /// MonoCode `onCompactContext`: the agent summarises the older context
    /// (Claude's `/compact`, Grok's own request) as a turn of its own,
    /// shown only as status lines.
    pub fn compact_context(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if self.is_agent_running_in(session_id) {
            return;
        }
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else {
            return;
        };
        if !can_compact(&session.harness) {
            let harness = harness::HarnessKind::from_id(&session.harness)
                .map_or(session.harness.clone(), |kind| kind.label().to_string());
            push_notice(
                session,
                &format!("{harness} does not support manual context compaction."),
                now_ms(),
            );
            self.persist_session(session_id);
            cx.notify();
            return;
        }
        push_notice(session, COMPACTING_NOTICE, now_ms());
        self.persist_session(session_id);
        let request = RunRequest {
            prompt: COMPACT_COMMAND.to_string(),
            attachments: Vec::new(),
            plan: false,
            purpose: RunPurpose::Compact,
        };
        self.start_run(session_id, request, cx);
    }

    /// Spawns the thread's harness for `request` and follows its events; a
    /// failure to start lands in the transcript.
    fn start_run(&mut self, session_id: &str, request: RunRequest, cx: &mut Context<Self>) {
        let mode = self.session_permission_mode(self.sessions.iter().find(|s| s.id == session_id));
        let pinned = self.pin_session_account(session_id);
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        let spawn = pinned
            .and_then(|()| {
                spawn_request(session, &request.prompt, mode, self.claude_hooks_disabled)
            })
            .map(|spawn| SpawnRequest {
                attachments: request.attachments.clone(),
                plan: request.plan,
                compact: request.purpose == RunPurpose::Compact,
                ..spawn
            });
        // The baseline is queued before the agent can edit anything.
        if request.purpose == RunPurpose::Turn {
            self.checkpoints.begin_turn(session_id, session.work_dir());
        }
        let started = spawn.and_then(|spawn| {
            harness::spawn(&spawn)
                .map_err(|err| format!("Failed to start {}: {err:#}", spawn.harness.label()))
        });
        match started {
            Ok((handle, events)) => {
                let auto_approve = mode == PermissionMode::FullAccess && !request.plan;
                let steers = handle.can_steer();
                self.track_run(session_id.to_string(), handle, events, auto_approve, cx);
                if let Some(run) = self.runs.get_mut(session_id) {
                    run.can_steer = steers && request.purpose == RunPurpose::Turn;
                    run.purpose = request.purpose;
                }
                self.sync_in_flight();
            }
            Err(message) => {
                log::error!("{message}");
                if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
                    let now = now_ms();
                    push_notice(session, &message, now);
                    finish_turn(session, now);
                }
                self.persist_session(session_id);
            }
        }
        cx.notify();
    }

    fn track_run(
        &mut self,
        session_id: String,
        handle: HarnessProcessHandle,
        mut events: harness::EventRx,
        auto_approve: bool,
        cx: &mut Context<Self>,
    ) {
        self.next_run_id += 1;
        let run_id = self.next_run_id;
        self.runs.insert(
            session_id.clone(),
            AgentRun {
                id: run_id,
                handle,
                pending_permission: None,
                outcome: None,
                automation_run_id: None,
                auto_approve,
                can_steer: false,
                purpose: RunPurpose::Turn,
                compacted: None,
                edit_paths: HashMap::new(),
                last_saved: Instant::now(),
            },
        );

        cx.spawn(async move |this, cx| {
            while let Some(first) = events.recv().await {
                // Coalesce bursts of deltas into one update and one re-render.
                let mut batch = vec![first];
                while let Ok(next) = events.try_recv() {
                    batch.push(next);
                }
                let applied = this.update(cx, |app, cx| {
                    for event in batch {
                        if let Some(request) = app.on_agent_event(&session_id, run_id, event) {
                            app.announce_input(&session_id, &request, cx);
                        }
                    }
                    cx.notify();
                });
                if applied.is_err() {
                    return;
                }
            }
            if let Err(err) = this.update(cx, |app, cx| app.finish_run(&session_id, run_id, cx)) {
                log::debug!("agent run ended after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The live run of `session_id`, if it is still `run_id`.
    fn current_run(&mut self, session_id: &str, run_id: u64) -> Option<&mut AgentRun> {
        self.runs.get_mut(session_id).filter(|run| run.id == run_id)
    }

    /// Folds one event in; returns a permission request the user now has
    /// to answer, for its notification.
    fn on_agent_event(
        &mut self,
        session_id: &str,
        run_id: u64,
        event: AgentEvent,
    ) -> Option<PermissionRequest> {
        let focused = self.selected_session_id.as_deref() == Some(session_id);
        let run = self.current_run(session_id, run_id)?;
        let save = save_due(&event, run.last_saved.elapsed());
        match plan_event(run.auto_approve, &mut run.edit_paths, focused, event) {
            EventStep::UsageLimited(resets_at) => self.record_usage_limit(session_id, resets_at),
            EventStep::Compacted(tokens_after) => run.compacted = Some(tokens_after),
            EventStep::AutoApprove(request) => {
                if !run.handle.respond_permission(&request, true) {
                    log::warn!("harness rejected auto-approval for {}", request.request_id);
                }
            }
            EventStep::Ask {
                request,
                focus_question,
            } => {
                run.pending_permission = Some(request.clone());
                run.last_saved = Instant::now();
                // The form takes the keys, as MonoCode focuses its options.
                if focus_question {
                    self.question_focus_wanted = true;
                }
                // The user may walk away while the prompt waits.
                self.persist_session(session_id);
                return Some(request);
            }
            EventStep::Apply {
                event,
                checkpoint,
                done,
            } => {
                if let Some(status) = done {
                    run.outcome = Some(status);
                }
                if save {
                    run.last_saved = Instant::now();
                }
                if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
                    if let Some((paths, completed)) = checkpoint {
                        let cwd = session.work_dir();
                        if completed {
                            self.checkpoints.capture(session_id, cwd, paths);
                        } else {
                            self.checkpoints.prepare(session_id, cwd, paths);
                        }
                    }
                    apply_event(session, event, now_ms());
                }
                if save {
                    self.persist_session(session_id);
                }
            }
        }
        None
    }

    fn finish_run(&mut self, session_id: &str, run_id: u64, cx: &mut Context<Self>) {
        if self.current_run(session_id, run_id).is_none() {
            return;
        }
        let Some(run) = self.runs.remove(session_id) else {
            return;
        };
        self.sync_in_flight();
        if self
            .sessions
            .iter()
            .any(|s| s.id == session_id && s.harness == crate::harness::agy_accounts::PROVIDER)
        {
            self.reload_antigravity_usage(cx);
        }
        if run.purpose == RunPurpose::Compact
            && let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id)
        {
            match compact_end(run.compacted, run.outcome) {
                CompactEnd::Confirmed(tokens) => {
                    push_notice(session, COMPACTED_NOTICE, now_ms());
                    if let Some(tokens) = tokens {
                        session.context_used = Some(tokens);
                    }
                }
                CompactEnd::Unconfirmed => {
                    let notice = unconfirmed_compact(&session.harness);
                    push_notice(session, &notice, now_ms());
                }
                CompactEnd::Cancelled => {}
            }
        }
        self.persist_session(session_id);
        self.close_automation_run(&run, automation_status(run.outcome));
        if run.purpose == RunPurpose::Turn {
            self.announce_turn_finished(session_id, cx);
        }
        // The agent has most likely edited files.
        self.refresh_workspace(cx);
        self.load_session_review(session_id, cx);
        self.send_next_queued(session_id, cx);
        cx.notify();
    }

    /// Starts the oldest queued prompt of a thread whose turn just ended
    /// (MonoCode `canDispatchQueuedHead`).
    fn send_next_queued(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let editing_head = self
            .queue_editing
            .as_ref()
            .is_some_and(|(sid, ix)| sid == session_id && *ix == 0);
        let (paused, usage_limited, queued) =
            self.thread(session_id).map_or((false, false, false), |t| {
                (t.queue_paused, t.usage_limit.is_some(), !t.queue.is_empty())
            });
        match queue_dispatch(paused, usage_limited, editing_head) {
            QueueDispatch::Wait => return,
            QueueDispatch::Hold => {
                self.thread_mut(session_id).queue_held = true;
                return;
            }
            QueueDispatch::Send => {}
        }
        // The head stays queued; Resume sends it once the thread can run.
        if let Some(why) = self.turn_blocked(session_id) {
            if queued {
                log::warn!("thread {session_id} {why}; queued prompt kept");
                self.thread_mut(session_id).queue_paused = true;
            }
            return;
        }
        let Some(next) = self.threads.get_mut(session_id).and_then(|thread| {
            pop_queue_head(&mut thread.queue, &mut self.queue_editing, session_id)
        }) else {
            return;
        };
        self.send_turn(session_id, next, cx);
    }

    /// Stops a thread's agent. Its queue is kept, but nothing is sent until
    /// the user sends again (MonoCode pauses the queue on interrupt).
    pub fn stop_agent(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(run) = self.runs.remove(session_id) else {
            return;
        };
        run.handle.cancel();
        self.sync_in_flight();
        self.close_automation_run(&run, RunStatus::Cancelled);
        if let Some(thread) = self.threads.get_mut(session_id)
            && !thread.queue.is_empty()
        {
            thread.queue_paused = true;
        }
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            let now = now_ms();
            push_notice(session, STOPPED_NOTICE, now);
            finish_turn(session, now);
        }
        self.persist_session(session_id);
        self.refresh_workspace(cx);
        cx.notify();
    }

    /// Links the running turn of `session_id` to an automation run row.
    pub fn attach_automation_run(&mut self, session_id: &str, run_id: String) {
        if let Some(run) = self.runs.get_mut(session_id) {
            run.automation_run_id = Some(run_id);
        }
    }

    /// ⌘Q: quits at once unless agents are running (MonoCode asks then).
    pub fn request_quit(&mut self, cx: &mut Context<Self>) {
        if self.runs.is_empty() {
            cx.quit();
            return;
        }
        self.quit_confirm_open = true;
        cx.notify();
    }

    /// Quit or window close: stops every run, marks its turn interrupted,
    /// saves it, and waits for the writer. The in-flight list is left as
    /// it is, for the next launch to resume. Needs no `cx`: it also runs
    /// from `on_release`.
    pub(crate) fn interrupt_runs_for_quit(&mut self) {
        let runs: Vec<(String, AgentRun)> = self.runs.drain().collect();
        for (session_id, run) in &runs {
            run.handle.cancel();
            self.close_automation_run(run, RunStatus::Cancelled);
            if let Some(session) = self.sessions.iter_mut().find(|s| &s.id == session_id) {
                mark_turn_interrupted(session, now_ms());
            }
            self.persist_session(session_id);
        }
        // Without runs too: other saves may still be queued.
        if let Some(writer) = &self.db_writer
            && !writer.flush(Duration::from_secs(3))
        {
            log::error!("quit before every thread was saved");
        }
    }

    fn close_automation_run(&self, run: &AgentRun, status: RunStatus) {
        let Some(run_id) = &run.automation_run_id else {
            return;
        };
        let error = (status == RunStatus::Failed).then_some("The agent turn failed.");
        let run_id = run_id.clone();
        self.db_write("close automation run", move |db| {
            db.finish_automation_run(&run_id, status, error)
        });
    }

    /// Answers the permission prompt of `session_id`'s run.
    /// Replies to the agent's `AskUserQuestion`: `Some(input)` allows the
    /// tool with the answers filled in, `None` skips it.
    pub fn answer_question(
        &mut self,
        session_id: &str,
        answered: Option<Value>,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = self.runs.get_mut(session_id) else {
            return;
        };
        let Some(mut request) = run.pending_permission.take() else {
            return;
        };
        let allow = answered.is_some();
        if let Some(input) = answered {
            request.input = input;
        }
        if !run.handle.respond_permission(&request, allow) {
            log::warn!("harness rejected the answer to {}", request.request_id);
        }
        if let Some(thread) = self.threads.get_mut(session_id) {
            thread.question_ui = None;
        }
        cx.notify();
    }

    pub fn answer_permission(&mut self, session_id: &str, allow: bool, cx: &mut Context<Self>) {
        let Some(run) = self.runs.get_mut(session_id) else {
            return;
        };
        let Some(request) = run.pending_permission.take() else {
            return;
        };
        if !run.handle.respond_permission(&request, allow) {
            log::warn!(
                "harness rejected permission reply for {}",
                request.request_id
            );
        }
        cx.notify();
    }

    /// Saves one session to SQLite: queued on the writer thread, so the UI
    /// does not wait for it. Errors are logged, not swallowed.
    pub fn persist_session(&self, session_id: &str) {
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        match &self.db_writer {
            // A snapshot: the transcript keeps changing while the write waits.
            Some(writer) => writer.save(session.clone()),
            None => {
                if let Err(err) = self.db.upsert_session(session) {
                    log::error!("failed to save session {session_id}: {err:#}");
                }
            }
        }
    }
}

fn spawn_request(
    session: &SessionRow,
    prompt: &str,
    mode: PermissionMode,
    disable_hooks: bool,
) -> Result<SpawnRequest, String> {
    let harness = HarnessKind::from_id(&session.harness).ok_or_else(|| {
        format!(
            "BenCode cannot drive the `{}` harness yet.",
            session.harness
        )
    })?;
    Ok(SpawnRequest {
        harness,
        cwd: session.work_dir().to_string(),
        prompt: prompt.to_string(),
        model: catalog::cli_model_id(&session.model),
        permission: mode.policy(),
        resume_id: session
            .provider_session_id
            .clone()
            .filter(|id| !id.is_empty()),
        disable_hooks,
        attachments: Vec::new(),
        plan: false,
        compact: false,
        settings: catalog::resolved_settings(&session.model, session.model_settings.as_ref()),
        account: AccountProfile::resolve(&session.harness, session.provider_account_id.as_deref()),
    })
}

pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

fn turn_model(session: &SessionRow) -> TurnModel {
    TurnModel {
        harness: Some(session.harness.clone()),
        id: Some(session.model.clone()),
        name: Some(catalog::label_for(&session.model)),
    }
}

/// Appends the user's prompt and names the thread after it if still untitled.
pub fn start_turn(session: &mut SessionRow, prompt: &str, now: i64, files: &[Attachment]) {
    let mut block = Block::new(format!("usr-{now}"), "user", prompt);
    block.started_at = Some(now);
    block.turn_model = Some(turn_model(session));
    if !files.is_empty() {
        let kept: Vec<Value> = files.iter().map(Attachment::to_block_json).collect();
        block.extra.insert("attachments".into(), Value::Array(kept));
    }
    session.blocks.push(block);
    session.updated_at = now;

    if session.title.is_empty() || session.title == NEW_SESSION_TITLE {
        session.title = title_from_prompt(prompt);
    }
}

fn title_from_prompt(prompt: &str) -> String {
    let first_line = prompt.lines().next().unwrap_or(prompt).trim();
    match first_line.char_indices().nth(TITLE_PREVIEW_CHARS) {
        Some((cut, _)) => format!("{}…", &first_line[..cut]),
        None => first_line.to_string(),
    }
}

/// MonoCode `applyTurnMetrics`: the latest user block's `turnMetrics`
/// takes every count the provider reported, keeping the others.
fn record_turn_metrics(session: &mut SessionRow, metrics: &TurnMetrics) {
    let Some(user) = session.blocks.iter_mut().rev().find(|b| b.role == "user") else {
        return;
    };
    let entry = user
        .extra
        .entry("turnMetrics")
        .or_insert_with(|| Value::Object(Default::default()));
    if !entry.is_object() {
        *entry = Value::Object(Default::default());
    }
    let Some(stored) = entry.as_object_mut() else {
        return;
    };
    let counts = [
        ("inputTokens", metrics.input_tokens),
        ("outputTokens", metrics.output_tokens),
        ("cacheReadTokens", metrics.cache_read_tokens),
        ("cacheWriteTokens", metrics.cache_write_tokens),
    ];
    for (key, value) in counts {
        if let Some(value) = value {
            stored.insert(key.into(), value.into());
        }
    }
    if let Some(percent) = metrics.cache_hit_percent {
        stored.insert("cacheHitPercent".into(), json!(percent));
    }
}

/// Records how long the latest user turn took, as MonoCode does.
fn finish_turn(session: &mut SessionRow, now: i64) {
    if let Some(user) = session.blocks.iter_mut().rev().find(|b| b.role == "user")
        && let Some(started) = user.started_at
    {
        user.duration_ms = Some(now.saturating_sub(started));
    }
}

/// MonoCode `markTurnInterrupted` (inFlight.ts): open tools become
/// cancelled and the turn ends with an interrupt notice (once).
pub(super) fn mark_turn_interrupted(session: &mut SessionRow, now: i64) {
    for block in &mut session.blocks {
        let Some(tool) = block.tool.as_mut().and_then(Value::as_object_mut) else {
            continue;
        };
        let open = tool
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| {
                matches!(
                    status.to_ascii_lowercase().as_str(),
                    "in_progress" | "pending" | "running"
                )
            });
        if open {
            tool.insert("status".into(), json!("cancelled"));
        }
    }
    let noted = session
        .blocks
        .last()
        .is_some_and(|b| b.role == "system" && b.text.as_deref() == Some(INTERRUPT_NOTICE));
    if !noted {
        let mut block = Block::new(
            format!("sys-{now}-{}", session.blocks.len()),
            "system",
            INTERRUPT_NOTICE,
        );
        block.extra.insert("notice".into(), json!("interrupt"));
        session.blocks.push(block);
    }
    finish_turn(session, now);
}

fn push_notice(session: &mut SessionRow, message: &str, now: i64) {
    let mut block = Block::new(
        format!("sys-{now}-{}", session.blocks.len()),
        "system",
        message,
    );
    block.extra.insert("notice".into(), json!("error"));
    session.blocks.push(block);
}

/// Folds one harness event into the transcript.
pub fn apply_event(session: &mut SessionRow, event: AgentEvent, now: i64) {
    session.updated_at = now;
    match event {
        AgentEvent::SessionStarted {
            provider_session_id,
        } => {
            session.provider_session_id = Some(provider_session_id);
        }
        AgentEvent::TextDelta(delta) => append_text(session, "assistant", &delta, now),
        AgentEvent::ThinkingDelta(delta) => append_text(session, "reasoning", &delta, now),
        AgentEvent::ToolCallStart { id, name, input } => {
            start_tool(session, &id, &name, &input, now)
        }
        AgentEvent::ToolCallFinish {
            id,
            output,
            success,
        } => finish_tool(session, &id, &output, success),
        AgentEvent::Usage {
            total_tokens,
            context_window,
            ..
        } => {
            session.context_used = i64::try_from(total_tokens).ok();
            // The CLI's own window wins over the catalog's guess.
            if let Some(window) = context_window.and_then(|w| i64::try_from(w).ok()) {
                session.context_window = Some(window);
            }
            // The ring appears with the first report (MonoCode `contextUsage`).
            if session.context_window.is_none() {
                session.context_window = Some(crate::harness::catalog::context_window_tokens(
                    &session.model,
                    session.model_settings.as_ref(),
                ));
            }
        }
        AgentEvent::Tasks(items) => crate::app::task_list::upsert(session, &items, now),
        AgentEvent::TurnMetrics(metrics) => record_turn_metrics(session, &metrics),
        // Kept by the run and the app rather than the transcript.
        AgentEvent::Compacted { .. } | AgentEvent::UsageLimited { .. } => {}
        AgentEvent::Error(message) => push_notice(session, &message, now),
        AgentEvent::Done(status) => {
            finish_turn(session, now);
            if status == DoneStatus::Failed {
                log::warn!("agent turn in session {} failed", session.id);
            }
        }
        // Held by the app until the user answers; never persisted.
        AgentEvent::PermissionRequest(_) => {}
    }
}

/// Streams into the trailing block of `role`, or opens a new one.
fn append_text(session: &mut SessionRow, role: &str, delta: &str, now: i64) {
    if let Some(last) = session
        .blocks
        .last_mut()
        .filter(|b| b.role == role && b.tool.is_none())
    {
        last.text.get_or_insert_with(String::new).push_str(delta);
        return;
    }
    let prefix = if role == "assistant" { "ast" } else { "rsn" };
    let mut block = Block::new(
        format!("{prefix}-{now}-{}", session.blocks.len()),
        role,
        delta,
    );
    block.started_at = Some(now);
    if role == "assistant" {
        block.turn_model = Some(turn_model(session));
    }
    session.blocks.push(block);
}

fn tool_block_mut<'a>(session: &'a mut SessionRow, call_id: &str) -> Option<&'a mut Block> {
    session.blocks.iter_mut().rev().find(|b| {
        b.tool
            .as_ref()
            .and_then(|t| t.get("callId"))
            .and_then(Value::as_str)
            == Some(call_id)
    })
}

fn start_tool(session: &mut SessionRow, call_id: &str, name: &str, input: &Value, now: i64) {
    let title = summarize_tool_input(name, input);
    if let Some(existing) = tool_block_mut(session, call_id) {
        if let Some(tool) = existing.tool.as_mut().and_then(Value::as_object_mut) {
            tool.insert("title".into(), json!(title));
        }
        return;
    }
    let mut block = Block::new(format!("tool-{call_id}"), "tool", "");
    block.started_at = Some(now);
    block.tool = Some(json!({
        "callId": call_id,
        "kind": tool_kind(name),
        "title": title,
        "status": "in_progress",
    }));
    session.blocks.push(block);
}

fn finish_tool(session: &mut SessionRow, call_id: &str, output: &str, success: bool) {
    let Some(block) = tool_block_mut(session, call_id) else {
        log::debug!("result for unknown tool call {call_id}");
        return;
    };
    let Some(tool) = block.tool.as_mut().and_then(Value::as_object_mut) else {
        return;
    };
    tool.insert(
        "status".into(),
        json!(if success { "completed" } else { "failed" }),
    );
    if !output.is_empty() {
        tool.insert(
            "detail".into(),
            json!(truncate(output, MAX_TOOL_OUTPUT_CHARS)),
        );
    }
}

/// MonoCode's tool `kind` vocabulary, which the transcript keys icons off.
fn tool_kind(name: &str) -> &'static str {
    match name.to_ascii_lowercase().as_str() {
        "bash" | "shell" | "run_command" | "exec" => "execute",
        "edit"
        | "multiedit"
        | "write"
        | "notebookedit"
        | "write_to_file"
        | "replace_file_content" => "edit",
        "read" | "view_file" => "read",
        "grep" | "glob" | "websearch" | "grep_search" | "find_by_name" | "search_web" => "search",
        "task" | "agent" | "invoke_subagent" => "agent",
        "skill" => "skill",
        _ => "other",
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}\n… (truncated)", &text[..cut]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionRow {
        SessionRow {
            id: "s1".into(),
            title: NEW_SESSION_TITLE.into(),
            cwd: "/tmp".into(),
            harness: "claude".into(),
            model: "claude:opus".into(),
            ..Default::default()
        }
    }

    fn roles(session: &SessionRow) -> Vec<&str> {
        session.blocks.iter().map(|b| b.role.as_str()).collect()
    }

    #[test]
    fn start_turn_adds_user_block_and_titles_thread() {
        let mut s = session();
        start_turn(&mut s, "Fix the login bug\nwith details", 10, &[]);
        assert_eq!(roles(&s), ["user"]);
        assert_eq!(s.title, "Fix the login bug");
        assert_eq!(
            s.blocks[0].turn_model.as_ref().unwrap().name.as_deref(),
            Some("Opus")
        );
    }

    #[test]
    fn deltas_stream_into_separate_reasoning_and_assistant_blocks() {
        let mut s = session();
        start_turn(&mut s, "hi", 1, &[]);
        for event in [
            AgentEvent::ThinkingDelta("think ".into()),
            AgentEvent::ThinkingDelta("more".into()),
            AgentEvent::TextDelta("Hel".into()),
            AgentEvent::TextDelta("lo".into()),
        ] {
            apply_event(&mut s, event, 2);
        }
        assert_eq!(roles(&s), ["user", "reasoning", "assistant"]);
        assert_eq!(s.blocks[1].text.as_deref(), Some("think more"));
        assert_eq!(s.blocks[2].text.as_deref(), Some("Hello"));
    }

    #[test]
    fn tool_calls_become_tool_blocks_and_split_text() {
        let mut s = session();
        apply_event(&mut s, AgentEvent::TextDelta("Let me look.".into()), 1);
        let start = AgentEvent::ToolCallStart {
            id: "t1".into(),
            name: "Bash".into(),
            input: json!({"command": "ls"}),
        };
        apply_event(&mut s, start.clone(), 2);
        apply_event(&mut s, start, 2); // duplicate announcement is ignored
        apply_event(
            &mut s,
            AgentEvent::ToolCallFinish {
                id: "t1".into(),
                output: "a.rs".into(),
                success: true,
            },
            3,
        );
        apply_event(&mut s, AgentEvent::TextDelta("Done.".into()), 4);

        assert_eq!(roles(&s), ["assistant", "tool", "assistant"]);
        let tool = s.blocks[1].tool.as_ref().unwrap();
        assert_eq!(tool["kind"], "execute");
        assert_eq!(tool["title"], "ls");
        assert_eq!(tool["status"], "completed");
        assert_eq!(tool["detail"], "a.rs");
    }

    #[test]
    fn session_started_and_usage_update_session() {
        let mut s = session();
        apply_event(
            &mut s,
            AgentEvent::SessionStarted {
                provider_session_id: "abc".into(),
            },
            1,
        );
        apply_event(
            &mut s,
            AgentEvent::Usage {
                input_tokens: 5,
                output_tokens: 5,
                total_tokens: 10,
                context_window: None,
            },
            1,
        );
        assert_eq!(s.provider_session_id.as_deref(), Some("abc"));
        assert_eq!(s.context_used, Some(10));
    }

    #[test]
    fn errors_become_system_notices_and_done_records_duration() {
        let mut s = session();
        start_turn(&mut s, "hi", 100, &[]);
        apply_event(&mut s, AgentEvent::Error("boom".into()), 150);
        apply_event(&mut s, AgentEvent::Done(DoneStatus::Failed), 400);
        assert_eq!(roles(&s), ["user", "system"]);
        assert_eq!(s.blocks[1].extra["notice"], "error");
        assert_eq!(s.blocks[0].duration_ms, Some(300));
    }

    #[test]
    fn spawn_request_maps_model_policy_and_resume() {
        let mut s = session();
        s.provider_session_id = Some("resume-me".into());
        let req = spawn_request(&s, "go", PermissionMode::Supervised, false).unwrap();
        assert_eq!(req.harness, HarnessKind::Claude);
        assert_eq!(req.model.as_deref(), Some("opus"));
        assert_eq!(req.permission, PermissionPolicy::Ask);
        assert_eq!(req.resume_id.as_deref(), Some("resume-me"));

        s.harness = "pi".into();
        assert!(spawn_request(&s, "go", PermissionMode::FullAccess, false).is_err());
    }

    #[test]
    fn automation_status_maps_turn_outcomes() {
        assert_eq!(
            automation_status(Some(DoneStatus::Completed)),
            RunStatus::Succeeded
        );
        assert_eq!(
            automation_status(Some(DoneStatus::Cancelled)),
            RunStatus::Cancelled
        );
        assert_eq!(automation_status(None), RunStatus::Failed);
    }

    #[test]
    fn long_output_is_truncated_on_char_boundary() {
        let text = "ü".repeat(MAX_TOOL_OUTPUT_CHARS + 5);
        assert!(truncate(&text, MAX_TOOL_OUTPUT_CHARS).ends_with("(truncated)"));
    }

    fn permission(tool: &str) -> AgentEvent {
        AgentEvent::PermissionRequest(PermissionRequest {
            request_id: "r1".into(),
            tool: tool.into(),
            description: String::new(),
            input: json!({}),
        })
    }

    fn edit_start(id: &str, name: &str, input: Value) -> AgentEvent {
        AgentEvent::ToolCallStart {
            id: id.into(),
            name: name.into(),
            input,
        }
    }

    fn tool_finish(id: &str, success: bool) -> AgentEvent {
        AgentEvent::ToolCallFinish {
            id: id.into(),
            output: String::new(),
            success,
        }
    }

    /// The checkpoint an applied event asks for.
    fn checkpoint_of(step: EventStep) -> Option<(Vec<String>, bool)> {
        match step {
            EventStep::Apply { checkpoint, .. } => checkpoint,
            other => panic!("not applied: {other:?}"),
        }
    }

    #[test]
    fn usage_limit_and_compaction_skip_the_transcript() {
        let mut edits = HashMap::new();
        let limited = AgentEvent::UsageLimited { resets_at: Some(5) };
        assert_eq!(
            plan_event(false, &mut edits, true, limited),
            EventStep::UsageLimited(Some(5))
        );
        let compacted = AgentEvent::Compacted {
            tokens_after: Some(9),
        };
        assert_eq!(
            plan_event(false, &mut edits, true, compacted),
            EventStep::Compacted(Some(9))
        );
    }

    #[test]
    fn full_access_approves_all_but_questions() {
        let mut edits = HashMap::new();
        assert!(matches!(
            plan_event(true, &mut edits, true, permission("Bash")),
            EventStep::AutoApprove(_)
        ));
        assert!(matches!(
            plan_event(true, &mut edits, true, permission(QUESTION_TOOL)),
            EventStep::Ask {
                focus_question: true,
                ..
            }
        ));
        assert!(matches!(
            plan_event(true, &mut edits, false, permission(QUESTION_TOOL)),
            EventStep::Ask {
                focus_question: false,
                ..
            }
        ));
    }

    #[test]
    fn supervised_permissions_ask_without_focus() {
        let mut edits = HashMap::new();
        assert!(matches!(
            plan_event(false, &mut edits, true, permission("Bash")),
            EventStep::Ask {
                focus_question: false,
                ..
            }
        ));
    }

    #[test]
    fn edit_tools_prepare_then_capture() {
        let mut edits = HashMap::new();
        let start = edit_start("t1", "Edit", json!({"file_path": "/r/a.rs"}));
        let step = plan_event(false, &mut edits, true, start);
        assert!(matches!(step, EventStep::Apply { done: None, .. }));
        assert_eq!(
            checkpoint_of(step),
            Some((vec!["/r/a.rs".to_string()], false))
        );
        assert!(edits.contains_key("t1"));

        let step = plan_event(false, &mut edits, true, tool_finish("t1", true));
        assert_eq!(
            checkpoint_of(step),
            Some((vec!["/r/a.rs".to_string()], true))
        );
        assert!(edits.is_empty());
    }

    #[test]
    fn a_failed_edit_captures_nothing() {
        let mut edits = HashMap::new();
        let start = edit_start("t1", "Edit", json!({"file_path": "/r/a.rs"}));
        drop(plan_event(false, &mut edits, true, start));
        let step = plan_event(false, &mut edits, true, tool_finish("t1", false));
        assert_eq!(checkpoint_of(step), None);
        assert!(edits.is_empty());
    }

    #[test]
    fn non_edit_tools_and_pathless_edits_take_no_checkpoint() {
        let mut edits = HashMap::new();
        let bash = edit_start("t1", "Bash", json!({"command": "ls"}));
        assert_eq!(
            checkpoint_of(plan_event(false, &mut edits, true, bash)),
            None
        );
        assert!(edits.is_empty());

        // An edit that names no file is still tracked until it finishes.
        let write = edit_start("t2", "Write", json!({}));
        assert_eq!(
            checkpoint_of(plan_event(false, &mut edits, true, write)),
            None
        );
        assert_eq!(edits.get("t2"), Some(&Vec::new()));

        let unknown = tool_finish("nope", true);
        assert_eq!(
            checkpoint_of(plan_event(false, &mut edits, true, unknown)),
            None
        );
    }

    #[test]
    fn done_reports_its_status() {
        let mut edits = HashMap::new();
        let done = AgentEvent::Done(DoneStatus::Completed);
        assert!(matches!(
            plan_event(false, &mut edits, true, done),
            EventStep::Apply {
                done: Some(DoneStatus::Completed),
                ..
            }
        ));
        let text = AgentEvent::TextDelta("x".into());
        assert!(matches!(
            plan_event(false, &mut edits, true, text),
            EventStep::Apply {
                done: None,
                checkpoint: None,
                ..
            }
        ));
    }

    #[test]
    fn recorded_claude_edit_turn_prepares_and_captures() {
        use crate::harness::process::LineParser;

        let lines = [
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Edit","input":{"file_path":"/r/a.rs","old_string":"x","new_string":"y"}}]},"parent_tool_use_id":null}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"ok"}],"is_error":false}]},"parent_tool_use_id":null}"#,
        ];
        let mut parser = crate::harness::claude::ClaudeParser::default();
        let mut edits = HashMap::new();
        let mut checkpoints = Vec::new();
        for line in lines {
            for event in parser.parse_line(line) {
                if let EventStep::Apply {
                    checkpoint: Some(checkpoint),
                    ..
                } = plan_event(false, &mut edits, true, event)
                {
                    checkpoints.push(checkpoint);
                }
            }
        }
        let path = vec!["/r/a.rs".to_string()];
        assert_eq!(checkpoints, [(path.clone(), false), (path, true)]);
    }

    fn queued(text: &str) -> TurnInput {
        TurnInput {
            text: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn queue_dispatch_waits_holds_or_sends() {
        assert_eq!(queue_dispatch(true, false, false), QueueDispatch::Wait);
        assert_eq!(queue_dispatch(false, true, false), QueueDispatch::Wait);
        assert_eq!(queue_dispatch(false, false, true), QueueDispatch::Hold);
        // A pause wins over an edit of the head.
        assert_eq!(queue_dispatch(true, false, true), QueueDispatch::Wait);
        assert_eq!(queue_dispatch(false, false, false), QueueDispatch::Send);
    }

    #[test]
    fn popping_the_head_drops_an_empty_queue_and_moves_the_edit_up() {
        let mut queue = vec![queued("a"), queued("b")];
        let mut editing = Some(("s1".to_string(), 1));
        let head = pop_queue_head(&mut queue, &mut editing, "s1").unwrap();
        assert_eq!(head.text, "a");
        assert_eq!(editing, Some(("s1".to_string(), 0)));
        assert_eq!(queue.len(), 1);

        let head = pop_queue_head(&mut queue, &mut editing, "s1").unwrap();
        assert_eq!(head.text, "b");
        assert!(queue.is_empty());

        let before = editing.clone();
        assert!(pop_queue_head(&mut queue, &mut editing, "s1").is_none());
        assert_eq!(editing, before);

        // An edit in another thread's queue stays where it is.
        let mut queue = vec![queued("a")];
        let mut editing = Some(("other".to_string(), 2));
        assert!(pop_queue_head(&mut queue, &mut editing, "s1").is_some());
        assert_eq!(editing, Some(("other".to_string(), 2)));
    }

    #[test]
    fn taking_the_last_item_unpauses() {
        let mut thread = ThreadState {
            queue: vec![queued("a")],
            queue_paused: true,
            ..Default::default()
        };
        assert!(take_queued(&mut thread, 5).is_none());
        assert_eq!(thread.queue.len(), 1);
        assert!(thread.queue_paused);

        assert!(take_queued(&mut thread, 0).is_some());
        assert!(thread.queue.is_empty());
        assert!(!thread.queue_paused);
    }

    #[test]
    fn compaction_end_states() {
        use DoneStatus::{Cancelled, Completed, Failed};
        assert_eq!(
            compact_end(Some(Some(1289)), Some(Completed)),
            CompactEnd::Confirmed(Some(1289))
        );
        assert_eq!(compact_end(Some(None), None), CompactEnd::Confirmed(None));
        assert_eq!(compact_end(None, Some(Cancelled)), CompactEnd::Cancelled);
        assert_eq!(compact_end(None, Some(Failed)), CompactEnd::Unconfirmed);
        assert_eq!(compact_end(None, None), CompactEnd::Unconfirmed);
    }

    #[test]
    fn session_started_and_done_save_at_once() {
        let started = AgentEvent::SessionStarted {
            provider_session_id: "p1".into(),
        };
        assert!(save_due(&started, Duration::ZERO));
        assert!(save_due(
            &AgentEvent::Done(DoneStatus::Completed),
            Duration::ZERO
        ));
    }

    #[test]
    fn streaming_text_saves_every_few_seconds() {
        let text = AgentEvent::TextDelta("x".into());
        assert!(!save_due(&text, Duration::from_secs(1)));
        assert!(save_due(&text, Duration::from_secs(3)));
        let thinking = AgentEvent::ThinkingDelta("x".into());
        assert!(!save_due(&thinking, Duration::from_millis(2900)));
    }

    #[test]
    fn finished_tools_save_sooner() {
        let finish = tool_finish("t1", true);
        assert!(!save_due(&finish, Duration::from_millis(100)));
        assert!(save_due(&finish, Duration::from_millis(500)));
    }

    fn tool_status(session: &SessionRow, call_id: &str) -> String {
        let block = session
            .blocks
            .iter()
            .find(|b| b.id == format!("tool-{call_id}"))
            .unwrap();
        block.tool.as_ref().unwrap()["status"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn interrupt_cancels_open_tools_and_adds_one_notice() {
        let mut s = session();
        start_turn(&mut s, "go", 10, &[]);
        apply_event(
            &mut s,
            edit_start("t1", "Bash", json!({"command": "ls"})),
            11,
        );
        mark_turn_interrupted(&mut s, 20);
        mark_turn_interrupted(&mut s, 21);
        assert_eq!(tool_status(&s, "t1"), "cancelled");
        let notices: Vec<&Block> = s
            .blocks
            .iter()
            .filter(|b| b.text.as_deref() == Some(INTERRUPT_NOTICE))
            .collect();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].extra["notice"], "interrupt");
    }

    #[test]
    fn interrupt_keeps_finished_tools() {
        let mut s = session();
        start_turn(&mut s, "go", 10, &[]);
        apply_event(
            &mut s,
            edit_start("t1", "Bash", json!({"command": "ls"})),
            11,
        );
        apply_event(&mut s, tool_finish("t1", true), 12);
        mark_turn_interrupted(&mut s, 20);
        assert_eq!(tool_status(&s, "t1"), "completed");
    }
}
