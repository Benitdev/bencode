//! The Agent Client Protocol as BenCode speaks it to an agent CLI over
//! stdio, after MonoCode's `grokProtocol.ts` and `antigravity.ts`: each turn
//! starts its own agent process, binds the thread's session
//! (`session/resume`, else `session/load`, else `session/new`), puts it on
//! the thread's model and mode, sends one `session/prompt`, and closes stdin
//! once the prompt returns. What differs between CLIs is a `Dialect`
//! (`grok.rs`, `antigravity.rs`). Grok's `x.ai` extensions are read
//! wherever they appear; only Grok sends them.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::harness::attachments::Attachment;
use crate::harness::events::{
    AgentEvent, DoneStatus, PermissionRequest, TaskItem, TaskStatus, TurnMetrics,
};
use crate::harness::process::LineParser;
use crate::harness::{PermissionPolicy, SpawnRequest, str_field};

/// The tool name the composer's question form answers (`app::QUESTION_TOOL`).
pub(crate) const QUESTION_TOOL: &str = "AskUserQuestion";
/// JSON-RPC "method not found".
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
const MAX_DETAIL_CHARS: usize = 8_000;
/// How long a turn whose prompt returned with tool calls still running
/// waits, with nothing heard from the agent, before it is let go anyway.
pub(crate) const OPEN_TOOLS_GRACE: Duration = Duration::from_secs(60);

/// Where one CLI's ACP differs from another's.
pub struct Dialect {
    /// The harness's name in messages: "Grok Build".
    pub label: &'static str,
    /// Its name in the log: "grok".
    pub log: &'static str,
    /// The sign-in to `authenticate` with after `initialize`, if any.
    pub auth_method: fn(&Value) -> Option<String>,
    /// Why the agent did not start, from what it said.
    pub start_error: fn(&str) -> String,
    /// A failed prompt's message, with what to do about it.
    pub prompt_error: fn(String) -> String,
    /// `session/new`'s params: cwd, access, plan mode and MCP servers.
    pub new_session_params: fn(&str, PermissionPolicy, bool, &Value) -> Value,
    /// Extra `_meta` attached to `session/new`, `session/resume` and
    /// `session/load` (Antigravity's `systemPrompt.append`).
    pub session_meta: fn() -> Option<Value>,
    /// `session/resume` names the cwd and MCP servers even when there are
    /// none (Grok's resume has always been sent bare).
    pub resume_with_cwd: bool,
    /// The session mode a turn runs in (`session/set_mode`); `None` leaves
    /// it to the CLI's flags.
    pub mode: fn(PermissionPolicy, bool) -> Option<&'static str>,
    /// The request that compacts a session's context, if the CLI has one.
    pub compact: Option<&'static str>,
    /// A follow-up reaches the running turn (`x.ai/interject`); else it
    /// waits to be the next prompt.
    pub interject: bool,
    /// `cachedReadTokens` are part of `inputTokens` (xAI's API) rather than
    /// beside them.
    pub cached_in_input: bool,
    /// In Auto, what the agent still asks is approved (MonoCode's Grok);
    /// else it goes to the user, as Antigravity's own Auto mode asks only
    /// before what leaves its sandbox.
    pub approves_in_auto: bool,
    /// A prompt that returns `end_turn` while tool calls it started still
    /// run (Antigravity's background commands) ends the turn only once they
    /// finish, or after `OPEN_TOOLS_GRACE` of silence; the process stays up
    /// meanwhile so they are not killed with it. No prompt is added.
    pub wait_open_tools: bool,
}

/// MonoCode `CLIENT_CAPABILITIES` and `clientInfo`.
pub fn initialize_params() -> Value {
    json!({
        "protocolVersion": 1,
        "clientCapabilities": {
            "fs": { "readTextFile": false, "writeTextFile": false },
            "terminal": false,
        },
        "clientInfo": { "name": "bencode", "version": env!("CARGO_PKG_VERSION") },
    })
}

/// The MCP servers a session gets, in ACP's stdio shape: those BenCode
/// offers this turn (`SpawnRequest::mcp_servers`), maybe none.
pub fn mcp_servers(offered: &[crate::mcp::McpLaunch]) -> Value {
    Value::Array(
        offered
            .iter()
            .map(|launch| {
                json!({
                    "name": launch.name,
                    "command": launch.command,
                    "args": launch.args,
                    "env": [],
                })
            })
            .collect(),
    )
}

/// What a client request was for, so its response knows the next step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Initialize,
    Authenticate,
    Resume,
    Load,
    New,
    /// Putting the session on the turn's model or mode.
    Setup,
    Prompt,
    Compact,
}

pub struct AcpParser {
    dialect: &'static Dialect,
    cwd: String,
    /// `mcp_servers(..)`, for whichever way the session is bound.
    mcp_servers: Value,
    prompt: Vec<Value>,
    resume_id: Option<String>,
    model: Option<String>,
    permission: PermissionPolicy,
    plan: bool,
    /// Compact the session's context instead of prompting it.
    compact: bool,
    next_id: u64,
    /// The one handshake or prompt request in flight.
    waiting: Option<(u64, Step)>,
    /// What to ask of a bound session before prompting it.
    setup: VecDeque<(&'static str, Value)>,
    session_id: Option<String>,
    /// `session/load` replays the thread's history; that is not new output.
    muted: bool,
    context_window: Option<u64>,
    /// The context in use, as the agent last reported it.
    context_used: Option<u64>,
    /// What the context held after the last compaction Grok reported.
    compacted_to: Option<u64>,
    /// This prompt's token counts are out already (`turn_completed` and
    /// the prompt's own result both carry them).
    metrics_reported: bool,
    /// Tool calls started, and whether their start carried an input yet.
    tools: HashMap<String, bool>,
    finished_tools: HashSet<String>,
    /// Follow-ups handed to the running turn, by request id, until Grok
    /// acknowledges them; one it refuses becomes a prompt of its own.
    interjections: HashMap<u64, Vec<Value>>,
    /// Follow-ups to prompt with once the running prompt returns.
    followups: Vec<Value>,
    /// The prompt returned while follow-ups were unacknowledged; the turn
    /// ends with their answers.
    /// With whether the model ended it (`finish_prompt`).
    finishing: Option<(DoneStatus, bool)>,
    /// The prompt returned while tool calls still ran: the turn ends when
    /// they finish (`wait_open_tools`).
    awaiting_tools: bool,
    replies: Vec<String>,
}

impl AcpParser {
    pub fn new(req: &SpawnRequest, dialect: &'static Dialect) -> Self {
        Self {
            dialect,
            cwd: req.cwd.clone(),
            mcp_servers: mcp_servers(&req.mcp_servers),
            prompt: crate::harness::attachments::acp_prompt_blocks(&req.prompt, &req.attachments),
            resume_id: req.resume_id.clone().filter(|id| !id.trim().is_empty()),
            model: req.model.clone(),
            permission: req.permission,
            plan: req.plan,
            compact: req.compact,
            next_id: 0,
            waiting: None,
            setup: VecDeque::new(),
            session_id: None,
            muted: false,
            context_window: None,
            context_used: None,
            compacted_to: None,
            metrics_reported: false,
            tools: HashMap::new(),
            finished_tools: HashSet::new(),
            interjections: HashMap::new(),
            followups: Vec::new(),
            finishing: None,
            awaiting_tools: false,
            replies: Vec::new(),
        }
    }

    /// The first line on stdin.
    pub fn initialize(&mut self) -> String {
        self.request_line("initialize", initialize_params(), Step::Initialize)
    }

    fn request_line(&mut self, method: &str, params: Value, step: Step) -> String {
        self.next_id += 1;
        self.waiting = Some((self.next_id, step));
        json!({ "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params })
            .to_string()
    }

    fn request(&mut self, method: &str, params: Value, step: Step) {
        let line = self.request_line(method, params, step);
        self.replies.push(line);
    }

    fn respond(&mut self, id: &Value, result: Value) {
        self.replies
            .push(json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string());
    }

    fn on_response(
        &mut self,
        step: Step,
        outcome: Result<&Value, String>,
        events: &mut Vec<AgentEvent>,
    ) {
        match (step, outcome) {
            (Step::Initialize, Ok(init)) => {
                self.context_window = context_window(init);
                match (self.dialect.auth_method)(init) {
                    Some(method) => self.request(
                        "authenticate",
                        json!({ "methodId": method, "_meta": { "headless": true } }),
                        Step::Authenticate,
                    ),
                    None => self.open_session(events),
                }
            }
            (Step::Initialize, Err(err)) => fail((self.dialect.start_error)(&err), events),
            // MonoCode goes on either way: a cached token may still work.
            (Step::Authenticate, outcome) => {
                if let Err(err) = outcome {
                    log::debug!("{} authenticate: {err}", self.dialect.log);
                }
                self.open_session(events);
            }
            (Step::Resume | Step::Load, Ok(result)) => {
                self.muted = false;
                let fallback = self.resume_id.clone();
                self.bind(result, fallback, events);
            }
            (Step::Resume, Err(err)) => {
                log::debug!(
                    "{} session/resume: {err}; loading instead",
                    self.dialect.log
                );
                let id = self.resume_id.clone().unwrap_or_default();
                self.muted = true;
                let mut params =
                    json!({ "sessionId": id, "cwd": self.cwd, "mcpServers": self.mcp_servers });
                self.attach_session_meta(&mut params);
                self.request("session/load", params, Step::Load);
            }
            (Step::Load, Err(err)) => {
                self.muted = false;
                if self.compact {
                    fail(format!("{} ({err})", self.nothing_to_compact()), events);
                } else {
                    let log = self.dialect.log;
                    log::debug!("{log} session/load: {err}; starting a new session");
                    self.new_session();
                }
            }
            (Step::New, Ok(result)) => self.bind(result, None, events),
            (Step::New, Err(err)) => fail((self.dialect.start_error)(&err), events),
            (Step::Setup, outcome) => {
                if let Err(err) = outcome {
                    log::debug!("{} session setup: {err}", self.dialect.log);
                }
                self.next_setup(events);
            }
            (Step::Prompt, Ok(result)) => {
                events.extend(self.usage_events(result));
                let stop = str_field(result, "stopReason");
                let status = match stop {
                    Some("cancelled") => DoneStatus::Cancelled,
                    _ => DoneStatus::Completed,
                };
                // Only a turn the model ended itself waits on its tools; one
                // cut short (`max_tokens`, `refusal`…) will not pick them up.
                let ended = matches!(stop, None | Some("end_turn"));
                self.finish_prompt(status, ended, events);
            }
            (Step::Prompt, Err(err)) => fail((self.dialect.prompt_error)(err), events),
            (Step::Compact, Ok(result)) => {
                let tokens_after = ["tokens_after", "tokensAfter", "post_tokens", "postTokens"]
                    .iter()
                    .find_map(|key| result.get(*key)?.as_u64())
                    .or(self.compacted_to);
                events.push(AgentEvent::Compacted { tokens_after });
                events.push(AgentEvent::Done(DoneStatus::Completed));
            }
            (Step::Compact, Err(err)) => fail(err, events),
        }
    }

    fn attach_session_meta(&self, params: &mut Value) {
        if let Some(meta) = (self.dialect.session_meta)()
            && let Some(obj) = params.as_object_mut()
        {
            obj.insert("_meta".into(), meta);
        }
    }

    fn has_unfinished_tools(&self) -> bool {
        self.tools
            .keys()
            .any(|id| !self.finished_tools.contains(id))
    }

    /// The last tool the turn waited on finished: the turn ends.
    fn settle_open_tools(&mut self, events: &mut Vec<AgentEvent>) {
        if self.awaiting_tools && !self.has_unfinished_tools() {
            self.awaiting_tools = false;
            events.push(AgentEvent::Done(DoneStatus::Completed));
        }
    }

    fn open_session(&mut self, events: &mut Vec<AgentEvent>) {
        match self.resume_id.clone() {
            Some(id) => {
                let mut params = json!({ "sessionId": id });
                let servers = self.mcp_servers.as_array().is_some_and(|s| !s.is_empty());
                if servers || self.dialect.resume_with_cwd {
                    params["cwd"] = json!(self.cwd);
                    params["mcpServers"] = self.mcp_servers.clone();
                }
                self.attach_session_meta(&mut params);
                self.request("session/resume", params, Step::Resume)
            }
            None if self.compact => fail(self.nothing_to_compact(), events),
            None => self.new_session(),
        }
    }

    fn new_session(&mut self) {
        let mut params = (self.dialect.new_session_params)(
            &self.cwd,
            self.permission,
            self.plan,
            &self.mcp_servers,
        );
        self.attach_session_meta(&mut params);
        self.request("session/new", params, Step::New);
    }

    fn nothing_to_compact(&self) -> String {
        format!(
            "{} has no saved session for this chat to compact.",
            self.dialect.label
        )
    }

    /// The session is ready: announce it, then compact it, or put it on the
    /// thread's model and send the prompt.
    fn bind(&mut self, result: &Value, fallback: Option<String>, events: &mut Vec<AgentEvent>) {
        let Some(id) = session_id(result).or(fallback) else {
            let label = self.dialect.label;
            fail(format!("{label} did not return a session id"), events);
            return;
        };
        self.session_id = Some(id.clone());
        events.push(AgentEvent::SessionStarted {
            provider_session_id: id.clone(),
        });
        if let Some(window) = context_window(result) {
            self.context_window = Some(window);
        }
        if self.compact {
            match self.dialect.compact {
                // MonoCode `compactGrokContext`.
                Some(method) => self.request(method, json!({ "sessionId": id }), Step::Compact),
                None => fail(self.nothing_to_compact(), events),
            }
            return;
        }
        // A resumed session keeps the model and mode it had.
        let current = current_model_id(result);
        if let Some(model) = self.model.clone()
            && current.is_some_and(|current| current != model)
        {
            let params = json!({ "sessionId": id, "modelId": model });
            self.setup.push_back(("session/set_model", params));
        }
        if let Some(mode) = (self.dialect.mode)(self.permission, self.plan)
            && current_mode_id(result).as_deref() != Some(mode)
        {
            let params = json!({ "sessionId": id, "modeId": mode });
            self.setup.push_back(("session/set_mode", params));
        }
        self.next_setup(events);
    }

    /// The next setup request, or the prompt once there is none left; a
    /// failed one is logged and the turn goes on as the session is.
    fn next_setup(&mut self, events: &mut Vec<AgentEvent>) {
        match self.setup.pop_front() {
            Some((method, params)) => self.request(method, params, Step::Setup),
            None => self.send_prompt(events),
        }
    }

    fn send_prompt(&mut self, events: &mut Vec<AgentEvent>) {
        if self.prompt.is_empty() {
            events.push(AgentEvent::Done(DoneStatus::Completed));
            return;
        }
        self.metrics_reported = false;
        let params = json!({ "sessionId": self.session_id, "prompt": self.prompt });
        self.request("session/prompt", params, Step::Prompt);
    }

    /// The prompt returned. Follow-ups Grok could not take mid-turn are
    /// prompted now, as one more turn of this run; ones it has not answered
    /// for yet are waited on first. A prompt the model ended (`ended`) with
    /// tool calls still running waits for them (`wait_open_tools`).
    fn finish_prompt(&mut self, status: DoneStatus, ended: bool, events: &mut Vec<AgentEvent>) {
        if status != DoneStatus::Completed {
            events.push(AgentEvent::Done(status));
        } else if !self.interjections.is_empty() {
            self.finishing = Some((status, ended));
        } else if !self.followups.is_empty() {
            self.prompt = std::mem::take(&mut self.followups);
            self.send_prompt(events);
        } else if self.dialect.wait_open_tools && ended && self.has_unfinished_tools() {
            self.awaiting_tools = true;
        } else {
            events.push(AgentEvent::Done(status));
        }
    }

    /// Grok answered a follow-up handed to the running turn.
    fn on_interjection(
        &mut self,
        blocks: Vec<Value>,
        outcome: Result<&Value, String>,
        events: &mut Vec<AgentEvent>,
    ) {
        if let Err(err) = outcome {
            // An agent without `x.ai/interject`, or a turn that just ended.
            log::debug!("grok interject: {err}; sending it as the next prompt");
            self.followups.extend(blocks);
        }
        if self.interjections.is_empty()
            && let Some((status, ended)) = self.finishing.take()
        {
            self.finish_prompt(status, ended, events);
        }
    }

    /// Whether the prompt is with the model now, so a follow-up can go into
    /// its turn.
    pub(crate) fn prompt_running(&self) -> bool {
        matches!(self.waiting, Some((_, Step::Prompt))) && self.finishing.is_none()
    }

    fn on_notification(&mut self, method: &str, params: &Value, events: &mut Vec<AgentEvent>) {
        let update = match method {
            "session/update" | "_x.ai/session/update" | "x.ai/session/update" => params,
            "_x.ai/session_notification" | "x.ai/session_notification" => {
                unwrap_session_notification(params)
            }
            _ => return,
        };
        if self.muted {
            return;
        }
        self.on_update(update, events);
        self.settle_open_tools(events);
        // Grok stamps its updates with the context the last model call
        // used: the meter's number, where a turn's totals count every call.
        let used = params.pointer("/_meta/totalTokens").and_then(Value::as_u64);
        if let Some(used) = used.filter(|used| self.context_used != Some(*used)) {
            self.context_used = Some(used);
            events.push(AgentEvent::Usage {
                input_tokens: 0,
                output_tokens: 0,
                total_tokens: used,
                context_window: self.context_window,
            });
        }
    }

    /// MonoCode `eventsFromAcpUpdate`.
    fn on_update(&mut self, params: &Value, events: &mut Vec<AgentEvent>) {
        let update = params
            .get("update")
            .filter(|u| u.is_object())
            .unwrap_or(params);
        let kind = ["sessionUpdate", "session_update", "type"]
            .iter()
            .find_map(|key| str_field(update, key))
            .unwrap_or_default();
        let content = || update.get("content").or_else(|| update.get("text"));
        match kind {
            "agent_message_chunk" | "agent_message" => {
                let separator = if kind == "agent_message" { "\n" } else { "" };
                let text = text_from_content(content(), separator);
                if !text.is_empty() {
                    events.push(AgentEvent::TextDelta(text));
                }
            }
            "agent_thought_chunk" | "agent_thought" => {
                let separator = if kind == "agent_thought" { "\n" } else { "" };
                let text = text_from_content(content(), separator);
                if !text.is_empty() {
                    events.push(AgentEvent::ThinkingDelta(text));
                }
            }
            "tool_call" | "tool_call_update" | "tool_call_content_chunk" => {
                self.on_tool(update, events)
            }
            // Announced before the call has its input; `tool_call` follows.
            "tool_call_delta_chunk" | "session_summary_generated" => {}
            "plan" | "current_plan" => match update.get("entries").or_else(|| update.get("plan")) {
                Some(Value::Array(entries)) => events.push(AgentEvent::Tasks(task_items(entries))),
                _ => {
                    if let Some(text) = str_field(update, "text").filter(|t| !t.trim().is_empty()) {
                        events.push(AgentEvent::TextDelta(text.to_string()));
                    }
                }
            },
            // ACP's own meter: the context in use and the window.
            "usage_update" => {
                let number = |key: &str| update.get(key).and_then(Value::as_u64);
                if let Some(window) = number("size") {
                    self.context_window = Some(window);
                }
                // A new session opens at 0: nothing to show yet.
                let used = number("used").filter(|used| *used > 0);
                if let Some(used) = used.filter(|used| self.context_used != Some(*used)) {
                    self.context_used = Some(used);
                    events.push(AgentEvent::Usage {
                        input_tokens: 0,
                        output_tokens: 0,
                        total_tokens: used,
                        context_window: self.context_window,
                    });
                }
            }
            // Grok compacted by itself, or for `compact_conversation`.
            "auto_compact_completed" => {
                self.compacted_to = update.get("tokens_after").and_then(Value::as_u64);
                if !self.compact {
                    events.push(AgentEvent::Compacted {
                        tokens_after: self.compacted_to,
                    });
                }
            }
            _ => events.extend(self.usage_events(update)),
        }
    }

    fn on_tool(&mut self, update: &Value, events: &mut Vec<AgentEvent>) {
        let tool = ["toolCall", "tool_call"]
            .iter()
            .find_map(|key| update.get(*key).filter(|t| t.is_object()))
            .unwrap_or(update);
        let fields = ToolFields::read(update, tool);
        let Some(id) = fields.call_id.clone() else {
            return;
        };
        // `todo_write`: the `plan` update beside it is the task list.
        if fields.kind.as_deref() == Some("plan") {
            self.finished_tools.insert(id);
            return;
        }
        if self.finished_tools.contains(&id) && !self.tools.contains_key(&id) {
            return;
        }
        self.start_tool(&id, &fields, events);
        let status = str_field(update, "status").or_else(|| str_field(tool, "status"));
        if let Some(status @ ("completed" | "failed")) = status
            && self.finished_tools.insert(id.clone())
        {
            events.push(AgentEvent::ToolCallFinish {
                id,
                output: tool_detail(update, tool),
                success: status == "completed",
                patch: None,
            });
        }
    }

    /// Starts a call once, and again when its first start had no input (the
    /// transcript retitles it).
    fn start_tool(&mut self, id: &str, fields: &ToolFields, events: &mut Vec<AgentEvent>) {
        let has_input = fields.input.is_some();
        match self.tools.get(id) {
            Some(true) => return,
            Some(false) if !has_input => return,
            _ => {}
        }
        self.tools.insert(id.to_string(), has_input);
        events.push(AgentEvent::ToolCallStart {
            id: id.to_string(),
            name: fields.name(),
            input: fields.summary_input(),
        });
    }

    fn on_request(
        &mut self,
        id: &Value,
        method: &str,
        params: &Value,
        events: &mut Vec<AgentEvent>,
    ) {
        match method {
            "session/request_permission" => self.on_permission(id, params, events),
            "_x.ai/ask_user_question" | "x.ai/ask_user_question" => {
                let questions = questions_from(params);
                let description = questions
                    .first()
                    .and_then(|q| str_field(q, "header").or_else(|| str_field(q, "question")))
                    .unwrap_or("Question")
                    .to_string();
                events.push(AgentEvent::PermissionRequest(PermissionRequest {
                    request_id: json!({ "id": id, "question": true }).to_string(),
                    tool: QUESTION_TOOL.into(),
                    description,
                    input: json!({ "questions": questions }),
                }));
            }
            "_x.ai/exit_plan_mode" | "x.ai/exit_plan_mode" => {
                let plan = plan_from_exit_plan(params);
                if !plan.is_empty() {
                    events.push(AgentEvent::TextDelta(plan));
                }
                // Ends the plan turn without approving it: building is a
                // turn of its own, as in MonoCode.
                self.respond(id, json!({ "outcome": "abandoned" }));
            }
            _ => self.replies.push(
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": METHOD_NOT_FOUND, "message": format!("Method not found: {method}") },
                })
                .to_string(),
            ),
        }
    }

    fn on_permission(&mut self, id: &Value, params: &Value, events: &mut Vec<AgentEvent>) {
        let tool = ["toolCall", "tool_call"]
            .iter()
            .find_map(|key| params.get(*key).filter(|t| t.is_object()))
            .or_else(|| params.pointer("/subject/toolCall"))
            .or_else(|| params.get("subject").filter(|s| s.is_object()))
            .unwrap_or(params);
        let fields = ToolFields::read(tool, tool);
        let options: Vec<&str> = params
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|o| str_field(o, "optionId").or_else(|| str_field(o, "option_id")))
            .collect();
        let kind = fields.kind.as_deref().unwrap_or("").to_ascii_lowercase();
        if let Some(call_id) = fields.call_id.clone().filter(|_| kind != "plan") {
            self.start_tool(&call_id, &fields, events);
        }
        let auto = if self.plan {
            // Plan mode reads, searches and keeps its todo list; it changes
            // nothing.
            let read_only = matches!(kind.as_str(), "read" | "search" | "list" | "plan");
            Some(option_id(read_only, &options))
        } else {
            let permission = match self.permission {
                PermissionPolicy::Auto if !self.dialect.approves_in_auto => PermissionPolicy::Ask,
                permission => permission,
            };
            auto_option(permission, &kind, &options).map(String::from)
        };
        if let Some(option) = auto {
            self.respond(id, selected(&option));
            return;
        }
        let input = fields.summary_input();
        events.push(AgentEvent::PermissionRequest(PermissionRequest {
            request_id: json!({
                "id": id,
                "allow": option_id(true, &options),
                "deny": option_id(false, &options),
            })
            .to_string(),
            description: fields
                .title
                .clone()
                .unwrap_or_else(|| crate::harness::summarize_tool_input(&fields.name(), &input)),
            tool: fields.name(),
            input,
        }));
    }

    /// MonoCode `usageFromUpdate`: the context meter and the turn's tokens.
    fn usage_events(&mut self, update: &Value) -> Vec<AgentEvent> {
        let usage = ["usage", "tokenUsage", "token_usage", "_meta"]
            .iter()
            .find_map(|key| update.get(*key).filter(|u| has_usage_fields(u)))
            .or_else(|| has_usage_fields(update).then_some(update));
        let Some(usage) = usage else {
            return Vec::new();
        };
        let number = |keys: &[&str]| keys.iter().find_map(|key| usage.get(*key)?.as_u64());
        let input = number(&["inputTokens", "input_tokens"]);
        let output = number(&["outputTokens", "output_tokens"]);
        let used = number(&["totalTokens", "used", "usedTokens", "used_tokens"]).or_else(|| {
            (input.is_some() || output.is_some()).then(|| input.unwrap_or(0) + output.unwrap_or(0))
        });
        let window = number(&[
            "window",
            "size",
            "contextWindow",
            "context_window",
            "maxTokens",
            "max_tokens",
        ])
        .or(self.context_window);
        let mut events = Vec::new();
        // Once Grok has stamped the context itself, a turn's totals (every
        // model call added up) would only overstate it.
        if let Some(used) = used.filter(|_| self.context_used.is_none()) {
            events.push(AgentEvent::Usage {
                input_tokens: input.unwrap_or(0),
                output_tokens: output.unwrap_or(0),
                total_tokens: used,
                context_window: window,
            });
        }
        if self.metrics_reported {
            return events;
        }
        // Grok's own `cachedReadTokens` are part of `inputTokens`, as in
        // xAI's API; other spellings, and Antigravity's, count beside it.
        let own_read = number(&["cachedReadTokens"]).filter(|_| self.dialect.cached_in_input);
        let read = own_read.or_else(|| {
            number(&[
                "cachedReadTokens",
                "cacheReadTokens",
                "cache_read_input_tokens",
            ])
        });
        let write = number(&[
            "cachedWriteTokens",
            "cacheCreationTokens",
            "cacheWriteTokens",
            "cache_creation_input_tokens",
        ]);
        let (input_n, read_n, write_n) =
            (input.unwrap_or(0), read.unwrap_or(0), write.unwrap_or(0));
        let cacheable = match own_read {
            Some(_) => Some(input_n),
            None => (read.is_some() || write.is_some()).then_some(input_n + read_n + write_n),
        };
        let metrics =
            TurnMetrics::from_counts(input_n, output.unwrap_or(0), read_n, write_n, cacheable);
        if let Some(metrics) = metrics {
            self.metrics_reported = true;
            events.push(AgentEvent::TurnMetrics(metrics));
        }
        events
    }
}

impl LineParser for AcpParser {
    fn parse_line(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(rec) = serde_json::from_str::<Value>(line.trim()) else {
            if !line.trim().is_empty() {
                log::debug!("{}: non-JSON stdout line: {line}", self.dialect.log);
            }
            return Vec::new();
        };
        let mut events = Vec::new();
        let params = rec.get("params").unwrap_or(&Value::Null);
        match (str_field(&rec, "method"), rec.get("id")) {
            (Some(method), Some(id)) => self.on_request(id, method, params, &mut events),
            (Some(method), None) => self.on_notification(method, params, &mut events),
            (None, Some(id)) => {
                let outcome = match rec.get("error") {
                    Some(error) => Err(error_message(error)),
                    None => Ok(rec.get("result").unwrap_or(&Value::Null)),
                };
                let id = id.as_u64();
                if let Some(blocks) = id.and_then(|id| self.interjections.remove(&id)) {
                    self.on_interjection(blocks, outcome, &mut events);
                } else if let Some((_, step)) = self.waiting.filter(|(want, _)| id == Some(*want)) {
                    self.waiting = None;
                    self.on_response(step, outcome, &mut events);
                }
            }
            (None, None) => {}
        }
        events
    }

    fn take_replies(&mut self) -> Vec<String> {
        std::mem::take(&mut self.replies)
    }

    /// Grok's mid-turn interjection (`x.ai/interject`): the message reaches
    /// the model at its next safe point, without stopping the turn. Before
    /// the prompt is with the model, or on a CLI without it, it waits to be
    /// the next prompt.
    fn steer(&mut self, prompt: &str, attachments: &[Attachment]) {
        let blocks = crate::harness::attachments::acp_prompt_blocks(prompt, attachments);
        if blocks.is_empty() {
            return;
        }
        let running = self.dialect.interject && self.prompt_running();
        let Some(session) = self.session_id.clone().filter(|_| running) else {
            self.followups.extend(blocks);
            // The turn only waits on its tools: the user's message goes now.
            if self.awaiting_tools {
                self.awaiting_tools = false;
                self.prompt = std::mem::take(&mut self.followups);
                let mut events = Vec::new();
                self.send_prompt(&mut events);
            }
            return;
        };
        self.next_id += 1;
        let (text, images): (Vec<&Value>, Vec<&Value>) = blocks
            .iter()
            .partition(|block| str_field(block, "type") == Some("text"));
        let text = text
            .iter()
            .filter_map(|block| str_field(block, "text"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut params = json!({
            "sessionId": session,
            "text": text,
            "interjectionId": format!("bencode-{}-{}", std::process::id(), self.next_id),
        });
        if !images.is_empty() {
            params["content"] = json!(images);
        }
        self.replies.push(
            json!({ "jsonrpc": "2.0", "id": self.next_id, "method": "_x.ai/interject", "params": params })
                .to_string(),
        );
        self.interjections.insert(self.next_id, blocks);
    }

    /// Counted from the agent's last line, while the turn waits on tools.
    fn quiet_limit(&self) -> Option<Duration> {
        self.awaiting_tools.then_some(OPEN_TOOLS_GRACE)
    }

    /// The tools never reported back: the turn is let go.
    fn on_quiet(&mut self) -> Vec<AgentEvent> {
        if !std::mem::take(&mut self.awaiting_tools) {
            return Vec::new();
        }
        log::debug!(
            "{}: tool calls still open {OPEN_TOOLS_GRACE:?} after the prompt; ending the turn",
            self.dialect.log
        );
        vec![AgentEvent::Done(DoneStatus::Completed)]
    }
}

/// MonoCode `planEvent`'s entries: Grok's todo list.
pub(crate) fn task_items(entries: &[Value]) -> Vec<TaskItem> {
    entries
        .iter()
        .filter_map(|entry| {
            let text = ["content", "text", "title"]
                .iter()
                .find_map(|key| str_field(entry, key))
                .map(str::trim)
                .filter(|text| !text.is_empty())?;
            Some(TaskItem {
                id: str_field(entry, "id").map(String::from),
                text: text.to_string(),
                status: TaskStatus::parse(str_field(entry, "status").unwrap_or_default()),
            })
        })
        .collect()
}

pub(crate) fn fail(message: String, events: &mut Vec<AgentEvent>) {
    events.push(AgentEvent::Error(message));
    events.push(AgentEvent::Done(DoneStatus::Failed));
}

pub(crate) fn error_message(error: &Value) -> String {
    str_field(error, "message")
        .map(String::from)
        .unwrap_or_else(|| error.to_string())
}

pub(crate) fn selected(option: &str) -> Value {
    json!({ "outcome": { "outcome": "selected", "optionId": option } })
}

/// The reply to a permission or question the user answered; the request id
/// carries the JSON-RPC id and the options to pick from.
pub(crate) fn permission_response(request: &PermissionRequest, allow: bool) -> String {
    let meta: Value = serde_json::from_str(&request.request_id).unwrap_or(Value::Null);
    let result = if meta.get("question") == Some(&Value::Bool(true)) {
        match request.input.get("answers").filter(|_| allow) {
            Some(answers) => json!({ "outcome": "accepted", "answers": answers }),
            None => json!({ "outcome": "skip_interview" }),
        }
    } else {
        let key = if allow { "allow" } else { "deny" };
        selected(str_field(&meta, key).unwrap_or(if allow { "allow-once" } else { "reject-once" }))
    };
    json!({ "jsonrpc": "2.0", "id": meta.get("id").cloned().unwrap_or(Value::Null), "result": result })
        .to_string()
}

pub(crate) const ALLOW_ONCE_FIRST: [&str; 5] = [
    "allow-once",
    "allow_once",
    "allow-always",
    "allow_always",
    "allow",
];
pub(crate) const ALLOW_ALWAYS_FIRST: [&str; 5] = [
    "allow-always",
    "allow_always",
    "allow-once",
    "allow_once",
    "allow",
];
pub(crate) const REJECT: [&str; 6] = [
    "reject-once",
    "reject_once",
    "reject-always",
    "reject_always",
    "reject",
    "deny",
];

pub(crate) fn pick<'a>(options: &[&'a str], preferred: &[&str]) -> Option<&'a str> {
    preferred
        .iter()
        .find_map(|want| options.iter().find(|o| *o == want).copied())
}

/// MonoCode `permissionOptionId`.
pub(crate) fn option_id(allow: bool, options: &[&str]) -> String {
    let (preferred, fallback): (&[&str], _) = if allow {
        (&ALLOW_ONCE_FIRST, "allow-once")
    } else {
        (&REJECT, "reject-once")
    };
    pick(options, preferred).unwrap_or(fallback).to_string()
}

/// MonoCode `pickAutoOption`: the option the access mode picks by itself;
/// None asks the user.
pub(crate) fn auto_option<'a>(
    permission: PermissionPolicy,
    kind: &str,
    options: &[&'a str],
) -> Option<&'a str> {
    match permission {
        PermissionPolicy::Ask => None,
        PermissionPolicy::AcceptEdits if matches!(kind, "execute" | "other" | "fetch") => None,
        PermissionPolicy::AutoApprove => pick(options, &ALLOW_ALWAYS_FIRST),
        PermissionPolicy::AcceptEdits | PermissionPolicy::Auto => pick(options, &ALLOW_ONCE_FIRST),
    }
}

/// MonoCode `VARIANT_KIND`: Grok's tool names onto ACP kinds (and
/// Antigravity's `invoke_subagent`, which it sends as kind `other`).
pub(crate) fn kind_from_variant(variant: &str) -> Option<&'static str> {
    let key: String = variant
        .to_ascii_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    Some(match key.as_str() {
        "readfile" | "read" => "read",
        "listdir" => "list",
        "write" | "edit" | "searchreplace" => "edit",
        "bash" | "execute" | "runterminalcommand" | "runterminalcmd" => "execute",
        "grep" | "search" | "websearch" => "search",
        "webfetch" => "fetch",
        "agent" | "task" | "subagent" | "invokesubagent" => "agent",
        "todowrite" => "plan",
        _ => return None,
    })
}

/// MonoCode `grokToolFields`: what a call is, from Grok's `x.ai/tool` meta
/// or ACP's own fields.
#[derive(Debug, Default)]
pub(crate) struct ToolFields {
    pub(crate) call_id: Option<String>,
    pub(crate) kind: Option<String>,
    pub(crate) title: Option<String>,
    /// What the agent calls the tool (Grok's `x.ai/tool` name or input
    /// variant, else ACP's `name`).
    pub(crate) meta_name: Option<String>,
    pub(crate) input: Option<Map<String, Value>>,
}

impl ToolFields {
    pub(crate) fn read(update: &Value, tool: &Value) -> Self {
        let meta = update
            .pointer("/_meta/x.ai~1tool")
            .or_else(|| tool.pointer("/_meta/x.ai~1tool"))
            .filter(|m| m.is_object());
        let input = meta
            .and_then(|m| m.get("input"))
            .into_iter()
            .chain(
                ["rawInput", "raw_input", "input"]
                    .iter()
                    .flat_map(|key| [update.get(*key), tool.get(*key)])
                    .flatten(),
            )
            .find_map(Value::as_object)
            .cloned();
        let variant = input
            .as_ref()
            .and_then(|i| i.get("variant"))
            .and_then(Value::as_str)
            .or_else(|| meta.and_then(|m| str_field(m, "name")))
            .unwrap_or_default();
        let text = |rec: Option<&Value>, key: &str| {
            rec.and_then(|r| str_field(r, key))
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(String::from)
        };
        Self {
            call_id: ["toolCallId", "tool_call_id"]
                .iter()
                .find_map(|key| text(Some(update), key).or_else(|| text(Some(tool), key))),
            kind: text(meta, "kind")
                .or_else(|| kind_from_variant(variant).map(String::from))
                .or_else(|| {
                    ["kind", "name"].iter().find_map(|key| {
                        let named = text(Some(update), key).or_else(|| text(Some(tool), key))?;
                        let known = kind_from_variant(&named).map(String::from);
                        if *key == "name" {
                            known
                        } else {
                            // `other` says nothing; the call's name may.
                            (named != "other").then_some(named)
                        }
                    })
                })
                .or_else(|| text(Some(update), "kind"))
                .or_else(|| text(Some(tool), "kind")),
            title: text(Some(update), "title")
                .or_else(|| text(Some(tool), "title"))
                .or_else(|| text(meta, "label")),
            meta_name: text(meta, "name")
                .or_else(|| (!variant.is_empty()).then(|| variant.to_string()))
                .or_else(|| text(Some(update), "name"))
                .or_else(|| text(Some(tool), "name")),
            input,
        }
    }

    /// A name the transcript knows the kind of (`app::agent::tool_kind`).
    pub(crate) fn name(&self) -> String {
        match self.kind.as_deref().map(str::to_ascii_lowercase).as_deref() {
            Some("edit" | "write" | "delete" | "move") => "Edit".into(),
            Some("read" | "list") => "Read".into(),
            Some("execute" | "shell") => "Bash".into(),
            Some("search") => "Grep".into(),
            Some("fetch") => "WebFetch".into(),
            Some("agent" | "task") => "Task".into(),
            _ => self.meta_name.clone().unwrap_or_else(|| "tool".into()),
        }
    }

    /// The call's input, with the agent's spellings (Grok's, Antigravity's
    /// `CommandLine`, `TargetFile`, …) also under the keys the transcript's
    /// one-line summary and the review's edit tracking read, and the call's
    /// title when nothing else describes it.
    pub(crate) fn summary_input(&self) -> Value {
        let mut input = self.input.clone().unwrap_or_default();
        let aliases: [(&str, &[&str]); 7] = [
            ("command", &["CommandLine"]),
            (
                "path",
                &[
                    "absolute_path",
                    "file_path",
                    "target_file",
                    "target_directory",
                    "directory",
                    "AbsolutePath",
                    "TargetFile",
                    "File",
                    "DirectoryPath",
                    "SearchPath",
                ],
            ),
            ("pattern", &["Pattern"]),
            ("query", &["search", "Query"]),
            ("url", &["Url", "URL"]),
            // Antigravity names a subagent by its `Role`.
            ("description", &["toolSummary", "toolAction", "Role"]),
            ("prompt", &["Prompt"]),
        ];
        for (key, spellings) in aliases {
            if input.contains_key(key) {
                continue;
            }
            if let Some(value) = spellings.iter().find_map(|name| input.get(*name).cloned()) {
                input.insert(key.into(), value);
            }
        }
        let described = [
            "command",
            "file_path",
            "path",
            "pattern",
            "url",
            "query",
            "description",
        ]
        .iter()
        .any(|key| input.get(*key).is_some_and(Value::is_string));
        if !described && let Some(title) = &self.title {
            input.insert("description".into(), json!(title));
        }
        Value::Object(input)
    }
}

/// MonoCode `toolDetail`: what the call printed.
pub(crate) fn tool_detail(update: &Value, tool: &Value) -> String {
    let content = [update.get("content"), tool.get("content")]
        .into_iter()
        .map(|c| text_from_content(c, "\n"))
        .find(|t| !t.trim().is_empty());
    let output = || {
        let raw = update.get("rawOutput").or_else(|| tool.get("rawOutput"))?;
        if let Some(text) = raw.as_str() {
            return Some(text.to_string());
        }
        let text = text_from_content(Some(raw), "");
        if !text.trim().is_empty() {
            return Some(text);
        }
        // Grok's typed results: `{ type: "ReadFile", FileNotFound: "…" }`.
        raw.as_object()?
            .iter()
            .filter(|(key, _)| key.as_str() != "type")
            .find_map(|(_, value)| value.as_str())
            .map(String::from)
    };
    let detail = content.or_else(output).unwrap_or_default();
    let detail = detail.trim();
    match detail.char_indices().nth(MAX_DETAIL_CHARS) {
        Some((cut, _)) => format!("{}\n…", &detail[..cut]),
        None => detail.to_string(),
    }
}

/// MonoCode `textFromContent`.
pub(crate) fn text_from_content(content: Option<&Value>, separator: &str) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| text_from_content(Some(item), separator))
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(separator),
        Some(rec @ Value::Object(_)) => match rec.get("text") {
            Some(Value::String(text)) => text.clone(),
            _ => text_from_content(rec.get("content"), separator),
        },
        _ => String::new(),
    }
}

pub(crate) fn unwrap_session_notification(params: &Value) -> &Value {
    if params.get("update").is_some() || params.get("sessionUpdate").is_some() {
        return params;
    }
    ["notification", "payload"]
        .iter()
        .find_map(|key| params.get(*key).filter(|v| v.is_object()))
        .unwrap_or(params)
}

pub(crate) fn has_usage_fields(rec: &Value) -> bool {
    [
        "used",
        "usedTokens",
        "used_tokens",
        "totalTokens",
        "inputTokens",
        "input_tokens",
        "outputTokens",
        "output_tokens",
        "window",
        "size",
        "contextWindow",
        "context_window",
        "maxTokens",
        "max_tokens",
    ]
    .iter()
    .any(|key| rec.get(*key).is_some_and(Value::is_number))
}

/// MonoCode `questionsFromUnknown`'s lookup: the questions array, wherever
/// the request put it.
pub(crate) fn questions_from(params: &Value) -> Vec<Value> {
    [
        params.get("questions"),
        params.pointer("/input/questions"),
        params.pointer("/params/questions"),
        params.pointer("/question/questions"),
        params.is_array().then_some(params),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_array)
    .cloned()
    .unwrap_or_default()
}

/// MonoCode `planFromExitPlan`.
pub(crate) fn plan_from_exit_plan(params: &Value) -> String {
    ["planContent", "plan", "content"]
        .iter()
        .find_map(|key| str_field(params, key))
        .or_else(|| {
            let input = params.get("input")?;
            str_field(input, "plan").or_else(|| str_field(input, "planContent"))
        })
        .map(|plan| plan.trim().to_string())
        .unwrap_or_default()
}

/// MonoCode `sessionIdFromResult`.
pub(crate) fn session_id(result: &Value) -> Option<String> {
    ["sessionId", "session_id", "id"]
        .iter()
        .find_map(|key| str_field(result, key))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(String::from)
}

/// MonoCode `currentModelId`.
pub fn current_model_id(result: &Value) -> Option<String> {
    [
        "/models/currentModelId",
        "/_meta/modelState/currentModelId",
        "/_meta/currentModelId",
    ]
    .iter()
    .find_map(|pointer| result.pointer(pointer)?.as_str())
    .filter(|id| !id.trim().is_empty())
    .map(String::from)
}

/// The mode a bound session is in (`modes.currentModeId`).
fn current_mode_id(result: &Value) -> Option<String> {
    result
        .pointer("/modes/currentModeId")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(String::from)
}

/// The models `initialize` (`_meta.modelState`) or `session/new`
/// (`models`) lists.
pub(crate) fn available_models(result: &Value) -> Vec<&Value> {
    [
        "/models/availableModels",
        "/_meta/availableModels",
        "/_meta/modelState/availableModels",
        "/availableModels",
    ]
    .iter()
    .filter_map(|pointer| result.pointer(pointer)?.as_array())
    .flatten()
    .collect()
}

pub(crate) fn model_id(model: &Value) -> Option<&str> {
    ["modelId", "model_id", "id", "value"]
        .iter()
        .find_map(|key| str_field(model, key))
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

/// MonoCode `contextWindowFromSetup`: the current model's window, else the
/// first listed one's.
pub fn context_window(result: &Value) -> Option<u64> {
    let models = available_models(result);
    let current = current_model_id(result);
    let model = models
        .iter()
        .find(|m| current.is_some() && model_id(m) == current.as_deref())
        .or_else(|| models.first())?;
    let meta = model.get("_meta").unwrap_or(model);
    ["totalContextTokens", "contextWindow"]
        .iter()
        .find_map(|key| meta.get(*key).or_else(|| model.get(*key))?.as_u64())
}
