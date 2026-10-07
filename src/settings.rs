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
    /// MonoCode `monocode.composerRunner` switched off: no mascot runs on
    /// the composer during a turn. On by default.
    pub composer_mascot_off: bool,
    /// MonoCode `monocode.sidebarOpacity` (0.15–1); `None` is the default.
    pub sidebar_opacity: Option<f32>,
    /// MonoCode `monocode.bodyGlass` switched off. On by default.
    pub body_glass_off: bool,
    /// MonoCode `monocode.inboxSeen`: each Inbox item's `updatedAt` (ms)
    /// when last read, and whether the first list was taken as read.
    pub inbox_seen: std::collections::BTreeMap<String, i64>,
    pub inbox_seen_seeded: bool,
    /// The Inbox list column's width; `None` is the default.
    pub inbox_list_width: Option<f32>,
    /// CI repairs sent from the Inbox, oldest first.
    pub inbox_repairs: Vec<crate::ui::inbox_view::Repair>,
    /// MonoCode `monocode.changesView`: the Changes panel as a tree.
    pub changes_tree: bool,
    /// MonoCode `monocode.favoriteModels`: starred model keys.
    pub favorite_models: Vec<String>,
    /// MonoCode recent model choices, newest first (⌘. menu).
    pub recent_models: Vec<String>,
    /// MonoCode `monocode.lastModelSettings`: the last effort / fast / …
    /// chosen, carried to new threads and other models that accept them.
    pub last_model_settings: Map<String, Value>,
    /// MonoCode `monocode.sessionFolders`: each project's sidebar folders.
    pub session_folders:
        std::collections::BTreeMap<String, Vec<crate::app::session_folders::SessionFolder>>,
    /// MonoCode `monocode.sessionSidebarFilters`.
    pub session_sidebar_filters: crate::app::session_list::SessionFilters,
    /// MonoCode `monocode.pinnedSessionsCollapsed`: projects whose Pinned
    /// group is folded.
    pub pinned_sessions_collapsed: std::collections::BTreeMap<String, bool>,
    /// MonoCode `monocode.reminderSessionsCollapsed`.
    pub reminder_sessions_collapsed: std::collections::BTreeMap<String, bool>,
    /// MonoCode `monocode.sidebarTabOrder` (`sessions`, `files`, `changes`).
    pub sidebar_tab_order: Vec<String>,
    /// Each project's terminal dock side and size (MonoCode keeps them on
    /// its `ProjectTerminalDock`); projects with the default bottom dock
    /// are left out.
    pub terminal_docks: std::collections::BTreeMap<String, crate::ui::terminal_pane::DockLayout>,
    /// MonoCode `monocode.pinnedProjects`: projects on the rail's Pinned
    /// list, in rail order.
    pub pinned_projects: Vec<String>,
    /// Account profiles added here, in MonoCode's
    /// `monocode.providerAccounts.v1` shape (MonoCode's own are read from
    /// its storage and are not copied in).
    pub provider_accounts: crate::harness::accounts::StoredAccounts,
    /// MonoCode `monocode.providerAccountSelections.v1`.
    pub provider_account_selections: crate::harness::accounts::StoredSelections,
    /// The project rail's order, width, archive, groups, appearance and
    /// notification mutes, under MonoCode's names (see `ui::rail::model`).
    /// Declared before `extra` so its keys are not also kept as unknown.
    #[serde(flatten)]
    pub rail: crate::ui::rail::model::RailPrefs,
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
            composer_mascot_off: true,
            sidebar_opacity: Some(0.6),
            body_glass_off: true,
            inbox_seen: std::collections::BTreeMap::from([("o/r:issue:1".to_string(), 5)]),
            inbox_seen_seeded: true,
            inbox_list_width: Some(400.0),
            changes_tree: true,
            inbox_repairs: vec![crate::ui::inbox_view::Repair {
                item_key: "o/r:pr:7".into(),
                head_oid: "abc".into(),
                checks: vec!["test".into()],
                session_id: "s1".into(),
            }],
            favorite_models: vec!["claude:opus".into()],
            recent_models: Vec::new(),
            last_model_settings: Map::new(),
            session_folders: std::collections::BTreeMap::from([(
                "/repo".to_string(),
                vec![crate::app::session_folders::SessionFolder {
                    id: "f1".into(),
                    name: "Bugs".into(),
                    session_ids: vec!["s1".into()],
                    collapsed: true,
                    ..Default::default()
                }],
            )]),
            session_sidebar_filters: crate::app::session_list::SessionFilters {
                show_archived: true,
                hidden_harnesses: vec!["codex".into()],
                time: crate::app::session_list::TimeFilter::Week,
                status: crate::app::session_list::StatusFilter {
                    working: true,
                    ..Default::default()
                },
            },
            pinned_sessions_collapsed: std::collections::BTreeMap::from([("/repo".to_string(), true)]),
            reminder_sessions_collapsed: std::collections::BTreeMap::from([("/repo".to_string(), true)]),
            sidebar_tab_order: vec!["files".into(), "sessions".into(), "changes".into()],
            terminal_docks: std::collections::BTreeMap::from([(
                "/repo".to_string(),
                crate::ui::terminal_pane::DockLayout {
                    side: crate::ui::terminal_pane::DockSide::Left,
                    size: 400.0,
                },
            )]),
            pinned_projects: vec!["/repo".into()],
            provider_accounts: [(
                "claude".to_string(),
                vec![crate::harness::accounts::ProviderAccount {
                    id: "account-1".into(),
                    provider: "claude".into(),
                    label: "Work".into(),
                }],
            )]
            .into(),
            provider_account_selections: [(
                "/repo".to_string(),
                [("claude".to_string(), "account-1".to_string())].into(),
            )]
            .into(),
            rail: crate::ui::rail::model::RailPrefs {
                project_rail_order: vec!["/repo".into()],
                project_rail_width: Some(240.0),
                ..Default::default()
            }
            .with_new_group("g1".into())
            .with_assignment("/repo", Some("g1"))
            .with_label("/repo", "Repo"),
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
        // Rail keys are read into `rail`, not kept twice as unknown.
        std::fs::write(dir.join(FILE_NAME), r#"{"projectRailOrder":["/a"]}"#).unwrap();
        let loaded = load_from(&dir);
        assert_eq!(loaded.rail.project_rail_order, ["/a"]);
        assert!(loaded.extra.is_empty(), "{:?}", loaded.extra);
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
