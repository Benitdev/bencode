//! Grok Build (xAI's `grok`) over ACP, as MonoCode's `providers/grok/grok.ts`
//! and `grokProtocol.ts` drive it. MonoCode keeps one `grok agent stdio` per
//! thread; here each turn starts its own, binds the thread's ACP session
//! (`session/resume`, else `session/load`, else `session/new`), sends one
//! `session/prompt`, and closes stdin once the prompt returns.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use serde_json::{Map, Value, json};

use crate::harness::attachments::Attachment;
use crate::harness::catalog::{ModelOption, ModelSetting, SettingKind};
use crate::harness::events::{
    AgentEvent, DoneStatus, PermissionRequest, TaskItem, TaskStatus, TurnMetrics,
};
use crate::harness::handle::HarnessProcessHandle;
use crate::harness::process::{self, LineParser, ProcessSpec, StdinMode};
use crate::harness::resolver::HarnessResolver;
use crate::harness::{EventRx, HarnessKind, PermissionPolicy, SpawnRequest, str_field};

/// MonoCode `AUTH_HELP`.
pub const AUTH_HELP: &str =
    "Grok Build is not signed in. Run `grok login` in a terminal, or set XAI_API_KEY.";

/// The tool name the composer's question form answers (`app::QUESTION_TOOL`).
const QUESTION_TOOL: &str = "AskUserQuestion";
/// JSON-RPC "method not found".
const METHOD_NOT_FOUND: i64 = -32601;
/// The browser OAuth method: it has no headless completion, so never pick it.
const BROWSER_AUTH: &str = "grok.com";
const MAX_DETAIL_CHARS: usize = 8_000;
const NOTHING_TO_COMPACT: &str = "Grok Build has no saved session for this chat to compact.";

pub fn spawn(req: &SpawnRequest) -> Result<(HarnessProcessHandle, EventRx)> {
    let program = HarnessResolver::resolve_grok().unwrap_or_else(|| "grok".into());
    let mut parser = GrokParser::new(req);
    let spec = ProcessSpec {
        program,
        args: build_args(req),
        cwd: req.cwd.clone(),
        stdin: StdinMode::Protocol {
            initial: format!("{}\n", parser.initialize()),
        },
        permission_responder: Some(permission_response),
        // A compaction is not a turn to add to.
        can_steer: !req.compact,
        account: None,
    };
    process::spawn(spec, parser)
}

/// MonoCode `grokEffort`.
fn effort(req: &SpawnRequest) -> Option<&str> {
    ["effort", "reasoning"]
        .iter()
        .filter_map(|id| req.settings.get(*id))
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
}

/// MonoCode `grokSpawnArgs`: global flags before `agent`, `stdio` last.
fn build_args(req: &SpawnRequest) -> Vec<String> {
    let mut args: Vec<String> = vec!["--no-auto-update".into()];
    if req.plan {
        args.extend(["--permission-mode".into(), "plan".into()]);
    }
    args.extend(["agent".into(), "--no-leader".into()]);
    if let Some(model) = &req.model {
        args.extend(["--model".into(), model.clone()]);
    }
    if let Some(effort) = effort(req) {
        args.extend(["--reasoning-effort".into(), effort.to_string()]);
    }
    if req.permission == PermissionPolicy::AutoApprove && !req.plan {
        args.push("--always-approve".into());
    }
    args.push("stdio".into());
    args
}

/// The ACP arguments for a model probe: no model, nothing approved.
pub fn probe_args() -> [&'static str; 4] {
    ["--no-auto-update", "agent", "--no-leader", "stdio"]
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

/// MonoCode `grokSessionNewParams`.
fn session_new_params(cwd: &str, permission: PermissionPolicy, plan: bool) -> Value {
    let mut params = json!({ "cwd": cwd, "mcpServers": [] });
    let meta = match permission {
        PermissionPolicy::AutoApprove if !plan => Some(json!({ "yoloMode": true })),
        PermissionPolicy::Auto => Some(json!({ "autoMode": true })),
        _ => None,
    };
    if let Some(meta) = meta {
        params["_meta"] = meta;
    }
    params
}

/// MonoCode `grokAuthMethodId`: an API key when the agent saw one, else its
/// default, else the cached `grok login` token; never the browser flow.
pub fn auth_method(init: &Value) -> Option<String> {
    let ids: Vec<&str> = init
        .get("authMethods")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|method| str_field(method, "id"))
        .map(str::trim)
        .filter(|id| !id.is_empty() && *id != BROWSER_AUTH)
        .collect();
    let default = init
        .pointer("/_meta/defaultAuthMethodId")
        .and_then(Value::as_str);
    ["xai.api_key"]
        .into_iter()
        .chain(default)
        .chain(["cached_token"])
        .find(|id| ids.contains(id))
        .or_else(|| ids.first().copied())
        .map(String::from)
}

fn mentions_auth(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    ["auth", "login", "credential", "api key", "xai_api_key"]
        .iter()
        .any(|word| lower.contains(word))
}

/// MonoCode `grokAuthError`: why the agent did not start.
fn start_error(detail: &str) -> String {
    if mentions_auth(detail) {
        format!("{}\n\n{AUTH_HELP}", detail.trim())
    } else if detail.to_ascii_lowercase().contains("timed out") {
        format!("Grok Build did not start. {AUTH_HELP}")
    } else {
        format!("Grok Build did not start. {detail}")
    }
}

/// What a client request was for, so its response knows the next step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Initialize,
    Authenticate,
    Resume,
    Load,
    New,
    SetModel,
    Prompt,
    Compact,
}

pub struct GrokParser {
    cwd: String,
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
    session_id: Option<String>,
    /// `session/load` replays the thread's history; that is not new output.
    muted: bool,
    context_window: Option<u64>,
    /// The context in use, as Grok last stamped an update with it.
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
    finishing: Option<DoneStatus>,
    replies: Vec<String>,
}

impl GrokParser {
    fn new(req: &SpawnRequest) -> Self {
        Self {
            cwd: req.cwd.clone(),
            prompt: crate::harness::attachments::acp_prompt_blocks(&req.prompt, &req.attachments),
            resume_id: req.resume_id.clone().filter(|id| !id.trim().is_empty()),
            model: req.model.clone(),
            permission: req.permission,
            plan: req.plan,
            compact: req.compact,
            next_id: 0,
            waiting: None,
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
            replies: Vec::new(),
        }
    }

    /// The first line on stdin.
    fn initialize(&mut self) -> String {
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
                match auth_method(init) {
                    Some(method) => self.request(
                        "authenticate",
                        json!({ "methodId": method, "_meta": { "headless": true } }),
                        Step::Authenticate,
                    ),
                    None => self.open_session(events),
                }
            }
            (Step::Initialize, Err(err)) => fail(start_error(&err), events),
            // MonoCode goes on either way: a cached token may still work.
            (Step::Authenticate, outcome) => {
                if let Err(err) = outcome {
                    log::debug!("grok authenticate: {err}");
                }
                self.open_session(events);
            }
            (Step::Resume | Step::Load, Ok(result)) => {
                self.muted = false;
                let fallback = self.resume_id.clone();
                self.bind(result, fallback, events);
            }
            (Step::Resume, Err(err)) => {
                log::debug!("grok session/resume: {err}; loading instead");
                let id = self.resume_id.clone().unwrap_or_default();
                self.muted = true;
                self.request(
                    "session/load",
                    json!({ "sessionId": id, "cwd": self.cwd, "mcpServers": [] }),
                    Step::Load,
                );
            }
            (Step::Load, Err(err)) => {
                self.muted = false;
                if self.compact {
                    fail(format!("{NOTHING_TO_COMPACT} ({err})"), events);
                } else {
                    log::debug!("grok session/load: {err}; starting a new session");
                    self.new_session();
                }
            }
            (Step::New, Ok(result)) => self.bind(result, None, events),
            (Step::New, Err(err)) => fail(start_error(&err), events),
            (Step::SetModel, outcome) => {
                if let Err(err) = outcome {
                    log::debug!("grok session/set_model: {err}");
                }
                self.send_prompt(events);
            }
            (Step::Prompt, Ok(result)) => {
                events.extend(self.usage_events(result));
                let status = match str_field(result, "stopReason") {
                    Some("cancelled") => DoneStatus::Cancelled,
                    _ => DoneStatus::Completed,
                };
                self.finish_prompt(status, events);
            }
            (Step::Prompt, Err(err)) => {
                let message = if mentions_auth(&err) {
                    format!("{}\n\n{AUTH_HELP}", err.trim())
                } else {
                    err
                };
                fail(message, events);
            }
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

    fn open_session(&mut self, events: &mut Vec<AgentEvent>) {
        match self.resume_id.clone() {
            Some(id) => self.request("session/resume", json!({ "sessionId": id }), Step::Resume),
            None if self.compact => fail(NOTHING_TO_COMPACT.into(), events),
            None => self.new_session(),
        }
    }

    fn new_session(&mut self) {
        let params = session_new_params(&self.cwd, self.permission, self.plan);
        self.request("session/new", params, Step::New);
    }

    /// The session is ready: announce it, then compact it, or put it on the
    /// thread's model and send the prompt.
    fn bind(&mut self, result: &Value, fallback: Option<String>, events: &mut Vec<AgentEvent>) {
        let Some(id) = session_id(result).or(fallback) else {
            fail("Grok Build did not return a session id".into(), events);
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
            // MonoCode `compactGrokContext`.
            self.request(
                "_x.ai/compact_conversation",
                json!({ "sessionId": id }),
                Step::Compact,
            );
            return;
        }
        // A resumed session keeps the model it had; `--model` only picks a
        // new session's.
        let current = current_model_id(result);
        if let Some(model) = self.model.clone()
            && current.is_some_and(|current| current != model)
        {
            self.request(
                "session/set_model",
                json!({ "sessionId": id, "modelId": model }),
                Step::SetModel,
            );
            return;
        }
        self.send_prompt(events);
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
    /// for yet are waited on first.
    fn finish_prompt(&mut self, status: DoneStatus, events: &mut Vec<AgentEvent>) {
        if status != DoneStatus::Completed {
            events.push(AgentEvent::Done(status));
        } else if !self.interjections.is_empty() {
            self.finishing = Some(status);
        } else if self.followups.is_empty() {
            events.push(AgentEvent::Done(status));
        } else {
            self.prompt = std::mem::take(&mut self.followups);
            self.send_prompt(events);
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
            && let Some(status) = self.finishing.take()
        {
            self.finish_prompt(status, events);
        }
    }

    /// Whether the prompt is with the model now, so a follow-up can go into
    /// its turn.
    fn prompt_running(&self) -> bool {
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
            auto_option(self.permission, &kind, &options).map(String::from)
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
        // xAI's API; MonoCode's spellings count beside it.
        let own_read = number(&["cachedReadTokens"]);
        let read = own_read.or_else(|| number(&["cacheReadTokens", "cache_read_input_tokens"]));
        let write = number(&[
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

impl LineParser for GrokParser {
    fn parse_line(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(rec) = serde_json::from_str::<Value>(line.trim()) else {
            if !line.trim().is_empty() {
                log::debug!("grok: non-JSON stdout line: {line}");
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
    /// the prompt is with the model, it waits to be the next prompt.
    fn steer(&mut self, prompt: &str, attachments: &[Attachment]) {
        let blocks = crate::harness::attachments::acp_prompt_blocks(prompt, attachments);
        if blocks.is_empty() {
            return;
        }
        let Some(session) = self.session_id.clone().filter(|_| self.prompt_running()) else {
            self.followups.extend(blocks);
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
}

/// MonoCode `planEvent`'s entries: Grok's todo list.
fn task_items(entries: &[Value]) -> Vec<TaskItem> {
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

fn fail(message: String, events: &mut Vec<AgentEvent>) {
    events.push(AgentEvent::Error(message));
    events.push(AgentEvent::Done(DoneStatus::Failed));
}

fn error_message(error: &Value) -> String {
    str_field(error, "message")
        .map(String::from)
        .unwrap_or_else(|| error.to_string())
}

fn selected(option: &str) -> Value {
    json!({ "outcome": { "outcome": "selected", "optionId": option } })
}

/// The reply to a permission or question the user answered; the request id
/// carries the JSON-RPC id and the options to pick from.
fn permission_response(request: &PermissionRequest, allow: bool) -> String {
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

const ALLOW_ONCE_FIRST: [&str; 5] = [
    "allow-once",
    "allow_once",
    "allow-always",
    "allow_always",
    "allow",
];
const ALLOW_ALWAYS_FIRST: [&str; 5] = [
    "allow-always",
    "allow_always",
    "allow-once",
    "allow_once",
    "allow",
];
const REJECT: [&str; 6] = [
    "reject-once",
    "reject_once",
    "reject-always",
    "reject_always",
    "reject",
    "deny",
];

fn pick<'a>(options: &[&'a str], preferred: &[&str]) -> Option<&'a str> {
    preferred
        .iter()
        .find_map(|want| options.iter().find(|o| *o == want).copied())
}

/// MonoCode `permissionOptionId`.
fn option_id(allow: bool, options: &[&str]) -> String {
    let (preferred, fallback): (&[&str], _) = if allow {
        (&ALLOW_ONCE_FIRST, "allow-once")
    } else {
        (&REJECT, "reject-once")
    };
    pick(options, preferred).unwrap_or(fallback).to_string()
}

/// MonoCode `pickAutoOption`: the option the access mode picks by itself;
/// None asks the user.
fn auto_option<'a>(
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

/// MonoCode `VARIANT_KIND`: Grok's tool names onto ACP kinds.
fn kind_from_variant(variant: &str) -> Option<&'static str> {
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
        "agent" | "task" | "subagent" => "agent",
        "todowrite" => "plan",
        _ => return None,
    })
}

/// MonoCode `grokToolFields`: what a call is, from Grok's `x.ai/tool` meta
/// or ACP's own fields.
#[derive(Debug, Default)]
struct ToolFields {
    call_id: Option<String>,
    kind: Option<String>,
    title: Option<String>,
    meta_name: Option<String>,
    input: Option<Map<String, Value>>,
}

impl ToolFields {
    fn read(update: &Value, tool: &Value) -> Self {
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
                .or_else(|| text(Some(update), "kind"))
                .or_else(|| text(Some(tool), "kind")),
            title: text(Some(update), "title")
                .or_else(|| text(Some(tool), "title"))
                .or_else(|| text(meta, "label")),
            meta_name: text(meta, "name")
                .or_else(|| (!variant.is_empty()).then(|| variant.to_string())),
            input,
        }
    }

    /// A name the transcript knows the kind of (`app::agent::tool_kind`).
    fn name(&self) -> String {
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

    /// The call's input, with Grok's spellings under the keys the
    /// transcript's one-line summary and the review's edit tracking read,
    /// and the call's title when nothing else describes it.
    fn summary_input(&self) -> Value {
        let mut input = self.input.clone().unwrap_or_default();
        if !input.contains_key("path")
            && let Some(path) = [
                "absolute_path",
                "file_path",
                "target_file",
                "target_directory",
                "directory",
            ]
            .iter()
            .find_map(|key| input.get(*key).cloned())
        {
            input.insert("path".into(), path);
        }
        if !input.contains_key("query")
            && let Some(query) = input.get("search").cloned()
        {
            input.insert("query".into(), query);
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
fn tool_detail(update: &Value, tool: &Value) -> String {
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
fn text_from_content(content: Option<&Value>, separator: &str) -> String {
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

fn unwrap_session_notification(params: &Value) -> &Value {
    if params.get("update").is_some() || params.get("sessionUpdate").is_some() {
        return params;
    }
    ["notification", "payload"]
        .iter()
        .find_map(|key| params.get(*key).filter(|v| v.is_object()))
        .unwrap_or(params)
}

fn has_usage_fields(rec: &Value) -> bool {
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
fn questions_from(params: &Value) -> Vec<Value> {
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
fn plan_from_exit_plan(params: &Value) -> String {
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
fn session_id(result: &Value) -> Option<String> {
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

/// The models `initialize` (`_meta.modelState`) or `session/new`
/// (`models`) lists.
fn available_models(result: &Value) -> Vec<&Value> {
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

fn model_id(model: &Value) -> Option<&str> {
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

/// MonoCode `EFFORT_LABELS`.
fn effort_label(value: &str) -> Option<&'static str> {
    Some(match value {
        "xhigh" => "Extra High",
        "high" => "High",
        "medium" => "Medium",
        "low" => "Low",
        _ => return None,
    })
}

/// MonoCode `effortSetting`.
fn effort_setting(options: Vec<(String, String)>, default: Option<&str>) -> ModelSetting {
    let default = default
        .filter(|d| options.iter().any(|(v, _)| v == d))
        .map(String::from)
        .unwrap_or_else(|| options[0].0.clone());
    ModelSetting {
        id: "effort".into(),
        label: "Reasoning".into(),
        kind: SettingKind::Select,
        default,
        options,
    }
}

/// MonoCode `displayName`: `grok-4.6` → `Grok 4.6`.
fn display_name(native: &str) -> String {
    native
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn model_option(native: &str, label: String, settings: Vec<ModelSetting>) -> ModelOption {
    ModelOption {
        key: format!("grok:{native}"),
        label,
        harness: HarnessKind::Grok,
        native: native.to_string(),
        provider: None,
        settings,
    }
}

fn unique(models: Vec<ModelOption>) -> Vec<ModelOption> {
    let mut seen = HashSet::new();
    models
        .into_iter()
        .filter(|m| seen.insert(m.key.clone()))
        .collect()
}

/// MonoCode `modelsFromInitialize` / `modelsFromSessionNew`, effort menus
/// included.
pub fn models_from_acp(result: &Value) -> Vec<ModelOption> {
    let models = available_models(result)
        .into_iter()
        .filter_map(|model| {
            let native = model_id(model)?;
            let label = ["name", "displayName"]
                .iter()
                .find_map(|key| str_field(model, key))
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map_or_else(|| display_name(native), String::from);
            let meta = model.get("_meta").unwrap_or(model);
            let efforts: Vec<(String, String, bool)> = ["reasoningEfforts", "reasoning_efforts"]
                .iter()
                .find_map(|key| meta.get(*key)?.as_array())
                .into_iter()
                .flatten()
                .filter_map(|effort| {
                    let value = str_field(effort, "value")
                        .or_else(|| str_field(effort, "id"))?
                        .trim();
                    let label = effort_label(value).map(String::from).unwrap_or_else(|| {
                        let label = str_field(effort, "label").unwrap_or(value).trim();
                        label
                            .strip_suffix(" Effort")
                            .unwrap_or(label)
                            .trim()
                            .to_string()
                    });
                    let default = effort.get("default") == Some(&Value::Bool(true));
                    (!value.is_empty()).then(|| (value.to_string(), label, default))
                })
                .collect();
            let default = str_field(meta, "reasoningEffort")
                .map(String::from)
                .or_else(|| {
                    efforts
                        .iter()
                        .find(|(_, _, default)| *default)
                        .map(|(v, _, _)| v.clone())
                });
            let settings = if efforts.is_empty() {
                Vec::new()
            } else {
                let options = efforts.into_iter().map(|(v, l, _)| (v, l)).collect();
                vec![effort_setting(options, default.as_deref())]
            };
            Some(model_option(native, label, settings))
        })
        .collect();
    unique(models)
}

/// MonoCode `modelsFromGrokModelsOutput`: `* id` / `- id` lines of
/// `grok models`, colour codes stripped.
pub fn models_from_cli(stdout: &str) -> Vec<ModelOption> {
    let models = stdout
        .lines()
        .filter_map(|line| {
            let line = strip_ansi(line);
            let rest = line.trim().strip_prefix(['*', '+', '-'])?;
            let native = rest.split_whitespace().next()?;
            (rest.starts_with(char::is_whitespace) && !native.is_empty())
                .then(|| model_option(native, display_name(native), Vec::new()))
        })
        .collect();
    unique(models)
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// MonoCode `fallbackGrokModels`: the catalog until `grok` lists its own.
pub fn seed_models() -> Vec<ModelOption> {
    let efforts = |levels: &[&str]| {
        let options = levels
            .iter()
            .map(|l| (l.to_string(), effort_label(l).unwrap_or(l).to_string()))
            .collect();
        vec![effort_setting(options, Some("high"))]
    };
    vec![
        model_option(
            "grok-4.6",
            "Grok 4.6".into(),
            efforts(&["xhigh", "high", "medium", "low"]),
        ),
        model_option(
            "grok-4.5",
            "Grok 4.5".into(),
            efforts(&["high", "medium", "low"]),
        ),
    ]
}

/// MonoCode `is_grok_agent`: xAI's `grok`, not another CLI of that name.
/// The official installer puts it under `~/.grok`; elsewhere its binary
/// must name Grok Build.
pub fn is_grok_build(path: &Path) -> bool {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let in_grok_home = |p: &Path| p.components().any(|c| c.as_os_str() == ".grok");
    in_grok_home(path) || in_grok_home(&resolved) || file_mentions_grok_build(&resolved)
}

/// The markers can sit deep in the compiled binary, so the whole file is
/// read in chunks that overlap enough to catch one across a boundary.
fn file_mentions_grok_build(path: &Path) -> bool {
    use std::io::Read;
    const MARKERS: [&[u8]; 4] = [
        b"xai-grok",
        b"Grok Build",
        b"docs.x.ai/build",
        b"grok agent",
    ];
    const CHUNK: usize = 1024 * 1024;
    const OVERLAP: usize = 64;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = vec![0u8; CHUNK + OVERLAP];
    let mut carry = 0;
    loop {
        let n = match file.read(&mut buf[carry..]) {
            Ok(0) | Err(_) => return false,
            Ok(n) => n,
        };
        let filled = carry + n;
        let window = &buf[..filled];
        if MARKERS
            .iter()
            .any(|marker| window.windows(marker.len()).any(|w| w == *marker))
        {
            return true;
        }
        carry = filled.min(OVERLAP);
        buf.copy_within(filled - carry..filled, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(permission: PermissionPolicy) -> SpawnRequest {
        SpawnRequest {
            harness: HarnessKind::Grok,
            cwd: "/repo".into(),
            prompt: "hi".into(),
            model: Some("grok-4.6".into()),
            permission,
            resume_id: None,
            disable_hooks: false,
            attachments: Vec::new(),
            plan: false,
            compact: false,
            settings: [("effort".to_string(), "high".to_string())].into(),
            account: None,
        }
    }

    fn sent(parser: &mut GrokParser) -> Vec<Value> {
        parser
            .take_replies()
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn reply(id: u64, result: Value) -> String {
        json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
    }

    /// Drives a parser through initialize and session/new.
    fn bound(permission: PermissionPolicy) -> GrokParser {
        let mut parser = GrokParser::new(&request(permission));
        let init: Value = serde_json::from_str(&parser.initialize()).unwrap();
        assert_eq!(init["method"], "initialize");
        parser.parse_line(&reply(
            1,
            json!({
                "authMethods": [{ "id": "cached_token" }],
                "_meta": { "modelState": { "currentModelId": "grok-4.6", "availableModels": [
                    { "modelId": "grok-4.6", "name": "Grok 4.6", "_meta": { "totalContextTokens": 500000 } }
                ] } }
            }),
        ));
        let auth = sent(&mut parser);
        assert_eq!(auth[0]["method"], "authenticate");
        assert_eq!(auth[0]["params"]["methodId"], "cached_token");
        parser.parse_line(&reply(2, json!({})));
        let new = sent(&mut parser);
        assert_eq!(new[0]["method"], "session/new");
        assert_eq!(new[0]["params"]["cwd"], "/repo");
        let events = parser.parse_line(&reply(
            3,
            json!({ "sessionId": "ses-1", "models": { "currentModelId": "grok-4.6" } }),
        ));
        assert_eq!(
            events,
            vec![AgentEvent::SessionStarted {
                provider_session_id: "ses-1".into()
            }]
        );
        let prompt = sent(&mut parser);
        assert_eq!(prompt[0]["method"], "session/prompt");
        assert_eq!(prompt[0]["params"]["sessionId"], "ses-1");
        assert_eq!(prompt[0]["params"]["prompt"][0]["text"], "hi");
        parser
    }

    #[test]
    fn global_flags_go_before_agent_and_stdio_last() {
        assert_eq!(
            build_args(&request(PermissionPolicy::AutoApprove)),
            [
                "--no-auto-update",
                "agent",
                "--no-leader",
                "--model",
                "grok-4.6",
                "--reasoning-effort",
                "high",
                "--always-approve",
                "stdio",
            ]
        );
        let mut plan = request(PermissionPolicy::AutoApprove);
        plan.plan = true;
        plan.settings.clear();
        assert_eq!(
            build_args(&plan),
            [
                "--no-auto-update",
                "--permission-mode",
                "plan",
                "agent",
                "--no-leader",
                "--model",
                "grok-4.6",
                "stdio",
            ]
        );
    }

    #[test]
    fn session_meta_follows_the_access_mode() {
        assert_eq!(
            session_new_params("/repo", PermissionPolicy::Ask, false),
            json!({ "cwd": "/repo", "mcpServers": [] })
        );
        assert_eq!(
            session_new_params("/repo", PermissionPolicy::AutoApprove, false)["_meta"],
            json!({ "yoloMode": true })
        );
        assert_eq!(
            session_new_params("/repo", PermissionPolicy::Auto, false)["_meta"],
            json!({ "autoMode": true })
        );
        assert!(
            session_new_params("/repo", PermissionPolicy::AutoApprove, true)
                .get("_meta")
                .is_none()
        );
    }

    #[test]
    fn never_authenticates_through_the_browser() {
        let init = json!({
            "authMethods": [{ "id": "grok.com" }, { "id": "cached_token" }, { "id": "xai.api_key" }],
            "_meta": { "defaultAuthMethodId": "grok.com" },
        });
        assert_eq!(auth_method(&init).as_deref(), Some("xai.api_key"));
        let init = json!({ "authMethods": [{ "id": "cached_token" }, { "id": "grok.com" }] });
        assert_eq!(auth_method(&init).as_deref(), Some("cached_token"));
        assert_eq!(
            auth_method(&json!({ "authMethods": [{ "id": "grok.com" }] })),
            None
        );
    }

    #[test]
    fn a_turn_streams_text_thinking_tools_and_usage() {
        let mut parser = bound(PermissionPolicy::Ask);
        let update = |update: Value| {
            json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "ses-1", "update": update } })
                .to_string()
        };
        let mut events = Vec::new();
        for line in [
            update(
                json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "Hmm" } }),
            ),
            update(
                json!({ "sessionUpdate": "tool_call_delta_chunk", "tool_call_id": "call-1", "name": "read_file" }),
            ),
            update(json!({
                "sessionUpdate": "tool_call", "toolCallId": "call-1", "kind": "read",
                "title": "Read README.md", "status": "in_progress",
                "_meta": { "x.ai/tool": { "name": "read_file", "kind": "read", "input": { "path": "README.md" } } },
            })),
            update(json!({
                "sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "completed",
                "content": [{ "type": "content", "content": { "type": "text", "text": "# BenCode" } }],
            })),
            update(
                json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Hi" } }),
            ),
            update(
                json!({ "sessionUpdate": "turn_completed", "usage": { "inputTokens": 19762, "outputTokens": 36, "totalTokens": 19798 } }),
            ),
            reply(4, json!({ "stopReason": "end_turn" })),
        ] {
            events.extend(parser.parse_line(&line));
        }
        assert_eq!(
            events,
            vec![
                AgentEvent::ThinkingDelta("Hmm".into()),
                AgentEvent::ToolCallStart {
                    id: "call-1".into(),
                    name: "Read".into(),
                    input: json!({ "path": "README.md" }),
                },
                AgentEvent::ToolCallFinish {
                    id: "call-1".into(),
                    output: "# BenCode".into(),
                    success: true,
                },
                AgentEvent::TextDelta("Hi".into()),
                AgentEvent::Usage {
                    input_tokens: 19762,
                    output_tokens: 36,
                    total_tokens: 19798,
                    context_window: Some(500_000),
                },
                AgentEvent::TurnMetrics(TurnMetrics {
                    input_tokens: Some(19762),
                    output_tokens: Some(36),
                    ..Default::default()
                }),
                AgentEvent::Done(DoneStatus::Completed),
            ]
        );
    }

    /// Notifications in the shapes Grok Build 1.0.21 wrote to a session's
    /// `updates.jsonl`, their contents replaced.
    const RECORDED: &str = include_str!("../../tests/fixtures/grok_acp_turn.jsonl");

    #[test]
    fn a_recorded_turn_folds_into_the_transcripts_events() {
        let mut parser = bound(PermissionPolicy::AutoApprove);
        let mut events: Vec<_> = RECORDED
            .lines()
            .flat_map(|line| parser.parse_line(line))
            .collect();
        events.extend(parser.parse_line(&reply(4, json!({ "stopReason": "end_turn" }))));
        let context = |total_tokens| AgentEvent::Usage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens,
            context_window: Some(500_000),
        };
        let task = |text: &str, status| TaskItem {
            id: None,
            text: text.into(),
            status,
        };
        assert_eq!(
            events,
            vec![
                AgentEvent::ThinkingDelta("Searching first.".into()),
                context(34801),
                AgentEvent::ToolCallStart {
                    id: "call-1".into(),
                    name: "Grep".into(),
                    input: json!({ "pattern": "user_locations", "glob": "*.sql", "head_limit": 50 }),
                },
                AgentEvent::ToolCallFinish {
                    id: "call-1".into(),
                    output: "found 2 matches".into(),
                    success: true,
                },
                AgentEvent::ToolCallStart {
                    id: "call-2".into(),
                    name: "Read".into(),
                    input: json!({
                        "target_file": "/repo/missing.sql", "path": "/repo/missing.sql",
                        "offset": 1, "limit": 100,
                    }),
                },
                context(51006),
                AgentEvent::ToolCallFinish {
                    id: "call-2".into(),
                    output: "/repo/missing.sql does not exist".into(),
                    success: false,
                },
                // `todo_write` shows as the list, not as a tool row.
                AgentEvent::Tasks(vec![
                    task("Find the table", TaskStatus::Completed),
                    task("Write the migration", TaskStatus::InProgress),
                ]),
                AgentEvent::ToolCallStart {
                    id: "call-4".into(),
                    name: "Edit".into(),
                    input: json!({
                        "file_path": "/repo/db/001.sql", "path": "/repo/db/001.sql",
                        "old_string": "a", "new_string": "b",
                    }),
                },
                AgentEvent::ToolCallFinish {
                    id: "call-4".into(),
                    output: String::new(),
                    success: true,
                },
                AgentEvent::TextDelta("Done.".into()),
                context(60212),
                // The turn's totals add up four model calls: they are its
                // cost, not the context. Cached input is part of the input.
                AgentEvent::TurnMetrics(TurnMetrics {
                    input_tokens: Some(227724),
                    output_tokens: Some(1500),
                    cache_read_tokens: Some(170112),
                    cache_write_tokens: None,
                    cache_hit_percent: Some(170112.0 / 227724.0 * 100.0),
                }),
                AgentEvent::Done(DoneStatus::Completed),
            ]
        );
    }

    fn running_update(text: &str) -> String {
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "ses-1",
            "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } } } })
        .to_string()
    }

    #[test]
    fn a_follow_up_is_interjected_into_the_running_turn() {
        let mut parser = bound(PermissionPolicy::Ask);
        let image = Attachment {
            id: "a".into(),
            name: "a.png".into(),
            path: "/tmp/a.png".into(),
            mime_type: "image/png".into(),
            size: 3,
            data: Some("YWJj".into()),
        };
        parser.steer("use the staging table", &[image]);
        let sent = sent(&mut parser);
        assert_eq!(sent[0]["method"], "_x.ai/interject");
        assert_eq!(sent[0]["params"]["sessionId"], "ses-1");
        assert_eq!(sent[0]["params"]["text"], "use the staging table");
        assert!(sent[0]["params"]["interjectionId"].is_string());
        assert_eq!(
            sent[0]["params"]["content"],
            json!([{ "type": "image", "mimeType": "image/png", "data": "YWJj" }])
        );
        // Accepted: the turn goes on and ends as usual.
        assert!(parser.parse_line(&reply(5, json!({}))).is_empty());
        assert_eq!(
            parser.parse_line(&reply(4, json!({ "stopReason": "end_turn" }))),
            vec![AgentEvent::Done(DoneStatus::Completed)]
        );
    }

    #[test]
    fn a_refused_follow_up_becomes_the_next_prompt() {
        let mut parser = bound(PermissionPolicy::Ask);
        parser.steer("and add a test", &[]);
        sent(&mut parser);
        // The prompt returns before Grok answers for the follow-up.
        assert!(
            parser
                .parse_line(&reply(4, json!({ "stopReason": "end_turn" })))
                .is_empty()
        );
        let refused = json!({ "jsonrpc": "2.0", "id": 5, "error": { "code": METHOD_NOT_FOUND, "message": "Method not found" } });
        assert!(parser.parse_line(&refused.to_string()).is_empty());
        let next = sent(&mut parser);
        assert_eq!(next[0]["method"], "session/prompt");
        assert_eq!(next[0]["params"]["prompt"][0]["text"], "and add a test");
        // A follow-up now is not for a turn that is ending.
        assert!(parser.prompt_running());
        let mut events = parser.parse_line(&running_update("Added."));
        events.extend(parser.parse_line(&reply(6, json!({ "stopReason": "end_turn" }))));
        assert_eq!(
            events,
            vec![
                AgentEvent::TextDelta("Added.".into()),
                AgentEvent::Done(DoneStatus::Completed)
            ]
        );
    }

    #[test]
    fn a_follow_up_before_the_prompt_waits_its_turn() {
        let mut parser = GrokParser::new(&request(PermissionPolicy::Ask));
        parser.initialize();
        parser.steer("one more thing", &[]);
        assert!(
            parser.take_replies().is_empty(),
            "there is no turn to interject into yet"
        );
        parser.parse_line(&reply(1, json!({})));
        sent(&mut parser);
        parser.parse_line(&reply(2, json!({ "sessionId": "ses-1" })));
        assert_eq!(sent(&mut parser)[0]["params"]["prompt"][0]["text"], "hi");
        assert!(
            parser
                .parse_line(&reply(3, json!({ "stopReason": "end_turn" })))
                .is_empty()
        );
        assert_eq!(
            sent(&mut parser)[0]["params"]["prompt"][0]["text"],
            "one more thing"
        );
    }

    #[test]
    fn compaction_resumes_the_session_and_reports_what_is_left() {
        let mut req = request(PermissionPolicy::Ask);
        req.resume_id = Some("old".into());
        req.compact = true;
        req.prompt = "/compact".into();
        let mut parser = GrokParser::new(&req);
        parser.initialize();
        parser.parse_line(&reply(1, json!({})));
        assert_eq!(sent(&mut parser)[0]["method"], "session/resume");
        parser.parse_line(&reply(
            2,
            json!({ "models": { "currentModelId": "grok-4.5" } }),
        ));
        let compact = sent(&mut parser);
        assert_eq!(compact.len(), 1, "no model switch, no prompt");
        assert_eq!(compact[0]["method"], "_x.ai/compact_conversation");
        assert_eq!(compact[0]["params"], json!({ "sessionId": "old" }));

        let done = json!({ "jsonrpc": "2.0", "method": "_x.ai/session_notification", "params": { "sessionId": "old",
            "update": { "sessionUpdate": "auto_compact_completed", "tokens_before": 180000, "tokens_after": 12890, "summary_preview": "…" } } });
        assert!(parser.parse_line(&done.to_string()).is_empty());
        assert_eq!(
            parser.parse_line(&reply(3, json!({}))),
            vec![
                AgentEvent::Compacted {
                    tokens_after: Some(12890)
                },
                AgentEvent::Done(DoneStatus::Completed)
            ]
        );
    }

    #[test]
    fn compaction_needs_a_saved_session() {
        let mut req = request(PermissionPolicy::Ask);
        req.compact = true;
        let mut parser = GrokParser::new(&req);
        parser.initialize();
        let events = parser.parse_line(&reply(1, json!({})));
        assert_eq!(events[0], AgentEvent::Error(NOTHING_TO_COMPACT.into()));
        assert_eq!(events[1], AgentEvent::Done(DoneStatus::Failed));
        assert!(
            parser.take_replies().is_empty(),
            "no session is made to compact"
        );
    }

    #[test]
    fn a_resumed_thread_resumes_then_loads_muted_then_starts_over() {
        let mut req = request(PermissionPolicy::Ask);
        req.resume_id = Some("old".into());
        let mut parser = GrokParser::new(&req);
        parser.initialize();
        parser.parse_line(&reply(1, json!({})));
        let resume = sent(&mut parser);
        assert_eq!(resume[0]["method"], "session/resume");
        assert_eq!(resume[0]["params"]["sessionId"], "old");

        parser.parse_line(
            &json!({ "jsonrpc": "2.0", "id": 2, "error": { "message": "unknown" } }).to_string(),
        );
        assert_eq!(sent(&mut parser)[0]["method"], "session/load");
        let replay = json!({ "jsonrpc": "2.0", "method": "session/update", "params": {
            "update": { "sessionUpdate": "agent_message_chunk", "content": { "text": "old answer" } } } });
        assert!(
            parser.parse_line(&replay.to_string()).is_empty(),
            "history replay is muted"
        );

        parser.parse_line(
            &json!({ "jsonrpc": "2.0", "id": 3, "error": { "message": "gone" } }).to_string(),
        );
        assert_eq!(sent(&mut parser)[0]["method"], "session/new");
        let events = parser.parse_line(&reply(4, json!({ "sessionId": "new" })));
        assert_eq!(
            events,
            vec![AgentEvent::SessionStarted {
                provider_session_id: "new".into()
            }]
        );
        assert_eq!(sent(&mut parser)[0]["method"], "session/prompt");
    }

    #[test]
    fn a_resumed_session_on_another_model_is_switched() {
        let mut req = request(PermissionPolicy::Ask);
        req.resume_id = Some("old".into());
        let mut parser = GrokParser::new(&req);
        parser.initialize();
        parser.parse_line(&reply(1, json!({})));
        sent(&mut parser);
        parser.parse_line(&reply(
            2,
            json!({ "models": { "currentModelId": "grok-4.5" } }),
        ));
        let set = sent(&mut parser);
        assert_eq!(set[0]["method"], "session/set_model");
        assert_eq!(
            set[0]["params"],
            json!({ "sessionId": "old", "modelId": "grok-4.6" })
        );
        parser.parse_line(&reply(3, json!({})));
        assert_eq!(sent(&mut parser)[0]["method"], "session/prompt");
    }

    #[test]
    fn supervised_permissions_ask_and_the_reply_picks_the_option() {
        let mut parser = bound(PermissionPolicy::Ask);
        let ask = json!({
            "jsonrpc": "2.0", "id": 7, "method": "session/request_permission",
            "params": {
                "toolCall": { "toolCallId": "call-2", "kind": "execute", "title": "Execute `git status`",
                              "rawInput": { "variant": "Bash", "command": "git status" } },
                "options": [{ "optionId": "allow_once" }, { "optionId": "reject_once" }],
            },
        });
        let events = parser.parse_line(&ask.to_string());
        assert!(
            parser.take_replies().is_empty(),
            "nothing is answered for the user"
        );
        let AgentEvent::PermissionRequest(request) = &events[1] else {
            panic!("expected a permission request, got {events:?}");
        };
        assert_eq!(request.tool, "Bash");
        assert_eq!(request.description, "Execute `git status`");
        let allowed: Value = serde_json::from_str(&permission_response(request, true)).unwrap();
        assert_eq!(allowed["id"], 7);
        assert_eq!(allowed["result"], selected("allow_once"));
        let denied: Value = serde_json::from_str(&permission_response(request, false)).unwrap();
        assert_eq!(denied["result"], selected("reject_once"));
    }

    #[test]
    fn access_modes_answer_by_themselves() {
        let options = ["allow-once", "allow-always", "reject-once"];
        assert_eq!(auto_option(PermissionPolicy::Ask, "edit", &options), None);
        assert_eq!(
            auto_option(PermissionPolicy::AcceptEdits, "edit", &options),
            Some("allow-once")
        );
        assert_eq!(
            auto_option(PermissionPolicy::AcceptEdits, "execute", &options),
            None
        );
        assert_eq!(
            auto_option(PermissionPolicy::AutoApprove, "execute", &options),
            Some("allow-always")
        );
        assert_eq!(
            auto_option(PermissionPolicy::Auto, "execute", &options),
            Some("allow-once")
        );

        let mut plan = request(PermissionPolicy::AutoApprove);
        plan.plan = true;
        let mut parser = GrokParser::new(&plan);
        let ask = |kind: &str| {
            json!({ "jsonrpc": "2.0", "id": 9, "method": "session/request_permission", "params": {
                "toolCall": { "toolCallId": "c", "kind": kind },
                "options": [{ "optionId": "allow-once" }, { "optionId": "reject-once" }] } })
            .to_string()
        };
        parser.parse_line(&ask("edit"));
        assert_eq!(sent(&mut parser)[0]["result"], selected("reject-once"));
        parser.parse_line(&ask("read"));
        assert_eq!(sent(&mut parser)[0]["result"], selected("allow-once"));
    }

    #[test]
    fn questions_use_the_composer_form() {
        let mut parser = bound(PermissionPolicy::AutoApprove);
        let ask = json!({ "jsonrpc": "2.0", "id": 11, "method": "_x.ai/ask_user_question", "params": {
            "questions": [{ "question": "Which colour?", "options": [{ "label": "Red" }, { "label": "Blue" }] }] } });
        let events = parser.parse_line(&ask.to_string());
        let AgentEvent::PermissionRequest(request) = &events[0] else {
            panic!("expected a question, got {events:?}");
        };
        assert_eq!(request.tool, QUESTION_TOOL);
        assert_eq!(request.input["questions"][0]["question"], "Which colour?");

        let mut answered = request.clone();
        answered.input = json!({ "questions": [], "answers": { "Which colour?": "Blue" } });
        let reply: Value = serde_json::from_str(&permission_response(&answered, true)).unwrap();
        assert_eq!(reply["id"], 11);
        assert_eq!(
            reply["result"],
            json!({ "outcome": "accepted", "answers": { "Which colour?": "Blue" } })
        );
        let skipped: Value = serde_json::from_str(&permission_response(request, false)).unwrap();
        assert_eq!(skipped["result"], json!({ "outcome": "skip_interview" }));
    }

    #[test]
    fn exit_plan_mode_shows_the_plan_and_abandons_the_turn() {
        let mut parser = bound(PermissionPolicy::Ask);
        let exit = json!({ "jsonrpc": "2.0", "id": 12, "method": "_x.ai/exit_plan_mode", "params": { "planContent": " Ship it " } });
        assert_eq!(
            parser.parse_line(&exit.to_string()),
            vec![AgentEvent::TextDelta("Ship it".into())]
        );
        assert_eq!(
            sent(&mut parser)[0]["result"],
            json!({ "outcome": "abandoned" })
        );
    }

    #[test]
    fn unknown_requests_get_method_not_found() {
        let mut parser = bound(PermissionPolicy::Ask);
        parser.parse_line(
            &json!({ "jsonrpc": "2.0", "id": 13, "method": "fs/read_text_file", "params": {} })
                .to_string(),
        );
        assert_eq!(sent(&mut parser)[0]["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn a_failed_start_explains_the_sign_in() {
        let mut parser = GrokParser::new(&request(PermissionPolicy::Ask));
        parser.initialize();
        parser.parse_line(&reply(1, json!({})));
        sent(&mut parser);
        let events = parser.parse_line(
            &json!({ "jsonrpc": "2.0", "id": 2, "error": { "message": "Authentication required" } }).to_string(),
        );
        let AgentEvent::Error(message) = &events[0] else {
            panic!("expected an error, got {events:?}");
        };
        assert!(message.contains("grok login"), "{message}");
        assert_eq!(events[1], AgentEvent::Done(DoneStatus::Failed));
    }

    #[test]
    fn catalogs_come_from_acp_and_the_cli() {
        let models = models_from_acp(&json!({ "_meta": { "modelState": {
            "currentModelId": "grok-4.6",
            "availableModels": [{
                "modelId": "grok-4.6", "name": "Grok 4.6",
                "_meta": {
                    "totalContextTokens": 500000, "reasoningEffort": "high",
                    "reasoningEfforts": [
                        { "id": "xhigh", "value": "xhigh", "label": "Extra High Effort" },
                        { "id": "high", "value": "high", "label": "High Effort", "default": true },
                        { "id": "deep", "value": "deep", "label": "Deep Effort" },
                    ],
                },
            }],
        } } }));
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].key, "grok:grok-4.6");
        assert_eq!(models[0].settings[0].default, "high");
        assert_eq!(
            models[0].settings[0].options,
            [
                ("xhigh".to_string(), "Extra High".to_string()),
                ("high".to_string(), "High".to_string()),
                ("deep".to_string(), "Deep".to_string()),
            ]
        );
        let new = models_from_acp(
            &json!({ "models": { "availableModels": [{ "modelId": "grok-4.5" }] } }),
        );
        assert_eq!(new[0].label, "Grok 4.5");

        let cli = models_from_cli(
            "You are not authenticated.\n\nDefault model: grok-4.6\n\nAvailable models:\n  \u{1b}[1m* grok-4.6\u{1b}[0m (default)\n  - grok-4.5\n",
        );
        assert_eq!(
            cli.iter().map(|m| m.native.as_str()).collect::<Vec<_>>(),
            ["grok-4.6", "grok-4.5"]
        );
    }

    #[test]
    fn context_window_follows_the_current_model() {
        let setup = json!({ "models": { "currentModelId": "b", "availableModels": [
            { "modelId": "a", "_meta": { "totalContextTokens": 1 } },
            { "modelId": "b", "_meta": { "totalContextTokens": 2 } },
        ] } });
        assert_eq!(context_window(&setup), Some(2));
        assert_eq!(context_window(&json!({})), None);
    }

    #[test]
    fn only_xais_grok_counts() {
        let dir = std::env::temp_dir().join(format!("bencode-grok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("grok");
        std::fs::write(&other, "#!/bin/sh\necho some other grok\n").unwrap();
        assert!(!is_grok_build(&other));
        std::fs::write(&other, "#!/bin/sh\n# Grok Build\n").unwrap();
        assert!(is_grok_build(&other));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(is_grok_build(Path::new("/Users/x/.grok/bin/grok")));
    }
}
