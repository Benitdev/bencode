//! Codex CLI via `codex exec --json` (one non-interactive turn per process).
//! `exec` cannot prompt for approvals, so the permission policy maps onto
//! Codex's sandbox levels instead.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::harness::accounts::AccountProfile;
use crate::harness::events::{AgentEvent, DoneStatus, TurnMetrics};
use crate::harness::handle::HarnessProcessHandle;
use crate::harness::probe::AppServer;
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
        account: req.account.clone(),
    };
    process::spawn(spec, CodexParser::default())
}

/// How long a thread edit may take (the app-server loads the thread).
const REWIND_TIMEOUT: Duration = Duration::from_secs(30);

/// MonoCode `rewindCodexLastTurn`: reverts `thread_id` to before its latest
/// user turn, so an edited prompt can be sent in its place. `exec` cannot
/// do this, so a short-lived `codex app-server` loads the thread and
/// reverts it. Blocking; run it on a background executor.
pub fn rewind_last_turn(
    thread_id: &str,
    cwd: &Path,
    account: Option<&AccountProfile>,
) -> Result<()> {
    let program = HarnessResolver::resolve_codex().context("Codex is not installed")?;
    let mut server = AppServer::open(&program, cwd, REWIND_TIMEOUT, account)?;
    server.call(
        "thread/resume",
        json!({ "threadId": thread_id, "cwd": cwd.to_string_lossy() }),
    )?;
    let page = server.call(
        "thread/turns/list",
        json!({
            "threadId": thread_id,
            "limit": 100,
            "sortDirection": "desc",
            "itemsView": "summary",
        }),
    )?;
    let before = last_user_turn_id(&page).context("Codex did not expose a user turn id to edit")?;
    server.call(
        "thread/revert",
        json!({ "threadId": thread_id, "beforeTurnId": before }),
    )?;
    Ok(())
}

/// MonoCode `lastUserTurnId`: the newest turn (the page is newest first)
/// holding a user message.
fn last_user_turn_id(page: &Value) -> Option<&str> {
    page.get("data")?
        .as_array()?
        .iter()
        .find(|turn| {
            turn.get("items")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| str_field(item, "type") == Some("userMessage"))
                })
        })
        .and_then(|turn| str_field(turn, "id"))
}

fn build_args(req: &SpawnRequest) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "exec".into(),
        "--json".into(),
        "--skip-git-repo-check".into(),
    ];
    match req.permission {
        // Plan mode only reads, whatever the access mode.
        _ if req.plan => args.extend(["--sandbox".into(), "read-only".into()]),
        PermissionPolicy::AutoApprove => {
            args.push("--dangerously-bypass-approvals-and-sandbox".into())
        }
        // `codex exec` cannot ask mid-turn, so supervised stays read-only
        // (MonoCode's supervised sandbox) and the edit modes write the workspace.
        PermissionPolicy::AcceptEdits | PermissionPolicy::Auto => args.push("--full-auto".into()),
        PermissionPolicy::Ask => args.extend(["--sandbox".into(), "read-only".into()]),
    }
    if let Some(model) = &req.model {
        args.extend(["--model".into(), model.clone()]);
    }
    if let Some(effort) = req.settings.get("reasoningEffort") {
        args.extend(["-c".into(), format!("model_reasoning_effort=\"{effort}\"")]);
    }
    if let Some(tier) = req.settings.get("serviceTier").filter(|t| *t != "default") {
        args.extend(["-c".into(), format!("service_tier=\"{tier}\"")]);
    }
    if let Some(thread) = &req.resume_id {
        args.extend(["resume".into(), thread.clone()]);
    }
    // `--` keeps a prompt that starts with '-' from being parsed as a flag.
    let prompt = crate::harness::attachments::plain_prompt(&req.prompt, &req.attachments);
    args.extend(["--".into(), prompt]);
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
                    events.push(AgentEvent::SessionStarted {
                        provider_session_id: id.to_string(),
                    });
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
                        context_window: None,
                    });
                    // Codex counts cached input inside `input_tokens`
                    // (MonoCode `mapTokenUsage`).
                    let cached = field("cached_input_tokens");
                    let reported = usage.get("cached_input_tokens").is_some();
                    if let Some(metrics) = TurnMetrics::from_counts(
                        input_tokens,
                        output_tokens,
                        cached,
                        0,
                        reported.then_some(input_tokens),
                    ) {
                        events.push(AgentEvent::TurnMetrics(metrics));
                    }
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
            Some("agent_message" | "agentMessage") => {
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
                let Some(id) = str_field(item, "id") else {
                    return;
                };
                let Some(start) = tool_start(item) else {
                    return;
                };
                // Re-announcing is harmless (the app de-duplicates by call id)
                // and covers items that never had an `item.started`.
                events.push(start);
                let failed = str_field(item, "status") == Some("failed")
                    || item
                        .get("exit_code")
                        .and_then(Value::as_i64)
                        .is_some_and(|code| code != 0);
                events.push(AgentEvent::ToolCallFinish {
                    id: id.to_string(),
                    output: tool_output(item),
                    success: !failed,
                });
            }
        }
    }
}

const PARSED_COMMAND_KEYS: &[&str] = &[
    "commandActions",
    "command_actions",
    "parsed_cmd",
    "parsedCmd",
];
const PARSED_COMMAND_FIELDS: &[&str] = &["command", "cmd"];

/// A POSIX shell's command flag: `-c`, `-lc`, `-ic`, ...
fn is_posix_c_flag(part: &str) -> bool {
    part.strip_prefix('-').is_some_and(|rest| {
        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphabetic()) && rest.contains('c')
    })
}

/// The script of a shell launcher argv (`["/bin/zsh", "-lc", "rg --files"]`),
/// else the argv joined. Mirrors MonoCode's `codexCommandText`.
fn argv_command_text(parts: &[&str]) -> String {
    let launcher = parts[0]
        .trim_matches(|c| c == '\'' || c == '"')
        .replace('\\', "/");
    let launcher = launcher
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let posix = matches!(launcher.as_str(), "sh" | "bash" | "zsh" | "dash" | "ksh");
    let powershell = matches!(
        launcher.as_str(),
        "pwsh" | "pwsh.exe" | "powershell" | "powershell.exe"
    );
    let cmd = matches!(launcher.as_str(), "cmd" | "cmd.exe");

    let last = parts.len().saturating_sub(1);
    for (index, part) in parts.iter().enumerate().take(last).skip(1) {
        if powershell && (part.eq_ignore_ascii_case("-file") || part.eq_ignore_ascii_case("-f")) {
            break;
        }
        let is_flag = (posix && (part.eq_ignore_ascii_case("--command") || is_posix_c_flag(part)))
            || (powershell
                && (part.eq_ignore_ascii_case("-command") || part.eq_ignore_ascii_case("-c")))
            || (cmd && part.eq_ignore_ascii_case("/c"));
        if is_flag {
            return parts[index + 1].to_string();
        }
    }
    parts.join(" ")
}

/// The command a Codex command item ran: a plain string, a launcher argv, or
/// (last resort) the parsed command actions.
fn codex_command_text(item: &Value) -> Option<String> {
    match item.get("command") {
        Some(Value::String(cmd)) if !cmd.trim().is_empty() => return Some(cmd.trim().to_string()),
        Some(Value::Array(argv)) => {
            let parts: Vec<&str> = argv
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            if !parts.is_empty() {
                return Some(argv_command_text(&parts));
            }
        }
        _ => {}
    }
    PARSED_COMMAND_KEYS
        .iter()
        .filter_map(|key| item.get(*key).and_then(Value::as_array))
        .flatten()
        .flat_map(|action| {
            PARSED_COMMAND_FIELDS
                .iter()
                .filter_map(move |field| action.get(*field).and_then(Value::as_str))
        })
        .map(str::trim)
        .find(|cmd| !cmd.is_empty())
        .map(str::to_string)
}

fn tool_start(item: &Value) -> Option<AgentEvent> {
    let id = str_field(item, "id")?.to_string();
    let (name, input) = match str_field(item, "type")? {
        "command_execution" | "commandExecution" => (
            "Bash",
            json!({ "command": codex_command_text(item).unwrap_or_default() }),
        ),
        "file_change" | "fileChange" => (
            "Edit",
            json!({ "changes": item.get("changes").cloned().unwrap_or(Value::Null) }),
        ),
        "mcp_tool_call" | "mcpToolCall" => (
            str_field(item, "tool").unwrap_or("mcp"),
            item.get("arguments").cloned().unwrap_or(Value::Null),
        ),
        "web_search" | "webSearch" => (
            "WebSearch",
            json!({ "query": str_field(item, "query").unwrap_or("") }),
        ),
        _ => return None,
    };
    Some(AgentEvent::ToolCallStart {
        id,
        name: name.to_string(),
        input,
    })
}

fn tool_output(item: &Value) -> String {
    str_field(item, "aggregated_output")
        .or_else(|| str_field(item, "aggregatedOutput"))
        .or_else(|| str_field(item, "output"))
        .map(String::from)
        .or_else(|| item.get("result").map(Value::to_string))
        .or_else(|| item.get("changes").map(Value::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {

    #[test]
    fn rewind_targets_the_newest_turn_with_a_user_message() {
        let page = json!({ "data": [
            { "id": "turn_3", "items": [{ "type": "agentMessage" }] },
            { "id": "turn_2", "items": [{ "type": "userMessage" }, { "type": "agentMessage" }] },
            { "id": "turn_1", "items": [{ "type": "userMessage" }] },
        ]});
        assert_eq!(last_user_turn_id(&page), Some("turn_2"));
        assert_eq!(last_user_turn_id(&json!({ "data": [] })), None);
    }
    use super::*;
    use crate::harness::HarnessKind;

    fn parse_all(lines: &[&str]) -> Vec<AgentEvent> {
        let mut parser = CodexParser::default();
        lines
            .iter()
            .flat_map(|line| parser.parse_line(line))
            .collect()
    }

    #[test]
    fn args_put_prompt_last_after_separator() {
        let req = SpawnRequest {
            harness: HarnessKind::Codex,
            cwd: "/tmp".into(),
            prompt: "-fix it".into(),
            model: Some("gpt-5-codex".into()),
            permission: PermissionPolicy::Ask,
            resume_id: Some("th_1".into()),
            disable_hooks: false,
            attachments: Vec::new(),
            plan: false,
            settings: Default::default(),
            account: None,
        };
        let args = build_args(&req);
        assert_eq!(&args[..3], ["exec", "--json", "--skip-git-repo-check"]);
        assert!(args.windows(2).any(|w| w == ["--sandbox", "read-only"]));
        assert!(args.windows(2).any(|w| w == ["resume", "th_1"]));
        assert_eq!(&args[args.len() - 2..], ["--", "-fix it"]);
        assert!(
            !args.contains(&"-p".to_string()),
            "-p is --profile in codex"
        );
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
        let bash = || AgentEvent::ToolCallStart {
            id: "i1".into(),
            name: "Bash".into(),
            input: json!({"command": "ls"}),
        };
        assert_eq!(
            events,
            vec![
                AgentEvent::SessionStarted {
                    provider_session_id: "th_1".into()
                },
                bash(),
                bash(),
                AgentEvent::ToolCallFinish {
                    id: "i1".into(),
                    output: "a\nb".into(),
                    success: true
                },
                AgentEvent::TextDelta("First.".into()),
                AgentEvent::TextDelta("\n\nSecond.".into()),
                AgentEvent::Usage {
                    input_tokens: 7,
                    output_tokens: 3,
                    total_tokens: 10,
                    context_window: None,
                },
                AgentEvent::TurnMetrics(TurnMetrics {
                    input_tokens: Some(7),
                    output_tokens: Some(3),
                    ..Default::default()
                }),
                AgentEvent::Done(DoneStatus::Completed),
            ]
        );
    }

    #[test]
    fn failed_turn_reports_error() {
        let events = parse_all(&[r#"{"type":"turn.failed","error":{"message":"quota"}}"#]);
        assert_eq!(
            events,
            vec![
                AgentEvent::Error("quota".into()),
                AgentEvent::Done(DoneStatus::Failed)
            ]
        );
    }

    #[test]
    fn extracts_codex_command_formats() {
        // String command
        let item1 = json!({ "command": "cargo test" });
        assert_eq!(codex_command_text(&item1), Some("cargo test".into()));

        // Posix shell array with -c / -lc
        let item2 = json!({ "command": ["/bin/zsh", "-lc", "cargo check --all"] });
        assert_eq!(codex_command_text(&item2), Some("cargo check --all".into()));

        let item3 = json!({ "command": ["bash", "-c", "echo hello"] });
        assert_eq!(codex_command_text(&item3), Some("echo hello".into()));

        // PowerShell array
        let item4 = json!({ "command": ["pwsh.exe", "-Command", "Get-ChildItem"] });
        assert_eq!(codex_command_text(&item4), Some("Get-ChildItem".into()));

        // Cmd array
        let item5 = json!({ "command": ["cmd.exe", "/c", "dir /s"] });
        assert_eq!(codex_command_text(&item5), Some("dir /s".into()));

        // Plain argv array with no shell wrapper
        let item6 = json!({ "command": ["git", "status", "-s"] });
        assert_eq!(codex_command_text(&item6), Some("git status -s".into()));

        // Fallback commandActions / parsed_cmd
        let item7 = json!({
            "commandActions": [
                { "command": "rg --files -g AGENTS.md" }
            ]
        });
        assert_eq!(
            codex_command_text(&item7),
            Some("rg --files -g AGENTS.md".into())
        );

        let item8 = json!({
            "parsed_cmd": [
                { "cmd": "find . -name Cargo.toml" }
            ]
        });
        assert_eq!(
            codex_command_text(&item8),
            Some("find . -name Cargo.toml".into())
        );
    }
}
