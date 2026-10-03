//! Selectable models. Keys use MonoCode's `harness:model` format (e.g.
//! `claude:opus`) so sessions stay interchangeable with MonoCode's DB.
//!
//! As in MonoCode (`models.ts` `setHarnessModels`), each harness starts on a
//! built-in fallback list that the live catalog read from its CLI replaces
//! (`harness::discovery`), settings included.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, RwLock};

use serde_json::{Map, Value};

use crate::harness::HarnessKind;
use crate::harness::resolver::HarnessInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    pub key: String,
    pub label: String,
    pub harness: HarnessKind,
    /// The id the CLI takes as `--model`.
    pub native: String,
    /// MonoCode `AgentModel.provider.name` (OpenCode's upstream provider).
    pub provider: Option<String>,
    /// MonoCode `AgentModel.settings`: the options shown in the model menu.
    pub settings: Vec<ModelSetting>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    Select,
    Toggle,
}

/// MonoCode `ModelSetting`: stored in the thread's `modelSettings` under `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSetting {
    pub id: String,
    pub label: String,
    pub kind: SettingKind,
    pub default: String,
    /// `(value, label)` pairs.
    pub options: Vec<(String, String)>,
}

impl ModelSetting {
    pub fn select(id: &str, label: &str, default: &str, options: &[(&str, &str)]) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind: SettingKind::Select,
            default: default.into(),
            options: options
                .iter()
                .map(|(v, l)| (v.to_string(), l.to_string()))
                .collect(),
        }
    }

    pub fn toggle(id: &str, label: &str) -> Self {
        Self {
            kind: SettingKind::Toggle,
            ..Self::select(id, label, "false", &[("true", "On"), ("false", "Off")])
        }
    }

    /// MonoCode `settingLabel`: effort-like selects read "Effort".
    pub fn menu_label(&self) -> &str {
        if matches!(self.id.as_str(), "effort" | "reasoning") {
            "Effort"
        } else {
            &self.label
        }
    }

    /// The stored value, else the model's default.
    pub fn value<'a>(&'a self, values: Option<&'a Map<String, Value>>) -> &'a str {
        values
            .and_then(|v| v.get(&self.id))
            .and_then(Value::as_str)
            .unwrap_or(&self.default)
    }

    pub fn value_label<'a>(&'a self, values: Option<&'a Map<String, Value>>) -> &'a str {
        let value = self.value(values);
        self.options
            .iter()
            .find(|(v, _)| v == value)
            .map_or(value, |(_, label)| label)
    }

    /// MonoCode `EFFORT_SETTING_IDS`.
    pub fn is_effort(&self) -> bool {
        self.kind == SettingKind::Select
            && matches!(
                self.id.as_str(),
                "effort" | "reasoning" | "reasoningEffort" | "thinking" | "variant"
            )
    }

    /// MonoCode `compatibleSettingValue`: `value` if this setting offers it.
    fn accepts(&self, value: &str) -> bool {
        match self.kind {
            SettingKind::Toggle => matches!(value, "true" | "false"),
            SettingKind::Select => self.options.iter().any(|(v, _)| v == value),
        }
    }
}

/// MonoCode `EFFORT_LABELS`.
pub fn effort_label(level: &str) -> Option<&'static str> {
    Some(match level {
        "none" => "None",
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" | "extra-high" => "Extra High",
        "max" => "Max",
        "ultra" => "Ultra",
        _ => return None,
    })
}

/// MonoCode Claude `effortSetting(levels)`: the advertised levels, then
/// Ultracode when `xhigh` exists, then Ultrathink.
pub fn claude_effort(levels: &[String]) -> ModelSetting {
    let known: Vec<&str> = levels
        .iter()
        .map(String::as_str)
        .filter(|l| effort_label(l).is_some() && *l != "extra-high")
        .collect();
    let levels = if known.is_empty() {
        vec!["low", "medium", "high", "max"]
    } else {
        known
    };
    let mut options: Vec<(String, String)> = levels
        .iter()
        .map(|l| (l.to_string(), effort_label(l).unwrap_or(l).to_string()))
        .collect();
    if levels.contains(&"xhigh") {
        options.push(("ultracode".into(), "Ultracode".into()));
    }
    options.push(("ultrathink".into(), "Ultrathink".into()));
    let default = if levels.contains(&"high") {
        "high"
    } else {
        levels[0]
    };
    ModelSetting {
        id: "effort".into(),
        label: "Reasoning".into(),
        kind: SettingKind::Select,
        default: default.into(),
        options,
    }
}

pub fn context_window(default: &str) -> ModelSetting {
    ModelSetting::select(
        "context",
        "Context",
        default,
        &[("200k", "200k"), ("1m", "1M")],
    )
}

fn option(
    key: &str,
    label: &str,
    harness: HarnessKind,
    settings: Vec<ModelSetting>,
) -> ModelOption {
    let native = key.split_once(':').map_or(key, |(_, id)| id).to_string();
    ModelOption {
        key: key.into(),
        label: label.into(),
        harness,
        native,
        provider: None,
        settings,
    }
}

/// Built-in lists used until a CLI reports its own (MonoCode
/// `CLAUDE_MODEL_CATALOG` and the antigravity seeds).
fn seeds(harness: HarnessKind) -> Vec<ModelOption> {
    let xhigh: Vec<String> = ["low", "medium", "high", "xhigh", "max"]
        .map(String::from)
        .to_vec();
    match harness {
        HarnessKind::Claude => vec![
            option(
                "claude:opus",
                "Opus",
                harness,
                vec![claude_effort(&xhigh), ModelSetting::toggle("fast", "Fast")],
            ),
            option(
                "claude:sonnet",
                "Sonnet",
                harness,
                vec![claude_effort(&xhigh)],
            ),
            option(
                "claude:haiku",
                "Haiku",
                harness,
                vec![ModelSetting::toggle("thinking", "Thinking")],
            ),
        ],
        HarnessKind::Antigravity => vec![
            option(
                "antigravity:gemini-3.8-flash-high",
                "Gemini 3.8 Flash (High)",
                harness,
                Vec::new(),
            ),
            option(
                "antigravity:gemini-3.1-pro-high",
                "Gemini 3.1 Pro (High)",
                harness,
                Vec::new(),
            ),
            option(
                "antigravity:claude-sonnet-4-6",
                "Claude Sonnet 4.6 (Thinking)",
                harness,
                Vec::new(),
            ),
        ],
        // MonoCode ships no Codex or OpenCode seeds: they list once probed.
        HarnessKind::Codex | HarnessKind::OpenCode => Vec::new(),
    }
}

type Catalog = HashMap<HarnessKind, Arc<[ModelOption]>>;

static LIVE: LazyLock<RwLock<Catalog>> = LazyLock::new(Default::default);
static SEEDS: LazyLock<Catalog> = LazyLock::new(|| {
    crate::harness::ALL_HARNESSES
        .iter()
        .map(|&kind| (kind, Arc::from(seeds(kind))))
        .collect()
});

/// MonoCode `setHarnessModels`: an empty answer keeps what was there.
pub fn set_harness_models(harness: HarnessKind, models: Vec<ModelOption>) {
    if models.is_empty() {
        return;
    }
    match LIVE.write() {
        Ok(mut live) => {
            live.insert(harness, Arc::from(models));
        }
        Err(err) => log::error!("model catalog lock poisoned: {err}"),
    }
}

/// The harness' live catalog, else its built-in list.
pub fn models_for(harness: HarnessKind) -> Arc<[ModelOption]> {
    let live = LIVE
        .read()
        .ok()
        .and_then(|live| live.get(&harness).cloned());
    live.unwrap_or_else(|| {
        SEEDS
            .get(&harness)
            .cloned()
            .unwrap_or_else(|| Arc::from([]))
    })
}

/// A `harness:model` key of a harness BenCode drives, listed yet or not.
pub fn is_model_key(key: &str) -> bool {
    key.split_once(':')
        .is_some_and(|(h, id)| HarnessKind::from_id(h).is_some() && !id.trim().is_empty())
}

pub fn find(key: &str) -> Option<ModelOption> {
    let harness = HarnessKind::from_id(key.split_once(':')?.0)?;
    let found = models_for(harness).iter().find(|m| m.key == key).cloned();
    found.or_else(|| SEEDS.get(&harness)?.iter().find(|m| m.key == key).cloned())
}

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

/// MonoCode `pickerSettings`: the model's settings in menu order, without
/// OpenCode's agent row.
pub fn picker_settings(key: &str) -> Vec<ModelSetting> {
    let Some(model) = find(key) else {
        return Vec::new();
    };
    let mut settings: Vec<_> = model
        .settings
        .into_iter()
        .filter(|s| !(model.harness == HarnessKind::OpenCode && s.id == "agent"))
        .collect();
    settings.sort_by_key(|s| {
        SETTING_ORDER
            .iter()
            .position(|id| *id == s.id)
            .unwrap_or(99)
    });
    settings
}

/// MonoCode `effortSetting`: the reasoning select the chip names.
pub fn effort_setting(key: &str) -> Option<ModelSetting> {
    find(key)?
        .settings
        .into_iter()
        .find(ModelSetting::is_effort)
}

/// MonoCode `mergeModelSettings`: the model's defaults, overridden by
/// whatever in `current` the model accepts.
pub fn merge_settings(key: &str, current: Option<&Map<String, Value>>) -> Map<String, Value> {
    let Some(model) = find(key) else {
        return current.cloned().unwrap_or_default();
    };
    model
        .settings
        .iter()
        .map(|setting| {
            let value = current
                .and_then(|c| c.get(&setting.id))
                .and_then(Value::as_str)
                .filter(|v| setting.accepts(v))
                .unwrap_or(&setting.default);
            (setting.id.clone(), Value::String(value.to_string()))
        })
        .collect()
}

/// The thread's settings as plain strings, defaults filled in, for the CLI.
pub fn resolved_settings(
    key: &str,
    values: Option<&Map<String, Value>>,
) -> BTreeMap<String, String> {
    merge_settings(key, values)
        .into_iter()
        .filter_map(|(id, v)| Some((id, v.as_str()?.to_string())))
        .collect()
}

/// Human label for a stored model key, falling back to the raw id.
pub fn label_for(key: &str) -> String {
    match find(key) {
        Some(option) => option.label,
        None => key.split_once(':').map_or(key, |(_, id)| id).to_string(),
    }
}

/// The value to pass as `--model`, or None to let the CLI pick its default.
/// Legacy BenCode sessions stored display names ("Claude 3.7 Sonnet"); those
/// are not valid CLI ids and are dropped rather than passed through.
pub fn cli_model_id(key: &str) -> Option<String> {
    if let Some(model) = find(key) {
        return Some(model.native).filter(|id| !id.is_empty() && id != "default");
    }
    let (harness, id) = key.split_once(':').unwrap_or(("", key));
    let id = id.trim();
    let unusable = id.is_empty() || id == "default" || id.contains(char::is_whitespace);
    if unusable {
        return None;
    }
    Some(if harness == "claude" {
        claude_native_id(&id.replace('.', "-"))
    } else {
        id.to_string()
    })
}

/// MonoCode `claudeNativeId`: digit-bearing slugs need the `claude-` prefix
/// (`opus-5-5` is not a CLI model, `claude-opus-5-5` and `opus` are).
pub fn claude_native_id(native: &str) -> String {
    if native.starts_with("claude-") || !native.contains(|c: char| c.is_ascii_digit()) {
        native.to_string()
    } else {
        format!("claude-{native}")
    }
}

/// First model whose harness is installed; Claude's first if nothing is.
pub fn default_model(installed: &[HarnessInfo]) -> ModelOption {
    let installed_kinds = crate::harness::ALL_HARNESSES
        .iter()
        .filter(|kind| installed.iter().any(|h| h.available && h.id == kind.id()));
    installed_kinds
        .chain(crate::harness::ALL_HARNESSES.iter())
        .find_map(|&kind| models_for(kind).first().cloned())
        .expect("Claude has built-in models")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_model_id_uses_the_native_id() {
        assert_eq!(cli_model_id("claude:opus").as_deref(), Some("opus"));
        assert_eq!(
            cli_model_id("antigravity:gemini-3.8-flash-high").as_deref(),
            Some("gemini-3.8-flash-high")
        );
        assert_eq!(cli_model_id("sonnet").as_deref(), Some("sonnet"));
        // MonoCode keys a dotted version; the CLI wants the full id.
        assert_eq!(
            cli_model_id("claude:opus-4.8").as_deref(),
            Some("claude-opus-4-8")
        );
    }

    #[test]
    fn cli_model_id_drops_defaults_and_display_names() {
        assert_eq!(cli_model_id("opencode:default"), None);
        assert_eq!(cli_model_id(""), None);
        assert_eq!(cli_model_id("Claude 3.7 Sonnet"), None);
    }

    #[test]
    fn every_seed_key_is_prefixed_with_its_harness() {
        for kind in crate::harness::ALL_HARNESSES {
            for option in seeds(kind) {
                assert!(
                    option.key.starts_with(&format!("{}:", kind.id())),
                    "{}",
                    option.key
                );
            }
        }
    }

    #[test]
    fn settings_follow_monocode_menu_order_and_defaults() {
        let ids: Vec<_> = picker_settings("claude:opus")
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, ["fast", "effort"]);
        let effort = effort_setting("claude:opus").unwrap();
        assert_eq!(effort.menu_label(), "Effort");
        assert!(effort.options.iter().any(|(v, _)| v == "ultracode"));
        assert!(effort_setting("claude:haiku").is_none());

        let mut values = Map::new();
        values.insert("effort".into(), "xhigh".into());
        assert_eq!(effort.value_label(Some(&values)), "Extra High");
        let resolved = resolved_settings("claude:sonnet", Some(&values));
        assert_eq!(resolved["effort"], "xhigh");
        values.insert("effort".into(), "bogus".into());
        assert_eq!(
            merge_settings("claude:sonnet", Some(&values))["effort"],
            "high"
        );
    }

    #[test]
    fn claude_effort_without_xhigh_has_no_ultracode() {
        let levels = ["low", "medium", "high", "max"].map(String::from);
        let effort = claude_effort(&levels);
        let values: Vec<_> = effort.options.iter().map(|(v, _)| v.as_str()).collect();
        assert_eq!(values, ["low", "medium", "high", "max", "ultrathink"]);
        assert_eq!(effort.default, "high");
    }

    #[test]
    fn default_model_prefers_installed_harness() {
        let installed = [HarnessInfo {
            id: "antigravity",
            name: "Antigravity",
            binary_path: None,
            available: true,
        }];
        assert_eq!(default_model(&installed).harness, HarnessKind::Antigravity);
        assert_eq!(default_model(&[]).key, "claude:opus");
    }
}
