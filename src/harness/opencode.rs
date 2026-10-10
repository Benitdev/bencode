//! OpenCode via `opencode run --format json`, which prints one JSON event per
//! line (`text`, `reasoning`, `tool_use`, `step_finish`, `error`). MonoCode
//! drives `opencode serve` over HTTP instead; the one-shot CLI keeps this
//! port dependency-free at the cost of interactive approvals.

use anyhow::Result;
use serde_json::Value;

use crate::harness::events::{AgentEvent, TurnMetrics};
use crate::harness::handle::HarnessProcessHandle;
use crate::harness::process::{self, LineParser, ProcessSpec, StdinMode};
use crate::harness::resolver::HarnessResolver;
use crate::harness::{EventRx, SpawnRequest, str_field};

pub fn spawn(req: &SpawnRequest) -> Result<(HarnessProcessHandle, EventRx)> {
    let program = HarnessResolver::resolve_opencode().unwrap_or_else(|| "opencode".into());
    let spec = ProcessSpec {
        program,
        args: build_args(req),
        cwd: req.cwd.clone(),
        stdin: StdinMode::Null,
        permission_responder: None,
        can_steer: false,
        account: None,
        env: inline_config(req).into_iter().collect(),
    };
    process::spawn(spec, OpenCodeParser::default())
}

/// The in-app browser's MCP server as OpenCode's inline config, which it
/// merges over the user's own.
fn inline_config(req: &SpawnRequest) -> Option<(String, String)> {
    let launch = req.browser_mcp.as_ref()?;
    let command: Vec<&str> = std::iter::once(launch.command.as_str())
        .chain(launch.args.iter().map(String::as_str))
        .collect();
    let config = serde_json::json!({
        "mcp": {
            crate::browser::MCP_SERVER_NAME: { "type": "local", "command": command, "enabled": true },
        },
    });
    Some(("OPENCODE_CONFIG_CONTENT".into(), config.to_string()))
}

fn build_args(req: &SpawnRequest) -> Vec<String> {
    let mut args: Vec<String> = vec!["run".into(), "--format".into(), "json".into()];
    if let Some(model) = &req.model {
        args.extend(["--model".into(), model.clone()]);
    }
    // MonoCode sends these with each prompt; `opencode run` takes them as flags.
    for (id, flag) in [("variant", "--variant"), ("agent", "--agent")] {
        if let Some(value) = req.settings.get(id) {
            args.extend([flag.into(), value.clone()]);
        }
    }
    if let Some(session) = &req.resume_id {
        args.extend(["--session".into(), session.clone()]);
    }
    let prompt = crate::harness::attachments::plain_prompt(&req.prompt, &req.attachments);
    args.extend(["--".into(), prompt]);
    args
}

#[derive(Default)]
pub struct OpenCodeParser {
    session_announced: bool,
}

impl LineParser for OpenCodeParser {
    fn parse_line(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(rec) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        if !self.session_announced
            && let Some(id) = str_field(&rec, "sessionID")
        {
            self.session_announced = true;
            events.push(AgentEvent::SessionStarted {
                provider_session_id: id.to_string(),
            });
        }

        let part = rec.get("part").unwrap_or(&Value::Null);
        match str_field(&rec, "type") {
            Some("text") => {
                if let Some(text) = str_field(part, "text").filter(|t| !t.is_empty()) {
                    events.push(AgentEvent::TextDelta(text.to_string()));
                }
            }
            Some("reasoning") => {
                if let Some(text) = str_field(part, "text").filter(|t| !t.is_empty()) {
                    events.push(AgentEvent::ThinkingDelta(text.to_string()));
                }
            }
            Some("tool_use") => on_tool_use(part, &mut events),
            Some("step_finish") => {
                if let Some(tokens) = part.get("tokens") {
                    let field = |name: &str| tokens.get(name).and_then(Value::as_u64).unwrap_or(0);
                    let input_tokens = field("input");
                    let output_tokens = field("output") + field("reasoning");
                    events.push(AgentEvent::Usage {
                        input_tokens,
                        output_tokens,
                        total_tokens: input_tokens + output_tokens,
                        context_window: None,
                    });
                    // MonoCode `turnMetricsFromMessageInfo`.
                    let cache = tokens.get("cache");
                    let cached = |name: &str| {
                        cache
                            .and_then(|c| c.get(name))
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                    };
                    let (read, write) = (cached("read"), cached("write"));
                    if let Some(metrics) = TurnMetrics::from_counts(
                        input_tokens,
                        output_tokens,
                        read,
                        write,
                        cache.is_some().then_some(input_tokens + read + write),
                    ) {
                        events.push(AgentEvent::TurnMetrics(metrics));
                    }
                }
            }
            Some("error") => {
                let error = rec.get("error").unwrap_or(&Value::Null);
                let message = error
                    .pointer("/data/message")
                    .and_then(Value::as_str)
                    .or_else(|| str_field(error, "message"))
                    .or_else(|| str_field(error, "name"))
                    .unwrap_or("OpenCode error");
                events.push(AgentEvent::Error(message.to_string()));
            }
            _ => {}
        }
        events
    }
}

fn on_tool_use(part: &Value, events: &mut Vec<AgentEvent>) {
    let Some(id) = str_field(part, "callID").or_else(|| str_field(part, "id")) else {
        return;
    };
    let state = part.get("state").unwrap_or(&Value::Null);
    events.push(AgentEvent::ToolCallStart {
        id: id.to_string(),
        name: str_field(part, "tool").unwrap_or("tool").to_string(),
        input: state.get("input").cloned().unwrap_or(Value::Null),
    });
    let status = str_field(state, "status");
    if matches!(status, Some("completed" | "error")) {
        let output = str_field(state, "output")
            .or_else(|| str_field(state, "error"))
            .unwrap_or_default();
        events.push(AgentEvent::ToolCallFinish {
            id: id.to_string(),
            output: output.to_string(),
            success: status == Some("completed"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_open_browser_tab_goes_in_the_inline_config() {
        let mut req = SpawnRequest {
            harness: crate::harness::HarnessKind::OpenCode,
            cwd: "/tmp".into(),
            prompt: "go".into(),
            model: None,
            permission: crate::harness::PermissionPolicy::Ask,
            resume_id: None,
            disable_hooks: false,
            attachments: Vec::new(),
            plan: false,
            compact: false,
            settings: Default::default(),
            account: None,
            browser_mcp: None,
        };
        assert!(inline_config(&req).is_none());
        req.browser_mcp = Some(crate::browser::McpLaunch {
            command: "/bin/bencode".into(),
            args: vec!["--browser-mcp".into(), "/tmp/b.sock".into()],
        });
        let (key, value) = inline_config(&req).unwrap();
        assert_eq!(key, "OPENCODE_CONFIG_CONTENT");
        let config: serde_json::Value = serde_json::from_str(&value).unwrap();
        assert_eq!(
            config["mcp"]["bencode-browser"],
            json!({ "type": "local", "command": ["/bin/bencode", "--browser-mcp", "/tmp/b.sock"], "enabled": true })
        );
    }

    #[test]
    fn parses_run_json_events() {
        let mut parser = OpenCodeParser::default();
        let events: Vec<_> = [
            r#"{"type":"step_start","sessionID":"ses_1","part":{}}"#,
            r#"{"type":"text","sessionID":"ses_1","part":{"type":"text","text":"Hi"}}"#,
            r#"{"type":"tool_use","sessionID":"ses_1","part":{"callID":"c1","tool":"bash","state":{"status":"completed","input":{"command":"ls"},"output":"x"}}}"#,
            r#"{"type":"step_finish","sessionID":"ses_1","part":{"tokens":{"input":4,"output":2,"reasoning":1}}}"#,
        ]
        .iter()
        .flat_map(|line| parser.parse_line(line))
        .collect();

        assert_eq!(
            events,
            vec![
                AgentEvent::SessionStarted {
                    provider_session_id: "ses_1".into()
                },
                AgentEvent::TextDelta("Hi".into()),
                AgentEvent::ToolCallStart {
                    id: "c1".into(),
                    name: "bash".into(),
                    input: json!({"command": "ls"})
                },
                AgentEvent::ToolCallFinish {
                    id: "c1".into(),
                    output: "x".into(),
                    success: true
                },
                AgentEvent::Usage {
                    input_tokens: 4,
                    output_tokens: 3,
                    total_tokens: 7,
                    context_window: None,
                },
                AgentEvent::TurnMetrics(TurnMetrics {
                    input_tokens: Some(4),
                    output_tokens: Some(3),
                    ..Default::default()
                }),
            ]
        );
    }
}
