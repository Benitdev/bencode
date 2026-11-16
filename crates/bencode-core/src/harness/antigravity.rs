//! Antigravity (`agy`). From 1.5.1 a turn runs `agy acp` (`acp.rs`), as
//! MonoCode runs its separate `agy_acp_server.par`: the thread's
//! conversation is the ACP session (print mode's conversation ids resume
//! there), its model and mode are set on the session, and the MCP servers
//! BenCode offers (`SpawnRequest::mcp_servers`: the browser, Backlog) go
//! with it, which print mode cannot take. Commands are asked about as
//! `agy`'s own `toolPermission` setting says: with `always-proceed` it asks
//! nothing, whatever the thread's access.
//!
//! An older `agy` runs in print mode, `--output-format stream-json`.
//! Events are `{"event": "init" | "step_update" | "result" | "error", …}`.
//! A tool runs as a `"tool"` step (`tool_info`: name, parameters, output),
//! reported `ACTIVE` then `DONE`; older builds sent `"tool_call"` steps.
//! The result's `usage` adds up every model call of the turn, so the
//! context meter reads the last call's instead; a result that is not
//! `SUCCESS` says why in its `error`.

use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde_json::{Map, Value, json};

use crate::harness::acp::{self, AcpParser, Dialect};
use crate::harness::events::{AgentEvent, DoneStatus, TurnMetrics};
use crate::harness::handle::HarnessProcessHandle;
use crate::harness::process::{self, LineParser, ProcessSpec, StdinMode};
use crate::harness::resolver::{HarnessResolver, cli_version};
use crate::harness::{EventRx, PermissionPolicy, SpawnRequest, str_field};

const SUCCESS_STATUS: &str = "SUCCESS";
/// `agy acp` came in 1.5.0; 1.5.1 stopped its turns hanging and its
/// updates arriving after the prompt returned.
const ACP_SINCE: [u64; 3] = [1, 5, 1];
const SIGN_IN_HELP: &str = "Antigravity is not signed in. Run `agy` once in a terminal to sign in.";

pub fn spawn(req: &SpawnRequest) -> Result<(HarnessProcessHandle, EventRx)> {
    let program = HarnessResolver::resolve_antigravity_cli().unwrap_or_else(|| "agy".into());
    if speaks_acp(&program) {
        return spawn_acp(req, program);
    }
    let spec = ProcessSpec {
        program,
        args: build_args(req),
        cwd: req.cwd.clone(),
        stdin: StdinMode::Null,
        permission_responder: None,
        can_steer: false,
        account: None,
        env: super::app_env(req),
    };
    process::spawn(spec, AntigravityParser::default())
}

// ---------------------------------------------------------------------------
// ACP
// ---------------------------------------------------------------------------

const ACP_BACKGROUND_NOTE: &str = "ACP execution note: This client runs one session/prompt per turn and \
    does not auto-resume after end_turn. Never end your turn while waiting for a background \
    command, subagent, schedule, or workflow you started; wait for all tasks to finish (or check \
    them with manage_task) before giving your final reply.";

fn acp_session_meta() -> Option<Value> {
    Some(json!({
        "antigravity": {
            "systemPrompt": {
                "append": ACP_BACKGROUND_NOTE
            }
        }
    }))
}

/// Antigravity's ACP.
static AGY: Dialect = Dialect {
    label: "Antigravity",
    log: "agy",
    // `agy` is signed in by itself (Keychain); nothing to authenticate.
    auth_method: |_| None,
    start_error,
    prompt_error,
    new_session_params: |cwd, _, _, servers| json!({ "cwd": cwd, "mcpServers": servers }),
    session_meta: acp_session_meta,
    resume_with_cwd: true,
    mode: acp_mode,
    compact: None,
    interject: false,
    cached_in_input: false,
    approves_in_auto: false,
    wait_open_tools: true,
};

fn spawn_acp(req: &SpawnRequest, program: PathBuf) -> Result<(HarnessProcessHandle, EventRx)> {
    let mut parser = AcpParser::new(req, &AGY);
    let spec = ProcessSpec {
        program,
        args: acp_args(req),
        cwd: req.cwd.clone(),
        stdin: StdinMode::Protocol {
            initial: format!("{}\n", parser.initialize()),
        },
        permission_responder: Some(acp::permission_response),
        can_steer: false,
        account: None,
        env: super::app_env(req),
    };
    process::spawn(spec, parser)
}

/// Full access approves everything, as print mode's flag of the same name.
fn acp_args(req: &SpawnRequest) -> Vec<String> {
    let mut args = vec!["acp".to_string()];
    if req.permission == PermissionPolicy::AutoApprove && !req.plan {
        args.push("--dangerously-skip-permissions".into());
    }
    args
}

/// The session mode for the thread's access: set each turn, as a resumed
/// session keeps the last one (a plan turn's `plan`).
fn acp_mode(permission: PermissionPolicy, plan: bool) -> Option<&'static str> {
    Some(match (plan, permission) {
        (true, _) => "plan",
        (false, PermissionPolicy::AcceptEdits) => "accept-edits",
        (false, PermissionPolicy::Auto) => "auto",
        (false, PermissionPolicy::Ask | PermissionPolicy::AutoApprove) => "default",
    })
}

fn mentions_sign_in(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    ["auth", "login", "log in", "sign in", "credential"]
        .iter()
        .any(|word| lower.contains(word))
}

fn start_error(detail: &str) -> String {
    match mentions_sign_in(detail) {
        true => format!("{}\n\n{SIGN_IN_HELP}", detail.trim()),
        false => format!("Antigravity did not start. {}", detail.trim()),
    }
}

fn prompt_error(err: String) -> String {
    match mentions_sign_in(&err) {
        true => format!("{}\n\n{SIGN_IN_HELP}", err.trim()),
        false => format!("Antigravity: {}", err.trim()),
    }
}

/// The `agy` whose answer is kept: its path, when it was written and its
/// size, so an update asks again.
type Stamp = (PathBuf, Option<SystemTime>, u64);

static SPEAKS_ACP: Mutex<Option<(Stamp, bool)>> = Mutex::new(None);

/// Whether `program` runs `agy acp` (1.5.1 or later). Asks its version
/// once per binary (60ms); model discovery asks first, off the UI thread.
pub fn speaks_acp(program: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(program) else {
        return false;
    };
    let stamp = (program.to_path_buf(), meta.modified().ok(), meta.len());
    let mut cached = SPEAKS_ACP.lock().unwrap_or_else(|err| err.into_inner());
    if let Some((seen, speaks)) = cached.as_ref()
        && *seen == stamp
    {
        return *speaks;
    }
    let version = cli_version(program);
    let speaks = version
        .as_deref()
        .is_some_and(|version| version >= &ACP_SINCE[..]);
    log::info!(
        "Antigravity {version:?}: {}",
        if speaks { "ACP" } else { "print mode" }
    );
    *cached = Some((stamp, speaks));
    speaks
}

// ---------------------------------------------------------------------------
// Print mode
// ---------------------------------------------------------------------------

fn build_args(req: &SpawnRequest) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--output-format".into(),
        "stream-json".into(),
        "--print".into(),
        crate::harness::attachments::plain_prompt(&req.prompt, &req.attachments),
    ];
    match req.permission {
        PermissionPolicy::AutoApprove => args.push("--dangerously-skip-permissions".into()),
        PermissionPolicy::Ask | PermissionPolicy::AcceptEdits | PermissionPolicy::Auto => {}
    }
    if let Some(model) = &req.model {
        args.extend(["--model".into(), model.clone()]);
    }
    if let Some(conversation) = &req.resume_id {
        args.extend(["--conversation".into(), conversation.clone()]);
    }
    args
}

#[derive(Default)]
pub struct AntigravityParser {
    /// The last model call's usage: how full the context is now.
    last_usage: Option<Value>,
    /// Tool steps already started, so `DONE` only finishes them.
    started: HashSet<String>,
}

impl LineParser for AntigravityParser {
    fn parse_line(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(rec) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        match str_field(&rec, "event") {
            Some("init") => {
                if let Some(id) = str_field(&rec, "conversation_id") {
                    events.push(AgentEvent::SessionStarted {
                        provider_session_id: id.to_string(),
                    });
                }
            }
            Some("step_update") => {
                if let Some(step) = rec.get("step_update") {
                    self.on_step(step, &mut events);
                }
            }
            Some("result") => {
                let result = rec.get("result").unwrap_or(&Value::Null);
                if let Some(usage) = self
                    .last_usage
                    .take()
                    .or_else(|| result.get("usage").cloned())
                {
                    events.push(usage_event(&usage));
                }
                if let Some(usage) = result.get("usage") {
                    events.extend(metrics_event(usage));
                }
                match str_field(result, "status") {
                    None | Some(SUCCESS_STATUS) => {
                        events.push(AgentEvent::Done(DoneStatus::Completed))
                    }
                    Some(status) if recovered(result) => {
                        // The retry answered: the turn is whole, the failed
                        // attempt is only `agy`'s to report.
                        log::warn!("{} (recovered)", result_error(status, result));
                        events.push(AgentEvent::Done(DoneStatus::Completed));
                    }
                    Some(status) => {
                        events.push(AgentEvent::Error(result_error(status, result)));
                        events.push(AgentEvent::Done(DoneStatus::Failed));
                    }
                }
            }
            Some("error") => {
                let message = str_field(&rec, "error")
                    .or_else(|| rec.pointer("/error/message").and_then(Value::as_str))
                    .unwrap_or("Antigravity error");
                events.push(AgentEvent::Error(message.to_string()));
            }
            _ => {}
        }
        events
    }
}

impl AntigravityParser {
    fn on_step(&mut self, step: &Value, events: &mut Vec<AgentEvent>) {
        match str_field(step, "step_type") {
            Some("agent_response") => {
                if let Some(thinking) = str_field(step, "thinking_delta").filter(|t| !t.is_empty())
                {
                    events.push(AgentEvent::ThinkingDelta(thinking.to_string()));
                }
                if let Some(text) = str_field(step, "text_delta").filter(|t| !t.is_empty()) {
                    events.push(AgentEvent::TextDelta(text.to_string()));
                }
                if let Some(usage) = step.get("usage") {
                    self.last_usage = Some(usage.clone());
                }
            }
            Some("tool") => {
                let Some(info) = step.get("tool_info") else {
                    return;
                };
                let index = step
                    .get("step_index")
                    .map(Value::to_string)
                    .unwrap_or_default();
                let conversation = str_field(step, "conversation_id").unwrap_or_default();
                let id = format!("agy-{conversation}-{index}");
                let name = str_field(info, "name")
                    .or_else(|| str_field(step, "tool_name"))
                    .unwrap_or("tool");
                let parameters = info.get("parameters").unwrap_or(&Value::Null);
                self.tool(events, &id, name, parameters, info, step);
            }
            // Older builds' shape.
            Some("tool_call") => {
                let Some(call) = step.get("tool_call") else {
                    return;
                };
                let Some(id) = str_field(call, "id") else {
                    return;
                };
                let name = str_field(call, "name").unwrap_or("tool");
                let input = call.get("input").unwrap_or(&Value::Null);
                self.tool(events, id, name, input, call, step);
            }
            _ => {}
        }
    }

    /// A tool step: started once, finished when it is `DONE` (or failed).
    fn tool(
        &mut self,
        events: &mut Vec<AgentEvent>,
        id: &str,
        name: &str,
        input: &Value,
        info: &Value,
        step: &Value,
    ) {
        if self.started.insert(id.to_string()) {
            events.push(AgentEvent::ToolCallStart {
                id: id.to_string(),
                name: name.to_string(),
                input: with_common_keys(name, input),
            });
        }
        let state = str_field(step, "state").unwrap_or("");
        let failed = info.get("error").is_some_and(|e| !e.is_null())
            || matches!(state, "ERROR" | "FAILED" | "CANCELED" | "CANCELLED");
        if state == "DONE" || failed {
            let output = str_field(info, "output")
                .or_else(|| str_field(info, "error"))
                .unwrap_or_default();
            events.push(AgentEvent::ToolCallFinish {
                id: id.to_string(),
                output: output.to_string(),
                success: !failed,
                patch: None,
            });
        }
    }
}

/// `agy`'s parameters (`CommandLine`, `TargetFile`, `AbsolutePath`, …) with
/// the keys the transcript titles tools by added alongside. A call with
/// none of them (`manage_task`'s `Action` and `TaskId`) is titled by its
/// own summary when it gives one, else by its name, never its JSON.
fn with_common_keys(name: &str, input: &Value) -> Value {
    let Some(params) = input.as_object() else {
        return input.clone();
    };
    let mut out: Map<String, Value> = params.clone();
    let aliases = [
        ("command", &["CommandLine"][..]),
        (
            "path",
            &[
                "AbsolutePath",
                "TargetFile",
                "File",
                "DirectoryPath",
                "SearchPath",
            ][..],
        ),
        ("pattern", &["Pattern"][..]),
        ("query", &["Query"][..]),
        ("url", &["Url", "URL"][..]),
        ("description", &["toolSummary", "toolAction"][..]),
    ];
    for (key, from) in aliases {
        if out.contains_key(key) {
            continue;
        }
        if let Some(value) = from.iter().find_map(|name| params.get(*name)) {
            out.insert(key.to_string(), value.clone());
        }
    }
    let titled = ["command", "path", "pattern", "query", "url", "description"]
        .iter()
        .any(|key| out.get(*key).is_some_and(Value::is_string));
    if !titled {
        out.insert(
            "description".into(),
            Value::String(tool_label(name, params)),
        );
    }
    Value::Object(out)
}

/// `manage_task` with `Action: kill` reads "Manage task: kill".
fn tool_label(name: &str, params: &Map<String, Value>) -> String {
    let mut label = name.replace('_', " ");
    if let Some(first) = label.get(..1) {
        label = first.to_uppercase() + &label[1..];
    }
    match params.get("Action").and_then(Value::as_str) {
        Some(action) if !action.trim().is_empty() => format!("{label}: {}", action.trim()),
        _ => label,
    }
}

/// A turn `agy` calls an error only for a model call it retried: its
/// `error` names the failed attempt ("API error (attempt 1): … connection
/// reset by peer") and the retry's reply is the `response`.
fn recovered(result: &Value) -> bool {
    let retried = str_field(result, "error").is_some_and(|e| e.starts_with("API error (attempt"));
    let answered = str_field(result, "response").is_some_and(|r| !r.trim().is_empty());
    retried && answered
}

/// What a failed turn says: `agy`'s own reason when it gives one.
fn result_error(status: &str, result: &Value) -> String {
    match str_field(result, "error")
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        Some(error) => format!("Antigravity: {error}"),
        None => format!("Antigravity finished with status {status}"),
    }
}

/// MonoCode `antigravityProtocol` usage: camelCase or snake_case counts.
fn metrics_event(usage: &Value) -> Option<AgentEvent> {
    let field = |camel: &str, snake: &str| {
        usage
            .get(camel)
            .or_else(|| usage.get(snake))
            .and_then(Value::as_u64)
    };
    let read = field("cacheReadTokens", "cache_read_input_tokens");
    let write = field("cacheWriteTokens", "cache_creation_input_tokens");
    let input = field("inputTokens", "input_tokens").unwrap_or(0);
    let (read_n, write_n) = (read.unwrap_or(0), write.unwrap_or(0));
    TurnMetrics::from_counts(
        input,
        field("outputTokens", "output_tokens").unwrap_or(0),
        read_n,
        write_n,
        (read.is_some() || write.is_some()).then_some(input + read_n + write_n),
    )
    .map(AgentEvent::TurnMetrics)
}

fn usage_event(usage: &Value) -> AgentEvent {
    let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
    let input_tokens = field("input_tokens");
    let output_tokens = field("output_tokens");
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(input_tokens + output_tokens);
    AgentEvent::Usage {
        input_tokens,
        output_tokens,
        total_tokens,
        context_window: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recorded from `agy -p "Reply with exactly: hi" --output-format stream-json`.
    const RECORDED: &[&str] = &[
        r#"{"event":"init","conversation_id":"d42c","init":{"cwd":"/tmp","permission_mode":"always-proceed"}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"d42c","step_index":0,"state":"DONE","step_type":"user_input"}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"d42c","step_index":1,"state":"DONE","step_type":"agent_response","text_delta":"hi\n","usage":{"input_tokens":12231,"output_tokens":22,"total_tokens":12253}}}"#,
        r#"{"event":"result","result":{"conversation_id":"d42c","status":"SUCCESS","response":"hi\n","usage":{"input_tokens":12231,"output_tokens":22,"total_tokens":12253}}}"#,
    ];

    #[test]
    fn parses_recorded_print_session() {
        let mut parser = AntigravityParser::default();
        let events: Vec<_> = RECORDED
            .iter()
            .flat_map(|line| parser.parse_line(line))
            .collect();
        assert_eq!(
            events,
            vec![
                AgentEvent::SessionStarted {
                    provider_session_id: "d42c".into()
                },
                AgentEvent::TextDelta("hi\n".into()),
                AgentEvent::Usage {
                    input_tokens: 12231,
                    output_tokens: 22,
                    total_tokens: 12253,
                    context_window: None,
                },
                AgentEvent::TurnMetrics(TurnMetrics {
                    input_tokens: Some(12231),
                    output_tokens: Some(22),
                    ..Default::default()
                }),
                AgentEvent::Done(DoneStatus::Completed),
            ]
        );
    }

    #[test]
    fn non_success_result_is_an_error() {
        let mut parser = AntigravityParser::default();
        let events = parser.parse_line(r#"{"event":"result","result":{"status":"CANCELLED"}}"#);
        assert_eq!(events.last(), Some(&AgentEvent::Done(DoneStatus::Failed)));
    }

    #[test]
    fn a_failed_result_says_why() {
        let mut parser = AntigravityParser::default();
        // Recorded from `agy --conversation <unknown id>`.
        let events = parser.parse_line(
            r#"{"event":"result","result":{"conversation_id":"30e6","status":"ERROR","response":"","error":"failed to send message: trajectory not found: 30e6","usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0}}}"#,
        );
        assert!(events.contains(&AgentEvent::Error(
            "Antigravity: failed to send message: trajectory not found: 30e6".into()
        )));
    }

    #[test]
    fn a_retried_call_that_answered_is_a_finished_turn() {
        let mut parser = AntigravityParser::default();
        // As `agy` reports a turn whose first model call was cut off.
        let events = parser.parse_line(
            r#"{"event":"result","result":{"conversation_id":"83bc","status":"ERROR","response":"Có chứ!\n","error":"API error (attempt 1): request failed: Post \"https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse\": read tcp 10.10.1.211:56340->172.217.115.4:443: read: connection reset by peer"}}"#,
        );
        assert_eq!(events, [AgentEvent::Done(DoneStatus::Completed)]);
        // Without a reply it is still the failure it says.
        let events = parser.parse_line(
            r#"{"event":"result","result":{"status":"ERROR","response":"","error":"API error (attempt 3): request failed"}}"#,
        );
        assert_eq!(events.last(), Some(&AgentEvent::Done(DoneStatus::Failed)));
    }

    /// Recorded from `agy -p "run the shell command: ls /tmp | head -3, then
    /// tell me the result" --output-format stream-json` (agy 2026-10).
    const TOOL_TURN: &[&str] = &[
        r#"{"event":"step_update","step_update":{"conversation_id":"8b5f","step_index":1,"state":"DONE","step_type":"agent_response","usage":{"input_tokens":18668,"output_tokens":314,"total_tokens":18982}}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"8b5f","step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"ls /tmp | head -3"}}}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"8b5f","step_index":2,"state":"DONE","step_type":"tool","tool_name":"run_command","duration_seconds":0.07,"tool_info":{"name":"run_command","parameters":{"CommandLine":"ls /tmp | head -3"},"output":"after.txt\r\n"}}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"8b5f","step_index":3,"state":"DONE","step_type":"agent_response","text_delta":"Done.","usage":{"input_tokens":19081,"output_tokens":65,"total_tokens":19146}}}"#,
        r#"{"event":"result","result":{"conversation_id":"8b5f","status":"SUCCESS","response":"Done.","usage":{"input_tokens":37749,"output_tokens":379,"total_tokens":38128}}}"#,
    ];

    #[test]
    fn a_call_without_a_path_or_command_is_titled_by_name() {
        let task = serde_json::json!({ "Action": "kill", "TaskId": "abc/task-1" });
        let input = with_common_keys("manage_task", &task);
        assert_eq!(
            crate::harness::summarize_tool_input("manage_task", &input),
            "Manage task: kill"
        );
        let summarized =
            serde_json::json!({ "Action": "status", "toolSummary": "Check test task status" });
        let input = with_common_keys("manage_task", &summarized);
        assert_eq!(
            crate::harness::summarize_tool_input("manage_task", &input),
            "Check test task status"
        );
        let command = with_common_keys("run_command", &serde_json::json!({ "CommandLine": "ls" }));
        assert_eq!(command.get("description"), None);
    }

    #[test]
    fn tool_steps_start_once_and_finish() {
        let mut parser = AntigravityParser::default();
        let events: Vec<_> = TOOL_TURN
            .iter()
            .flat_map(|line| parser.parse_line(line))
            .collect();
        let starts: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ToolCallStart { id, name, input } => Some((id, name, input)),
                _ => None,
            })
            .collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].0, "agy-8b5f-2");
        assert_eq!(starts[0].1, "run_command");
        assert_eq!(starts[0].2["command"], "ls /tmp | head -3");
        assert!(events.contains(&AgentEvent::ToolCallFinish {
            id: "agy-8b5f-2".into(),
            output: "after.txt\r\n".into(),
            success: true,
            patch: None,
        }));
    }

    #[test]
    fn the_context_is_the_last_calls_not_the_turns_total() {
        let mut parser = AntigravityParser::default();
        let events: Vec<_> = TOOL_TURN
            .iter()
            .flat_map(|line| parser.parse_line(line))
            .collect();
        assert!(events.contains(&AgentEvent::Usage {
            input_tokens: 19081,
            output_tokens: 65,
            total_tokens: 19146,
            context_window: None,
        }));
        // What the turn cost is still the whole of it.
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::TurnMetrics(TurnMetrics {
                input_tokens: Some(37749),
                ..
            })
        )));
    }

    // ---- ACP --------------------------------------------------------------

    use crate::harness::HarnessKind;
    use crate::harness::events::PermissionRequest;

    fn acp_request(permission: PermissionPolicy) -> SpawnRequest {
        SpawnRequest {
            harness: HarnessKind::Antigravity,
            cwd: "/work".into(),
            prompt: "hi".into(),
            model: Some("gemini-4-argon-low".into()),
            permission,
            resume_id: None,
            disable_hooks: false,
            attachments: Vec::new(),
            plan: false,
            compact: false,
            settings: Default::default(),
            account: None,
            mcp_servers: vec![crate::mcp::McpLaunch {
                name: "bencode-backlog".into(),
                command: "/bin/bencode".into(),
                args: vec!["--backlog-mcp".into()],
            }],
            app: None,
            system_prompt: None,
        }
    }

    fn sent(parser: &mut AcpParser) -> Vec<Value> {
        parser
            .take_replies()
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn reply(id: &Value, result: Value) -> String {
        json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
    }

    /// `session/new`'s answer as `agy acp` 1.5.8 gives it, cut down.
    fn new_session_result() -> Value {
        json!({
            "sessionId": "ses-1",
            "models": {
                "currentModelId": "gemini-4-argon-medium",
                "availableModels": [
                    { "modelId": "gemini-4-argon-medium", "name": "Gemini 4 Argon (Medium)" },
                    { "modelId": "gemini-4-argon-low", "name": "Gemini 4 Argon (Low)" },
                ],
            },
            "modes": { "currentModeId": "default", "availableModes": [
                { "id": "default" }, { "id": "accept-edits" }, { "id": "auto" }, { "id": "plan" },
            ] },
        })
    }

    /// Through `initialize` and `session/new` to the setup it asks for;
    /// returns the parser and the requests sent before the prompt.
    fn opened(req: &SpawnRequest) -> (AcpParser, Vec<Value>) {
        let mut parser = AcpParser::new(req, &AGY);
        let init: Value = serde_json::from_str(&parser.initialize()).unwrap();
        assert_eq!(init["method"], "initialize");
        parser.parse_line(&reply(&init["id"], json!({ "protocolVersion": 1 })));
        let mut asked = sent(&mut parser);
        // No `authenticate`: `agy` is signed in by itself.
        assert_eq!(asked.len(), 1);
        let open = asked.remove(0);
        let result = match open["method"].as_str() {
            Some("session/new") => new_session_result(),
            _ => json!({ "sessionId": req.resume_id, "modes": { "currentModeId": "plan" } }),
        };
        let events = parser.parse_line(&reply(&open["id"], result));
        assert!(matches!(&events[0], AgentEvent::SessionStarted { .. }));
        let mut requests = vec![open];
        loop {
            let next = sent(&mut parser).remove(0);
            if next["method"] == "session/prompt" {
                requests.push(next);
                return (parser, requests);
            }
            parser.parse_line(&reply(&next["id"], json!({})));
            requests.push(next);
        }
    }

    #[test]
    fn full_access_skips_permissions_and_plan_mode_does_not() {
        assert_eq!(acp_args(&acp_request(PermissionPolicy::Ask)), ["acp"]);
        assert_eq!(
            acp_args(&acp_request(PermissionPolicy::AutoApprove)),
            ["acp", "--dangerously-skip-permissions"]
        );
        let plan = SpawnRequest {
            plan: true,
            ..acp_request(PermissionPolicy::AutoApprove)
        };
        assert_eq!(acp_args(&plan), ["acp"]);
    }

    #[test]
    fn a_new_session_gets_the_servers_model_and_mode() {
        let (_, requests) = opened(&acp_request(PermissionPolicy::AcceptEdits));
        let methods: Vec<&str> = requests
            .iter()
            .filter_map(|r| r["method"].as_str())
            .collect();
        assert_eq!(
            methods,
            [
                "session/new",
                "session/set_model",
                "session/set_mode",
                "session/prompt"
            ]
        );
        assert_eq!(requests[0]["params"]["cwd"], "/work");
        assert_eq!(
            requests[0]["params"]["mcpServers"][0]["name"],
            "bencode-backlog"
        );
        assert_eq!(
            requests[0]["params"]["mcpServers"][0]["args"],
            json!(["--backlog-mcp"])
        );
        assert_eq!(requests[1]["params"]["modelId"], "gemini-4-argon-low");
        assert_eq!(requests[2]["params"]["modeId"], "accept-edits");
        assert_eq!(requests[3]["params"]["sessionId"], "ses-1");
        assert_eq!(requests[3]["params"]["prompt"][0]["text"], "hi");
        assert!(
            requests[0]["params"]["_meta"]["antigravity"]["systemPrompt"]["append"]
                .as_str()
                .is_some_and(|s| s.contains("manage_task"))
        );
    }

    #[test]
    fn a_thread_resumes_its_conversation_and_leaves_plan_mode() {
        let req = SpawnRequest {
            resume_id: Some("a60b1698".into()),
            model: None,
            mcp_servers: Vec::new(),
            ..acp_request(PermissionPolicy::Ask)
        };
        let (_, requests) = opened(&req);
        assert_eq!(requests[0]["method"], "session/resume");
        assert_eq!(requests[0]["params"]["sessionId"], "a60b1698");
        // `agy` binds a resumed session to a folder even without servers.
        assert_eq!(requests[0]["params"]["cwd"], "/work");
        assert_eq!(requests[0]["params"]["mcpServers"], json!([]));
        assert_eq!(requests[1]["method"], "session/set_mode");
        assert_eq!(requests[1]["params"]["modeId"], "default");
    }

    /// Feeds a recorded turn; the prompt's answer takes the prompt's id.
    fn play(parser: &mut AcpParser, prompt_id: &Value, fixture: &str) -> Vec<AgentEvent> {
        fixture
            .lines()
            .map(|line| line.replace("\"id\":\"PROMPT\"", &format!("\"id\":{prompt_id}")))
            .flat_map(|line| parser.parse_line(&line))
            .collect()
    }

    const TURN: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/antigravity_acp_turn.jsonl"
    ));
    const PERMISSION: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/antigravity_acp_permission.jsonl"
    ));

    #[test]
    fn parses_a_recorded_acp_turn() {
        let (mut parser, requests) = opened(&acp_request(PermissionPolicy::AutoApprove));
        let events = play(&mut parser, &requests.last().unwrap()["id"], TURN);
        let usage = |used| AgentEvent::Usage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: used,
            context_window: Some(1_048_576),
        };
        let starts: Vec<(&str, &Value)> = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::ToolCallStart { name, input, .. } => Some((name.as_str(), input)),
                _ => None,
            })
            .collect();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[0].0, "Bash");
        assert_eq!(starts[0].1["command"], "echo hello-acp > note.txt");
        assert_eq!(starts[1].0, "mcp_bencode-backlog_backlog_list_projects");
        assert_eq!(starts[1].1["description"], "List active Backlog projects");
        let finished = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolCallFinish { success: true, .. }))
            .count();
        assert_eq!(finished, 2);
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::TextDelta(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.ends_with("tool lists 4 projects."), "{text}");
        // The meter follows `usage_update`; the turn's totals (every model
        // call added up) do not move it.
        let meters: Vec<&AgentEvent> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::Usage { .. }))
            .collect();
        assert_eq!(meters, [&usage(17_761), &usage(18_168)]);
        let metrics = events.iter().find_map(|e| match e {
            AgentEvent::TurnMetrics(metrics) => Some(metrics),
            _ => None,
        });
        let metrics = metrics.expect("turn metrics");
        assert_eq!(metrics.input_tokens, Some(20_138));
        assert_eq!(metrics.cache_read_tokens, Some(15_622));
        // Cached reads count beside the input, as Antigravity reports them.
        let hit = metrics.cache_hit_percent.unwrap();
        assert!((hit - 15_622.0 / 35_760.0 * 100.0).abs() < 0.01, "{hit}");
        assert_eq!(
            events.last(),
            Some(&AgentEvent::Done(DoneStatus::Completed))
        );
    }

    fn permission_requests(events: &[AgentEvent]) -> Vec<&PermissionRequest> {
        events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::PermissionRequest(request) => Some(request),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_command_agy_asks_about_goes_to_the_user_in_ask_and_auto() {
        for permission in [PermissionPolicy::Ask, PermissionPolicy::Auto] {
            let (mut parser, requests) = opened(&acp_request(permission));
            let events = play(&mut parser, &requests.last().unwrap()["id"], PERMISSION);
            let asked = permission_requests(&events);
            assert_eq!(asked.len(), 1, "{permission:?}");
            assert_eq!(asked[0].tool, "Bash");
            assert_eq!(asked[0].description, "rm scratch.txt");
            assert!(parser.take_replies().is_empty());
            let answer: Value =
                serde_json::from_str(&acp::permission_response(asked[0], true)).unwrap();
            assert_eq!(answer["id"], "1");
            assert_eq!(answer["result"]["outcome"]["optionId"], "allow");
            let answer: Value =
                serde_json::from_str(&acp::permission_response(asked[0], false)).unwrap();
            assert_eq!(answer["result"]["outcome"]["optionId"], "deny");
        }
    }

    #[test]
    fn full_access_answers_what_agy_asks_by_itself() {
        let (mut parser, requests) = opened(&acp_request(PermissionPolicy::AutoApprove));
        let events = play(&mut parser, &requests.last().unwrap()["id"], PERMISSION);
        assert!(permission_requests(&events).is_empty());
        let answers = sent(&mut parser);
        assert_eq!(answers[0]["id"], "1");
        assert_eq!(answers[0]["result"]["outcome"]["optionId"], "allow");
    }

    #[test]
    fn a_missing_agy_is_not_asked_its_version() {
        assert!(!speaks_acp(Path::new("/nonexistent/agy")));
    }

    /// Drives the real `agy acp` through a turn and a resumed one. Costs
    /// two small turns, so it only runs with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_acp_turn_and_resume() {
        let dir = std::env::temp_dir().join(format!("bencode-agy-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let run = |prompt: &str, resume_id: Option<String>| {
            let req = SpawnRequest {
                cwd: dir.to_string_lossy().into_owned(),
                prompt: prompt.into(),
                resume_id,
                mcp_servers: Vec::new(),
                ..acp_request(PermissionPolicy::AutoApprove)
            };
            let (handle, mut rx) = crate::harness::spawn(&req).unwrap();
            crate::harness::runtime::runtime().block_on(async move {
                let mut seen = Vec::new();
                while let Some(event) = rx.recv().await {
                    if let AgentEvent::PermissionRequest(request) = &event {
                        assert!(handle.respond_permission(request, true));
                    }
                    seen.push(event);
                }
                seen
            })
        };
        let first = run(
            "Run this shell command and nothing else: echo hi > hi.txt",
            None,
        );
        let session = first.iter().find_map(|e| match e {
            AgentEvent::SessionStarted {
                provider_session_id,
            } => Some(provider_session_id.clone()),
            _ => None,
        });
        assert!(session.is_some(), "{first:#?}");
        assert!(
            first
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolCallFinish { success: true, .. })),
            "{first:#?}"
        );
        assert_eq!(first.last(), Some(&AgentEvent::Done(DoneStatus::Completed)));
        assert_eq!(
            std::fs::read_to_string(dir.join("hi.txt")).unwrap().trim(),
            "hi"
        );

        let second = run(
            "What file did you just write? Answer with its name only.",
            session.clone(),
        );
        let text: String = second
            .iter()
            .filter_map(|e| match e {
                AgentEvent::TextDelta(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.contains("hi.txt"), "{second:#?}");
        assert!(second.contains(&AgentEvent::SessionStarted {
            provider_session_id: session.unwrap()
        }));
        assert_eq!(
            second.last(),
            Some(&AgentEvent::Done(DoneStatus::Completed))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn update(update: Value) -> String {
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": { "sessionId": "ses-1", "update": update },
        })
        .to_string()
    }

    /// The turn opened, `run_command` started in the background, and the
    /// prompt answered with `stop`.
    fn returned_with_open_tool(stop: &str) -> (AcpParser, Vec<AgentEvent>) {
        let (mut parser, requests) = opened(&acp_request(PermissionPolicy::AutoApprove));
        let prompt_id = requests.last().unwrap()["id"].clone();
        let started = parser.parse_line(&update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call-bg-1",
            "name": "run_command",
            "status": "in_progress",
            "kind": "execute",
            "title": "npm test",
        })));
        assert!(
            started
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolCallStart { id, .. } if id == "call-bg-1"))
        );
        let events = parser.parse_line(&reply(&prompt_id, json!({ "stopReason": stop })));
        (parser, events)
    }

    fn is_done(events: &[AgentEvent]) -> bool {
        events.iter().any(|e| matches!(e, AgentEvent::Done(_)))
    }

    #[test]
    fn an_open_tool_call_keeps_the_turn_until_it_finishes_without_a_prompt() {
        let (mut parser, returned) = returned_with_open_tool("end_turn");
        assert!(!is_done(&returned), "{returned:?}");
        assert!(
            sent(&mut parser).is_empty(),
            "nothing is asked of the agent"
        );
        assert_eq!(parser.quiet_limit(), Some(acp::OPEN_TOOLS_GRACE));

        let finished = parser.parse_line(&update(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call-bg-1",
            "status": "completed",
            "content": [{ "type": "content", "content": { "type": "text", "text": "ok" } }],
        })));
        assert!(finished.iter().any(
            |e| matches!(e, AgentEvent::ToolCallFinish { id, success: true, .. } if id == "call-bg-1")
        ));
        assert_eq!(
            finished.last(),
            Some(&AgentEvent::Done(DoneStatus::Completed))
        );
        assert_eq!(parser.quiet_limit(), None);
    }

    #[test]
    fn a_tool_that_never_reports_back_is_let_go_after_the_grace() {
        let (mut parser, _) = returned_with_open_tool("end_turn");
        assert_eq!(parser.on_quiet(), [AgentEvent::Done(DoneStatus::Completed)]);
        assert!(parser.on_quiet().is_empty());
        assert!(sent(&mut parser).is_empty());
    }

    #[test]
    fn a_prompt_cut_short_does_not_wait_on_its_tools() {
        let (parser, returned) = returned_with_open_tool("max_tokens");
        assert!(returned.contains(&AgentEvent::Done(DoneStatus::Completed)));
        assert_eq!(parser.quiet_limit(), None);
        let (_, cancelled) = returned_with_open_tool("cancelled");
        assert!(cancelled.contains(&AgentEvent::Done(DoneStatus::Cancelled)));
    }

    #[test]
    fn a_follow_up_while_tools_run_is_prompted_at_once() {
        let (mut parser, _) = returned_with_open_tool("end_turn");
        parser.steer("and then?", &[]);
        let asked = sent(&mut parser);
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0]["method"], "session/prompt");
        assert_eq!(asked[0]["params"]["prompt"][0]["text"], "and then?");
        assert_eq!(parser.quiet_limit(), None);
        let done = parser.parse_line(&reply(&asked[0]["id"], json!({ "stopReason": "end_turn" })));
        // `call-bg-1` is still open: the follow-up's turn waits on it too.
        assert!(!is_done(&done));
    }
}
