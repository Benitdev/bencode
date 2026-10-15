//! What Claude runs besides the turn itself, ported from MonoCode
//! `claude.ts` (`handleAgentLifecycle`, `noteSubagent*`, `maybeFinishTurn`)
//! and `claudeProtocol.ts` (`parseTask*`): subagents and backgrounded
//! commands, reported as `system` records, and a subagent's own messages,
//! which carry the Agent call they belong to as `parent_tool_use_id`.
//!
//! Claude yields (`result`) with such tasks still going; each one's
//! notification wakes it for another turn in the same process. The BenCode
//! turn stays open through all of that: `Done` waits until Claude has
//! yielded with nothing left running.

use std::time::Instant;

use serde_json::{Value, json};

use super::{AgentTask, BackgroundTask, ClaudeParser, tool_results};
use crate::harness::events::{AgentEvent, AgentStep, StepKind};
use crate::harness::{str_field, summarize_tool_input};

/// A record a subagent wrote: its messages carry the Agent call's id.
pub(super) fn is_subagent_message(rec: &Value) -> bool {
    str_field(rec, "parent_tool_use_id").is_some_and(|id| !id.is_empty())
}

/// MonoCode `isAgentToolName`.
pub(super) fn is_agent_tool(name: &str) -> bool {
    matches!(name, "Agent" | "Task")
}

/// MonoCode `isAgentTaskType`.
fn is_agent_task_type(task_type: Option<&str>) -> bool {
    matches!(
        task_type.map(str::to_ascii_lowercase).as_deref(),
        Some("local_agent" | "remote_agent")
    )
}

/// MonoCode `isTerminalAgentTaskStatus`.
fn is_terminal_status(status: Option<&str>) -> bool {
    matches!(
        status.map(str::to_ascii_lowercase).as_deref(),
        Some("completed" | "failed" | "killed" | "stopped")
    )
}

fn flag(rec: &Value, key: &str) -> Option<bool> {
    rec.get(key).and_then(Value::as_bool)
}

impl ClaudeParser {
    /// MonoCode `handleAgentLifecycle`: `task_started`, `task_progress`,
    /// `task_updated`, `task_notification` and `background_tasks_changed`.
    /// Whether `rec` was one of them.
    pub(super) fn on_task_lifecycle(&mut self, rec: &Value, events: &mut Vec<AgentEvent>) -> bool {
        if str_field(rec, "type") != Some("system") {
            return false;
        }
        let task_id = str_field(rec, "task_id").map(String::from);
        match (str_field(rec, "subtype"), task_id) {
            (Some("task_started"), Some(id)) => self.on_task_started(rec, id, events),
            (Some("task_progress"), Some(id)) => self.on_task_progress(rec, &id, events),
            (Some("task_updated"), Some(id)) => self.on_task_updated(rec, &id, events),
            (Some("task_notification"), Some(id)) => {
                if flag(rec, "ambient") != Some(true) {
                    self.on_task_notification(rec, &id, events);
                }
            }
            (Some("background_tasks_changed"), _) => self.on_background_tasks(rec, events),
            (
                Some("task_started" | "task_progress" | "task_updated" | "task_notification"),
                None,
            ) => {}
            _ => return false,
        }
        true
    }

    fn on_task_started(&mut self, rec: &Value, id: String, events: &mut Vec<AgentEvent>) {
        if flag(rec, "ambient") == Some(true) {
            return;
        }
        let description = str_field(rec, "description")
            .unwrap_or("Subagent")
            .to_string();
        let tool_use_id = str_field(rec, "tool_use_id").map(String::from);
        if !self.background.iter().any(|task| task.id == id) {
            self.background.push(BackgroundTask {
                id: id.clone(),
                description: description.clone(),
                tool_use_id: tool_use_id.clone(),
            });
        }
        if is_agent_task_type(str_field(rec, "task_type")) {
            // Claude does not always say so on the record; the call's own
            // `run_in_background` does.
            let backgrounded = flag(rec, "is_backgrounded") == Some(true)
                || tool_use_id
                    .as_deref()
                    .is_some_and(|call| self.asked_background(call));
            let call_id = self.agent_row(tool_use_id.as_deref(), &description, events);
            self.agents.insert(
                id,
                AgentTask {
                    call_id,
                    backgrounded,
                },
            );
        }
        self.sync_background(events);
    }

    fn on_task_progress(&mut self, rec: &Value, id: &str, events: &mut Vec<AgentEvent>) {
        if self.agents.contains_key(id) {
            return;
        }
        // Progress for a run whose start never came: give it a row.
        let description = str_field(rec, "description").unwrap_or("Subagent");
        let tool_use_id = str_field(rec, "tool_use_id");
        let call_id = self.agent_row(tool_use_id, description, events);
        self.agents.insert(
            id.to_string(),
            AgentTask {
                call_id,
                backgrounded: true,
            },
        );
    }

    fn on_task_updated(&mut self, rec: &Value, id: &str, events: &mut Vec<AgentEvent>) {
        let patch = rec.get("patch").unwrap_or(&Value::Null);
        if let (Some(task), Some(backgrounded)) =
            (self.agents.get_mut(id), flag(patch, "is_backgrounded"))
        {
            task.backgrounded = backgrounded;
        }
        if let (Some(task), Some(description)) = (
            self.background.iter_mut().find(|task| task.id == id),
            str_field(patch, "description"),
        ) {
            task.description = description.to_string();
        }
        let status = str_field(patch, "status");
        if !is_terminal_status(status) {
            return;
        }
        let succeeded = status == Some("completed");
        let error = str_field(patch, "error");
        self.settle_background_row(id, succeeded, error, events);
        self.finish_background_task(id, events);
        self.complete_agent_task(id, succeeded, error, events);
    }

    fn on_task_notification(&mut self, rec: &Value, id: &str, events: &mut Vec<AgentEvent>) {
        let succeeded = str_field(rec, "status").is_none_or(|s| s == "completed");
        let summary = str_field(rec, "summary").filter(|s| !s.trim().is_empty());
        // MonoCode `noteTaskNotification`: a command's row takes the summary;
        // a subagent's is kept for the trail until Claude picks back up.
        if self.background_rows.contains_key(id) {
            self.settle_background_row(id, succeeded, summary, events);
        }
        // `task_updated` may have settled the run already; its report
        // comes only now, and finds the row by the call.
        let call_id = self
            .agents
            .get(id)
            .map(|task| task.call_id.clone())
            .or_else(|| {
                str_field(rec, "tool_use_id")
                    .filter(|call| {
                        self.tools
                            .get(*call)
                            .is_some_and(|(name, _)| is_agent_tool(name))
                    })
                    .map(String::from)
            });
        if let Some(call_id) = &call_id {
            if self.yielded.is_some() {
                self.task_notes
                    .push(summary.unwrap_or("Subagent finished.").to_string());
            }
            if !self.agents.contains_key(id)
                && let Some(summary) = summary
            {
                events.push(AgentEvent::ToolCallFinish {
                    id: call_id.clone(),
                    output: summary.to_string(),
                    success: succeeded,
                });
            }
        }
        self.finish_background_task(id, events);
        self.complete_agent_task(id, succeeded, summary, events);
    }

    /// Every task Claude runs for the session, whole.
    fn on_background_tasks(&mut self, rec: &Value, events: &mut Vec<AgentEvent>) {
        let listed: Vec<(String, Option<&str>, String)> = rec
            .get("tasks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|row| flag(row, "ambient") != Some(true))
            .filter_map(|row| {
                Some((
                    str_field(row, "task_id")?.to_string(),
                    str_field(row, "task_type"),
                    str_field(row, "description")
                        .unwrap_or("Subagent")
                        .to_string(),
                ))
            })
            .collect();
        let gone: Vec<String> = self
            .background
            .iter()
            .map(|task| task.id.clone())
            .filter(|id| !listed.iter().any(|(listed, ..)| listed == id))
            .collect();
        for id in &gone {
            self.settle_background_row(id, true, None, events);
            self.finish_background_task(id, events);
        }
        let stale: Vec<String> = self
            .agents
            .keys()
            .filter(|id| !listed.iter().any(|(listed, ..)| listed == *id))
            .cloned()
            .collect();
        for id in stale {
            self.complete_agent_task(&id, true, None, events);
        }
        for (id, task_type, description) in listed {
            if !self.background.iter().any(|task| task.id == id) {
                self.background.push(BackgroundTask {
                    id: id.clone(),
                    description: description.clone(),
                    tool_use_id: None,
                });
            }
            if !is_agent_task_type(task_type) || self.agents.contains_key(&id) {
                continue;
            }
            // The list carries no tool_use_id and often lands before
            // task_started: find the Agent call that spawned it rather
            // than opening a second row.
            let call = self.unclaimed_agent_call(&description);
            let call_id = self.agent_row(call.as_deref(), &description, events);
            self.agents.insert(
                id,
                AgentTask {
                    call_id,
                    backgrounded: true,
                },
            );
        }
        self.maybe_finish(events);
        self.sync_background(events);
    }

    /// Whether the parent's call `call_id` asked to run in the background.
    fn asked_background(&self, call_id: &str) -> bool {
        self.tools.get(call_id).is_some_and(|(_, input)| {
            input.get("run_in_background").and_then(Value::as_bool) == Some(true)
        })
    }

    /// MonoCode `isBackgroundedAgentTool`: its tool result only says it
    /// was launched; the report comes with its notification.
    pub(super) fn is_backgrounded_agent(&self, call_id: &str) -> bool {
        self.agents
            .values()
            .any(|task| task.call_id == call_id && task.backgrounded)
            || self.asked_background(call_id)
    }

    /// MonoCode `unclaimedAgentCall`: the latest Agent call with this
    /// description that no task has claimed yet.
    fn unclaimed_agent_call(&self, description: &str) -> Option<String> {
        self.agent_calls
            .iter()
            .rev()
            .filter(|id| !self.agents.values().any(|task| &task.call_id == *id))
            .find(|id| {
                self.tools
                    .get(*id)
                    .and_then(|(_, input)| str_field(input, "description"))
                    == Some(description)
            })
            .cloned()
    }

    /// MonoCode `upsertAgentTool`: the row a run reports on. Its Agent call
    /// when the transcript has it, else a row of its own.
    fn agent_row(
        &mut self,
        call_id: Option<&str>,
        description: &str,
        events: &mut Vec<AgentEvent>,
    ) -> String {
        if let Some(call) = call_id.filter(|id| self.tools.contains_key(*id)) {
            return call.to_string();
        }
        let id = call_id.map_or_else(|| format!("agent:{description}"), String::from);
        if !self.tools.contains_key(&id) {
            let input = json!({ "description": description });
            self.tools
                .insert(id.clone(), ("Agent".into(), input.clone()));
            self.agent_calls.push(id.clone());
            events.push(AgentEvent::ToolCallStart {
                id: id.clone(),
                name: "Agent".into(),
                input,
            });
        }
        id
    }

    /// MonoCode `completeAgentTask`: the run's row settles with what it
    /// reported.
    fn complete_agent_task(
        &mut self,
        task_id: &str,
        succeeded: bool,
        detail: Option<&str>,
        events: &mut Vec<AgentEvent>,
    ) {
        if let Some(task) = self.agents.remove(task_id) {
            let output = detail
                .map(String::from)
                .unwrap_or_else(|| if succeeded { "" } else { "Subagent failed." }.to_string());
            events.push(AgentEvent::ToolCallFinish {
                id: task.call_id,
                output,
                success: succeeded,
            });
        }
        self.maybe_finish(events);
    }

    /// MonoCode `settleInlineAgentTask`: a subagent that was never
    /// backgrounded reports back on the parent's own tool result, and
    /// Claude sends no task record for one that ended inline.
    pub(super) fn settle_inline_agent(&mut self, call_id: &str, events: &mut Vec<AgentEvent>) {
        let settled: Vec<String> = self
            .agents
            .iter()
            .filter(|(_, task)| task.call_id == call_id && !task.backgrounded)
            .map(|(id, _)| id.clone())
            .collect();
        if settled.is_empty() {
            return;
        }
        for id in &settled {
            self.agents.remove(id);
            self.background.retain(|task| &task.id != id);
        }
        self.maybe_finish(events);
        self.sync_background(events);
    }

    /// MonoCode `finishBackgroundTask`: once Claude has yielded, the
    /// notification about a finished task starts a follow-up turn, so the
    /// BenCode turn is held open for that too.
    fn finish_background_task(&mut self, task_id: &str, events: &mut Vec<AgentEvent>) {
        let before = self.background.len();
        self.background.retain(|task| task.id != task_id);
        if self.background.len() == before {
            return;
        }
        if self.yielded.is_none() {
            self.wake_queued = true;
        } else if self.awaiting_resume.is_none() {
            self.awaiting_resume = Some(Instant::now());
        }
        self.maybe_finish(events);
        self.sync_background(events);
    }

    /// Claude yielded. A task that finished during the turn has its
    /// notification queued behind this result, and it starts the next
    /// turn: wait for that as for one finishing now. (Claude may have taken
    /// it in mid-turn instead; then the grace lets the turn go.)
    pub(super) fn note_yielded(&mut self) {
        if std::mem::take(&mut self.wake_queued) && self.awaiting_resume.is_none() {
            self.awaiting_resume = Some(Instant::now());
        }
    }

    /// MonoCode `noteClaudeTurnStarted`: Claude began another turn inside
    /// this one, woken by a finished task or by a follow-up written in
    /// while it waited. Its own result decides when the turn ends.
    pub(super) fn note_turn_started(&mut self, events: &mut Vec<AgentEvent>) {
        if self.yielded.take().is_none() {
            return;
        }
        self.awaiting_resume = None;
        // The notes also close the message Claude left off with, so the
        // reply starts a block of its own.
        for note in self.task_notes.drain(..) {
            events.push(AgentEvent::Status(note));
        }
        self.sync_background(events);
    }

    /// MonoCode `showBackgroundRows`: Claude yielded with commands still
    /// running; each gets a live row under the message it left off with.
    /// Subagents already have a row of their own.
    pub(super) fn show_background_rows(&mut self, events: &mut Vec<AgentEvent>) {
        if self.finished {
            return;
        }
        for task in &self.background {
            if self.background_rows.contains_key(&task.id) || self.agents.contains_key(&task.id) {
                continue;
            }
            let source = task.tool_use_id.as_ref().and_then(|id| self.tools.get(id));
            if source.is_some_and(|(name, _)| is_agent_tool(name)) {
                continue;
            }
            let call_id = format!("background:{}", task.id);
            self.background_rows
                .insert(task.id.clone(), call_id.clone());
            let (name, input) = source
                .cloned()
                .unwrap_or_else(|| ("Bash".into(), json!({ "description": task.description })));
            events.push(AgentEvent::ToolCallStart {
                id: call_id,
                name,
                input,
            });
        }
    }

    fn settle_background_row(
        &mut self,
        task_id: &str,
        succeeded: bool,
        detail: Option<&str>,
        events: &mut Vec<AgentEvent>,
    ) {
        if let Some(call_id) = self.background_rows.get(task_id) {
            events.push(AgentEvent::ToolCallFinish {
                id: call_id.clone(),
                output: detail.unwrap_or_default().to_string(),
                success: succeeded,
            });
        }
    }

    /// MonoCode `syncBackgroundWait`: what the turn waits on once Claude
    /// has yielded with work still running.
    pub(super) fn sync_background(&mut self, events: &mut Vec<AgentEvent>) {
        let waiting: Vec<String> = if self.yielded.is_some() && !self.finished {
            self.background
                .iter()
                .map(|task| task.description.clone())
                .collect()
        } else {
            Vec::new()
        };
        if waiting == self.background_sent {
            return;
        }
        self.background_sent = waiting.clone();
        events.push(AgentEvent::Background(waiting));
    }

    /// MonoCode `maybeFinishTurn`: the turn ends once Claude has yielded
    /// and nothing it started still runs or is about to wake it.
    pub(super) fn maybe_finish(&mut self, events: &mut Vec<AgentEvent>) {
        let Some(status) = self.yielded else {
            return;
        };
        if self.finished
            || !self.agents.is_empty()
            || !self.background.is_empty()
            || self.awaiting_resume.is_some()
        {
            return;
        }
        self.finished = true;
        if !self.background_sent.is_empty() {
            self.background_sent.clear();
            events.push(AgentEvent::Background(Vec::new()));
        }
        events.push(AgentEvent::Done(status));
    }

    /// A subagent's own record, mirrored onto the Agent call that spawned
    /// it (MonoCode `noteSubagentNarration`, `noteSubagentTool`,
    /// `noteSubagentResults`). Its prose never joins the parent
    /// transcript: that would read as the main agent talking.
    pub(super) fn on_subagent(&mut self, rec: &Value, events: &mut Vec<AgentEvent>) {
        let Some(parent) = str_field(rec, "parent_tool_use_id")
            .filter(|id| {
                self.tools
                    .get(*id)
                    .is_some_and(|(name, _)| is_agent_tool(name))
            })
            .map(String::from)
        else {
            return;
        };
        match str_field(rec, "type") {
            Some("assistant") => self.subagent_message(rec, &parent, events),
            Some("user") => {
                for (id, output, success) in tool_results(rec) {
                    let detail = (!success && !output.trim().is_empty()).then_some(output);
                    events.push(AgentEvent::AgentStep {
                        call_id: parent.clone(),
                        step: AgentStep {
                            id,
                            kind: StepKind::Tool,
                            text: String::new(),
                            tool_name: None,
                            status: Some(if success { "completed" } else { "failed" }.into()),
                            detail,
                        },
                    });
                }
            }
            Some("tool_progress") => self.on_tool_progress(rec, events),
            _ => {}
        }
    }

    fn subagent_message(&mut self, rec: &Value, parent: &str, events: &mut Vec<AgentEvent>) {
        let Some(message) = rec.get("message") else {
            return;
        };
        if let Some(model) = str_field(message, "model")
            && self.agent_models.get(parent).map(String::as_str) != Some(model)
        {
            self.agent_models
                .insert(parent.to_string(), model.to_string());
            events.push(AgentEvent::AgentModel {
                call_id: parent.to_string(),
                model: model.to_string(),
            });
        }
        let message_id = str_field(message, "id").unwrap_or("message");
        let content = message
            .get("content")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let joined = |kind: &str, field: &str| -> String {
            content
                .iter()
                .filter(|block| str_field(block, "type") == Some(kind))
                .filter_map(|block| str_field(block, field))
                .collect::<String>()
                .trim()
                .to_string()
        };
        let prose = [
            (
                StepKind::Reasoning,
                "thinking",
                joined("thinking", "thinking"),
            ),
            (StepKind::Message, "text", joined("text", "text")),
        ];
        for (kind, suffix, text) in prose {
            if text.is_empty() {
                continue;
            }
            events.push(AgentEvent::AgentStep {
                call_id: parent.to_string(),
                step: AgentStep {
                    id: format!("{message_id}:{suffix}"),
                    kind,
                    text,
                    tool_name: None,
                    status: None,
                    detail: None,
                },
            });
        }
        for block in content
            .iter()
            .filter(|block| str_field(block, "type") == Some("tool_use"))
        {
            let Some(id) = str_field(block, "id") else {
                continue;
            };
            let name = str_field(block, "name").unwrap_or("tool");
            let input = block.get("input").unwrap_or(&Value::Null);
            events.push(AgentEvent::AgentStep {
                call_id: parent.to_string(),
                step: AgentStep {
                    id: id.to_string(),
                    kind: StepKind::Tool,
                    text: summarize_tool_input(name, input),
                    tool_name: Some(name.to_string()),
                    status: Some("in_progress".into()),
                    detail: None,
                },
            });
        }
    }

    /// MonoCode `handleToolProgress`: the call in flight inside a run is a
    /// step of it.
    pub(super) fn on_tool_progress(&mut self, rec: &Value, events: &mut Vec<AgentEvent>) {
        let Some(id) = str_field(rec, "tool_use_id") else {
            return;
        };
        let agent_call = |call: &str| {
            self.tools
                .get(call)
                .is_some_and(|(name, _)| is_agent_tool(name))
        };
        let parent = [Some(id), str_field(rec, "parent_tool_use_id")]
            .into_iter()
            .flatten()
            .find(|call| agent_call(call));
        let (Some(parent), Some(tool)) = (parent, str_field(rec, "tool_name")) else {
            return;
        };
        if parent == id {
            // Progress on the Agent call itself names nothing to show.
            return;
        }
        events.push(AgentEvent::AgentStep {
            call_id: parent.to_string(),
            step: AgentStep {
                id: id.to_string(),
                kind: StepKind::Tool,
                text: tool.to_string(),
                tool_name: Some(tool.to_string()),
                status: Some("in_progress".into()),
                detail: None,
            },
        });
    }
}
