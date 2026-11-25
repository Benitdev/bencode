//! Codex CLI via `codex exec --json` (one non-interactive turn per process).
//! `exec` cannot prompt for approvals, so the permission policy maps onto
//! Codex's sandbox levels instead.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
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
    let home = codex_home(req.account.as_ref());
    let spec = ProcessSpec {
        program,
        args: build_args(req),
        cwd: req.cwd.clone(),
        stdin: StdinMode::Null,
        permission_responder: None,
        can_steer: false,
        account: req.account.clone(),
        env: super::app_env(req),
    };
    process::spawn(spec, CodexParser::for_session(home, req.resume_id.clone()))
}

/// The `CODEX_HOME` a spawned Codex process writes its session rollout under.
fn codex_home(account: Option<&AccountProfile>) -> Option<PathBuf> {
    if let Some(account) = account {
        return Some(account.dir.clone());
    }
    std::env::var_os("CODEX_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
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
        PermissionPolicy::AcceptEdits | PermissionPolicy::Auto => {
            args.push("--full-auto".into());
            // MonoCode `withNetwork`: the workspace sandbox blocks loopback
            // too, and `bencode app` needs the app's socket.
            if req.app.is_some() {
                args.extend([
                    "-c".into(),
                    "sandbox_workspace_write.network_access=true".into(),
                ]);
            }
        }
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
    for launch in &req.mcp_servers {
        // TOML values; a JSON string or array of strings is one.
        let key = format!("mcp_servers.{}", launch.name);
        let command = serde_json::to_string(&launch.command).unwrap_or_default();
        let launch_args = serde_json::to_string(&launch.args).unwrap_or_default();
        args.extend([
            "-c".into(),
            format!("{key}.command={command}"),
            "-c".into(),
            format!("{key}.args={launch_args}"),
        ]);
    }
    if let Some(thread) = &req.resume_id {
        args.extend(["resume".into(), thread.clone()]);
    }
    // `--` keeps a prompt that starts with '-' from being parsed as a flag.
    let prompt = crate::harness::attachments::plain_prompt(&req.prompt, &req.attachments);
    args.extend(["--".into(), prompt]);
    args
}

/// Latest per-request context usage read from Codex's rollout JSONL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RolloutUsage {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    context_window: Option<u64>,
}

/// Backwards scan window when reading the tail of a Codex rollout JSONL.
const ROLLOUT_CHUNK_BYTES: usize = 64 * 1024;
const ROLLOUT_MAX_SCAN_BYTES: usize = 16 * 1024 * 1024;
const ROLLOUT_MAX_CARRY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub struct CodexParser {
    wrote_text: bool,
    /// `CODEX_HOME` for the running turn; `None` in pure unit tests.
    codex_home: Option<PathBuf>,
    thread_id: Option<String>,
    rollout_path: Option<PathBuf>,
    last_rollout_len: Option<u64>,
    last_emitted_usage: Option<RolloutUsage>,
}

impl CodexParser {
    fn for_session(codex_home: Option<PathBuf>, thread_id: Option<String>) -> Self {
        Self {
            codex_home,
            thread_id,
            ..Self::default()
        }
    }

    /// Reads the latest `last_token_usage` and `model_context_window` from
    /// Codex's rollout file when it has grown since the last check.
    ///
    /// `codex exec --json`'s `turn.completed.usage` is the cumulative token
    /// spend across every model request in the turn (good for `TurnMetrics`,
    /// wrong for context occupancy) and carries no context window. The rollout
    /// JSONL records both the last request's size and `model_context_window`.
    fn poll_rollout_usage(&mut self, events: &mut Vec<AgentEvent>) {
        let Some(usage) = self.read_rollout_usage() else {
            return;
        };
        if self.last_emitted_usage == Some(usage) {
            return;
        }
        self.last_emitted_usage = Some(usage);
        events.push(AgentEvent::Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            context_window: usage.context_window,
        });
    }

    fn read_rollout_usage(&mut self) -> Option<RolloutUsage> {
        let home = self.codex_home.as_deref()?;
        let thread_id = self.thread_id.as_deref()?;
        if self.rollout_path.is_none() {
            self.rollout_path = find_rollout(home, thread_id);
        }
        let path = self.rollout_path.as_deref()?;
        let len = std::fs::metadata(path).ok()?.len();
        if self.last_rollout_len == Some(len) {
            return None;
        }
        self.last_rollout_len = Some(len);
        read_last_rollout_usage(path)
    }
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
                    if self.thread_id.as_deref() != Some(id) {
                        self.thread_id = Some(id.to_string());
                        self.rollout_path = None;
                        self.last_rollout_len = None;
                    }
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
                self.poll_rollout_usage(&mut events);
            }
            Some("turn.completed") => {
                self.poll_rollout_usage(&mut events);
                if let Some(usage) = rec.get("usage") {
                    let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
                    let input_tokens = field("input_tokens");
                    let output_tokens = field("output_tokens");
                    // Codex counts cached input inside `input_tokens`
                    // (MonoCode `mapTokenUsage`). `turn.completed.usage` is the
                    // whole turn's spend, so it feeds `TurnMetrics` only; the
                    // context meter comes from the rollout's `last_token_usage`.
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
                    patch: None,
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

fn find_rollout(codex_home: &Path, thread_id: &str) -> Option<PathBuf> {
    if thread_id.is_empty()
        || !thread_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    let suffix = format!("-{thread_id}.jsonl");
    find_rollout_in(&codex_home.join("sessions"), &suffix, 3)
}

/// Walks `sessions/YYYY/MM/DD` newest-first so active sessions are found on
/// the first dated directory check.
fn find_rollout_in(dir: &Path, suffix: &str, depth: usize) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        if kind.is_dir() {
            if depth > 0 {
                subdirs.push((name, entry.path()));
            }
        } else if name
            .to_str()
            .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(suffix))
        {
            return Some(entry.path());
        }
    }
    subdirs.sort_by(|(a, _), (b, _)| b.cmp(a));
    for (_, path) in subdirs {
        if let Some(found) = find_rollout_in(&path, suffix, depth - 1) {
            return Some(found);
        }
    }
    None
}

/// Scans backwards from EOF in 64 KiB chunks for the latest `token_count`
/// event carrying `last_token_usage` and `model_context_window`.
fn read_last_rollout_usage(path: &Path) -> Option<RolloutUsage> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut pos = file.metadata().ok()?.len();
    let mut scanned = 0usize;
    let mut carry = Vec::<u8>::new();
    while pos > 0 && scanned < ROLLOUT_MAX_SCAN_BYTES {
        let take = usize::try_from(pos)
            .unwrap_or(ROLLOUT_CHUNK_BYTES)
            .min(ROLLOUT_CHUNK_BYTES);
        pos -= take as u64;
        scanned += take;
        file.seek(SeekFrom::Start(pos)).ok()?;
        let mut chunk = vec![0u8; take];
        file.read_exact(&mut chunk).ok()?;
        if !carry.is_empty() {
            chunk.extend_from_slice(&carry);
            carry.clear();
        }
        let mut iter = chunk.rsplit(|&b| b == b'\n').peekable();
        while let Some(line) = iter.next() {
            if pos > 0 && iter.peek().is_none() {
                if line.len() <= ROLLOUT_MAX_CARRY_BYTES {
                    carry.extend_from_slice(line);
                }
                break;
            }
            if let Some(usage) = parse_rollout_token_count(line) {
                return Some(usage);
            }
        }
    }
    None
}

fn parse_rollout_token_count(line: &[u8]) -> Option<RolloutUsage> {
    if !line
        .windows(b"token_count".len())
        .any(|w| w == b"token_count")
    {
        return None;
    }
    let rec: Value = serde_json::from_slice(line).ok()?;
    if str_field(&rec, "type") != Some("event_msg") {
        return None;
    }
    let payload = rec.get("payload")?;
    if str_field(payload, "type") != Some("token_count") {
        return None;
    }
    let info = payload.get("info").filter(|v| v.is_object())?;
    let last = info.get("last_token_usage").filter(|v| v.is_object())?;
    let field = |name: &str| last.get(name).and_then(Value::as_u64).unwrap_or(0);
    let input_tokens = field("input_tokens");
    let output_tokens = field("output_tokens");
    let total = field("total_tokens");
    let total_tokens = if total > 0 {
        total
    } else {
        input_tokens + output_tokens
    };
    let context_window = info
        .get("model_context_window")
        .and_then(Value::as_u64)
        .filter(|&w| w > 0);
    (total_tokens > 0).then_some(RolloutUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        context_window,
    })
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
    fn app_tools_add_their_mcp_servers_ahead_of_resume() {
        let mut req = SpawnRequest {
            harness: HarnessKind::Codex,
            cwd: "/tmp".into(),
            prompt: "go".into(),
            model: None,
            permission: PermissionPolicy::Ask,
            resume_id: Some("th_1".into()),
            disable_hooks: false,
            attachments: Vec::new(),
            plan: false,
            compact: false,
            settings: Default::default(),
            account: None,
            mcp_servers: vec![crate::mcp::McpLaunch {
                name: "bencode-browser".into(),
                command: "/Apps/Ben Code/bencode".into(),
                args: vec!["--browser-mcp".into(), "/tmp/b.sock".into()],
            }],
            app: None,
            system_prompt: None,
        };
        let args = build_args(&req);
        assert!(args.windows(2).any(|w| w
            == [
                "-c",
                r#"mcp_servers.bencode-browser.command="/Apps/Ben Code/bencode""#
            ]));
        assert!(args.windows(2).any(|w| w
            == [
                "-c",
                r#"mcp_servers.bencode-browser.args=["--browser-mcp","/tmp/b.sock"]"#
            ]));
        // Config overrides go before the `resume` subcommand.
        let config = args
            .iter()
            .position(|a| a.starts_with("mcp_servers"))
            .unwrap();
        let resume = args.iter().position(|a| a == "resume").unwrap();
        assert!(config < resume);
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
            compact: false,
            settings: Default::default(),
            account: None,
            mcp_servers: Vec::new(),
            app: None,
            system_prompt: None,
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
    fn app_access_opens_the_sandbox_to_the_apps_socket() {
        let mut req = SpawnRequest {
            harness: HarnessKind::Codex,
            cwd: "/tmp".into(),
            prompt: "hi".into(),
            model: None,
            permission: PermissionPolicy::Auto,
            resume_id: None,
            disable_hooks: false,
            attachments: Vec::new(),
            plan: false,
            compact: false,
            settings: Default::default(),
            account: None,
            mcp_servers: Vec::new(),
            app: None,
            system_prompt: None,
        };
        let network = "sandbox_workspace_write.network_access=true".to_string();
        assert!(!build_args(&req).contains(&network));
        req.app = Some(crate::harness::AppAccess {
            socket: "/tmp/a.sock".into(),
            token: "t".into(),
        });
        assert!(build_args(&req).contains(&network));
        assert_eq!(
            crate::harness::app_env(&req),
            [
                ("BENCODE_APP_SOCKET".to_string(), "/tmp/a.sock".to_string()),
                ("BENCODE_APP_TOKEN".to_string(), "t".to_string()),
            ]
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
                    success: true,
                    patch: None,
                },
                AgentEvent::TextDelta("First.".into()),
                AgentEvent::TextDelta("\n\nSecond.".into()),
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
    fn context_usage_comes_from_rollout_last_request_and_window() {
        let dir =
            std::env::temp_dir().join(format!("bencode-codex-rollout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());

        let rollout = dir.join("sessions/2026/11/25/rollout-2026-11-25T14-00-00-th_sol.jsonl");
        std::fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        // Pad > 64 KiB between token_count lines to exercise backwards chunk scanning.
        let padding = "x".repeat(ROLLOUT_CHUNK_BYTES + 128);
        let content = format!(
            concat!(
                "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":null}}}}\n",
                "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"input_tokens\":500000,\"output_tokens\":2000,\"total_tokens\":502000}},\"last_token_usage\":{{\"input_tokens\":150000,\"output_tokens\":8483,\"total_tokens\":158483}},\"model_context_window\":475000}}}}}}\n",
                "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call_output\",\"output\":\"{}\"}}}}\n",
                "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":null}}}}\n"
            ),
            padding
        );
        std::fs::write(&rollout, content).unwrap();

        let mut parser = CodexParser::for_session(Some(dir), None);
        let events: Vec<_> = [
            r#"{"type":"thread.started","thread_id":"th_sol"}"#,
            r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"Done."}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":740000,"cached_input_tokens":700000,"output_tokens":3700}}"#,
        ]
        .iter()
        .flat_map(|line| parser.parse_line(line))
        .collect();

        // Context ring gets last_token_usage + model_context_window (158.5K / 475K),
        // NOT cumulative turn spend (743.7K / 200K).
        let usages: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::Usage { .. }))
            .collect();
        assert_eq!(
            usages,
            vec![&AgentEvent::Usage {
                input_tokens: 150_000,
                output_tokens: 8_483,
                total_tokens: 158_483,
                context_window: Some(475_000),
            }]
        );
        // Turn metrics still reflect the whole turn's cumulative spend.
        let metrics = events
            .iter()
            .find_map(|e| match e {
                AgentEvent::TurnMetrics(m) => Some(m),
                _ => None,
            })
            .unwrap();
        assert_eq!(metrics.input_tokens, Some(740_000));
        assert_eq!(metrics.output_tokens, Some(3_700));
        assert_eq!(metrics.cache_read_tokens, Some(700_000));
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
