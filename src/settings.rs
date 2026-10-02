//! BenCode's own preferences (MonoCode keeps these in webview storage, not in
//! its SQLite DB). Stored as JSON under `~/Library/Application Support/BenCode`.
//! Unknown keys round-trip, so newer builds' settings survive older ones.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const FILE_NAME: &str = "settings.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    Dark,
    Light,
    /// Follow the OS appearance (MonoCode "System").
    System,
}

/// Access mode for new threads, in MonoCode's `RuntimeMode` ids. Older
/// BenCode files said `confirm` / `read-only`; both now mean Supervised.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionPreference {
    #[default]
    #[serde(alias = "confirm", alias = "read-only")]
    Supervised,
    AutoAcceptEdits,
    Auto,
    FullAccess,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub theme: ThemePreference,
    /// Model key in `harness:model` form; `None` picks the first installed harness.
    pub default_model: Option<String>,
    pub permission_mode: PermissionPreference,
    pub terminal_open: bool,
    /// MonoCode "Claude Code hooks" switched off: Claude runs with
    /// `disableAllHooks`. Hooks are on by default.
    pub claude_hooks_disabled: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `~/Library/Application Support/BenCode` on macOS, `~/.config/bencode` elsewhere.
pub fn settings_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/BenCode")
    } else {
        home.join(".config/bencode")
    })
}

/// Reads settings; a missing file yields defaults, a corrupt one is logged
/// and replaced by defaults rather than blocking startup.
pub fn load_from(dir: &Path) -> AppSettings {
    let path = dir.join(FILE_NAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return AppSettings::default(),
        Err(err) => {
            log::warn!("cannot read {}: {err}", path.display());
            return AppSettings::default();
        }
    };
    serde_json::from_str(&text).unwrap_or_else(|err| {
        log::warn!("ignoring corrupt {}: {err}", path.display());
        AppSettings::default()
    })
}

/// Writes atomically: a temp file in the same directory, then rename.
pub fn save_to(dir: &Path, settings: &AppSettings) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let json = serde_json::to_string_pretty(settings)?;
    let tmp = dir.join(format!("{FILE_NAME}.tmp"));
    std::fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, dir.join(FILE_NAME)).context("replacing settings.json")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("bencode-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_file_gives_defaults_and_save_round_trips() {
        let dir = temp_dir("roundtrip");
        assert_eq!(load_from(&dir), AppSettings::default());
        let settings = AppSettings {
            theme: ThemePreference::Light,
            default_model: Some("claude:opus".into()),
            permission_mode: PermissionPreference::AutoAcceptEdits,
            terminal_open: true,
            claude_hooks_disabled: true,
            extra: Map::new(),
        };
        save_to(&dir, &settings).unwrap();
        assert_eq!(load_from(&dir), settings);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_file_falls_back_and_unknown_keys_survive() {
        let dir = temp_dir("extra");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE_NAME), "{not json").unwrap();
        assert_eq!(load_from(&dir), AppSettings::default());

        std::fs::write(dir.join(FILE_NAME), r#"{"theme":"light","futureKey":42}"#).unwrap();
        let loaded = load_from(&dir);
        assert_eq!(loaded.theme, ThemePreference::Light);
        save_to(&dir, &loaded).unwrap();
        let text = std::fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        assert!(text.contains("futureKey"), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn legacy_permission_values_read_as_supervised() {
        for legacy in ["confirm", "read-only", "supervised"] {
            let parsed: PermissionPreference =
                serde_json::from_value(Value::String(legacy.into())).unwrap();
            assert_eq!(parsed, PermissionPreference::Supervised, "{legacy}");
        }
        let full: PermissionPreference =
            serde_json::from_value(Value::String("full-access".into())).unwrap();
        assert_eq!(full, PermissionPreference::FullAccess);
    }
}
