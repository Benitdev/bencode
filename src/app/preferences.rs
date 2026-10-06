//! Applies `settings.json` at startup and writes it back, off the UI thread,
//! whenever a persisted preference changes.

use ely_gpui_component::theme::{Mode, Theme};
use gpui::{Context, WindowAppearance};

use crate::app::{BenCodeApp, PermissionMode};
use crate::harness::catalog;
use crate::settings::{self, AppSettings, PermissionPreference, ThemePreference};

impl From<PermissionPreference> for PermissionMode {
    fn from(pref: PermissionPreference) -> Self {
        match pref {
            PermissionPreference::Supervised => Self::Supervised,
            PermissionPreference::AutoAcceptEdits => Self::AutoAcceptEdits,
            PermissionPreference::Auto => Self::Auto,
            PermissionPreference::FullAccess => Self::FullAccess,
        }
    }
}

impl From<PermissionMode> for PermissionPreference {
    fn from(mode: PermissionMode) -> Self {
        match mode {
            PermissionMode::Supervised => Self::Supervised,
            PermissionMode::AutoAcceptEdits => Self::AutoAcceptEdits,
            PermissionMode::Auto => Self::Auto,
            PermissionMode::FullAccess => Self::FullAccess,
        }
    }
}

/// The palette for `pref`; `system_dark` is what the OS currently shows.
pub fn theme_mode(pref: ThemePreference, system_dark: bool) -> Mode {
    match pref {
        ThemePreference::Dark => Mode::Dark,
        ThemePreference::Light => Mode::Light,
        ThemePreference::System if system_dark => Mode::Dark,
        ThemePreference::System => Mode::Light,
    }
}

pub fn is_dark_appearance(appearance: WindowAppearance) -> bool {
    matches!(
        appearance,
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    )
}

const RECENT_MODELS_KEPT: usize = 6;

impl BenCodeApp {
    /// Seeds app state from saved preferences; unknown model keys are ignored.
    pub fn apply_settings(&mut self, saved: AppSettings) {
        // Live catalogs arrive after startup, so any harness-prefixed key
        // is kept rather than only the built-in ones.
        if let Some(model) = saved
            .default_model
            .as_deref()
            .filter(|key| catalog::is_model_key(key))
        {
            self.selected_model = model.to_string();
        }
        self.permission_mode = saved.permission_mode.into();
        self.is_terminal_open = saved.terminal_open;
        self.theme_preference = saved.theme;
        self.claude_hooks_disabled = saved.claude_hooks_disabled;
        self.composer_mascot_off = saved.composer_mascot_off;
        self.sidebar_opacity = saved
            .sidebar_opacity
            .map_or(crate::ui::glass::OPACITY_DEFAULT, crate::ui::glass::clamp_opacity);
        self.body_glass = !saved.body_glass_off;
        self.inbox.seen = saved.inbox_seen.clone();
        self.inbox.seen_seeded = saved.inbox_seen_seeded;
        self.inbox.repairs = saved.inbox_repairs.clone();
        self.changes_ui.tree = saved.changes_tree;
        if let Some(width) = saved.inbox_list_width {
            self.inbox.list_width = width;
        }
        self.favorite_models = saved.favorite_models.clone();
        self.recent_models = saved.recent_models.clone();
        self.last_model_settings = saved.last_model_settings.clone();
        self.session_folders = saved.session_folders.clone();
        self.sessions_ui.filters = saved.session_sidebar_filters.clone();
        self.sessions_ui.pinned_collapsed = saved.pinned_sessions_collapsed.clone();
        self.sessions_ui.reminders_collapsed = saved.reminder_sessions_collapsed.clone();
        self.sidebar_tab_order = crate::ui::sidebar::parse_tab_order(&saved.sidebar_tab_order);
        self.settings = saved;
    }

    fn current_settings(&self) -> AppSettings {
        AppSettings {
            theme: self.theme_preference,
            default_model: Some(self.selected_model.clone()),
            permission_mode: self.permission_mode.into(),
            terminal_open: self.is_terminal_open,
            claude_hooks_disabled: self.claude_hooks_disabled,
            composer_mascot_off: self.composer_mascot_off,
            sidebar_opacity: Some(self.sidebar_opacity)
                .filter(|o| (o - crate::ui::glass::OPACITY_DEFAULT).abs() > f32::EPSILON),
            body_glass_off: !self.body_glass,
            inbox_seen: self.inbox.seen.clone(),
            inbox_seen_seeded: self.inbox.seen_seeded,
            inbox_repairs: self.inbox.repairs.clone(),
            changes_tree: self.changes_ui.tree,
            inbox_list_width: Some(self.inbox.list_width)
                .filter(|w| *w != crate::ui::inbox_view::DEFAULT_LIST_WIDTH),
            favorite_models: self.favorite_models.clone(),
            recent_models: self.recent_models.clone(),
            last_model_settings: self.last_model_settings.clone(),
            session_folders: self.session_folders.clone(),
            session_sidebar_filters: self.sessions_ui.filters.clone(),
            pinned_sessions_collapsed: self.sessions_ui.pinned_collapsed.clone(),
            reminder_sessions_collapsed: self.sessions_ui.reminders_collapsed.clone(),
            sidebar_tab_order: self
                .sidebar_tab_order
                .iter()
                .map(|t| crate::ui::sidebar::tab_id(*t).to_string())
                .collect(),
            pinned_projects: self.settings.pinned_projects.clone(),
            rail: self.settings.rail.clone(),
            extra: self.settings.extra.clone(),
        }
    }

    /// Persists preferences if they differ from what was last saved.
    pub fn save_settings(&mut self, cx: &mut Context<Self>) {
        let next = self.current_settings();
        if next == self.settings {
            return;
        }
        self.write_settings(next, cx);
    }

    /// MonoCode `savePinnedProjects`.
    pub fn set_pinned_projects(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let mut next = self.current_settings();
        next.pinned_projects = paths;
        if next == self.settings {
            return;
        }
        self.write_settings(next, cx);
        cx.notify();
    }

    /// Saves the rail's state as `update` returns it from the current one
    /// (MonoCode writes each of these to storage as it changes).
    pub fn update_rail_prefs(
        &mut self,
        update: impl FnOnce(&crate::ui::rail::model::RailPrefs) -> crate::ui::rail::model::RailPrefs,
        cx: &mut Context<Self>,
    ) {
        let mut next = self.current_settings();
        next.rail = update(&next.rail);
        if next == self.settings {
            return;
        }
        self.write_settings(next, cx);
        cx.notify();
    }

    fn write_settings(&mut self, next: AppSettings, cx: &mut Context<Self>) {
        self.settings = next.clone();
        let Some(dir) = settings::settings_dir() else {
            return;
        };
        cx.background_executor()
            .spawn(async move {
                if let Err(err) = settings::save_to(&dir, &next) {
                    log::error!("failed to save settings: {err:#}");
                }
            })
            .detach();
    }

    /// MonoCode `toggleFavorite`: stars or unstars a model.
    pub fn toggle_favorite_model(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(ix) = self.favorite_models.iter().position(|k| k == key) {
            self.favorite_models.remove(ix);
        } else {
            self.favorite_models.push(key.to_string());
        }
        self.save_settings(cx);
        cx.notify();
    }

    /// MonoCode `saveLastModelSettings`: `fill` only adds settings not
    /// remembered yet; otherwise `settings` win.
    pub fn save_last_model_settings(
        &mut self,
        settings: &serde_json::Map<String, serde_json::Value>,
        fill: bool,
        cx: &mut Context<Self>,
    ) {
        for (id, value) in settings {
            if !value.is_string() || (fill && self.last_model_settings.contains_key(id)) {
                continue;
            }
            self.last_model_settings.insert(id.clone(), value.clone());
        }
        self.save_settings(cx);
    }

    /// MonoCode `preferredModelSettings`: the model's defaults, then
    /// `current`, then the last settings chosen, as far as the model takes them.
    pub fn preferred_model_settings(
        &self,
        key: &str,
        current: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> serde_json::Map<String, serde_json::Value> {
        let mut wanted = current.cloned().unwrap_or_default();
        wanted.extend(self.last_model_settings.clone());
        catalog::merge_settings(key, Some(&wanted))
    }

    /// MonoCode `recordRecentModelChoice`: newest first, a few kept.
    pub fn record_recent_model(&mut self, key: &str, cx: &mut Context<Self>) {
        self.recent_models.retain(|k| k != key);
        self.recent_models.insert(0, key.to_string());
        self.recent_models.truncate(RECENT_MODELS_KEPT);
        self.save_settings(cx);
    }

    /// Sets the focused thread's access mode and remembers it for new
    /// threads. Applies from the next turn, as in MonoCode.
    pub fn set_permission_mode(&mut self, mode: PermissionMode, cx: &mut Context<Self>) {
        self.permission_mode = mode;
        if let Some(id) = self.selected_session_id.clone()
            && let Some(session) = self.sessions.iter_mut().find(|s| s.id == id)
        {
            session.runtime_mode = Some(mode.id().to_string());
            self.persist_session(&id);
        }
        self.save_settings(cx);
        cx.notify();
    }

    /// Access mode of a thread: its stored mode, else the default for new
    /// threads.
    pub fn session_permission_mode(
        &self,
        session: Option<&crate::db::SessionRow>,
    ) -> PermissionMode {
        session
            .and_then(|s| s.runtime_mode.as_deref())
            .and_then(PermissionMode::from_id)
            .unwrap_or(self.permission_mode)
    }

    pub fn set_terminal_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.is_terminal_open = open;
        if open {
            self.ensure_project_terminal(cx);
        }
        self.save_settings(cx);
        cx.notify();
    }

    /// Light, Dark or System (MonoCode Appearance › Theme).
    pub fn set_theme_preference(&mut self, pref: ThemePreference, cx: &mut Context<Self>) {
        self.theme_preference = pref;
        let system_dark = is_dark_appearance(cx.window_appearance());
        Theme::set_mode(theme_mode(pref, system_dark), cx);
        self.save_settings(cx);
        cx.notify();
    }

    /// Re-applies the palette when the OS appearance changes under System.
    pub fn on_system_appearance_changed(
        &mut self,
        appearance: WindowAppearance,
        cx: &mut Context<Self>,
    ) {
        if self.theme_preference == ThemePreference::System {
            Theme::set_mode(
                theme_mode(ThemePreference::System, is_dark_appearance(appearance)),
                cx,
            );
            cx.notify();
        }
    }

    /// MonoCode Appearance › "Composer mascot"; a running mascot leaves.
    pub fn set_composer_mascot(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.composer_mascot_off = !enabled;
        self.save_settings(cx);
        cx.notify();
    }

    /// MonoCode Appearance › Translucency › "Sidebar opacity".
    pub fn set_sidebar_opacity(&mut self, opacity: f32, cx: &mut Context<Self>) {
        self.sidebar_opacity = crate::ui::glass::clamp_opacity(opacity);
        self.save_settings(cx);
        cx.notify();
    }

    /// MonoCode Appearance › Translucency › "Main pane glass".
    pub fn set_body_glass(&mut self, on: bool, cx: &mut Context<Self>) {
        self.body_glass = on;
        self.save_settings(cx);
        cx.notify();
    }

    /// MonoCode Advanced › "Claude Code hooks"; applies from the next turn.
    pub fn set_claude_hooks(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.claude_hooks_disabled = !enabled;
        self.save_settings(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_modes_round_trip_through_preferences() {
        for mode in PermissionMode::ALL {
            let pref: PermissionPreference = mode.into();
            assert_eq!(PermissionMode::from(pref), mode);
        }
        assert_eq!(theme_mode(ThemePreference::Light, true), Mode::Light);
        assert_eq!(theme_mode(ThemePreference::System, true), Mode::Dark);
        assert_eq!(theme_mode(ThemePreference::System, false), Mode::Light);
    }
}
