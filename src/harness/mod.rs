//! Coding-agent CLIs driven over stdio. `spawn` is the single entry point;
//! each provider module owns only its argv and its stdout parser.

pub mod account_identity;
pub mod accounts;
pub mod agy_accounts;
pub mod attachments;
pub use attachments::Attachment;
pub mod antigravity;
pub mod catalog;
pub mod claude;
pub mod codex;
pub mod discovery;
pub mod events;
pub mod grok;
pub mod handle;
pub mod login;
pub mod opencode;
pub mod probe;
pub mod process;
pub mod resolver;
pub mod runtime;
pub mod updates;

pub use events::{AgentEvent, DoneStatus, PermissionRequest, TaskItem, TaskStatus};
pub use handle::HarnessProcessHandle;
pub use resolver::{HarnessInfo, HarnessResolver};

use anyhow::{Result, bail};
use serde_json::Value;
use tokio::sync::mpsc;

pub type EventRx = mpsc::UnboundedReceiver<AgentEvent>;

const SUMMARY_MAX_CHARS: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HarnessKind {
    Claude,
    Antigravity,
    Codex,
    Grok,
    OpenCode,
}

/// Every harness, in picker order.
pub const ALL_HARNESSES: [HarnessKind; 5] = [
    HarnessKind::Claude,
    HarnessKind::Antigravity,
    HarnessKind::Codex,
    HarnessKind::Grok,
    HarnessKind::OpenCode,
];

impl HarnessKind {
    /// Parses MonoCode's `sessions.harness` column.
    pub fn from_id(id: &str) -> Option<Self> {
        match id.to_ascii_lowercase().as_str() {
            "claude" => Some(Self::Claude),
            "antigravity" | "agy" => Some(Self::Antigravity),
            "codex" => Some(Self::Codex),
            "grok" => Some(Self::Grok),
            "opencode" => Some(Self::OpenCode),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Antigravity => "antigravity",
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Antigravity => "Antigravity",
            Self::Codex => "Codex",
            Self::Grok => "Grok Build",
            Self::OpenCode => "OpenCode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPolicy {
    /// Run every tool without asking (MonoCode full-access).
    AutoApprove,
    /// Ask the user through `AgentEvent::PermissionRequest` where the harness
    /// supports it; otherwise fall back to the harness' sandboxed default
    /// (MonoCode supervised).
    Ask,
    /// Apply file edits without asking; ask for anything else.
    AcceptEdits,
    /// Let the harness' own reviewer approve or deny actions.
    Auto,
}

#[derive(Debug, Clone)]
pub struct SpawnRequest {
    pub harness: HarnessKind,
    pub cwd: String,
    pub prompt: String,
    /// CLI model id (already stripped of the `harness:` prefix).
    pub model: Option<String>,
    pub permission: PermissionPolicy,
    /// Provider session to continue, from a previous `SessionStarted`.
    pub resume_id: Option<String>,
    /// Claude only: run with `disableAllHooks` (MonoCode "Claude Code hooks" off).
    pub disable_hooks: bool,
    /// Files the user attached to this prompt.
    pub attachments: Vec<Attachment>,
    /// MonoCode Plan mode: review a plan before building.
    pub plan: bool,
    /// MonoCode `compactContext`: summarise the thread's older context
    /// instead of taking a turn. Claude does it for the `/compact` prompt
    /// itself; Grok Build has a request of its own for it.
    pub compact: bool,
    /// The thread's model settings (`effort`, `fast`, …), defaults filled in.
    pub settings: std::collections::BTreeMap<String, String>,
    /// The thread's account profile; None runs under the default one.
    pub account: Option<accounts::AccountProfile>,
    /// The in-app browser's MCP server, offered to the agent while a
    /// browser tab is open (`app/browser.rs`).
    pub browser_mcp: Option<crate::browser::McpLaunch>,
}

/// Starts one agent turn. Non-blocking apart from a fork/exec; safe to call
/// from the UI thread or a background executor.
pub fn spawn(req: &SpawnRequest) -> Result<(HarnessProcessHandle, EventRx)> {
    if !std::path::Path::new(&req.cwd).is_dir() {
        bail!("working directory does not exist: {}", req.cwd);
    }
    match req.harness {
        HarnessKind::Claude => claude::spawn(req),
        HarnessKind::Antigravity => antigravity::spawn(req),
        HarnessKind::Codex => codex::spawn(req),
        HarnessKind::Grok => grok::spawn(req),
        HarnessKind::OpenCode => opencode::spawn(req),
    }
}

pub(crate) fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// One-line, human-readable summary of a tool call for approval prompts.
pub fn summarize_tool_input(tool: &str, input: &Value) -> String {
    let preferred = [
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
    ];
    let summary = preferred
        .iter()
        .find_map(|key| str_field(input, key))
        .map(String::from)
        .unwrap_or_else(|| match input {
            Value::Null => tool.to_string(),
            other => other.to_string(),
        });
    truncate_chars(&summary, SUMMARY_MAX_CHARS)
}

fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn harness_ids_round_trip() {
        for kind in ALL_HARNESSES {
            assert_eq!(HarnessKind::from_id(kind.id()), Some(kind));
        }
        assert_eq!(
            HarnessKind::from_id("pi"),
            None,
            "unsupported harnesses must not silently become Claude"
        );
    }

    #[test]
    fn summary_prefers_meaningful_fields_and_truncates_on_char_boundary() {
        assert_eq!(
            summarize_tool_input("Bash", &json!({"command": "ls -la"})),
            "ls -la"
        );
        assert_eq!(
            summarize_tool_input("Edit", &json!({"file_path": "/a.rs", "old": "x"})),
            "/a.rs"
        );
        let long = "é".repeat(SUMMARY_MAX_CHARS + 10);
        let summary = summarize_tool_input("Bash", &json!({ "command": long }));
        assert_eq!(summary.chars().count(), SUMMARY_MAX_CHARS + 1);
    }

    #[test]
    fn spawn_rejects_missing_cwd() {
        let req = SpawnRequest {
            harness: HarnessKind::Claude,
            cwd: "/definitely/not/here".into(),
            prompt: "hi".into(),
            model: None,
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
        assert!(spawn(&req).is_err());
    }
}
