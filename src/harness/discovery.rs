//! Live model catalogs read from each CLI, as MonoCode's `*Catalog.ts`
//! `discover*Models` do:
//! - Claude: `list_models` control request over stream-json;
//! - Codex: `model/list` on `codex app-server`;
//! - Antigravity: `agy models` (MonoCode asks its ACP server, which BenCode
//!   does not run; the CLI lists the same models);
//! - OpenCode: `opencode models --verbose` and `opencode agent list`.
//!
//! The parsers are pure; `discover` blocks on a child process and must run
//! on a background executor.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::harness::HarnessKind;
use crate::harness::catalog::{self, ModelOption, ModelSetting, SettingKind};
use crate::harness::probe::{AppServer, LineProbe, run_to_end};
use crate::harness::resolver::HarnessResolver;

/// MonoCode `DISCOVERY_TIMEOUT_MS`.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);

/// Asks `harness`' CLI for its models. Blocking.
pub fn discover(harness: HarnessKind) -> Result<Vec<ModelOption>> {
    match harness {
        HarnessKind::Claude => discover_claude(),
        HarnessKind::Codex => discover_codex(),
        HarnessKind::Antigravity => discover_antigravity(),
        HarnessKind::OpenCode => discover_opencode(),
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

fn str_field<'a>(rec: &'a Value, key: &str) -> Option<&'a str> {
    rec.get(key)?.as_str().filter(|s| !s.is_empty())
}

// ---- Claude ----------------------------------------------------------------

const CLAUDE_INIT_ID: &str = "monocode_init";
const CLAUDE_LIST_ID: &str = "monocode_list_models";

fn discover_claude() -> Result<Vec<ModelOption>> {
    let program = HarnessResolver::resolve_claude().context("Claude Code is not installed")?;
    let mut probe = LineProbe::spawn(
        Command::new(program).args([
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
            "--settings",
            r#"{"disableAllHooks":true}"#,
        ]),
        &home(),
        DISCOVERY_TIMEOUT,
    )?;
    let request = |id: &str, subtype: &str| json!({ "type": "control_request", "request_id": id, "request": { "subtype": subtype } });
    probe.send(&request(CLAUDE_INIT_ID, "initialize"))?;
    let mut asked = false;
    loop {
        let rec = probe.next_json()?;
        let kind = str_field(&rec, "type");
        let response = rec.get("response");
        let response_id = response.and_then(|r| str_field(r, "request_id"));
        let initialized = (kind == Some("system")
            && matches!(str_field(&rec, "subtype"), Some("init" | "initialized")))
            || (kind == Some("control_response") && response_id == Some(CLAUDE_INIT_ID));
        if initialized && !asked {
            asked = true;
            probe.send(&request(CLAUDE_LIST_ID, "list_models"))?;
        }
        if kind == Some("control_response") && response_id == Some(CLAUDE_LIST_ID) {
            let payload = response
                .and_then(|r| r.get("response"))
                .unwrap_or(&Value::Null);
            return Ok(claude_models(payload));
        }
    }
}

/// MonoCode `modelsFromClaudeListModels`.
pub fn claude_models(payload: &Value) -> Vec<ModelOption> {
    let rows = payload
        .as_array()
        .or_else(|| payload.get("models")?.as_array())
        .cloned()
        .unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    rows.iter()
        .filter_map(claude_model)
        .filter(|m| seen.insert(m.native.clone()))
        .collect()
}

fn claude_model(rec: &Value) -> Option<ModelOption> {
    if rec.get("disabled") == Some(&Value::Bool(true)) {
        return None;
    }
    let value = str_field(rec, "value")?;
    if value == "default" || value.starts_with("cc-update-required") {
        return None;
    }
    let resolved = str_field(rec, "resolvedModel").unwrap_or("");
    let (value_id, value_1m) = split_claude_value(value);
    let (resolved_id, resolved_1m) = split_claude_value(resolved);
    let native = claude_launch_id(value_id, resolved_id);
    if native.is_empty() {
        return None;
    }
    let name = claude_picker_name(
        str_field(rec, "displayName").unwrap_or(""),
        str_field(rec, "description").unwrap_or(""),
        &native,
        resolved_id,
    );
    let levels: Vec<String> = rec
        .get("supportedEffortLevels")
        .and_then(Value::as_array)
        .map(|levels| {
            levels
                .iter()
                .filter_map(|l| l.as_str().map(str::trim).filter(|l| !l.is_empty()))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let flag = |key: &str| rec.get(key) == Some(&Value::Bool(true));
    let mut settings = Vec::new();
    if flag("supportsEffort") || !levels.is_empty() {
        settings.push(catalog::claude_effort(&levels));
    } else if flag("supportsAdaptiveThinking") {
        settings.push(ModelSetting::toggle("thinking", "Thinking"));
    }
    if flag("supportsFastMode") {
        settings.push(ModelSetting::toggle("fast", "Fast"));
    }
    if value_1m || resolved_1m {
        settings.push(catalog::context_window("1m"));
    }
    let slug = native.strip_prefix("claude-").unwrap_or(&native);
    Some(ModelOption {
        key: format!("claude:{slug}"),
        label: name,
        harness: HarnessKind::Claude,
        native: native.clone(),
        provider: None,
        settings,
    })
}

/// `opus[1m]` -> (`opus`, true).
fn split_claude_value(value: &str) -> (&str, bool) {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    match lower.strip_suffix("[1m]") {
        Some(id) if !id.trim().is_empty() => (value[..id.len()].trim(), true),
        _ => (value, false),
    }
}

/// MonoCode `claudeLaunchId`: aliases stay bare, versioned ids get `claude-`.
fn claude_launch_id(value_id: &str, resolved_id: &str) -> String {
    let native = if value_id.is_empty() {
        resolved_id
    } else {
        value_id
    };
    if native.is_empty()
        || native.starts_with("claude-")
        || !native.contains(|c: char| c.is_ascii_digit())
    {
        return native.to_string();
    }
    if resolved_id.starts_with("claude-") {
        resolved_id.to_string()
    } else {
        format!("claude-{native}")
    }
}

/// MonoCode `pickerName` + `qualifyClaudeAliasName`.
fn claude_picker_name(display: &str, description: &str, fallback: &str, resolved: &str) -> String {
    let name = display.trim();
    let head = description.split('·').next().unwrap_or("").trim();
    let mut picked = if !name.is_empty() {
        name
    } else if !head.is_empty() {
        head
    } else {
        fallback
    };
    if !head.is_empty()
        && !name.is_empty()
        && head.to_lowercase().starts_with(&name.to_lowercase())
        && head.len() > name.len()
    {
        picked = head;
    }
    qualify_alias_name(picked, resolved)
}

fn is_version_part(part: &str) -> bool {
    !part.is_empty()
        && part
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && !(part.len() == 8 && part.bytes().all(|b| b.is_ascii_digit()))
}

/// `claude-opus-5-5` -> ("Opus", "5.5").
fn resolved_family_version(model: &str) -> Option<(String, String)> {
    let (id, _) = split_claude_value(model);
    let rest = id.to_lowercase().strip_prefix("claude-")?.to_string();
    let parts: Vec<&str> = rest.split('-').collect();
    let start = parts.iter().position(|p| is_version_part(p))?;
    if start == 0 {
        return None;
    }
    let version: Vec<&str> = parts[start..]
        .iter()
        .take_while(|p| is_version_part(p))
        .flat_map(|p| p.split('.'))
        .collect();
    let family = parts[..start]
        .iter()
        .map(|p| {
            let mut chars = p.chars();
            chars.next().map_or_else(String::new, |c| {
                c.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    Some((family, version.join(".")))
}

/// A bare alias name ("Opus") gains the version it resolves to ("Opus 5.5").
fn qualify_alias_name(name: &str, resolved: &str) -> String {
    let Some((family, version)) = resolved_family_version(resolved) else {
        return name.to_string();
    };
    let lower = name.to_lowercase();
    let prefix_len = if lower.starts_with("claude ") {
        name.len() - name[7..].trim_start().len()
    } else {
        0
    };
    if !lower[prefix_len..].starts_with(&family.to_lowercase()) {
        return name.to_string();
    }
    let split = prefix_len + family.len();
    let suffix = &name[split..];
    let trimmed = suffix.trim_start();
    let versioned = suffix.len() != trimmed.len()
        && trimmed
            .trim_start_matches(['v', 'V'])
            .starts_with(|c: char| c.is_ascii_digit());
    if versioned {
        return name.to_string();
    }
    format!("{} {version}{suffix}", &name[..split])
}

// ---- Codex -----------------------------------------------------------------

fn discover_codex() -> Result<Vec<ModelOption>> {
    let program = HarnessResolver::resolve_codex().context("Codex is not installed")?;
    let mut server = AppServer::open(&program, &home(), DISCOVERY_TIMEOUT, None)?;
    let mut rows = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let params = cursor
            .as_ref()
            .map_or(json!({}), |c| json!({ "cursor": c }));
        let page = server.call("model/list", params)?;
        rows.extend(
            page.get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        cursor = str_field(&page, "nextCursor").map(String::from);
        if cursor.is_none() {
            break;
        }
    }
    Ok(codex_models(&rows))
}

/// MonoCode `parseCodexModelList`.
pub fn codex_models(rows: &[Value]) -> Vec<ModelOption> {
    let mut seen = std::collections::HashSet::new();
    let mut models: Vec<ModelOption> = rows
        .iter()
        .filter_map(codex_model)
        .filter(|m| seen.insert(m.native.clone()))
        .collect();
    let default = rows
        .iter()
        .find(|r| r.get("isDefault") == Some(&Value::Bool(true)))
        .and_then(codex_native);
    if let Some(ix) = default.and_then(|d| models.iter().position(|m| m.native == d))
        && ix > 0
    {
        let model = models.remove(ix);
        models.insert(0, model);
    }
    models
}

fn codex_native(rec: &Value) -> Option<&str> {
    str_field(rec, "model")
        .or_else(|| str_field(rec, "slug"))
        .or_else(|| str_field(rec, "id"))
}

fn codex_model(rec: &Value) -> Option<ModelOption> {
    if rec.get("hidden") == Some(&Value::Bool(true)) {
        return None;
    }
    let native = codex_native(rec)?.to_string();
    let raw_name = str_field(rec, "displayName")
        .or_else(|| str_field(rec, "name"))
        .unwrap_or(&native);
    Some(ModelOption {
        key: format!("codex:{native}"),
        label: codex_display_name(raw_name),
        harness: HarnessKind::Codex,
        native,
        provider: None,
        settings: codex_settings(rec),
    })
}

/// MonoCode `formatDisplayName`: `gpt-5-codex` -> `GPT-5-Codex`.
fn codex_display_name(name: &str) -> String {
    let name = match name.get(..3) {
        Some(head) if head.eq_ignore_ascii_case("gpt") => format!("GPT{}", &name[3..]),
        _ => name.to_string(),
    };
    let mut out = String::with_capacity(name.len());
    let mut after_dash = false;
    for c in name.chars() {
        out.extend(if after_dash && c.is_ascii_lowercase() {
            Some(c.to_ascii_uppercase())
        } else {
            Some(c)
        });
        after_dash = c == '-';
    }
    out
}

/// MonoCode `parseModelSettings`: reasoning effort and service tier.
fn codex_settings(rec: &Value) -> Vec<ModelSetting> {
    let mut settings = Vec::new();
    let efforts: Vec<(String, String)> = rec
        .get("supportedReasoningEfforts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let value = entry.as_str().or_else(|| {
                str_field(entry, "reasoningEffort").or_else(|| str_field(entry, "id"))
            })?;
            let label = catalog::effort_label(value)
                .map(String::from)
                .or_else(|| str_field(entry, "label").map(String::from))
                .unwrap_or_else(|| value.to_string());
            Some((value.to_string(), label))
        })
        .collect();
    if let Some((first, _)) = efforts.first() {
        let default = str_field(rec, "defaultReasoningEffort").unwrap_or(first);
        settings.push(ModelSetting {
            id: "reasoningEffort".into(),
            label: "Reasoning".into(),
            kind: SettingKind::Select,
            default: default.to_string(),
            options: efforts.clone(),
        });
    }
    let tiers = rec
        .get("serviceTiers")
        .and_then(Value::as_array)
        .filter(|t| !t.is_empty())
        .or_else(|| rec.get("additionalSpeedTiers").and_then(Value::as_array));
    let mut options = vec![("default".to_string(), "Standard".to_string())];
    for entry in tiers.into_iter().flatten() {
        let (value, label) = match entry.as_str() {
            Some(id) => (id, if id == "fast" { "Fast" } else { id }),
            None => {
                let Some(id) = str_field(entry, "id") else {
                    continue;
                };
                (id, str_field(entry, "name").unwrap_or(id))
            }
        };
        if value != "default" {
            options.push((value.to_string(), label.to_string()));
        }
    }
    if options.len() > 1 {
        let wanted = str_field(rec, "defaultServiceTier").unwrap_or("default");
        let default = if options.iter().any(|(v, _)| v == wanted) {
            wanted
        } else {
            "default"
        };
        settings.push(ModelSetting {
            id: "serviceTier".into(),
            label: "Service Tier".into(),
            kind: SettingKind::Select,
            default: default.to_string(),
            options,
        });
    }
    settings
}

// ---- Antigravity -----------------------------------------------------------

fn discover_antigravity() -> Result<Vec<ModelOption>> {
    let program =
        HarnessResolver::resolve_antigravity_cli().context("Antigravity is not installed")?;
    let out = run_to_end(
        Command::new(program).arg("models"),
        &home(),
        DISCOVERY_TIMEOUT,
    )?;
    Ok(antigravity_models(&out))
}

/// `agy models`: `id<TAB>name` per model, after a "Fetching…" line.
pub fn antigravity_models(stdout: &str) -> Vec<ModelOption> {
    let mut seen = std::collections::HashSet::new();
    stdout
        .lines()
        .filter_map(|line| {
            let (id, name) = line.split_once('\t')?;
            let (id, name) = (id.trim(), name.trim());
            (!id.is_empty() && !id.contains(char::is_whitespace)).then_some((id, name))
        })
        .filter(|(id, _)| seen.insert(id.to_string()))
        .map(|(id, name)| ModelOption {
            key: format!("antigravity:{id}"),
            label: if name.is_empty() { id } else { name }.to_string(),
            harness: HarnessKind::Antigravity,
            native: id.to_string(),
            provider: None,
            settings: Vec::new(),
        })
        .collect()
}

// ---- OpenCode --------------------------------------------------------------

/// MonoCode `KNOWN_HIDDEN_AGENTS`.
const HIDDEN_AGENTS: [&str; 3] = ["compaction", "summary", "title"];
const VARIANT_ORDER: [&str; 9] = [
    "none",
    "minimal",
    "low",
    "medium",
    "high",
    "xhigh",
    "extra-high",
    "max",
    "ultra",
];

fn discover_opencode() -> Result<Vec<ModelOption>> {
    let program = HarnessResolver::resolve_opencode().context("OpenCode is not installed")?;
    let models = run_to_end(
        Command::new(&program).args(["models", "--verbose"]),
        &home(),
        DISCOVERY_TIMEOUT,
    )?;
    let agents = match run_to_end(
        Command::new(&program).args(["agent", "list"]),
        &home(),
        DISCOVERY_TIMEOUT,
    ) {
        Ok(out) => opencode_agents(&out),
        Err(err) => {
            log::debug!("opencode agents: {err:#}");
            Vec::new()
        }
    };
    Ok(opencode_models(&models, &agents))
}

/// MonoCode `parseAgentListCliOutput`: `name (mode)` headers; the primary,
/// visible ones.
pub fn opencode_agents(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim_end();
            let open = line.rfind(" (")?;
            let mode = line[open + 2..].strip_suffix(')')?;
            let name = line[..open].trim();
            (!name.is_empty() && !mode.contains(char::is_whitespace)).then_some((name, mode))
        })
        .filter(|(name, mode)| !HIDDEN_AGENTS.contains(name) && matches!(*mode, "primary" | "all"))
        .map(|(name, _)| name.to_string())
        .collect()
}

/// MonoCode `parseModelsCliOutput` + `flattenOpenCodeModels`: a
/// `provider/model` line, then that model's JSON.
pub fn opencode_models(stdout: &str, agents: &[String]) -> Vec<ModelOption> {
    let mut blocks: Vec<(String, String)> = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        let is_slug = !line.trim_start().starts_with('{')
            && !trimmed.is_empty()
            && !trimmed.contains(char::is_whitespace)
            && trimmed
                .split_once('/')
                .is_some_and(|(p, m)| !p.is_empty() && !m.is_empty());
        if is_slug {
            blocks.push((trimmed.to_string(), String::new()));
        } else if let Some((_, json)) = blocks.last_mut() {
            json.push_str(line);
            json.push('\n');
        }
    }
    let mut models: Vec<ModelOption> = blocks
        .into_iter()
        .filter_map(|(slug, json)| {
            let model: Value = serde_json::from_str(json.trim()).ok()?;
            let (provider, model_id) = slug.split_once('/')?;
            let id = str_field(&model, "id").unwrap_or(model_id);
            let native = format!("{provider}/{id}");
            let label = str_field(&model, "name")
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map_or_else(|| title_case_slug(model_id), String::from);
            Some(ModelOption {
                key: format!("opencode:{native}"),
                label,
                harness: HarnessKind::OpenCode,
                native,
                provider: Some(opencode_provider_name(provider)),
                settings: opencode_settings(provider, &model, agents),
            })
        })
        .collect();
    models.sort_by(|a, b| a.label.cmp(&b.label));
    models
}

fn opencode_provider_name(id: &str) -> String {
    match id {
        "opencode" => "OpenCode".into(),
        "opencode-go" => "OpenCode Go".into(),
        "openai" => "OpenAI".into(),
        "xai" => "xAI".into(),
        "github-copilot" => "GitHub Copilot".into(),
        _ => title_case_slug(id),
    }
}

fn title_case_slug(value: &str) -> String {
    value
        .split(['-', '_', '/'])
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut chars = s.chars();
            chars.next().map_or_else(String::new, |c| {
                c.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn opencode_settings(provider: &str, model: &Value, agents: &[String]) -> Vec<ModelSetting> {
    let mut settings = Vec::new();
    let mut variants: Vec<String> = model
        .get("variants")
        .and_then(Value::as_object)
        .map(|v| v.keys().cloned().collect())
        .unwrap_or_default();
    let rank = |v: &str| {
        VARIANT_ORDER
            .iter()
            .position(|o| *o == v.to_lowercase())
            .unwrap_or(usize::MAX)
    };
    variants.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
    if let Some(first) = variants.first() {
        // MonoCode `inferDefaultVariant`.
        let has = |v: &str| variants.iter().any(|x| x == v);
        let default = if variants.len() == 1 {
            Some(first.as_str())
        } else if provider == "anthropic" || provider.starts_with("google") {
            has("high").then_some("high")
        } else if has("medium") {
            Some("medium")
        } else {
            has("high").then_some("high")
        };
        settings.push(ModelSetting {
            id: "variant".into(),
            label: "Variant".into(),
            kind: SettingKind::Select,
            default: default.unwrap_or(first).to_string(),
            options: variants
                .iter()
                .map(|v| {
                    let label = catalog::effort_label(&v.to_lowercase())
                        .map_or_else(|| title_case_slug(v), String::from);
                    (v.clone(), label)
                })
                .collect(),
        });
    }
    if let Some(first) = agents.first() {
        let default = agents.iter().find(|a| *a == "build").unwrap_or(first);
        settings.push(ModelSetting {
            id: "agent".into(),
            label: "Agent".into(),
            kind: SettingKind::Select,
            default: default.clone(),
            options: agents
                .iter()
                .map(|a| (a.clone(), title_case_slug(a)))
                .collect(),
        });
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_fixture() -> Vec<ModelOption> {
        let line = include_str!("../../tests/fixtures/claude_list_models.jsonl");
        let rec: Value = serde_json::from_str(line.trim()).unwrap();
        claude_models(&rec["response"]["response"])
    }

    #[test]
    fn claude_list_models_becomes_the_catalog() {
        let models = claude_fixture();
        let keys: Vec<_> = models.iter().map(|m| m.key.as_str()).collect();
        assert!(!keys.contains(&"claude:default"));
        assert_eq!(
            &keys[..4],
            [
                "claude:opus",
                "claude:sonnet",
                "claude:fable",
                "claude:haiku"
            ]
        );
        let opus = &models[0];
        assert_eq!(opus.native, "opus");
        assert_eq!(opus.label, "Opus 5.5");
        let ids: Vec<_> = opus.settings.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["effort", "fast"]);
        let haiku = models.iter().find(|m| m.key == "claude:haiku").unwrap();
        assert!(haiku.settings.is_empty());
        let old = models.iter().find(|m| m.key == "claude:opus-4-6").unwrap();
        assert_eq!(old.native, "claude-opus-4-6");
        assert!(
            !old.settings[0]
                .options
                .iter()
                .any(|(v, _)| v == "ultracode")
        );
    }

    #[test]
    fn claude_alias_names_gain_their_version() {
        assert_eq!(qualify_alias_name("Opus", "claude-opus-5-5"), "Opus 5.5");
        assert_eq!(
            qualify_alias_name("Claude Opus", "claude-opus-5-5"),
            "Claude Opus 5.5"
        );
        assert_eq!(
            qualify_alias_name("Haiku 4.5", "claude-haiku-4-5-20251001"),
            "Haiku 4.5"
        );
        assert_eq!(
            qualify_alias_name("Sonnet (1M)", "claude-sonnet-5[1m]"),
            "Sonnet 5 (1M)"
        );
        assert_eq!(claude_launch_id("opus-5-5", ""), "claude-opus-5-5");
        assert_eq!(split_claude_value("opus[1m]"), ("opus", true));
    }

    #[test]
    fn codex_model_list_rows() {
        let rows = vec![
            json!({ "model": "gpt-5", "displayName": "gpt-5",
                    "supportedReasoningEfforts": [{ "reasoningEffort": "low" }, { "reasoningEffort": "high" }],
                    "defaultReasoningEffort": "high" }),
            json!({ "model": "gpt-5-codex", "isDefault": true, "serviceTiers": ["fast"] }),
            json!({ "model": "secret", "hidden": true }),
        ];
        let models = codex_models(&rows);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].key, "codex:gpt-5-codex");
        assert_eq!(models[0].label, "GPT-5-Codex");
        assert_eq!(models[0].settings[0].id, "serviceTier");
        assert_eq!(models[1].label, "GPT-5");
        assert_eq!(models[1].settings[0].default, "high");
        assert_eq!(models[1].settings[0].options[1].1, "High");
    }

    #[test]
    fn agy_models_output() {
        let out = "Fetching available models...\ngemini-3.8-flash-high\tGemini 3.8 Flash (High)\nclaude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)\n";
        let models = antigravity_models(out);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].key, "antigravity:gemini-3.8-flash-high");
        assert_eq!(models[1].label, "Claude Sonnet 4.6 (Thinking)");
    }

    #[test]
    fn opencode_models_and_agents() {
        let out = "anthropic/claude-sonnet-4\n{\n  \"id\": \"claude-sonnet-4\",\n  \"name\": \"Claude Sonnet 4\",\n  \"variants\": { \"high\": {}, \"low\": {}, \"max\": {} }\n}\nopencode/glm-5\n{ \"name\": \"GLM 5\" }\n";
        let agents = opencode_agents(
            "build (primary)\n  tools\nplan (primary)\ntitle (primary)\ngeneral (subagent)\n",
        );
        assert_eq!(agents, ["build", "plan"]);
        let models = opencode_models(out, &agents);
        assert_eq!(models[0].label, "Claude Sonnet 4");
        assert_eq!(models[0].native, "anthropic/claude-sonnet-4");
        let variant = &models[0].settings[0];
        assert_eq!(variant.default, "high");
        let values: Vec<_> = variant.options.iter().map(|(v, _)| v.as_str()).collect();
        assert_eq!(values, ["low", "high", "max"]);
        assert_eq!(models[1].provider.as_deref(), Some("OpenCode"));
        assert_eq!(models[1].settings[0].id, "agent");
    }
}
