//! Codex CLI via `codex exec --json` (one non-interactive turn per process).
//! `exec` cannot prompt for approvals, so the permission policy maps onto
//! Codex's sandbox levels instead.

use anyhow::Result;
use serde_json::{Value, json};

use crate::harness::events::{AgentEvent, DoneStatus};
use crate::harness::handle::HarnessProcessHandle;
use crate::harness::process::{self, LineParser, ProcessSpec, StdinMode};
use crate::harness::resolver::HarnessResolver;
use crate::harness::{EventRx, PermissionPolicy, SpawnRequest, str_field};

pub fn spawn(req: &SpawnRequest) -> Result<(HarnessProcessHandle, EventRx)> {
    let program = HarnessResolver::resolve_codex().unwrap_or_else(|| "codex".into());
    let spec = ProcessSpec {
        program,
        args: build_args(req),
        cwd: req.cwd.clone(),
        stdin: StdinMode::Null,
        permission_responder: None,
    };
    process::spawn(spec, CodexParser::default())
}

fn build_args(req: &SpawnRequest) -> Vec<String> {
    let mut args: Vec<String> = vec!["exec".into(), "--json".into(), "--skip-git-repo-check".into()];
    match req.permission {
        PermissionPolicy::AutoApprove => args.push("--dangerously-bypass-approvals-and-sandbox".into()),
        PermissionPolicy::Ask => args.push("--full-auto".into()),
        PermissionPolicy::ReadOnly => args.extend(["--sandbox".into(), "read-only".into()]),
    }
    if let Some(model) = &req.model {
        args.extend(["--model".into(), model.clone()]);
    }
    if let Some(thread) = &req.resume_id {
        args.extend(["resume".into(), thread.clone()]);
    }
    // `--` keeps a prompt that starts with '-' from being parsed as a flag.
    args.extend(["--".into(), req.prompt.clone()]);
    args
}

#[derive(Default)]
pub struct CodexParser {
    wrote_text: bool,
}

impl LineParser for CodexParser {
    fn parse_line(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(rec) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        match str_field(&rec, "type") {
            Some("thread.started") => {
                if let Some(id) = str_field(&rec, "thread_id") {
                    events.push(AgentEvent::SessionStarted { provider_session_id: id.to_string() });
                }
            }
            Some("item.started") => {
                if let Some(start) = rec.get("item").and_then(tool_start) {
                    events.push(start);
                }
            }
            Some("item.completed") => {
                if let Some(item) = rec.get("item") {
                    self.on_item_completed(item, &mut events);
                }
            }
            Some("turn.completed") => {
                if let Some(usage) = rec.get("usage") {
                    let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
                    let input_tokens = field("input_tokens");
                    let output_tokens = field("output_tokens");
                    events.push(AgentEvent::Usage {
                        input_tokens,
                        output_tokens,
                        total_tokens: input_tokens + output_tokens,
                    });
                }
                events.push(AgentEvent::Done(DoneStatus::Completed));
            }
            Some("turn.failed") => {
                let message = rec
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex turn failed");
                events.push(AgentEvent::Error(message.to_string()));
                events.push(AgentEvent::Done(DoneStatus::Failed));
            }
            Some("error") => {
                let message = str_field(&rec, "message").unwrap_or("Codex error");
                events.push(AgentEvent::Error(message.to_string()));
            }
            _ => {}
        }
        events
    }
}

impl CodexParser {
    fn on_item_completed(&mut self, item: &Value, events: &mut Vec<AgentEvent>) {
        match str_field(item, "type") {
            Some("agent_message") => {
                if let Some(text) = str_field(item, "text").filter(|t| !t.is_empty()) {
                    // Each agent_message is a whole paragraph, not a delta.
                    let separator = if self.wrote_text { "\n\n" } else { "" };
                    self.wrote_text = true;
                    events.push(AgentEvent::TextDelta(format!("{separator}{text}")));
                }
            }
            Some("reasoning") => {
                if let Some(text) = str_field(item, "text").filter(|t| !t.is_empty()) {
                    events.push(AgentEvent::ThinkingDelta(format!("{text}\n")));
                }
            }
            _ => {
                let Some(id) = str_field(item, "id") else { return };
                let Some(start) = tool_start(item) else { return };
                // Re-announcing is harmless (the app de-duplicates by call id)
                // and covers items that never had an `item.started`.
                events.push(start);
                let failed = str_field(item, "status") == Some("failed")
                    || item.get("exit_code").and_then(Value::as_i64).is_some_and(|code| code != 0);
                events.push(AgentEvent::ToolCallFinish {
                    id: id.to_string(),
                    output: tool_output(item),
                    success: !failed,
                });
            }
        }
    }
}

fn tool_start(item: &Value) -> Option<AgentEvent> {
    let id = str_field(item, "id")?.to_string();
    let (name, input) = match str_field(item, "type")? {
        "command_execution" => ("Bash", json!({ "command": str_field(item, "command").unwrap_or("") })),
        "file_change" => ("Edit", json!({ "changes": item.get("changes").cloned().unwrap_or(Value::Null) })),
        "mcp_tool_call" => (
            str_field(item, "tool").unwrap_or("mcp"),
            item.get("arguments").cloned().unwrap_or(Value::Null),
        ),
        "web_search" => ("WebSearch", json!({ "query": str_field(item, "query").unwrap_or("") })),
        _ => return None,
    };
    Some(AgentEvent::ToolCallStart { id, name: name.to_string(), input })
}

fn tool_output(item: &Value) -> String {
    str_field(item, "aggregated_output")
        .map(String::from)
        .or_else(|| item.get("result").map(Value::to_string))
        .or_else(|| item.get("changes").map(Value::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::HarnessKind;

    fn parse_all(lines: &[&str]) -> Vec<AgentEvent> {
        let mut parser = CodexParser::default();
        lines.iter().flat_map(|line| parser.parse_line(line)).collect()
    }

    #[test]
    fn args_put_prompt_last_after_separator() {
        let req = SpawnRequest {
            harness: HarnessKind::Codex,
            cwd: "/tmp".into(),
            prompt: "-fix it".into(),
            model: Some("gpt-5-codex".into()),
            permission: PermissionPolicy::ReadOnly,
            resume_id: Some("th_1".into()),
        };
        let args = build_args(&req);
        assert_eq!(&args[..3], ["exec", "--json", "--skip-git-repo-check"]);
        assert!(args.windows(2).any(|w| w == ["--sandbox", "read-only"]));
        assert!(args.windows(2).any(|w| w == ["resume", "th_1"]));
        assert_eq!(&args[args.len() - 2..], ["--", "-fix it"]);
        assert!(!args.contains(&"-p".to_string()), "-p is --profile in codex");
    }

    #[test]
    fn parses_exec_json_stream() {
        let events = parse_all(&[
            r#"{"type":"thread.started","thread_id":"th_1"}"#,
            r#"{"type":"item.started","item":{"id":"i1","type":"command_execution","command":"ls","status":"in_progress"}}"#,
            r#"{"type":"item.completed","item":{"id":"i1","type":"command_execution","command":"ls","aggregated_output":"a\nb","exit_code":0,"status":"completed"}}"#,
            r#"{"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"First."}}"#,
            r#"{"type":"item.completed","item":{"id":"i3","type":"agent_message","text":"Second."}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":7,"output_tokens":3}}"#,
        ]);
        let bash = || AgentEvent::ToolCallStart { id: "i1".into(), name: "Bash".into(), input: json!({"command": "ls"}) };
        assert_eq!(
            events,
            vec![
                AgentEvent::SessionStarted { provider_session_id: "th_1".into() },
                bash(),
                bash(),
                AgentEvent::ToolCallFinish { id: "i1".into(), output: "a\nb".into(), success: true },
                AgentEvent::TextDelta("First.".into()),
                AgentEvent::TextDelta("\n\nSecond.".into()),
                AgentEvent::Usage { input_tokens: 7, output_tokens: 3, total_tokens: 10 },
                AgentEvent::Done(DoneStatus::Completed),
            ]
        );
    }

    #[test]
    fn failed_turn_reports_error() {
        let events = parse_all(&[r#"{"type":"turn.failed","error":{"message":"quota"}}"#]);
        assert_eq!(events, vec![AgentEvent::Error("quota".into()), AgentEvent::Done(DoneStatus::Failed)]);
    }
}
