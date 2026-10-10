//! Antigravity (`agy`) print mode with `--output-format stream-json`.
//! Events are `{"event": "init" | "step_update" | "result" | "error", …}`.

use anyhow::Result;
use serde_json::Value;

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
        env: Vec::new(),
    };
    process::spawn(spec, AntigravityParser)
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

pub struct AntigravityParser;

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
                    on_step(step, &mut events);
                }
            }
            Some("result") => {
                let result = rec.get("result").unwrap_or(&Value::Null);
                if let Some(usage) = result.get("usage") {
                    events.push(usage_event(usage));
                    events.extend(metrics_event(usage));
                }
                match str_field(result, "status") {
                    None | Some(SUCCESS_STATUS) => {
                        events.push(AgentEvent::Done(DoneStatus::Completed))
                    }
                    Some(status) => {
                        events.push(AgentEvent::Error(format!(
                            "Antigravity finished with status {status}"
                        )));
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

fn on_step(step: &Value, events: &mut Vec<AgentEvent>) {
    match str_field(step, "step_type") {
        Some("agent_response") => {
            if let Some(thinking) = str_field(step, "thinking_delta").filter(|t| !t.is_empty()) {
                events.push(AgentEvent::ThinkingDelta(thinking.to_string()));
            }
            if let Some(text) = str_field(step, "text_delta").filter(|t| !t.is_empty()) {
                events.push(AgentEvent::TextDelta(text.to_string()));
            }
        }
        Some("tool_call") => {
            let Some(call) = step.get("tool_call") else {
                return;
            };
            let Some(id) = str_field(call, "id") else {
                return;
            };
            events.push(AgentEvent::ToolCallStart {
                id: id.to_string(),
                name: str_field(call, "name").unwrap_or("tool").to_string(),
                input: call.get("input").cloned().unwrap_or(Value::Null),
            });
            if str_field(step, "state") == Some("DONE") {
                events.push(AgentEvent::ToolCallFinish {
                    id: id.to_string(),
                    output: str_field(call, "output").unwrap_or_default().to_string(),
                    success: call.get("error").is_none_or(Value::is_null),
                });
            }
        }
        _ => {}
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
        let mut parser = AntigravityParser;
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
        let mut parser = AntigravityParser;
        let events = parser.parse_line(r#"{"event":"result","result":{"status":"CANCELLED"}}"#);
        assert_eq!(events.last(), Some(&AgentEvent::Done(DoneStatus::Failed)));
    }
}
