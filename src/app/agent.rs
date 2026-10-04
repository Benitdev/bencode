//! Running an agent turn: spawning the harness, folding its events into the
//! session transcript, permission prompts and stop/cancel.
//!
//! `apply_event` is a pure reducer over `SessionRow` so transcript behaviour
//! is unit-tested without GPUI; `BenCodeApp` methods are thin glue around it.

use gpui::{Context, Focusable};
use serde_json::{Value, json};

use crate::app::{BenCodeApp, PermissionMode};
use crate::db::{Block, SessionRow, TurnModel};
use crate::harness::Attachment;
use crate::harness::events::TurnMetrics;
use crate::harness::{
    self, AgentEvent, DoneStatus, HarnessKind, HarnessProcessHandle, PermissionPolicy,
    PermissionRequest, SpawnRequest, catalog, summarize_tool_input,
};
use crate::ui::composer::mode_commands::{self, ModeCommand};

const TITLE_PREVIEW_CHARS: usize = 48;
const MAX_TOOL_OUTPUT_CHARS: usize = 4_000;
const STOPPED_NOTICE: &str = "Agent execution stopped by user.";
pub const NEW_SESSION_TITLE: &str = "New AI Thread";

/// One in-flight agent turn; each thread runs its own. `id` guards against
/// late events from a run that was already stopped or superseded.
/// What one turn sends: the text, attached files, and MonoCode Plan mode.
#[derive(Clone, Debug, Default)]
pub struct TurnInput {
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub plan: bool,
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
}

/// MonoCode's terminal automation-run status for a turn outcome.
fn automation_status(outcome: Option<DoneStatus>) -> &'static str {
    match outcome {
        Some(DoneStatus::Completed) => "succeeded",
        Some(DoneStatus::Cancelled) => "cancelled",
        Some(DoneStatus::Failed) | None => "failed",
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
        self.prompt_queues
            .get(session_id)
            .map_or(&[], |queue| queue.as_slice())
    }

    /// MonoCode's paused queue: the agent was stopped with messages waiting.
    pub fn queue_paused(&self, session_id: &str) -> bool {
        !self.queued_prompts(session_id).is_empty() && !self.is_agent_running_in(session_id)
    }

    /// Resume: sends the next waiting message.
    pub fn resume_queue(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if !self.is_agent_running_in(session_id) {
            self.send_next_queued(session_id, cx);
        }
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
            .prompt_queues
            .get_mut(&session_id)
            .and_then(|queue| queue.get_mut(ix))
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
        if self.queue_held.remove(session_id) {
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
        if let Some(queue) = self.prompt_queues.get_mut(session_id)
            && ix < queue.len()
        {
            queue.remove(ix);
            if queue.is_empty() {
                self.prompt_queues.remove(session_id);
            }
            cx.notify();
        }
    }

    /// The composer's button: Stop when the focused thread is running and
    /// nothing is typed; otherwise Send (which queues while running).
    pub fn handle_send_or_stop(&mut self, cx: &mut Context<Self>) {
        let typed = !self.prompt_input.read(cx).text().trim().is_empty();
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
        // A leading `/plan` or `/draft` acts as its mode and is not sent.
        let (command, text) = mode_commands::strip_leading_mode(&typed);
        if self.selected_session_id.is_none() {
            if text.is_empty() {
                return;
            }
            self.create_new_session(cx);
        }
        let Some(session_id) = self.selected_session_id.clone() else {
            return;
        };
        let centred = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .is_some_and(|s| !s.blocks.iter().any(|b| b.role == "user"));
        let has_files = self
            .composer_attachments
            .get(&session_id)
            .is_some_and(|files| !files.is_empty());
        if text.is_empty() && !has_files {
            return;
        }
        let input = TurnInput {
            text,
            attachments: self
                .composer_attachments
                .remove(&session_id)
                .unwrap_or_default(),
            plan: self.plan_mode.contains(&session_id) || command == Some(ModeCommand::Plan),
        };
        self.prompt_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.drafts.remove(&session_id);
        if centred {
            self.launch_dock_motion(&session_id);
        }
        // MonoCode clears Plan after each send; Draft stays chosen until a
        // draft is saved.
        self.plan_mode.remove(&session_id);
        if self.draft_mode.remove(&session_id) || command == Some(ModeCommand::Draft) {
            self.save_draft(&session_id, input, cx);
            return;
        }
        if self.is_agent_running_in(&session_id) {
            self.prompt_queues
                .entry(session_id)
                .or_default()
                .push(input);
            cx.notify();
            return;
        }
        self.send_turn(&session_id, input, cx);
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
            self.prompt_queues
                .entry(session_id.to_string())
                .or_default()
                .push(input);
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
    pub fn send_turn(&mut self, session_id: &str, input: TurnInput, cx: &mut Context<Self>) {
        let prompt = input.text.as_str();
        let mode = self.session_permission_mode(self.sessions.iter().find(|s| s.id == session_id));
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else {
            return;
        };
        let now = now_ms();
        start_turn(session, prompt, now, &input.attachments);
        // Skill bodies are small SKILL.md files; read them as MonoCode does
        // right before the turn starts.
        let agent_prompt = self.apply_skills(prompt);
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
        let request = spawn_request(session, &agent_prompt, mode, self.claude_hooks_disabled).map(
            |request| SpawnRequest {
                attachments: input.attachments.clone(),
                plan: input.plan,
                ..request
            },
        );
        self.persist_session(session_id);
        let started = request
            .map_err(|message| (message, now))
            .and_then(|request| {
                harness::spawn(&request).map_err(|err| {
                    let message = format!("Failed to start {}: {err:#}", request.harness.label());
                    (message, now_ms())
                })
            });
        match started {
            Ok((handle, events)) => {
                let auto_approve = mode == PermissionMode::FullAccess && !input.plan;
                self.track_run(session_id.to_string(), handle, events, auto_approve, cx)
            }
            Err((message, at)) => {
                log::error!("{message}");
                if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
                    push_notice(session, &message, at);
                    finish_turn(session, at);
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
                        app.on_agent_event(&session_id, run_id, event);
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

    fn on_agent_event(&mut self, session_id: &str, run_id: u64, event: AgentEvent) {
        let Some(run) = self.current_run(session_id, run_id) else {
            return;
        };
        if let AgentEvent::PermissionRequest(request) = event {
            // MonoCode full access answers everything but a question.
            if run.auto_approve && request.tool != QUESTION_TOOL {
                if !run.handle.respond_permission(&request, true) {
                    log::warn!("harness rejected auto-approval for {}", request.request_id);
                }
                return;
            }
            let question = request.tool == QUESTION_TOOL;
            run.pending_permission = Some(request);
            // The form takes the keys, as MonoCode focuses its options.
            if question && self.selected_session_id.as_deref() == Some(session_id) {
                self.question_focus_wanted = true;
            }
            return;
        }
        let is_done = matches!(event, AgentEvent::Done(_));
        if let AgentEvent::Done(status) = event {
            run.outcome = Some(status);
        }
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            apply_event(session, event, now_ms());
        }
        if is_done {
            self.persist_session(session_id);
        }
    }

    fn finish_run(&mut self, session_id: &str, run_id: u64, cx: &mut Context<Self>) {
        if self.current_run(session_id, run_id).is_none() {
            return;
        }
        let Some(run) = self.runs.remove(session_id) else {
            return;
        };
        self.persist_session(session_id);
        self.close_automation_run(&run, automation_status(run.outcome));
        // The agent has most likely edited files.
        self.refresh_workspace(cx);
        self.send_next_queued(session_id, cx);
        cx.notify();
    }

    /// Starts the oldest queued prompt of a thread whose turn just ended.
    fn send_next_queued(&mut self, session_id: &str, cx: &mut Context<Self>) {
        // MonoCode holds the queue while one of its messages is being edited.
        if self
            .queue_editing
            .as_ref()
            .is_some_and(|(sid, _)| sid == session_id)
        {
            self.queue_held.insert(session_id.to_string());
            return;
        }
        let Some(queue) = self.prompt_queues.get_mut(session_id) else {
            return;
        };
        let next = queue.remove(0);
        if queue.is_empty() {
            self.prompt_queues.remove(session_id);
        }
        self.send_turn(session_id, next, cx);
    }

    /// Stops a thread's agent. Its queue is kept, but nothing is sent until
    /// the user sends again (MonoCode pauses the queue on interrupt).
    pub fn stop_agent(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(run) = self.runs.remove(session_id) else {
            return;
        };
        run.handle.cancel();
        self.close_automation_run(&run, "cancelled");
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

    fn close_automation_run(&self, run: &AgentRun, status: &str) {
        let Some(run_id) = &run.automation_run_id else {
            return;
        };
        let error = (status == "failed").then_some("The agent turn failed.");
        if let Err(err) = self.db.finish_automation_run(run_id, status, error) {
            log::error!("failed to close automation run {run_id}: {err:#}");
        }
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
        self.question_ui.remove(session_id);
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

    /// Writes one session back to SQLite, logging instead of swallowing errors.
    pub fn persist_session(&self, session_id: &str) {
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        if let Err(err) = self.db.upsert_session(session) {
            log::error!("failed to save session {session_id}: {err:#}");
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
        settings: catalog::resolved_settings(&session.model, session.model_settings.as_ref()),
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
        AgentEvent::Usage { total_tokens, .. } => {
            session.context_used = i64::try_from(total_tokens).ok();
        }
        AgentEvent::TurnMetrics(metrics) => record_turn_metrics(session, &metrics),
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
        assert_eq!(automation_status(Some(DoneStatus::Completed)), "succeeded");
        assert_eq!(automation_status(Some(DoneStatus::Cancelled)), "cancelled");
        assert_eq!(automation_status(None), "failed");
    }

    #[test]
    fn long_output_is_truncated_on_char_boundary() {
        let text = "ü".repeat(MAX_TOOL_OUTPUT_CHARS + 5);
        assert!(truncate(&text, MAX_TOOL_OUTPUT_CHARS).ends_with("(truncated)"));
    }
}
