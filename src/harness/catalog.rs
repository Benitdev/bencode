//! Selectable models. Keys use MonoCode's `harness:model` format (e.g.
//! `claude:opus`) so sessions stay interchangeable with MonoCode's DB.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::harness::HarnessKind;
use crate::harness::resolver::HarnessInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelOption {
    pub key: &'static str,
    pub label: &'static str,
    pub harness: HarnessKind,
    /// MonoCode `AgentModel.settings`: the options shown in the model menu.
    pub settings: &'static [ModelSetting],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    Select,
    Toggle,
}

/// MonoCode `ModelSetting`: stored in the thread's `modelSettings` under `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSetting {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: SettingKind,
    pub default: &'static str,
    /// `(value, label)` pairs.
    pub options: &'static [(&'static str, &'static str)],
}

impl ModelSetting {
    /// MonoCode `settingLabel`: effort-like selects read "Effort".
    pub fn menu_label(&self) -> &'static str {
        if matches!(self.id, "effort" | "reasoning") {
            "Effort"
        } else {
            self.label
        }
    }

    /// The thread's value, else the model's default.
    pub fn value<'a>(&'a self, values: Option<&'a Map<String, Value>>) -> &'a str {
        values
            .and_then(|v| v.get(self.id))
            .and_then(Value::as_str)
            .unwrap_or(self.default)
    }

    pub fn value_label<'a>(&'a self, values: Option<&'a Map<String, Value>>) -> &'a str {
        let value = self.value(values);
        self.options
            .iter()
            .find(|(v, _)| *v == value)
            .map_or(value, |(_, label)| label)
    }

    pub fn is_effort(&self) -> bool {
        self.kind == SettingKind::Select
            && matches!(self.id, "effort" | "reasoning" | "reasoningEffort")
    }
}

const ON_OFF: &[(&str, &str)] = &[("true", "On"), ("false", "Off")];

/// MonoCode `EFFORT_WITH_XHIGH`.
const CLAUDE_EFFORT: ModelSetting = ModelSetting {
    id: "effort",
    label: "Reasoning",
    kind: SettingKind::Select,
    default: "high",
    options: &[
        ("low", "Low"),
        ("medium", "Medium"),
        ("high", "High"),
        ("xhigh", "Extra High"),
        ("max", "Max"),
        ("ultracode", "Ultracode"),
        ("ultrathink", "Ultrathink"),
    ],
};

const FAST_MODE: ModelSetting = ModelSetting {
    id: "fast",
    label: "Fast",
    kind: SettingKind::Toggle,
    default: "false",
    options: ON_OFF,
};

const THINKING: ModelSetting = ModelSetting {
    id: "thinking",
    label: "Thinking",
    kind: SettingKind::Toggle,
    default: "false",
    options: ON_OFF,
};

const fn context_window(default: &'static str) -> ModelSetting {
    ModelSetting {
        id: "context",
        label: "Context",
        kind: SettingKind::Select,
        default,
        options: &[("200k", "200k"), ("1m", "1M")],
    }
}

/// Codex `supportedReasoningEfforts` for the GPT-5 family.
const CODEX_EFFORT: ModelSetting = ModelSetting {
    id: "reasoningEffort",
    label: "Reasoning",
    kind: SettingKind::Select,
    default: "medium",
    options: &[
        ("minimal", "Minimal"),
        ("low", "Low"),
        ("medium", "Medium"),
        ("high", "High"),
    ],
};

const fn model(key: &'static str, label: &'static str, harness: HarnessKind) -> ModelOption {
    with_settings(key, label, harness, &[])
}

const fn with_settings(
    key: &'static str,
    label: &'static str,
    harness: HarnessKind,
    settings: &'static [ModelSetting],
) -> ModelOption {
    ModelOption {
        key,
        label,
        harness,
        settings,
    }
}

pub const MODELS: &[ModelOption] = &[
    with_settings(
        "claude:opus",
        "Claude Opus",
        HarnessKind::Claude,
        &[CLAUDE_EFFORT, FAST_MODE, context_window("1m")],
    ),
    with_settings(
        "claude:sonnet",
        "Claude Sonnet",
        HarnessKind::Claude,
        &[CLAUDE_EFFORT, context_window("200k")],
    ),
    with_settings(
        "claude:haiku",
        "Claude Haiku",
        HarnessKind::Claude,
        &[THINKING],
    ),
    model(
        "antigravity:gemini-3.8-flash-high",
        "Gemini 3.8 Flash (High)",
        HarnessKind::Antigravity,
    ),
    model(
        "antigravity:gemini-3.1-pro-high",
        "Gemini 3.1 Pro (High)",
        HarnessKind::Antigravity,
    ),
    model(
        "antigravity:claude-sonnet-4-6",
        "Claude Sonnet 4.6 (Antigravity)",
        HarnessKind::Antigravity,
    ),
    with_settings(
        "codex:gpt-5-codex",
        "GPT-5 Codex",
        HarnessKind::Codex,
        &[CODEX_EFFORT],
    ),
    with_settings("codex:gpt-5", "GPT-5", HarnessKind::Codex, &[CODEX_EFFORT]),
    model(
        "opencode:default",
        "OpenCode (default model)",
        HarnessKind::OpenCode,
    ),
];

/// MonoCode `SETTING_ORDER`: the order rows take in the model menu.
const SETTING_ORDER: [&str; 9] = [
    "fast",
    "effort",
    "reasoning",
    "reasoningEffort",
    "serviceTier",
    "thinking",
    "variant",
    "agent",
    "context",
];

/// MonoCode `pickerSettings`: the model's settings in menu order.
pub fn picker_settings(key: &str) -> Vec<&'static ModelSetting> {
    let mut settings: Vec<_> = find(key).map_or(&[][..], |m| m.settings).iter().collect();
    settings.sort_by_key(|s| {
        SETTING_ORDER
            .iter()
            .position(|id| *id == s.id)
            .unwrap_or(99)
    });
    settings
}

/// MonoCode `effortSetting`: the reasoning select the chip names.
pub fn effort_setting(key: &str) -> Option<&'static ModelSetting> {
    find(key)?.settings.iter().find(|s| s.is_effort())
}

/// The thread's settings as plain strings, defaults filled in, for the CLI.
pub fn resolved_settings(
    key: &str,
    values: Option<&Map<String, Value>>,
) -> BTreeMap<String, String> {
    find(key)
        .map_or(&[][..], |m| m.settings)
        .iter()
        .map(|s| (s.id.to_string(), s.value(values).to_string()))
        .collect()
}

pub fn find(key: &str) -> Option<&'static ModelOption> {
    MODELS.iter().find(|m| m.key == key)
}

pub fn models_for(harness: HarnessKind) -> impl Iterator<Item = &'static ModelOption> {
    MODELS.iter().filter(move |m| m.harness == harness)
}

/// Human label for a stored model key, falling back to the raw id.
pub fn label_for(key: &str) -> String {
    match find(key) {
        Some(option) => option.label.to_string(),
        None => key.split_once(':').map_or(key, |(_, id)| id).to_string(),
    }
}

/// The value to pass as `--model`, or None to let the CLI pick its default.
/// Legacy BenCode sessions stored display names ("Claude 3.7 Sonnet"); those
/// are not valid CLI ids and are dropped rather than passed through.
pub fn cli_model_id(key: &str) -> Option<String> {
    let id = key.split_once(':').map_or(key, |(_, id)| id).trim();
    let unusable = id.is_empty() || id == "default" || id.contains(char::is_whitespace);
    (!unusable).then(|| id.to_string())
}

/// First model whose harness is installed; Claude Opus if nothing is.
pub fn default_model(installed: &[HarnessInfo]) -> &'static ModelOption {
    MODELS
        .iter()
        .find(|m| {
            installed
                .iter()
                .any(|h| h.available && h.id == m.harness.id())
        })
        .unwrap_or(&MODELS[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_model_id_strips_harness_prefix() {
        assert_eq!(cli_model_id("claude:opus").as_deref(), Some("opus"));
        assert_eq!(
            cli_model_id("antigravity:gemini-3.8-flash-high").as_deref(),
            Some("gemini-3.8-flash-high")
        );
        assert_eq!(cli_model_id("sonnet").as_deref(), Some("sonnet"));
    }

    #[test]
    fn cli_model_id_drops_defaults_and_display_names() {
        assert_eq!(cli_model_id("opencode:default"), None);
        assert_eq!(cli_model_id(""), None);
        assert_eq!(cli_model_id("Claude 3.7 Sonnet"), None);
    }

    #[test]
    fn every_key_is_prefixed_with_its_harness() {
        for option in MODELS {
            assert!(
                option.key.starts_with(&format!("{}:", option.harness.id())),
                "{}",
                option.key
            );
        }
    }

    #[test]
    fn settings_follow_monocode_menu_order_and_defaults() {
        let ids: Vec<_> = picker_settings("claude:opus")
            .iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, ["fast", "effort", "context"]);
        assert_eq!(
            effort_setting("claude:opus").unwrap().menu_label(),
            "Effort"
        );
        assert!(effort_setting("claude:haiku").is_none());

        let mut values = Map::new();
        values.insert("effort".into(), "xhigh".into());
        let effort = effort_setting("claude:sonnet").unwrap();
        assert_eq!(effort.value_label(Some(&values)), "Extra High");
        let resolved = resolved_settings("claude:sonnet", Some(&values));
        assert_eq!(resolved["effort"], "xhigh");
        assert_eq!(resolved["context"], "200k");
        assert!(resolved_settings("opencode:default", None).is_empty());
    }

    #[test]
    fn default_model_prefers_installed_harness() {
        let installed = [HarnessInfo {
            id: "codex",
            name: "Codex",
            binary_path: None,
            available: true,
        }];
        assert_eq!(default_model(&installed).harness, HarnessKind::Codex);
        assert_eq!(default_model(&[]).key, "claude:opus");
    }
}
