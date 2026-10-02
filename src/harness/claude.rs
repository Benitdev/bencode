//! Claude Code CLI over its stream-json protocol, the same wire format the
//! Claude Agent SDK (and MonoCode's `claudeProtocol.ts`) uses:
//! the prompt goes in as an NDJSON user message on stdin, permission prompts
//! arrive as `control_request`/`can_use_tool` and are answered with a
//! `control_response` on stdin.

use std::collections::HashSet;

use anyhow::Result;
use serde_json::{Value, json};

use crate::harness::events::{AgentEvent, DoneStatus, PermissionRequest};
use crate::harness::handle::HarnessProcessHandle;
use crate::harness::process::{self, LineParser, ProcessSpec, StdinMode};
use crate::harness::resolver::HarnessResolver;
use crate::harness::{EventRx, PermissionPolicy, SpawnRequest, str_field, summarize_tool_input};

const DENY_MESSAGE: &str = "User declined tool execution.";

pub fn spawn(req: &SpawnRequest) -> Result<(HarnessProcessHandle, EventRx)> {
    let program = HarnessResolver::resolve_claude().unwrap_or_else(|| "claude".into());
    let spec = ProcessSpec {
        program,
        args: build_args(req),
        cwd: req.cwd.clone(),
        stdin: StdinMode::Protocol {
            initial: format!("{}\n", user_message(&req.prompt)),
        },
        permission_responder: Some(permission_response),
    };
    process::spawn(spec, ClaudeParser::default())
}

fn build_args(req: &SpawnRequest) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
    ]
    .map(String::from)
    .to_vec();

    match req.permission {
        PermissionPolicy::AutoApprove => args.push("--dangerously-skip-permissions".into()),
        PermissionPolicy::Ask => args.extend(["--permission-prompt-tool".into(), "stdio".into()]),
        PermissionPolicy::ReadOnly => args.extend(["--permission-mode".into(), "plan".into()]),
    }
    if let Some(model) = &req.model {
        args.extend(["--model".into(), model.clone()]);
    }
    if let Some(resume) = &req.resume_id {
        args.extend(["--resume".into(), resume.clone()]);
    }
    args
}

fn user_message(prompt: &str) -> String {
    json!({
        "type": "user",
        "session_id": "",
        "parent_tool_use_id": null,
        "message": { "role": "user", "content": [{ "type": "text", "text": prompt }] },
    })
    .to_string()
}

fn permission_response(request: &PermissionRequest, allow: bool) -> String {
    let decision = if allow {
        json!({ "behavior": "allow", "updatedInput": request.input })
    } else {
        json!({ "behavior": "deny", "message": DENY_MESSAGE })
    };
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": request.request_id, "response": decision },
    })
    .to_string()
}

#[derive(Default)]
pub struct ClaudeParser {
    session_announced: bool,
    /// Message ids whose text already arrived via `stream_event` deltas, so the
    /// trailing `assistant` snapshot must not repeat it.
    streamed_messages: HashSet<String>,
    current_stream_message: Option<String>,
    started_tools: HashSet<String>,
}

impl LineParser for ClaudeParser {
    fn parse_line(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(rec) = serde_json::from_str::<Value>(line.trim()) else {
            if !line.trim().is_empty() {
                log::debug!("claude: non-JSON stdout line: {line}");
            }
            return Vec::new();
        };
        // Subagent traffic belongs to the parent's Task tool call, not the transcript.
        if rec.get("parent_tool_use_id").is_some_and(|v| !v.is_null()) {
            return Vec::new();
        }

        let mut events = Vec::new();
        self.announce_session(&rec, &mut events);
        match str_field(&rec, "type") {
            Some("stream_event") => self.on_stream_event(&rec, &mut events),
            Some("assistant") => self.on_assistant(&rec, &mut events),
            Some("user") => on_user(&rec, &mut events),
            Some("control_request") => on_control_request(&rec, &mut events),
            Some("result") => on_result(&rec, &mut events),
            _ => {}
        }
        events
    }
}

impl ClaudeParser {
    fn announce_session(&mut self, rec: &Value, events: &mut Vec<AgentEvent>) {
        if self.session_announced || str_field(rec, "type") == Some("stream_event") {
            return;
        }
        if let Some(id) = str_field(rec, "session_id").filter(|id| !id.is_empty()) {
            self.session_announced = true;
            events.push(AgentEvent::SessionStarted {
                provider_session_id: id.to_string(),
            });
        }
    }

    fn on_stream_event(&mut self, rec: &Value, events: &mut Vec<AgentEvent>) {
        let Some(event) = rec.get("event") else {
            return;
        };
        match str_field(event, "type") {
            Some("message_start") => {
                self.current_stream_message = event
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .map(String::from);
            }
            Some("content_block_delta") => {
                let Some(delta) = event.get("delta") else {
                    return;
                };
                match str_field(delta, "type") {
                    Some("text_delta") => {
                        if let Some(text) = str_field(delta, "text").filter(|t| !t.is_empty()) {
                            if let Some(id) = &self.current_stream_message {
                                self.streamed_messages.insert(id.clone());
                            }
                            events.push(AgentEvent::TextDelta(text.to_string()));
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = str_field(delta, "thinking").filter(|t| !t.is_empty()) {
                            events.push(AgentEvent::ThinkingDelta(text.to_string()));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn on_assistant(&mut self, rec: &Value, events: &mut Vec<AgentEvent>) {
        let Some(message) = rec.get("message") else {
            return;
        };
        let already_streamed =
            str_field(message, "id").is_some_and(|id| self.streamed_messages.contains(id));
        let Some(content) = message.get("content").and_then(Value::as_array) else {
            return;
        };

        for block in content {
            match str_field(block, "type") {
                Some("text") if !already_streamed => {
                    if let Some(text) = str_field(block, "text").filter(|t| !t.is_empty()) {
                        events.push(AgentEvent::TextDelta(text.to_string()));
                    }
                }
                Some("tool_use") => {
                    let Some(id) = str_field(block, "id") else {
                        continue;
                    };
                    if !self.started_tools.insert(id.to_string()) {
                        continue;
                    }
                    events.push(AgentEvent::ToolCallStart {
                        id: id.to_string(),
                        name: str_field(block, "name").unwrap_or("tool").to_string(),
                        input: block.get("input").cloned().unwrap_or(Value::Null),
                    });
                }
                _ => {}
            }
        }
    }
}

fn on_user(rec: &Value, events: &mut Vec<AgentEvent>) {
    let Some(content) = rec.pointer("/message/content").and_then(Value::as_array) else {
        return;
    };
    for block in content
        .iter()
        .filter(|b| str_field(b, "type") == Some("tool_result"))
    {
        let Some(id) = str_field(block, "tool_use_id") else {
            continue;
        };
        events.push(AgentEvent::ToolCallFinish {
            id: id.to_string(),
            output: tool_result_text(block.get("content")),
            success: !block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
}

/// `tool_result.content` is either a string or a list of content blocks.
fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| str_field(item, "text"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn on_control_request(rec: &Value, events: &mut Vec<AgentEvent>) {
    let Some(request) = rec.get("request") else {
        return;
    };
    if str_field(request, "subtype") != Some("can_use_tool") {
        return;
    }
    let Some(request_id) = str_field(rec, "request_id") else {
        return;
    };
    let tool = str_field(request, "tool_name")
        .unwrap_or("tool")
        .to_string();
    let input = request.get("input").cloned().unwrap_or_else(|| json!({}));
    events.push(AgentEvent::PermissionRequest(PermissionRequest {
        request_id: request_id.to_string(),
        description: summarize_tool_input(&tool, &input),
        tool,
        input,
    }));
}

fn on_result(rec: &Value, events: &mut Vec<AgentEvent>) {
    if let Some(usage) = rec.get("usage") {
        let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
        let input_tokens = field("input_tokens")
            + field("cache_read_input_tokens")
            + field("cache_creation_input_tokens");
        let output_tokens = field("output_tokens");
        events.push(AgentEvent::Usage {
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
        });
    }

    let is_error = rec
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if is_error {
        let message = str_field(rec, "result")
            .or_else(|| str_field(rec, "subtype"))
            .unwrap_or("Claude reported an error");
        events.push(AgentEvent::Error(message.to_string()));
        events.push(AgentEvent::Done(DoneStatus::Failed));
    } else {
        events.push(AgentEvent::Done(DoneStatus::Completed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(lines: &[&str]) -> Vec<AgentEvent> {
        let mut parser = ClaudeParser::default();
        lines
            .iter()
            .flat_map(|line| parser.parse_line(line))
            .collect()
    }

    fn request(policy: PermissionPolicy) -> SpawnRequest {
        SpawnRequest {
            harness: crate::harness::HarnessKind::Claude,
            cwd: "/tmp".into(),
            prompt: "hi".into(),
            model: Some("opus".into()),
            permission: policy,
            resume_id: Some("sess-1".into()),
        }
    }

    #[test]
    fn args_use_stream_json_protocol_and_map_policy() {
        let ask = build_args(&request(PermissionPolicy::Ask));
        assert!(
            ask.windows(2)
                .any(|w| w == ["--input-format", "stream-json"])
        );
        assert!(
            ask.windows(2)
                .any(|w| w == ["--permission-prompt-tool", "stdio"])
        );
        assert!(ask.windows(2).any(|w| w == ["--model", "opus"]));
        assert!(ask.windows(2).any(|w| w == ["--resume", "sess-1"]));
        assert!(
            !ask.contains(&"hi".to_string()),
            "prompt must go over stdin, not argv"
        );

        let auto = build_args(&request(PermissionPolicy::AutoApprove));
        assert!(auto.contains(&"--dangerously-skip-permissions".to_string()));

        let read_only = build_args(&request(PermissionPolicy::ReadOnly));
        assert!(
            read_only
                .windows(2)
                .any(|w| w == ["--permission-mode", "plan"])
        );
    }

    #[test]
    fn streamed_text_is_not_duplicated_by_assistant_snapshot() {
        let events = parse_all(&[
            r#"{"type":"system","subtype":"init","session_id":"s1"}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}},"parent_tool_use_id":null}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}},"parent_tool_use_id":null}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}},"parent_tool_use_id":null}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"Hello"}]},"parent_tool_use_id":null,"session_id":"s1"}"#,
        ]);
        assert_eq!(
            events,
            vec![
                AgentEvent::SessionStarted {
                    provider_session_id: "s1".into()
                },
                AgentEvent::TextDelta("Hel".into()),
                AgentEvent::TextDelta("lo".into()),
            ]
        );
    }

    #[test]
    fn tool_use_and_result_round_trip() {
        let events = parse_all(&[
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"echo hi"}}]},"parent_tool_use_id":null}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"echo hi"}}]},"parent_tool_use_id":null}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"hi"}],"is_error":false}]},"parent_tool_use_id":null}"#,
        ]);
        assert_eq!(
            events,
            vec![
                AgentEvent::ToolCallStart {
                    id: "t1".into(),
                    name: "Bash".into(),
                    input: json!({"command": "echo hi"})
                },
                AgentEvent::ToolCallFinish {
                    id: "t1".into(),
                    output: "hi".into(),
                    success: true
                },
            ]
        );
    }

    #[test]
    fn subagent_messages_are_ignored() {
        let events = parse_all(&[
            r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"inner"}]},"parent_tool_use_id":"task-1"}"#,
        ]);
        assert!(events.is_empty());
    }

    #[test]
    fn can_use_tool_becomes_permission_request_and_reply_matches_protocol() {
        let events = parse_all(&[
            r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm -rf build"}}}"#,
        ]);
        let [AgentEvent::PermissionRequest(req)] = events.as_slice() else {
            panic!("unexpected {events:?}");
        };
        assert_eq!(req.description, "rm -rf build");

        let allow: Value = serde_json::from_str(&permission_response(req, true)).unwrap();
        assert_eq!(allow["type"], "control_response");
        assert_eq!(allow["response"]["request_id"], "r1");
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(
            allow["response"]["response"]["updatedInput"]["command"],
            "rm -rf build"
        );

        let deny: Value = serde_json::from_str(&permission_response(req, false)).unwrap();
        assert_eq!(deny["response"]["response"]["behavior"], "deny");
    }

    #[test]
    fn result_reports_usage_and_done() {
        let ok = parse_all(&[
            r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":10,"cache_read_input_tokens":90,"output_tokens":5}}"#,
        ]);
        assert_eq!(
            ok,
            vec![
                AgentEvent::Usage {
                    input_tokens: 100,
                    output_tokens: 5,
                    total_tokens: 105
                },
                AgentEvent::Done(DoneStatus::Completed),
            ]
        );

        let failed =
            parse_all(&[r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#]);
        assert_eq!(
            failed,
            vec![
                AgentEvent::Error("error_max_turns".into()),
                AgentEvent::Done(DoneStatus::Failed)
            ]
        );
    }

    /// Drives the real `claude` CLI through a permission round-trip.
    /// Costs a few tokens, so it only runs with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_permission_round_trip() {
        let dir = std::env::temp_dir().join(format!("bencode-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("hello.txt");
        let req = SpawnRequest {
            harness: crate::harness::HarnessKind::Claude,
            cwd: dir.to_string_lossy().into_owned(),
            prompt: format!(
                "Use the Write tool to create {} containing exactly: hi",
                target.display()
            ),
            model: Some("haiku".into()),
            permission: PermissionPolicy::Ask,
            resume_id: None,
        };
        let (handle, mut rx) = crate::harness::spawn(&req).unwrap();
        let events = crate::harness::runtime::runtime().block_on(async move {
            let mut seen = Vec::new();
            while let Some(event) = rx.recv().await {
                if let AgentEvent::PermissionRequest(request) = &event {
                    assert!(handle.respond_permission(request, true));
                }
                seen.push(event);
            }
            seen
        });

        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::SessionStarted { .. })),
            "{events:#?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::PermissionRequest(_))),
            "{events:#?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolCallFinish { success: true, .. })),
            "{events:#?}"
        );
        assert_eq!(
            events.last(),
            Some(&AgentEvent::Done(DoneStatus::Completed))
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap().trim(), "hi");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
