//! BenCode's own: reads the sessions Claude Code and Codex keep on disk,
//! so ones started in a terminal can join BenCode as threads that resume
//! them (Settings › Providers › Import sessions).
//!
//! - Claude: `projects/<cwd slug>/<id>.jsonl` in each config folder. Each
//!   line is a record; `user` and `assistant` ones carry the same message
//!   shape as `--output-format stream-json`. Subagents' records are
//!   `isSidechain` (or in `<id>/` beside the file) and are left out.
//! - Codex: `sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl` (and
//!   `archived_sessions/`). `session_meta` names the session and folder,
//!   `turn_context` the model, `response_item`s the conversation.
//!
//! A session becomes turns of [`AgentEvent`]s, the same events a live run
//! folds into a transcript, so the app builds its blocks the usual way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::harness::claude::tool_results;
use crate::harness::patch::EditPatch;
use crate::harness::session_purge::homes_by_account;
use crate::harness::{AgentEvent, HarnessKind, str_field};

/// One CLI session, read whole.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedSession {
    pub provider_session_id: String,
    pub cwd: String,
    /// The CLI's own model id, as it last reported it.
    pub model: Option<String>,
    /// The title the CLI gave it, when it keeps one.
    pub title: Option<String>,
    pub branch: Option<String>,
    /// The account profile whose folder it was read from; `None` is the
    /// default one.
    pub account_id: Option<String>,
    /// Codex moved it to `archived_sessions/`.
    pub archived: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub turns: Vec<ImportedTurn>,
}

/// A prompt and what the agent did with it, each event at its time (ms).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedTurn {
    pub prompt: String,
    pub at: i64,
    pub events: Vec<(i64, AgentEvent)>,
    /// The time of the turn's last record.
    pub ended_at: i64,
}

/// A session file found on disk, with the account it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFile {
    pub path: PathBuf,
    pub account_id: Option<String>,
    pub archived: bool,
}

/// Whether BenCode can import `kind`'s sessions.
pub fn supported(kind: HarnessKind) -> bool {
    matches!(kind, HarnessKind::Claude | HarnessKind::Codex)
}

/// Every session file `kind` keeps, in its default home and every account
/// profile's. Blocking.
pub fn session_files(kind: HarnessKind) -> Vec<SessionFile> {
    let mut found = Vec::new();
    for (account_id, home) in homes_by_account(kind.id()) {
        match kind {
            HarnessKind::Claude => {
                for path in claude_files(&home.join("projects")) {
                    found.push(SessionFile {
                        path,
                        account_id: account_id.clone(),
                        archived: false,
                    });
                }
            }
            HarnessKind::Codex => {
                for (root, archived) in [("sessions", false), ("archived_sessions", true)] {
                    let mut paths = Vec::new();
                    codex_rollouts(&home.join(root), 3, &mut paths);
                    found.extend(paths.into_iter().map(|path| SessionFile {
                        path,
                        account_id: account_id.clone(),
                        archived,
                    }));
                }
            }
            _ => {}
        }
    }
    found
}

/// Reads `file` as a session of `kind`; `None` when it holds no prompt.
/// Blocking.
pub fn read_session(
    kind: HarnessKind,
    file: &SessionFile,
) -> Result<Option<ImportedSession>, String> {
    let text = std::fs::read_to_string(&file.path)
        .map_err(|err| format!("{}: {err}", file.path.display()))?;
    let session = match kind {
        HarnessKind::Claude => {
            let fallback_id = file
                .path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default();
            parse_claude(&text, fallback_id)
        }
        HarnessKind::Codex => parse_codex(&text),
        _ => None,
    };
    Ok(session.map(|session| ImportedSession {
        account_id: file.account_id.clone(),
        archived: file.archived,
        ..session
    }))
}

/// The sessions of `kind` on this machine, each passed to `build` as it is
/// read (so only what `build` keeps stays in memory), one per session id:
/// where an account was switched mid-session, the copy that went
/// furthest. Unreadable files are logged and left out. Blocking.
pub fn read_all<T>(kind: HarnessKind, mut build: impl FnMut(ImportedSession) -> T) -> Vec<T> {
    let mut by_id: HashMap<String, (i64, T)> = HashMap::new();
    for file in session_files(kind) {
        let session = match read_session(kind, &file) {
            Ok(Some(session)) => session,
            Ok(None) => continue,
            Err(err) => {
                log::warn!("could not read a {} session: {err}", kind.id());
                continue;
            }
        };
        let further = by_id
            .get(&session.provider_session_id)
            .is_none_or(|(kept, _)| *kept < session.updated_at);
        if further {
            let (id, updated_at) = (session.provider_session_id.clone(), session.updated_at);
            by_id.insert(id, (updated_at, build(session)));
        }
    }
    let mut sessions: Vec<_> = by_id.into_values().collect();
    sessions.sort_by_key(|(updated_at, _)| std::cmp::Reverse(*updated_at));
    sessions.into_iter().map(|(_, built)| built).collect()
}

fn claude_files(projects: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(projects) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|entry| std::fs::read_dir(entry.path()).ok())
        .flat_map(|files| files.flatten())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl") && path.is_file())
        .collect()
}

/// Codex rollouts, `depth` folders deep at most (`YYYY/MM/DD`).
fn codex_rollouts(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth > 0 {
                codex_rollouts(&entry.path(), depth - 1, found);
            }
        } else if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
        {
            found.push(entry.path());
        }
    }
}

/// Builds turns as records arrive; text before the first prompt is dropped.
#[derive(Default)]
struct Turns {
    turns: Vec<ImportedTurn>,
    first_at: Option<i64>,
    last_at: i64,
}

impl Turns {
    fn at(&mut self, at: Option<i64>) -> i64 {
        let at = at.unwrap_or(self.last_at).max(self.last_at);
        self.first_at.get_or_insert(at);
        self.last_at = at;
        at
    }

    fn prompt(&mut self, prompt: String, at: i64) {
        self.turns.push(ImportedTurn {
            prompt,
            at,
            events: Vec::new(),
            ended_at: at,
        });
    }

    fn event(&mut self, event: AgentEvent, at: i64) {
        if let Some(turn) = self.turns.last_mut() {
            turn.events.push((at, event));
            turn.ended_at = at;
        }
    }
}

fn timestamp(rec: &Value) -> Option<i64> {
    str_field(rec, "timestamp")?
        .parse::<jiff::Timestamp>()
        .ok()
        .map(|t| t.as_millisecond())
}

/// What a person typed, not something the CLI wrote in their place:
/// slash-command echoes, task notifications, injected context and
/// interrupt markers all start with a tag or a bracket.
fn is_typed_prompt(text: &str) -> bool {
    let text = text.trim_start();
    !text.is_empty()
        && !text.starts_with('<')
        && !text.starts_with("[Request interrupted")
        && !text.starts_with("# AGENTS.md instructions")
}

/// Reads a Claude Code session file; `fallback_id` is its file name.
pub fn parse_claude(text: &str, fallback_id: &str) -> Option<ImportedSession> {
    let mut turns = Turns::default();
    let (mut id, mut cwd, mut branch) = (None::<String>, None::<String>, None::<String>);
    let (mut model, mut title, mut summary) = (None::<String>, None::<String>, None::<String>);
    let mut last_usage = None;
    for line in text.lines() {
        let Ok(rec) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let flag = |key: &str| rec.get(key).and_then(Value::as_bool) == Some(true);
        if flag("isSidechain") {
            continue;
        }
        match str_field(&rec, "type") {
            Some("ai-title") => {
                title = str_field(&rec, "aiTitle").map(String::from).or(title);
                continue;
            }
            Some("summary") => {
                summary = str_field(&rec, "summary").map(String::from).or(summary);
                continue;
            }
            Some("user" | "assistant") => {}
            _ => continue,
        }
        if id.is_none() {
            id = str_field(&rec, "sessionId").map(String::from);
        }
        if let Some(dir) = str_field(&rec, "cwd").filter(|dir| !dir.is_empty()) {
            cwd.get_or_insert_with(|| dir.to_string());
        }
        if let Some(name) = str_field(&rec, "gitBranch").filter(|b| !b.is_empty() && *b != "HEAD") {
            branch = Some(name.to_string());
        }
        let at = turns.at(timestamp(&rec));
        let Some(message) = rec.get("message") else {
            continue;
        };
        if str_field(&rec, "type") == Some("user") {
            if flag("isMeta") {
                continue;
            }
            let results = tool_results(&rec);
            if !results.is_empty() {
                // The record's `toolUseResult` describes its one result.
                let mut patch = rec
                    .get("toolUseResult")
                    .filter(|_| results.len() == 1)
                    .and_then(EditPatch::from_claude);
                for (id, output, success) in results {
                    let patch = patch.take().filter(|_| success);
                    turns.event(
                        AgentEvent::ToolCallFinish {
                            id,
                            output,
                            success,
                            patch,
                        },
                        at,
                    );
                }
                continue;
            }
            let prompt = match message.get("content") {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Array(blocks)) => blocks
                    .iter()
                    .filter(|b| str_field(b, "type") == Some("text"))
                    .filter_map(|b| str_field(b, "text"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            if is_typed_prompt(&prompt) {
                turns.prompt(prompt, at);
            }
            continue;
        }
        if let Some(name) = str_field(message, "model").filter(|m| !m.starts_with('<')) {
            model = Some(name.to_string());
        }
        if let Some(usage) = message.get("usage") {
            last_usage = Some(usage.clone());
        }
        let Some(content) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in content {
            let event = match str_field(block, "type") {
                Some("text") => str_field(block, "text")
                    .filter(|t| !t.is_empty())
                    .map(|t| AgentEvent::TextDelta(t.to_string())),
                Some("thinking") => str_field(block, "thinking")
                    .filter(|t| !t.is_empty())
                    .map(|t| AgentEvent::ThinkingDelta(t.to_string())),
                Some("tool_use") => str_field(block, "id").map(|id| AgentEvent::ToolCallStart {
                    id: id.to_string(),
                    name: str_field(block, "name").unwrap_or("tool").to_string(),
                    input: block.get("input").cloned().unwrap_or(Value::Null),
                }),
                _ => None,
            };
            if let Some(event) = event {
                turns.event(event, at);
            }
        }
    }
    if let Some(usage) = last_usage {
        let count = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
        let input = count("input_tokens")
            + count("cache_read_input_tokens")
            + count("cache_creation_input_tokens");
        let output = count("output_tokens");
        let at = turns.last_at;
        turns.event(
            AgentEvent::Usage {
                input_tokens: input,
                output_tokens: output,
                total_tokens: input + output,
                context_window: None,
            },
            at,
        );
    }
    finish(
        turns,
        id.unwrap_or_else(|| fallback_id.to_string()),
        cwd,
        model,
        title.or(summary),
        branch,
    )
}

/// Reads a Codex rollout file.
pub fn parse_codex(text: &str) -> Option<ImportedSession> {
    let mut turns = Turns::default();
    let (mut id, mut cwd, mut branch, mut model) = (None, None, None, None);
    // Tool names by call id, for their outputs.
    let mut calls: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        let Ok(rec) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(payload) = rec.get("payload") else {
            continue;
        };
        let at = turns.at(timestamp(&rec));
        match str_field(&rec, "type") {
            Some("session_meta") => {
                id = id.or_else(|| str_field(payload, "id").map(String::from));
                cwd = cwd.or_else(|| str_field(payload, "cwd").map(String::from));
                branch = payload
                    .pointer("/git/branch")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .or(branch);
            }
            Some("turn_context") => {
                model = str_field(payload, "model").map(String::from).or(model);
            }
            Some("event_msg") if str_field(payload, "type") == Some("token_count") => {
                let Some(info) = payload.get("info").filter(|i| !i.is_null()) else {
                    continue;
                };
                let count = |key: &str| {
                    info.pointer(&format!("/last_token_usage/{key}"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                };
                turns.event(
                    AgentEvent::Usage {
                        input_tokens: count("input_tokens"),
                        output_tokens: count("output_tokens"),
                        total_tokens: count("total_tokens"),
                        context_window: info.get("model_context_window").and_then(Value::as_u64),
                    },
                    at,
                );
            }
            Some("response_item") => {
                for event in codex_item(payload, &mut calls) {
                    match event {
                        CodexItem::Prompt(prompt) => turns.prompt(prompt, at),
                        CodexItem::Event(event) => turns.event(event, at),
                    }
                }
            }
            _ => {}
        }
    }
    finish(turns, id?, cwd, model, None, branch)
}

enum CodexItem {
    Prompt(String),
    Event(AgentEvent),
}

fn codex_item(item: &Value, calls: &mut HashMap<String, String>) -> Vec<CodexItem> {
    let content_text = |key: &str| {
        item.get(key)
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| str_field(part, "text"))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    };
    match str_field(item, "type") {
        Some("message") => {
            let text = content_text("content");
            match str_field(item, "role") {
                Some("user") if is_typed_prompt(&text) => vec![CodexItem::Prompt(text)],
                Some("assistant") if !text.is_empty() => {
                    vec![CodexItem::Event(AgentEvent::TextDelta(text))]
                }
                _ => Vec::new(),
            }
        }
        Some("reasoning") => {
            let text = content_text("summary");
            if text.is_empty() {
                Vec::new()
            } else {
                vec![CodexItem::Event(AgentEvent::ThinkingDelta(text))]
            }
        }
        Some("function_call" | "custom_tool_call" | "local_shell_call") => {
            let Some(call_id) = str_field(item, "call_id").or_else(|| str_field(item, "id")) else {
                return Vec::new();
            };
            let (name, input) = codex_tool(item);
            calls.insert(call_id.to_string(), name.clone());
            vec![CodexItem::Event(AgentEvent::ToolCallStart {
                id: call_id.to_string(),
                name,
                input,
            })]
        }
        Some("function_call_output" | "custom_tool_call_output") => {
            let Some(call_id) = str_field(item, "call_id") else {
                return Vec::new();
            };
            if !calls.contains_key(call_id) {
                return Vec::new();
            }
            let (output, success) = codex_output(item.get("output"));
            vec![CodexItem::Event(AgentEvent::ToolCallFinish {
                id: call_id.to_string(),
                output,
                success,
                patch: None,
            })]
        }
        _ => Vec::new(),
    }
}

/// A Codex call as the name and input BenCode's tool rows read, as the
/// live parser names them (`Bash`, `Edit`).
fn codex_tool(item: &Value) -> (String, Value) {
    let name = str_field(item, "name").unwrap_or("tool");
    let args: Value = match item.get("arguments") {
        Some(Value::String(text)) => serde_json::from_str(text).unwrap_or(Value::Null),
        Some(other) => other.clone(),
        None => Value::Null,
    };
    let command = |value: &Value| match value {
        Value::Array(parts) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        Value::String(text) => text.clone(),
        _ => String::new(),
    };
    if str_field(item, "type") == Some("local_shell_call") {
        let cmd = item
            .pointer("/action/command")
            .map(command)
            .unwrap_or_default();
        return ("Bash".into(), json!({ "command": shell_body(&cmd) }));
    }
    match name {
        "shell" | "shell_command" | "exec_command" | "container.exec" => {
            let cmd = ["command", "cmd"]
                .iter()
                .find_map(|key| args.get(*key))
                .map(command)
                .unwrap_or_default();
            ("Bash".into(), json!({ "command": shell_body(&cmd) }))
        }
        "apply_patch" => {
            let patch = str_field(item, "input")
                .map(String::from)
                .or_else(|| str_field(&args, "input").map(String::from))
                .unwrap_or_default();
            let file = patch.lines().find_map(|line| {
                ["*** Update File: ", "*** Add File: ", "*** Delete File: "]
                    .iter()
                    .find_map(|prefix| line.strip_prefix(prefix))
            });
            ("Edit".into(), json!({ "file_path": file.unwrap_or("") }))
        }
        _ => {
            let input = match (args, str_field(item, "input")) {
                (Value::Object(map), _) => Value::Object(map),
                (_, Some(text)) => json!({ "description": text.lines().next().unwrap_or("") }),
                _ => Value::Null,
            };
            (name.to_string(), input)
        }
    }
}

/// `bash -lc "ls"` shows as `ls`.
fn shell_body(command: &str) -> String {
    for launcher in ["bash -lc ", "bash -c ", "zsh -lc ", "zsh -c ", "sh -c "] {
        if let Some(body) = command.strip_prefix(launcher) {
            return body.to_string();
        }
    }
    command.to_string()
}

/// A Codex call's output: plain text, a list of text parts, or JSON with
/// `output` and an exit code.
fn codex_output(output: Option<&Value>) -> (String, bool) {
    match output {
        Some(Value::String(text)) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) if map.contains_key("output") => {
                let exit = map
                    .get("metadata")
                    .and_then(|m| m.get("exit_code"))
                    .and_then(Value::as_i64);
                let body = map
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                (body.to_string(), exit.is_none_or(|code| code == 0))
            }
            _ => (text.clone(), true),
        },
        Some(Value::Array(parts)) => (
            parts
                .iter()
                .filter_map(|part| str_field(part, "text"))
                .collect::<Vec<_>>()
                .join("\n"),
            true,
        ),
        Some(Value::Object(map)) => (
            map.get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            map.get("success").and_then(Value::as_bool).unwrap_or(true),
        ),
        _ => (String::new(), true),
    }
}

fn finish(
    turns: Turns,
    id: String,
    cwd: Option<String>,
    model: Option<String>,
    title: Option<String>,
    branch: Option<String>,
) -> Option<ImportedSession> {
    let valid_id = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid_id || turns.turns.is_empty() {
        return None;
    }
    Some(ImportedSession {
        provider_session_id: id,
        cwd: cwd.filter(|dir| !dir.is_empty())?,
        model,
        title: title.filter(|t| !t.trim().is_empty()),
        branch,
        account_id: None,
        archived: false,
        created_at: turns.first_at.unwrap_or(turns.last_at),
        updated_at: turns.last_at,
        turns: turns.turns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn a_claude_session_reads_as_turns() {
        let session = parse_claude(&fixture("claude_session_file.jsonl"), "file-id").unwrap();
        assert_eq!(session.provider_session_id, "c0ffee-1");
        assert_eq!(session.cwd, "/Users/me/app");
        assert_eq!(session.branch.as_deref(), Some("main"));
        assert_eq!(session.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(session.title.as_deref(), Some("Fix the login bug"));
        // The slash-command echo and the meta caveat are not prompts.
        let prompts: Vec<_> = session.turns.iter().map(|t| t.prompt.as_str()).collect();
        assert_eq!(prompts, ["Fix the login bug", "thanks"]);
        let events: Vec<_> = session.turns[0].events.iter().map(|(_, e)| e).collect();
        assert!(matches!(events[0], AgentEvent::ThinkingDelta(t) if t == "Looking."));
        assert!(
            matches!(events[1], AgentEvent::ToolCallStart { id, name, .. } if id == "t1" && name == "Edit")
        );
        assert!(matches!(
            events[2],
            AgentEvent::ToolCallFinish { id, success: true, patch: Some(_), .. } if id == "t1"
        ));
        assert!(matches!(events[3], AgentEvent::TextDelta(t) if t == "Fixed."));
        // The subagent's sidechain record stays out.
        assert_eq!(events.len(), 4);
        assert_eq!(session.created_at, 1_767_225_600_000);
        assert!(session.updated_at > session.created_at);
        // The last assistant message's usage gives the context.
        let last = session.turns[1].events.last().map(|(_, e)| e);
        assert!(matches!(
            last,
            Some(AgentEvent::Usage {
                total_tokens: 150,
                ..
            })
        ));
    }

    #[test]
    fn a_claude_file_without_a_prompt_is_no_session() {
        let text = r#"{"type":"queue-operation","operation":"enqueue","sessionId":"x"}"#;
        assert_eq!(parse_claude(text, "x"), None);
    }

    #[test]
    fn a_codex_rollout_reads_as_turns() {
        let session = parse_codex(&fixture("codex_rollout.jsonl")).unwrap();
        assert_eq!(session.provider_session_id, "019a-codex");
        assert_eq!(session.cwd, "/Users/me/app");
        assert_eq!(session.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(session.branch.as_deref(), Some("main"));
        // The environment context Codex injects is not a prompt.
        assert_eq!(session.turns.len(), 1);
        assert_eq!(session.turns[0].prompt, "List the files");
        let events: Vec<_> = session.turns[0].events.iter().map(|(_, e)| e).collect();
        assert!(matches!(events[0], AgentEvent::ThinkingDelta(t) if t == "Listing."));
        assert!(matches!(
            events[1],
            AgentEvent::ToolCallStart { name, input, .. }
                if name == "Bash" && input["command"] == "ls"
        ));
        assert!(matches!(
            events[2],
            AgentEvent::ToolCallFinish { output, success: true, .. } if output == "a\nb"
        ));
        assert!(matches!(
            events[3],
            AgentEvent::ToolCallStart { name, input, .. }
                if name == "Edit" && input["file_path"] == "a.txt"
        ));
        assert!(matches!(
            events[4],
            AgentEvent::ToolCallFinish { success: true, .. }
        ));
        assert!(matches!(
            events[5],
            AgentEvent::Usage {
                total_tokens: 120,
                context_window: Some(258_400),
                ..
            }
        ));
        assert!(matches!(events[6], AgentEvent::TextDelta(t) if t == "Two files."));
    }

    #[test]
    fn a_failed_command_reads_as_failed() {
        let output = json!("{\"output\":\"nope\",\"metadata\":{\"exit_code\":1}}");
        assert_eq!(codex_output(Some(&output)), ("nope".into(), false));
    }

    #[test]
    fn found_files_name_their_account() {
        let root = std::env::temp_dir().join(format!("bencode-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let file = root.join("projects/-Users-me-app/s-1.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "").unwrap();
        // A subagent's file under the session's folder is not a session.
        let sub = root.join("projects/-Users-me-app/s-1/subagents/agent-a.jsonl");
        std::fs::create_dir_all(sub.parent().unwrap()).unwrap();
        std::fs::write(&sub, "").unwrap();
        assert_eq!(claude_files(&root.join("projects")), vec![file]);
        let rollout = root.join("sessions/2026/01/02/rollout-2026-01-02T00-00-00-abc.jsonl");
        std::fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        std::fs::write(&rollout, "").unwrap();
        let mut found = Vec::new();
        codex_rollouts(&root.join("sessions"), 3, &mut found);
        assert_eq!(found, vec![rollout]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
