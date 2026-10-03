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
        if let Some(model) = saved
            .default_model
            .as_deref()
            .filter(|key| catalog::find(key).is_some())
        {
            self.selected_model = model.to_string();
        }
        self.permission_mode = saved.permission_mode.into();
        self.is_terminal_open = saved.terminal_open;
        self.theme_preference = saved.theme;
        self.claude_hooks_disabled = saved.claude_hooks_disabled;
        self.favorite_models = saved.favorite_models.clone();
        self.recent_models = saved.recent_models.clone();
        self.settings = saved;
    }

    fn current_settings(&self) -> AppSettings {
        AppSettings {
            theme: self.theme_preference,
            default_model: Some(self.selected_model.clone()),
            permission_mode: self.permission_mode.into(),
            terminal_open: self.is_terminal_open,
            claude_hooks_disabled: self.claude_hooks_disabled,
            favorite_models: self.favorite_models.clone(),
            recent_models: self.recent_models.clone(),
            extra: self.settings.extra.clone(),
        }
    }

    /// Persists preferences if they differ from what was last saved.
    pub fn save_settings(&mut self, cx: &mut Context<Self>) {
        let next = self.current_settings();
        if next == self.settings {
            return;
        }
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
