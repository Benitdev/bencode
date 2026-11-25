//! Grok Build (xAI's `grok`) over ACP (`acp.rs`), as MonoCode's
//! `providers/grok/grok.ts` and `grokProtocol.ts` drive it: one
//! `grok agent stdio` per turn, signed in by `authenticate`, follow-ups
//! handed to the running turn (`_x.ai/interject`), `/compact` as
//! `_x.ai/compact_conversation`; its models and effort menus.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use serde_json::{Value, json};

use crate::harness::acp::{self, AcpParser, Dialect, available_models, model_id};
use crate::harness::catalog::{ModelOption, ModelSetting, SettingKind};
use crate::harness::handle::HarnessProcessHandle;
use crate::harness::process::{self, ProcessSpec, StdinMode};
use crate::harness::resolver::HarnessResolver;
use crate::harness::{EventRx, HarnessKind, PermissionPolicy, SpawnRequest, str_field};

/// MonoCode `AUTH_HELP`.
pub const AUTH_HELP: &str =
    "Grok Build is not signed in. Run `grok login` in a terminal, or set XAI_API_KEY.";

/// The browser OAuth method: it has no headless completion, so never pick it.
const BROWSER_AUTH: &str = "grok.com";

/// Grok Build's ACP.
static GROK: Dialect = Dialect {
    label: "Grok Build",
    log: "grok",
    auth_method,
    start_error,
    prompt_error,
    new_session_params: session_new_params,
    session_meta: || None,
    resume_with_cwd: false,
    // Plan mode and approvals go on argv and in `session/new`'s meta.
    mode: |_, _| None,
    compact: Some("_x.ai/compact_conversation"),
    interject: true,
    cached_in_input: true,
    approves_in_auto: true,
    wait_open_tools: false,
    usage_limit: |_| None,
};

/// The parser for one Grok turn.
fn parser(req: &SpawnRequest) -> AcpParser {
    AcpParser::new(req, &GROK)
}

pub fn spawn(req: &SpawnRequest) -> Result<(HarnessProcessHandle, EventRx)> {
    let program = HarnessResolver::resolve_grok().unwrap_or_else(|| "grok".into());
    let mut parser = parser(req);
    let spec = ProcessSpec {
        program,
        args: build_args(req),
        cwd: req.cwd.clone(),
        stdin: StdinMode::Protocol {
            initial: format!("{}\n", parser.initialize()),
        },
        permission_responder: Some(acp::permission_response),
        can_steer: super::can_steer(HarnessKind::Grok, req.compact),
        account: None,
        env: super::app_env(req),
    };
    process::spawn(spec, parser)
}

/// MonoCode `grokEffort`.
fn effort(req: &SpawnRequest) -> Option<&str> {
    ["effort", "reasoning"]
        .iter()
        .filter_map(|id| req.settings.get(*id))
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
}

/// MonoCode `grokSpawnArgs`: global flags before `agent`, `stdio` last.
fn build_args(req: &SpawnRequest) -> Vec<String> {
    let mut args: Vec<String> = vec!["--no-auto-update".into()];
    if req.plan {
        args.extend(["--permission-mode".into(), "plan".into()]);
    }
    args.extend(["agent".into(), "--no-leader".into()]);
    if let Some(model) = &req.model {
        args.extend(["--model".into(), model.clone()]);
    }
    if let Some(effort) = effort(req) {
        args.extend(["--reasoning-effort".into(), effort.to_string()]);
    }
    if req.permission == PermissionPolicy::AutoApprove && !req.plan {
        args.push("--always-approve".into());
    }
    args.push("stdio".into());
    args
}

/// The ACP arguments for a model probe: no model, nothing approved.
pub fn probe_args() -> [&'static str; 4] {
    ["--no-auto-update", "agent", "--no-leader", "stdio"]
}

/// MonoCode `CLIENT_CAPABILITIES` and `clientInfo`.
pub fn initialize_params() -> Value {
    acp::initialize_params()
}

/// MonoCode `grokSessionNewParams`.
fn session_new_params(
    cwd: &str,
    permission: PermissionPolicy,
    plan: bool,
    mcp_servers: &Value,
) -> Value {
    let mut params = json!({ "cwd": cwd, "mcpServers": mcp_servers });
    let meta = match permission {
        PermissionPolicy::AutoApprove if !plan => Some(json!({ "yoloMode": true })),
        PermissionPolicy::Auto => Some(json!({ "autoMode": true })),
        _ => None,
    };
    if let Some(meta) = meta {
        params["_meta"] = meta;
    }
    params
}

/// MonoCode `grokAuthMethodId`: an API key when the agent saw one, else its
/// default, else the cached `grok login` token; never the browser flow.
pub fn auth_method(init: &Value) -> Option<String> {
    let ids: Vec<&str> = init
        .get("authMethods")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|method| str_field(method, "id"))
        .map(str::trim)
        .filter(|id| !id.is_empty() && *id != BROWSER_AUTH)
        .collect();
    let default = init
        .pointer("/_meta/defaultAuthMethodId")
        .and_then(Value::as_str);
    ["xai.api_key"]
        .into_iter()
        .chain(default)
        .chain(["cached_token"])
        .find(|id| ids.contains(id))
        .or_else(|| ids.first().copied())
        .map(String::from)
}

fn mentions_auth(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    ["auth", "login", "credential", "api key", "xai_api_key"]
        .iter()
        .any(|word| lower.contains(word))
}

/// A failed prompt that reads as a sign-in problem says how to sign in.
fn prompt_error(err: String) -> String {
    if mentions_auth(&err) {
        format!("{}\n\n{AUTH_HELP}", err.trim())
    } else {
        err
    }
}

/// MonoCode `grokAuthError`: why the agent did not start.
fn start_error(detail: &str) -> String {
    if mentions_auth(detail) {
        format!("{}\n\n{AUTH_HELP}", detail.trim())
    } else if detail.to_ascii_lowercase().contains("timed out") {
        format!("Grok Build did not start. {AUTH_HELP}")
    } else {
        format!("Grok Build did not start. {detail}")
    }
}

/// MonoCode `EFFORT_LABELS`.
fn effort_label(value: &str) -> Option<&'static str> {
    Some(match value {
        "xhigh" => "Extra High",
        "high" => "High",
        "medium" => "Medium",
        "low" => "Low",
        _ => return None,
    })
}

/// MonoCode `effortSetting`.
fn effort_setting(options: Vec<(String, String)>, default: Option<&str>) -> ModelSetting {
    let default = default
        .filter(|d| options.iter().any(|(v, _)| v == d))
        .map(String::from)
        .unwrap_or_else(|| options[0].0.clone());
    ModelSetting {
        id: "effort".into(),
        label: "Reasoning".into(),
        kind: SettingKind::Select,
        default,
        options,
    }
}

/// MonoCode `displayName`: `grok-4.6` → `Grok 4.6`.
fn display_name(native: &str) -> String {
    native
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn model_option(native: &str, label: String, settings: Vec<ModelSetting>) -> ModelOption {
    ModelOption {
        key: format!("grok:{native}"),
        label,
        harness: HarnessKind::Grok,
        native: native.to_string(),
        provider: None,
        settings,
    }
}

fn unique(models: Vec<ModelOption>) -> Vec<ModelOption> {
    let mut seen = HashSet::new();
    models
        .into_iter()
        .filter(|m| seen.insert(m.key.clone()))
        .collect()
}

/// MonoCode `modelsFromInitialize` / `modelsFromSessionNew`, effort menus
/// included.
pub fn models_from_acp(result: &Value) -> Vec<ModelOption> {
    let models = available_models(result)
        .into_iter()
        .filter_map(|model| {
            let native = model_id(model)?;
            let label = ["name", "displayName"]
                .iter()
                .find_map(|key| str_field(model, key))
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map_or_else(|| display_name(native), String::from);
            let meta = model.get("_meta").unwrap_or(model);
            let efforts: Vec<(String, String, bool)> = ["reasoningEfforts", "reasoning_efforts"]
                .iter()
                .find_map(|key| meta.get(*key)?.as_array())
                .into_iter()
                .flatten()
                .filter_map(|effort| {
                    let value = str_field(effort, "value")
                        .or_else(|| str_field(effort, "id"))?
                        .trim();
                    let label = effort_label(value).map(String::from).unwrap_or_else(|| {
                        let label = str_field(effort, "label").unwrap_or(value).trim();
                        label
                            .strip_suffix(" Effort")
                            .unwrap_or(label)
                            .trim()
                            .to_string()
                    });
                    let default = effort.get("default") == Some(&Value::Bool(true));
                    (!value.is_empty()).then(|| (value.to_string(), label, default))
                })
                .collect();
            let default = str_field(meta, "reasoningEffort")
                .map(String::from)
                .or_else(|| {
                    efforts
                        .iter()
                        .find(|(_, _, default)| *default)
                        .map(|(v, _, _)| v.clone())
                });
            let settings = if efforts.is_empty() {
                Vec::new()
            } else {
                let options = efforts.into_iter().map(|(v, l, _)| (v, l)).collect();
                vec![effort_setting(options, default.as_deref())]
            };
            Some(model_option(native, label, settings))
        })
        .collect();
    unique(models)
}

/// MonoCode `modelsFromGrokModelsOutput`: `* id` / `- id` lines of
/// `grok models`, colour codes stripped.
pub fn models_from_cli(stdout: &str) -> Vec<ModelOption> {
    let models = stdout
        .lines()
        .filter_map(|line| {
            let line = strip_ansi(line);
            let rest = line.trim().strip_prefix(['*', '+', '-'])?;
            let native = rest.split_whitespace().next()?;
            (rest.starts_with(char::is_whitespace) && !native.is_empty())
                .then(|| model_option(native, display_name(native), Vec::new()))
        })
        .collect();
    unique(models)
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// MonoCode `fallbackGrokModels`: the catalog until `grok` lists its own.
pub fn seed_models() -> Vec<ModelOption> {
    let efforts = |levels: &[&str]| {
        let options = levels
            .iter()
            .map(|l| (l.to_string(), effort_label(l).unwrap_or(l).to_string()))
            .collect();
        vec![effort_setting(options, Some("high"))]
    };
    vec![
        model_option(
            "grok-4.6",
            "Grok 4.6".into(),
            efforts(&["xhigh", "high", "medium", "low"]),
        ),
        model_option(
            "grok-4.5",
            "Grok 4.5".into(),
            efforts(&["high", "medium", "low"]),
        ),
    ]
}

/// MonoCode `is_grok_agent`: xAI's `grok`, not another CLI of that name.
/// The official installer puts it under `~/.grok`; elsewhere its binary
/// must name Grok Build.
pub fn is_grok_build(path: &Path) -> bool {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let in_grok_home = |p: &Path| p.components().any(|c| c.as_os_str() == ".grok");
    in_grok_home(path) || in_grok_home(&resolved) || file_mentions_grok_build(&resolved)
}

/// The markers can sit deep in the compiled binary, so the whole file is
/// read in chunks that overlap enough to catch one across a boundary.
fn file_mentions_grok_build(path: &Path) -> bool {
    use std::io::Read;
    const MARKERS: [&[u8]; 4] = [
        b"xai-grok",
        b"Grok Build",
        b"docs.x.ai/build",
        b"grok agent",
    ];
    const CHUNK: usize = 1024 * 1024;
    const OVERLAP: usize = 64;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = vec![0u8; CHUNK + OVERLAP];
    let mut carry = 0;
    loop {
        let n = match file.read(&mut buf[carry..]) {
            Ok(0) | Err(_) => return false,
            Ok(n) => n,
        };
        let filled = carry + n;
        let window = &buf[..filled];
        if MARKERS
            .iter()
            .any(|marker| window.windows(marker.len()).any(|w| w == *marker))
        {
            return true;
        }
        carry = filled.min(OVERLAP);
        buf.copy_within(filled - carry..filled, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::acp::*;
    use crate::harness::attachments::Attachment;
    use crate::harness::events::{AgentEvent, DoneStatus, TaskItem, TaskStatus, TurnMetrics};
    use crate::harness::process::LineParser;

    type GrokParser = AcpParser;

    fn request(permission: PermissionPolicy) -> SpawnRequest {
        SpawnRequest {
            harness: HarnessKind::Grok,
            cwd: "/repo".into(),
            prompt: "hi".into(),
            model: Some("grok-4.6".into()),
            permission,
            resume_id: None,
            disable_hooks: false,
            attachments: Vec::new(),
            plan: false,
            compact: false,
            settings: [("effort".to_string(), "high".to_string())].into(),
            account: None,
            computer_turn: None,
            mcp_servers: Vec::new(),
            app: None,
            system_prompt: None,
        }
    }

    fn sent(parser: &mut GrokParser) -> Vec<Value> {
        parser
            .take_replies()
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn reply(id: u64, result: Value) -> String {
        json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
    }

    /// Drives a parser through initialize and session/new.
    fn bound(permission: PermissionPolicy) -> GrokParser {
        let mut parser = parser(&request(permission));
        let init: Value = serde_json::from_str(&parser.initialize()).unwrap();
        assert_eq!(init["method"], "initialize");
        parser.parse_line(&reply(
            1,
            json!({
                "authMethods": [{ "id": "cached_token" }],
                "_meta": { "modelState": { "currentModelId": "grok-4.6", "availableModels": [
                    { "modelId": "grok-4.6", "name": "Grok 4.6", "_meta": { "totalContextTokens": 500000 } }
                ] } }
            }),
        ));
        let auth = sent(&mut parser);
        assert_eq!(auth[0]["method"], "authenticate");
        assert_eq!(auth[0]["params"]["methodId"], "cached_token");
        parser.parse_line(&reply(2, json!({})));
        let new = sent(&mut parser);
        assert_eq!(new[0]["method"], "session/new");
        assert_eq!(new[0]["params"]["cwd"], "/repo");
        let events = parser.parse_line(&reply(
            3,
            json!({ "sessionId": "ses-1", "models": { "currentModelId": "grok-4.6" } }),
        ));
        assert_eq!(
            events,
            vec![AgentEvent::SessionStarted {
                provider_session_id: "ses-1".into()
            }]
        );
        let prompt = sent(&mut parser);
        assert_eq!(prompt[0]["method"], "session/prompt");
        assert_eq!(prompt[0]["params"]["sessionId"], "ses-1");
        assert_eq!(prompt[0]["params"]["prompt"][0]["text"], "hi");
        parser
    }

    #[test]
    fn grok_ends_its_turn_with_tool_calls_still_open() {
        let mut parser = bound(PermissionPolicy::AutoApprove);
        parser.parse_line(
            &json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": { "sessionId": "ses-1", "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "call-1",
                    "status": "in_progress",
                    "kind": "execute",
                    "title": "sleep 60",
                } },
            })
            .to_string(),
        );
        let events = parser.parse_line(&reply(4, json!({ "stopReason": "end_turn" })));
        assert!(events.contains(&AgentEvent::Done(DoneStatus::Completed)));
        assert_eq!(parser.quiet_limit(), None);
    }

    #[test]
    fn global_flags_go_before_agent_and_stdio_last() {
        assert_eq!(
            build_args(&request(PermissionPolicy::AutoApprove)),
            [
                "--no-auto-update",
                "agent",
                "--no-leader",
                "--model",
                "grok-4.6",
                "--reasoning-effort",
                "high",
                "--always-approve",
                "stdio",
            ]
        );
        let mut plan = request(PermissionPolicy::AutoApprove);
        plan.plan = true;
        plan.settings.clear();
        assert_eq!(
            build_args(&plan),
            [
                "--no-auto-update",
                "--permission-mode",
                "plan",
                "agent",
                "--no-leader",
                "--model",
                "grok-4.6",
                "stdio",
            ]
        );
    }

    #[test]
    fn session_meta_follows_the_access_mode() {
        assert_eq!(
            session_new_params("/repo", PermissionPolicy::Ask, false, &json!([])),
            json!({ "cwd": "/repo", "mcpServers": [] })
        );
        assert_eq!(
            session_new_params("/repo", PermissionPolicy::AutoApprove, false, &json!([]))["_meta"],
            json!({ "yoloMode": true })
        );
        assert_eq!(
            session_new_params("/repo", PermissionPolicy::Auto, false, &json!([]))["_meta"],
            json!({ "autoMode": true })
        );
        assert!(
            session_new_params("/repo", PermissionPolicy::AutoApprove, true, &json!([]))
                .get("_meta")
                .is_none()
        );
    }

    #[test]
    fn an_open_browser_tab_gives_the_session_its_tools() {
        let launch = crate::mcp::McpLaunch {
            name: "bencode-browser".into(),
            command: "/Applications/BenCode.app/Contents/MacOS/bencode".into(),
            args: vec!["--browser-mcp".into(), "/tmp/b.sock".into()],
        };
        let servers = mcp_servers(&[launch.clone()], None);
        assert_eq!(
            servers,
            json!([{
                "name": "bencode-browser",
                "command": "/Applications/BenCode.app/Contents/MacOS/bencode",
                "args": ["--browser-mcp", "/tmp/b.sock"],
                "env": [],
            }])
        );
        assert_eq!(
            session_new_params("/repo", PermissionPolicy::Ask, false, &servers)["mcpServers"],
            servers
        );
        assert_eq!(mcp_servers(&[], None), json!([]));
        let computer = crate::mcp::McpLaunch {
            name: crate::mcp::COMPUTER_SERVER.into(),
            args: vec!["--computer-use-mcp".into(), "/tmp/b.sock".into()],
            command: launch.command.clone(),
        };
        let servers = mcp_servers(&[launch, computer], Some("synthetic-turn"));
        assert_eq!(servers[0]["env"], json!([]));
        assert_eq!(
            servers[1]["env"],
            json!([{ "name": "BENCODE_COMPUTER_TURN", "value": "synthetic-turn" }])
        );
    }

    #[test]
    fn never_authenticates_through_the_browser() {
        let init = json!({
            "authMethods": [{ "id": "grok.com" }, { "id": "cached_token" }, { "id": "xai.api_key" }],
            "_meta": { "defaultAuthMethodId": "grok.com" },
        });
        assert_eq!(auth_method(&init).as_deref(), Some("xai.api_key"));
        let init = json!({ "authMethods": [{ "id": "cached_token" }, { "id": "grok.com" }] });
        assert_eq!(auth_method(&init).as_deref(), Some("cached_token"));
        assert_eq!(
            auth_method(&json!({ "authMethods": [{ "id": "grok.com" }] })),
            None
        );
    }

    #[test]
    fn a_turn_streams_text_thinking_tools_and_usage() {
        let mut parser = bound(PermissionPolicy::Ask);
        let update = |update: Value| {
            json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "ses-1", "update": update } })
                .to_string()
        };
        let mut events = Vec::new();
        for line in [
            update(
                json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "Hmm" } }),
            ),
            update(
                json!({ "sessionUpdate": "tool_call_delta_chunk", "tool_call_id": "call-1", "name": "read_file" }),
            ),
            update(json!({
                "sessionUpdate": "tool_call", "toolCallId": "call-1", "kind": "read",
                "title": "Read README.md", "status": "in_progress",
                "_meta": { "x.ai/tool": { "name": "read_file", "kind": "read", "input": { "path": "README.md" } } },
            })),
            update(json!({
                "sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "completed",
                "content": [{ "type": "content", "content": { "type": "text", "text": "# BenCode" } }],
            })),
            update(
                json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Hi" } }),
            ),
            update(
                json!({ "sessionUpdate": "turn_completed", "usage": { "inputTokens": 19762, "outputTokens": 36, "totalTokens": 19798 } }),
            ),
            reply(4, json!({ "stopReason": "end_turn" })),
        ] {
            events.extend(parser.parse_line(&line));
        }
        assert_eq!(
            events,
            vec![
                AgentEvent::ThinkingDelta("Hmm".into()),
                AgentEvent::ToolCallStart {
                    id: "call-1".into(),
                    name: "Read".into(),
                    input: json!({ "path": "README.md" }),
                },
                AgentEvent::ToolCallFinish {
                    id: "call-1".into(),
                    output: "# BenCode".into(),
                    success: true,
                    patch: None,
                },
                AgentEvent::TextDelta("Hi".into()),
                AgentEvent::Usage {
                    input_tokens: 19762,
                    output_tokens: 36,
                    total_tokens: 19798,
                    context_window: Some(500_000),
                },
                AgentEvent::TurnMetrics(TurnMetrics {
                    input_tokens: Some(19762),
                    output_tokens: Some(36),
                    ..Default::default()
                }),
                AgentEvent::Done(DoneStatus::Completed),
            ]
        );
    }

    /// Notifications in the shapes Grok Build 1.0.21 wrote to a session's
    /// `updates.jsonl`, their contents replaced.
    const RECORDED: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/grok_acp_turn.jsonl"
    ));

    #[test]
    fn a_recorded_turn_folds_into_the_transcripts_events() {
        let mut parser = bound(PermissionPolicy::AutoApprove);
        let mut events: Vec<_> = RECORDED
            .lines()
            .flat_map(|line| parser.parse_line(line))
            .collect();
        events.extend(parser.parse_line(&reply(4, json!({ "stopReason": "end_turn" }))));
        let context = |total_tokens| AgentEvent::Usage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens,
            context_window: Some(500_000),
        };
        let task = |text: &str, status| TaskItem {
            id: None,
            text: text.into(),
            status,
        };
        assert_eq!(
            events,
            vec![
                AgentEvent::ThinkingDelta("Searching first.".into()),
                context(34801),
                AgentEvent::ToolCallStart {
                    id: "call-1".into(),
                    name: "Grep".into(),
                    input: json!({ "pattern": "user_locations", "glob": "*.sql", "head_limit": 50 }),
                },
                AgentEvent::ToolCallFinish {
                    id: "call-1".into(),
                    output: "found 2 matches".into(),
                    success: true,
                    patch: None,
                },
                AgentEvent::ToolCallStart {
                    id: "call-2".into(),
                    name: "Read".into(),
                    input: json!({
                        "target_file": "/repo/missing.sql", "path": "/repo/missing.sql",
                        "offset": 1, "limit": 100,
                    }),
                },
                context(51006),
                AgentEvent::ToolCallFinish {
                    id: "call-2".into(),
                    output: "/repo/missing.sql does not exist".into(),
                    success: false,
                    patch: None,
                },
                // `todo_write` shows as the list, not as a tool row.
                AgentEvent::Tasks(vec![
                    task("Find the table", TaskStatus::Completed),
                    task("Write the migration", TaskStatus::InProgress),
                ]),
                AgentEvent::ToolCallStart {
                    id: "call-4".into(),
                    name: "Edit".into(),
                    input: json!({
                        "file_path": "/repo/db/001.sql", "path": "/repo/db/001.sql",
                        "old_string": "a", "new_string": "b",
                    }),
                },
                AgentEvent::ToolCallFinish {
                    id: "call-4".into(),
                    output: String::new(),
                    success: true,
                    patch: None,
                },
                AgentEvent::TextDelta("Done.".into()),
                context(60212),
                // The turn's totals add up four model calls: they are its
                // cost, not the context. Cached input is part of the input.
                AgentEvent::TurnMetrics(TurnMetrics {
                    input_tokens: Some(227724),
                    output_tokens: Some(1500),
                    cache_read_tokens: Some(170112),
                    cache_write_tokens: None,
                    cache_hit_percent: Some(170112.0 / 227724.0 * 100.0),
                }),
                AgentEvent::Done(DoneStatus::Completed),
            ]
        );
    }

    fn running_update(text: &str) -> String {
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "ses-1",
            "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } } } })
        .to_string()
    }

    #[test]
    fn a_follow_up_is_interjected_into_the_running_turn() {
        let mut parser = bound(PermissionPolicy::Ask);
        let image = Attachment {
            id: "a".into(),
            name: "a.png".into(),
            path: "/tmp/a.png".into(),
            mime_type: "image/png".into(),
            size: 3,
            data: Some("YWJj".into()),
        };
        parser.steer("use the staging table", &[image]);
        let sent = sent(&mut parser);
        assert_eq!(sent[0]["method"], "_x.ai/interject");
        assert_eq!(sent[0]["params"]["sessionId"], "ses-1");
        assert_eq!(sent[0]["params"]["text"], "use the staging table");
        assert!(sent[0]["params"]["interjectionId"].is_string());
        assert_eq!(
            sent[0]["params"]["content"],
            json!([{ "type": "image", "mimeType": "image/png", "data": "YWJj" }])
        );
        // Accepted: the turn goes on and ends as usual.
        assert!(parser.parse_line(&reply(5, json!({}))).is_empty());
        assert_eq!(
            parser.parse_line(&reply(4, json!({ "stopReason": "end_turn" }))),
            vec![AgentEvent::Done(DoneStatus::Completed)]
        );
    }

    #[test]
    fn a_refused_follow_up_becomes_the_next_prompt() {
        let mut parser = bound(PermissionPolicy::Ask);
        parser.steer("and add a test", &[]);
        sent(&mut parser);
        // The prompt returns before Grok answers for the follow-up.
        assert!(
            parser
                .parse_line(&reply(4, json!({ "stopReason": "end_turn" })))
                .is_empty()
        );
        let refused = json!({ "jsonrpc": "2.0", "id": 5, "error": { "code": METHOD_NOT_FOUND, "message": "Method not found" } });
        assert!(parser.parse_line(&refused.to_string()).is_empty());
        let next = sent(&mut parser);
        assert_eq!(next[0]["method"], "session/prompt");
        assert_eq!(next[0]["params"]["prompt"][0]["text"], "and add a test");
        // A follow-up now is not for a turn that is ending.
        assert!(parser.prompt_running());
        let mut events = parser.parse_line(&running_update("Added."));
        events.extend(parser.parse_line(&reply(6, json!({ "stopReason": "end_turn" }))));
        assert_eq!(
            events,
            vec![
                AgentEvent::TextDelta("Added.".into()),
                AgentEvent::Done(DoneStatus::Completed)
            ]
        );
    }

    #[test]
    fn a_follow_up_before_the_prompt_waits_its_turn() {
        let mut parser = parser(&request(PermissionPolicy::Ask));
        parser.initialize();
        parser.steer("one more thing", &[]);
        assert!(
            parser.take_replies().is_empty(),
            "there is no turn to interject into yet"
        );
        parser.parse_line(&reply(1, json!({})));
        sent(&mut parser);
        parser.parse_line(&reply(2, json!({ "sessionId": "ses-1" })));
        assert_eq!(sent(&mut parser)[0]["params"]["prompt"][0]["text"], "hi");
        assert!(
            parser
                .parse_line(&reply(3, json!({ "stopReason": "end_turn" })))
                .is_empty()
        );
        assert_eq!(
            sent(&mut parser)[0]["params"]["prompt"][0]["text"],
            "one more thing"
        );
    }

    #[test]
    fn compaction_resumes_the_session_and_reports_what_is_left() {
        let mut req = request(PermissionPolicy::Ask);
        req.resume_id = Some("old".into());
        req.compact = true;
        req.prompt = "/compact".into();
        let mut parser = parser(&req);
        parser.initialize();
        parser.parse_line(&reply(1, json!({})));
        assert_eq!(sent(&mut parser)[0]["method"], "session/resume");
        parser.parse_line(&reply(
            2,
            json!({ "models": { "currentModelId": "grok-4.5" } }),
        ));
        let compact = sent(&mut parser);
        assert_eq!(compact.len(), 1, "no model switch, no prompt");
        assert_eq!(compact[0]["method"], "_x.ai/compact_conversation");
        assert_eq!(compact[0]["params"], json!({ "sessionId": "old" }));

        let done = json!({ "jsonrpc": "2.0", "method": "_x.ai/session_notification", "params": { "sessionId": "old",
            "update": { "sessionUpdate": "auto_compact_completed", "tokens_before": 180000, "tokens_after": 12890, "summary_preview": "…" } } });
        assert!(parser.parse_line(&done.to_string()).is_empty());
        assert_eq!(
            parser.parse_line(&reply(3, json!({}))),
            vec![
                AgentEvent::Compacted {
                    tokens_after: Some(12890)
                },
                AgentEvent::Done(DoneStatus::Completed)
            ]
        );
    }

    #[test]
    fn compaction_needs_a_saved_session() {
        let mut req = request(PermissionPolicy::Ask);
        req.compact = true;
        let mut parser = parser(&req);
        parser.initialize();
        let events = parser.parse_line(&reply(1, json!({})));
        assert_eq!(
            events[0],
            AgentEvent::Error("Grok Build has no saved session for this chat to compact.".into())
        );
        assert_eq!(events[1], AgentEvent::Done(DoneStatus::Failed));
        assert!(
            parser.take_replies().is_empty(),
            "no session is made to compact"
        );
    }

    #[test]
    fn a_resumed_thread_resumes_then_loads_muted_then_starts_over() {
        let mut req = request(PermissionPolicy::Ask);
        req.resume_id = Some("old".into());
        let mut parser = parser(&req);
        parser.initialize();
        parser.parse_line(&reply(1, json!({})));
        let resume = sent(&mut parser);
        assert_eq!(resume[0]["method"], "session/resume");
        assert_eq!(resume[0]["params"]["sessionId"], "old");

        parser.parse_line(
            &json!({ "jsonrpc": "2.0", "id": 2, "error": { "message": "unknown" } }).to_string(),
        );
        assert_eq!(sent(&mut parser)[0]["method"], "session/load");
        let replay = json!({ "jsonrpc": "2.0", "method": "session/update", "params": {
            "update": { "sessionUpdate": "agent_message_chunk", "content": { "text": "old answer" } } } });
        assert!(
            parser.parse_line(&replay.to_string()).is_empty(),
            "history replay is muted"
        );

        parser.parse_line(
            &json!({ "jsonrpc": "2.0", "id": 3, "error": { "message": "gone" } }).to_string(),
        );
        assert_eq!(sent(&mut parser)[0]["method"], "session/new");
        let events = parser.parse_line(&reply(4, json!({ "sessionId": "new" })));
        assert_eq!(
            events,
            vec![AgentEvent::SessionStarted {
                provider_session_id: "new".into()
            }]
        );
        assert_eq!(sent(&mut parser)[0]["method"], "session/prompt");
    }

    #[test]
    fn a_resumed_session_on_another_model_is_switched() {
        let mut req = request(PermissionPolicy::Ask);
        req.resume_id = Some("old".into());
        let mut parser = parser(&req);
        parser.initialize();
        parser.parse_line(&reply(1, json!({})));
        sent(&mut parser);
        parser.parse_line(&reply(
            2,
            json!({ "models": { "currentModelId": "grok-4.5" } }),
        ));
        let set = sent(&mut parser);
        assert_eq!(set[0]["method"], "session/set_model");
        assert_eq!(
            set[0]["params"],
            json!({ "sessionId": "old", "modelId": "grok-4.6" })
        );
        parser.parse_line(&reply(3, json!({})));
        assert_eq!(sent(&mut parser)[0]["method"], "session/prompt");
    }

    #[test]
    fn supervised_permissions_ask_and_the_reply_picks_the_option() {
        let mut parser = bound(PermissionPolicy::Ask);
        let ask = json!({
            "jsonrpc": "2.0", "id": 7, "method": "session/request_permission",
            "params": {
                "toolCall": { "toolCallId": "call-2", "kind": "execute", "title": "Execute `git status`",
                              "rawInput": { "variant": "Bash", "command": "git status" } },
                "options": [{ "optionId": "allow_once" }, { "optionId": "reject_once" }],
            },
        });
        let events = parser.parse_line(&ask.to_string());
        assert!(
            parser.take_replies().is_empty(),
            "nothing is answered for the user"
        );
        let AgentEvent::PermissionRequest(request) = &events[1] else {
            panic!("expected a permission request, got {events:?}");
        };
        assert_eq!(request.tool, "Bash");
        assert_eq!(request.description, "Execute `git status`");
        let allowed: Value = serde_json::from_str(&permission_response(request, true)).unwrap();
        assert_eq!(allowed["id"], 7);
        assert_eq!(allowed["result"], selected("allow_once"));
        let denied: Value = serde_json::from_str(&permission_response(request, false)).unwrap();
        assert_eq!(denied["result"], selected("reject_once"));
    }

    #[test]
    fn access_modes_answer_by_themselves() {
        let options = ["allow-once", "allow-always", "reject-once"];
        assert_eq!(auto_option(PermissionPolicy::Ask, "edit", &options), None);
        assert_eq!(
            auto_option(PermissionPolicy::AcceptEdits, "edit", &options),
            Some("allow-once")
        );
        assert_eq!(
            auto_option(PermissionPolicy::AcceptEdits, "execute", &options),
            None
        );
        assert_eq!(
            auto_option(PermissionPolicy::AutoApprove, "execute", &options),
            Some("allow-always")
        );
        assert_eq!(
            auto_option(PermissionPolicy::Auto, "execute", &options),
            Some("allow-once")
        );

        let mut plan = request(PermissionPolicy::AutoApprove);
        plan.plan = true;
        let mut parser = parser(&plan);
        let ask = |kind: &str| {
            json!({ "jsonrpc": "2.0", "id": 9, "method": "session/request_permission", "params": {
                "toolCall": { "toolCallId": "c", "kind": kind },
                "options": [{ "optionId": "allow-once" }, { "optionId": "reject-once" }] } })
            .to_string()
        };
        parser.parse_line(&ask("edit"));
        assert_eq!(sent(&mut parser)[0]["result"], selected("reject-once"));
        parser.parse_line(&ask("read"));
        assert_eq!(sent(&mut parser)[0]["result"], selected("allow-once"));
    }

    #[test]
    fn questions_use_the_composer_form() {
        let mut parser = bound(PermissionPolicy::AutoApprove);
        let ask = json!({ "jsonrpc": "2.0", "id": 11, "method": "_x.ai/ask_user_question", "params": {
            "questions": [{ "question": "Which colour?", "options": [{ "label": "Red" }, { "label": "Blue" }] }] } });
        let events = parser.parse_line(&ask.to_string());
        let AgentEvent::PermissionRequest(request) = &events[0] else {
            panic!("expected a question, got {events:?}");
        };
        assert_eq!(request.tool, QUESTION_TOOL);
        assert_eq!(request.input["questions"][0]["question"], "Which colour?");

        let mut answered = request.clone();
        answered.input = json!({ "questions": [], "answers": { "Which colour?": "Blue" } });
        let reply: Value = serde_json::from_str(&permission_response(&answered, true)).unwrap();
        assert_eq!(reply["id"], 11);
        assert_eq!(
            reply["result"],
            json!({ "outcome": "accepted", "answers": { "Which colour?": "Blue" } })
        );
        let skipped: Value = serde_json::from_str(&permission_response(request, false)).unwrap();
        assert_eq!(skipped["result"], json!({ "outcome": "skip_interview" }));
    }

    #[test]
    fn exit_plan_mode_shows_the_plan_and_abandons_the_turn() {
        let mut parser = bound(PermissionPolicy::Ask);
        let exit = json!({ "jsonrpc": "2.0", "id": 12, "method": "_x.ai/exit_plan_mode", "params": { "planContent": " Ship it " } });
        assert_eq!(
            parser.parse_line(&exit.to_string()),
            vec![AgentEvent::TextDelta("Ship it".into())]
        );
        assert_eq!(
            sent(&mut parser)[0]["result"],
            json!({ "outcome": "abandoned" })
        );
    }

    #[test]
    fn unknown_requests_get_method_not_found() {
        let mut parser = bound(PermissionPolicy::Ask);
        parser.parse_line(
            &json!({ "jsonrpc": "2.0", "id": 13, "method": "fs/read_text_file", "params": {} })
                .to_string(),
        );
        assert_eq!(sent(&mut parser)[0]["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn a_failed_start_explains_the_sign_in() {
        let mut parser = parser(&request(PermissionPolicy::Ask));
        parser.initialize();
        parser.parse_line(&reply(1, json!({})));
        sent(&mut parser);
        let events = parser.parse_line(
            &json!({ "jsonrpc": "2.0", "id": 2, "error": { "message": "Authentication required" } }).to_string(),
        );
        let AgentEvent::Error(message) = &events[0] else {
            panic!("expected an error, got {events:?}");
        };
        assert!(message.contains("grok login"), "{message}");
        assert_eq!(events[1], AgentEvent::Done(DoneStatus::Failed));
    }

    #[test]
    fn catalogs_come_from_acp_and_the_cli() {
        let models = models_from_acp(&json!({ "_meta": { "modelState": {
            "currentModelId": "grok-4.6",
            "availableModels": [{
                "modelId": "grok-4.6", "name": "Grok 4.6",
                "_meta": {
                    "totalContextTokens": 500000, "reasoningEffort": "high",
                    "reasoningEfforts": [
                        { "id": "xhigh", "value": "xhigh", "label": "Extra High Effort" },
                        { "id": "high", "value": "high", "label": "High Effort", "default": true },
                        { "id": "deep", "value": "deep", "label": "Deep Effort" },
                    ],
                },
            }],
        } } }));
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].key, "grok:grok-4.6");
        assert_eq!(models[0].settings[0].default, "high");
        assert_eq!(
            models[0].settings[0].options,
            [
                ("xhigh".to_string(), "Extra High".to_string()),
                ("high".to_string(), "High".to_string()),
                ("deep".to_string(), "Deep".to_string()),
            ]
        );
        let new = models_from_acp(
            &json!({ "models": { "availableModels": [{ "modelId": "grok-4.5" }] } }),
        );
        assert_eq!(new[0].label, "Grok 4.5");

        let cli = models_from_cli(
            "You are not authenticated.\n\nDefault model: grok-4.6\n\nAvailable models:\n  \u{1b}[1m* grok-4.6\u{1b}[0m (default)\n  - grok-4.5\n",
        );
        assert_eq!(
            cli.iter().map(|m| m.native.as_str()).collect::<Vec<_>>(),
            ["grok-4.6", "grok-4.5"]
        );
    }

    #[test]
    fn context_window_follows_the_current_model() {
        let setup = json!({ "models": { "currentModelId": "b", "availableModels": [
            { "modelId": "a", "_meta": { "totalContextTokens": 1 } },
            { "modelId": "b", "_meta": { "totalContextTokens": 2 } },
        ] } });
        assert_eq!(context_window(&setup), Some(2));
        assert_eq!(context_window(&json!({})), None);
    }

    #[test]
    fn only_xais_grok_counts() {
        let dir = std::env::temp_dir().join(format!("bencode-grok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("grok");
        std::fs::write(&other, "#!/bin/sh\necho some other grok\n").unwrap();
        assert!(!is_grok_build(&other));
        std::fs::write(&other, "#!/bin/sh\n# Grok Build\n").unwrap();
        assert!(is_grok_build(&other));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(is_grok_build(Path::new("/Users/x/.grok/bin/grok")));
    }
}
