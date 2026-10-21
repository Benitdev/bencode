//! Antigravity (`agy`) print mode with `--output-format stream-json`.
//! Events are `{"event": "init" | "step_update" | "result" | "error", …}`.
//!
//! A tool runs as a `"tool"` step (`tool_info`: name, parameters, output),
//! reported `ACTIVE` then `DONE`; older builds sent `"tool_call"` steps.
//! The result's `usage` adds up every model call of the turn, so the
//! context meter reads the last call's instead; a result that is not
//! `SUCCESS` says why in its `error`.

use anyhow::Result;
use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::harness::events::{AgentEvent, DoneStatus, TurnMetrics};
use crate::harness::handle::HarnessProcessHandle;
use crate::harness::process::{self, LineParser, ProcessSpec, StdinMode};
use crate::harness::resolver::HarnessResolver;
use crate::harness::{EventRx, PermissionPolicy, SpawnRequest, str_field};

const SUCCESS_STATUS: &str = "SUCCESS";

pub fn spawn(req: &SpawnRequest) -> Result<(HarnessProcessHandle, EventRx)> {
    let program = HarnessResolver::resolve_antigravity_cli().unwrap_or_else(|| "agy".into());
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
        out.insert("description".into(), Value::String(tool_label(name, params)));
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
        assert_eq!(crate::harness::summarize_tool_input("manage_task", &input), "Manage task: kill");
        let summarized = serde_json::json!({ "Action": "status", "toolSummary": "Check test task status" });
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
}
