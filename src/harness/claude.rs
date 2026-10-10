//! Claude Code CLI over its stream-json protocol, the same wire format the
//! Claude Agent SDK (and MonoCode's `claudeProtocol.ts`) uses:
//! the prompt goes in as an NDJSON user message on stdin, permission prompts
//! arrive as `control_request`/`can_use_tool` and are answered with a
//! `control_response` on stdin.

use std::collections::HashSet;

use anyhow::Result;
use serde_json::{Value, json};

use crate::harness::attachments::{self, Attachment};
use crate::harness::events::{AgentEvent, DoneStatus, PermissionRequest, TurnMetrics};
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
            initial: format!(
                "{}\n",
                user_message(&prompt_with_effort(req), &req.attachments)
            ),
        },
        permission_responder: Some(permission_response),
        can_steer: true,
        account: req.account.clone(),
        env: Vec::new(),
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
    // Added to the user's own servers. The flag takes several values, so
    // it goes before the others.
    if let Some(launch) = &req.browser_mcp {
        let config = serde_json::json!({
            "mcpServers": {
                crate::browser::MCP_SERVER_NAME: {
                    "type": "stdio",
                    "command": launch.command,
                    "args": launch.args,
                },
            },
        });
        args.extend(["--mcp-config".into(), config.to_string()]);
    }

    // MonoCode `runtimeModeToPermission`: always pass a mode, so the user's
    // `permissions.defaultMode` cannot silently change what the picker says.
    let asked_mode = if req.plan {
        Some("plan")
    } else {
        match req.permission {
            // MonoCode `full-access`: bypass, but keep the prompt channel so
            // AskUserQuestion still reaches the user (the app allows the rest).
            PermissionPolicy::AutoApprove => Some("bypassPermissions"),
            PermissionPolicy::Ask => Some("default"),
            PermissionPolicy::AcceptEdits => Some("acceptEdits"),
            PermissionPolicy::Auto => Some("auto"),
        }
    };
    if asked_mode == Some("bypassPermissions") {
        args.push("--allow-dangerously-skip-permissions".into());
    }
    match asked_mode {
        None => args.push("--dangerously-skip-permissions".into()),
        Some(mode) => args.extend([
            "--permission-mode".into(),
            mode.into(),
            "--permission-prompt-tool".into(),
            "stdio".into(),
        ]),
    }
    let setting = |id: &str| req.settings.get(id).map(String::as_str);
    if let Some(settings) = cli_settings(req) {
        args.extend(["--settings".into(), settings]);
    }
    if let Some(model) = &req.model {
        // MonoCode `resolveClaudeApiModelId`: the 1M window is a model suffix.
        let model = match setting("context") {
            Some("1m") => format!("{model}[1m]"),
            _ => model.clone(),
        };
        args.extend(["--model".into(), model]);
    }
    if let Some(effort) = cli_effort(setting("effort")) {
        args.extend(["--effort".into(), effort.into()]);
    }
    if let Some(resume) = &req.resume_id {
        args.extend(["--resume".into(), resume.clone()]);
    }
    args
}

/// MonoCode `normalizeClaudeCliEffort`: Ultracode runs at `xhigh`;
/// Ultrathink is a prompt prefix, not a CLI effort.
fn cli_effort(effort: Option<&str>) -> Option<&str> {
    match effort? {
        "ultrathink" | "" => None,
        "ultracode" => Some("xhigh"),
        other => Some(other),
    }
}

/// MonoCode `launchOptions().settings`, as the `--settings` JSON.
fn cli_settings(req: &SpawnRequest) -> Option<String> {
    let on = |id: &str| req.settings.get(id).is_some_and(|v| v == "true");
    let mut settings = serde_json::Map::new();
    if on("thinking") {
        settings.insert("alwaysThinkingEnabled".into(), true.into());
    }
    if on("fast") {
        settings.insert("fastMode".into(), true.into());
    }
    if req.settings.get("effort").is_some_and(|e| e == "ultracode") {
        settings.insert("ultracode".into(), true.into());
    }
    if req.disable_hooks {
        settings.insert("disableAllHooks".into(), true.into());
    }
    (!settings.is_empty()).then(|| Value::Object(settings).to_string())
}

/// MonoCode `applyClaudePromptEffortPrefix`.
fn prompt_with_effort(req: &SpawnRequest) -> String {
    if req.settings.get("effort").is_none_or(|e| e != "ultrathink") {
        return req.prompt.clone();
    }
    if req.prompt.is_empty() {
        "Ultrathink:".into()
    } else {
        format!("Ultrathink:\n{}", req.prompt)
    }
}

/// A follow-up written into a running turn (MonoCode `steerClaudeTurn`):
/// Claude folds it into the turn it is working on.
fn steer_message(prompt: &str, files: &[Attachment]) -> String {
    user_message(prompt, files)
}

fn user_message(prompt: &str, files: &[Attachment]) -> String {
    json!({
        "type": "user",
        "session_id": "",
        "parent_tool_use_id": null,
        "message": { "role": "user", "content": attachments::claude_content(prompt, files) },
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
    /// A `rate_limit_event` refused the turn; its reset time, when given.
    rate_limited: Option<Option<i64>>,
    /// Follow-ups waiting to be written into the running turn.
    replies: Vec<String>,
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
            Some("result") => on_result(&rec, self.rate_limited.take(), &mut events),
            Some("rate_limit_event") => {
                if let Some(limit) = usage_limit_from_rate_limit_event(&rec) {
                    self.rate_limited = Some(limit);
                }
            }
            // MonoCode `compactionConfirmed`.
            Some("system") if str_field(&rec, "subtype") == Some("compact_boundary") => {
                events.push(AgentEvent::Compacted {
                    tokens_after: rec
                        .pointer("/compact_metadata/post_tokens")
                        .and_then(Value::as_u64),
                });
            }
            _ => {}
        }
        events
    }

    fn take_replies(&mut self) -> Vec<String> {
        std::mem::take(&mut self.replies)
    }

    fn steer(&mut self, prompt: &str, attachments: &[Attachment]) {
        self.replies.push(steer_message(prompt, attachments));
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

/// MonoCode `usageLimitFromRateLimitEvent`: a refusal outside overage,
/// with its reset time (seconds → ms) when given.
fn usage_limit_from_rate_limit_event(rec: &Value) -> Option<Option<i64>> {
    let info = rec.get("rate_limit_info")?;
    if str_field(info, "status") != Some("rejected")
        || info.get("isUsingOverage") == Some(&Value::Bool(true))
    {
        return None;
    }
    Some(
        info.get("resetsAt")
            .and_then(Value::as_i64)
            .map(|secs| secs * 1000),
    )
}

/// MonoCode `isUsageLimitResult`: Claude also ends a limited turn with the
/// limit as its error text.
fn is_usage_limit_text(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("usage limit reached")
        || lower.contains("hit your limit")
        || lower.contains("hit your usage limit")
}

/// MonoCode `contextFromResult`: the top-level `usage` sums every
/// iteration of the turn, so the last of `usage.iterations` is what sits in
/// the window; `modelUsage` carries the window itself.
fn context_from_result(rec: &Value, usage: &Value) -> AgentEvent {
    let last = usage
        .get("iterations")
        .and_then(Value::as_array)
        .and_then(|iterations| iterations.last())
        .unwrap_or(usage);
    let field = |name: &str| last.get(name).and_then(Value::as_u64).unwrap_or(0);
    let input_tokens = field("input_tokens")
        + field("cache_read_input_tokens")
        + field("cache_creation_input_tokens");
    let output_tokens = field("output_tokens");
    let context_window = rec
        .get("modelUsage")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|models| models.values())
        .filter_map(|model| model.get("contextWindow").and_then(Value::as_u64))
        .filter(|window| *window > 0)
        .max();
    AgentEvent::Usage {
        input_tokens,
        output_tokens,
        total_tokens: input_tokens + output_tokens,
        context_window,
    }
}

fn on_result(rec: &Value, rate_limited: Option<Option<i64>>, events: &mut Vec<AgentEvent>) {
    if let Some(usage) = rec.get("usage") {
        let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
        events.push(context_from_result(rec, usage));
        let output_tokens = field("output_tokens");
        // MonoCode `turnMetricsFromResult`.
        let (input, read, write) = (
            field("input_tokens"),
            field("cache_read_input_tokens"),
            field("cache_creation_input_tokens"),
        );
        let cache_reported = usage.get("cache_read_input_tokens").is_some()
            || usage.get("cache_creation_input_tokens").is_some();
        if let Some(metrics) = TurnMetrics::from_counts(
            input,
            output_tokens,
            read,
            write,
            cache_reported.then_some(input + read + write),
        ) {
            events.push(AgentEvent::TurnMetrics(metrics));
        }
    }

    let is_error = rec
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if is_error {
        let errors = rec.get("errors").and_then(Value::as_array);
        // MonoCode `turnStatusFromResult`: a `success` result has already
        // streamed its text (a usage limit, say) as the reply, so only the
        // other subtypes add a notice.
        if str_field(rec, "subtype") != Some("success") {
            let message = errors
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .find(|e| !e.starts_with("[ede_diagnostic]"))
                .or_else(|| str_field(rec, "result"))
                .or_else(|| str_field(rec, "subtype"))
                .unwrap_or("Claude reported an error");
            events.push(AgentEvent::Error(message.to_string()));
        }
        let limited_text = str_field(rec, "result").is_some_and(is_usage_limit_text)
            || errors
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(is_usage_limit_text);
        if let Some(resets_at) = rate_limited.or(limited_text.then_some(None)) {
            events.push(AgentEvent::UsageLimited { resets_at });
        }
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
            disable_hooks: false,
            attachments: Vec::new(),
            plan: false,
            compact: false,
            settings: Default::default(),
            account: None,
            browser_mcp: None,
        }
    }

    #[test]
    fn an_open_browser_tab_adds_its_mcp_server() {
        assert!(!build_args(&request(PermissionPolicy::Ask)).contains(&"--mcp-config".into()));
        let req = SpawnRequest {
            browser_mcp: Some(crate::browser::McpLaunch {
                command: "/bin/bencode".into(),
                args: vec!["--browser-mcp".into(), "/tmp/b.sock".into()],
            }),
            ..request(PermissionPolicy::Ask)
        };
        let args = build_args(&req);
        let at = args.iter().position(|a| a == "--mcp-config").unwrap();
        let config: serde_json::Value = serde_json::from_str(&args[at + 1]).unwrap();
        assert_eq!(
            config["mcpServers"]["bencode-browser"],
            serde_json::json!({
                "type": "stdio",
                "command": "/bin/bencode",
                "args": ["--browser-mcp", "/tmp/b.sock"],
            })
        );
        // The flag takes several values: an option must follow it.
        assert!(args[at + 2].starts_with("--"));
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

        let mut no_hooks = request(PermissionPolicy::Ask);
        no_hooks.disable_hooks = true;
        let no_hooks = build_args(&no_hooks);
        assert!(
            no_hooks
                .windows(2)
                .any(|w| w == ["--settings", r#"{"disableAllHooks":true}"#])
        );
        assert!(!ask.contains(&"--settings".to_string()));

        let mut tuned = request(PermissionPolicy::Ask);
        tuned.settings = [("effort", "ultracode"), ("fast", "true"), ("context", "1m")]
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .into();
        let tuned_args = build_args(&tuned);
        assert!(tuned_args.windows(2).any(|w| w == ["--effort", "xhigh"]));
        assert!(tuned_args.windows(2).any(|w| w == ["--model", "opus[1m]"]));
        assert!(
            tuned_args
                .windows(2)
                .any(|w| w == ["--settings", r#"{"fastMode":true,"ultracode":true}"#])
        );
        tuned.settings.insert("effort".into(), "ultrathink".into());
        assert!(!build_args(&tuned).contains(&"--effort".to_string()));
        assert_eq!(prompt_with_effort(&tuned), "Ultrathink:\nhi");

        let auto = build_args(&request(PermissionPolicy::AutoApprove));
        assert!(
            auto.windows(2)
                .any(|w| w == ["--permission-mode", "bypassPermissions"])
        );
        assert!(auto.contains(&"--allow-dangerously-skip-permissions".to_string()));
        assert!(
            auto.windows(2)
                .any(|w| w == ["--permission-prompt-tool", "stdio"])
        );
        assert!(
            ask.windows(2)
                .any(|w| w == ["--permission-mode", "default"])
        );
        let edits = build_args(&request(PermissionPolicy::AcceptEdits));
        assert!(
            edits
                .windows(2)
                .any(|w| w == ["--permission-mode", "acceptEdits"])
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
    fn a_refused_turn_reports_the_usage_limit() {
        let events = parse_all(&[
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1791098400,"isUsingOverage":false}}"#,
            r#"{"type":"result","subtype":"success","is_error":true,"result":"You've hit your session limit"}"#,
        ]);
        assert!(events.contains(&AgentEvent::UsageLimited {
            resets_at: Some(1_791_098_400_000)
        }));
        // Its text came as the reply; no notice repeats it.
        assert!(!events.iter().any(|e| matches!(e, AgentEvent::Error(_))));
        // An allowed rate-limit event changes nothing.
        let allowed = parse_all(&[
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#,
            r#"{"type":"result","is_error":false}"#,
        ]);
        assert!(
            !allowed
                .iter()
                .any(|e| matches!(e, AgentEvent::UsageLimited { .. }))
        );
        let by_text = parse_all(&[
            r#"{"type":"result","is_error":true,"result":"Claude AI usage limit reached|1791098400"}"#,
        ]);
        assert!(by_text.contains(&AgentEvent::UsageLimited { resets_at: None }));
    }

    #[test]
    fn compaction_is_confirmed_with_the_tokens_left() {
        let events = parse_all(&[
            r#"{"type":"system","subtype":"compact_boundary","session_id":"s","compact_metadata":{"trigger":"manual","pre_tokens":22791,"post_tokens":1289}}"#,
        ]);
        assert!(events.contains(&AgentEvent::Compacted {
            tokens_after: Some(1289)
        }));
    }

    #[test]
    fn result_context_is_the_last_iteration_in_the_cli_window() {
        let events = parse_all(&[
            r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":30,"cache_read_input_tokens":300000,"output_tokens":900,"iterations":[{"input_tokens":10,"cache_read_input_tokens":100000,"output_tokens":400},{"input_tokens":20,"cache_read_input_tokens":200000,"output_tokens":500}]},"modelUsage":{"claude-haiku-5-5":{"contextWindow":200000},"claude-opus-5-5":{"contextWindow":1000000}}}"#,
        ]);
        assert_eq!(
            events[0],
            AgentEvent::Usage {
                input_tokens: 200_020,
                output_tokens: 500,
                total_tokens: 200_520,
                context_window: Some(1_000_000),
            }
        );
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
                    total_tokens: 105,
                    context_window: None,
                },
                AgentEvent::TurnMetrics(TurnMetrics {
                    input_tokens: Some(10),
                    output_tokens: Some(5),
                    cache_read_tokens: Some(90),
                    cache_write_tokens: None,
                    cache_hit_percent: Some(90.0),
                }),
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
            disable_hooks: false,
            attachments: Vec::new(),
            plan: false,
            compact: false,
            settings: Default::default(),
            account: None,
            browser_mcp: None,
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
