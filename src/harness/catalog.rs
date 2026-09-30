//! Selectable models. Keys use MonoCode's `harness:model` format (e.g.
//! `claude:opus`) so sessions stay interchangeable with MonoCode's DB.

use crate::harness::HarnessKind;
use crate::harness::resolver::HarnessInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelOption {
    pub key: &'static str,
    pub label: &'static str,
    pub harness: HarnessKind,
}

const fn model(key: &'static str, label: &'static str, harness: HarnessKind) -> ModelOption {
    ModelOption { key, label, harness }
}

pub const MODELS: &[ModelOption] = &[
    model("claude:opus", "Claude Opus", HarnessKind::Claude),
    model("claude:sonnet", "Claude Sonnet", HarnessKind::Claude),
    model("claude:haiku", "Claude Haiku", HarnessKind::Claude),
    model("antigravity:gemini-3.8-flash-high", "Gemini 3.8 Flash (High)", HarnessKind::Antigravity),
    model("antigravity:gemini-3.1-pro-high", "Gemini 3.1 Pro (High)", HarnessKind::Antigravity),
    model("antigravity:claude-sonnet-4-6", "Claude Sonnet 4.6 (Antigravity)", HarnessKind::Antigravity),
    model("codex:gpt-5-codex", "GPT-5 Codex", HarnessKind::Codex),
    model("codex:gpt-5", "GPT-5", HarnessKind::Codex),
    model("opencode:default", "OpenCode (default model)", HarnessKind::OpenCode),
];

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
        .find(|m| installed.iter().any(|h| h.available && h.id == m.harness.id()))
        .unwrap_or(&MODELS[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_model_id_strips_harness_prefix() {
        assert_eq!(cli_model_id("claude:opus").as_deref(), Some("opus"));
        assert_eq!(cli_model_id("antigravity:gemini-3.8-flash-high").as_deref(), Some("gemini-3.8-flash-high"));
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
            assert!(option.key.starts_with(&format!("{}:", option.harness.id())), "{}", option.key);
        }
    }

    #[test]
    fn default_model_prefers_installed_harness() {
        let installed = [HarnessInfo { id: "codex", name: "Codex", binary_path: None, available: true }];
        assert_eq!(default_model(&installed).harness, HarnessKind::Codex);
        assert_eq!(default_model(&[]).key, "claude:opus");
    }
}
