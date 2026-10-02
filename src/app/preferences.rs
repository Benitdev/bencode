//! Applies `settings.json` at startup and writes it back, off the UI thread,
//! whenever a persisted preference changes.

use ely_gpui_component::theme::{ActiveTheme, Mode, Theme};
use gpui::{App, Context};

use crate::app::{BenCodeApp, PermissionMode};
use crate::harness::catalog;
use crate::settings::{self, AppSettings, PermissionPreference, ThemePreference};

impl From<PermissionPreference> for PermissionMode {
    fn from(pref: PermissionPreference) -> Self {
        match pref {
            PermissionPreference::Auto => Self::Auto,
            PermissionPreference::Confirm => Self::Confirm,
            PermissionPreference::ReadOnly => Self::ReadOnly,
        }
    }
}

impl From<PermissionMode> for PermissionPreference {
    fn from(mode: PermissionMode) -> Self {
        match mode {
            PermissionMode::Auto => Self::Auto,
            PermissionMode::Confirm => Self::Confirm,
            PermissionMode::ReadOnly => Self::ReadOnly,
        }
    }
}

pub fn theme_mode(pref: ThemePreference) -> Mode {
    match pref {
        ThemePreference::Dark => Mode::Dark,
        ThemePreference::Light => Mode::Light,
    }
}

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
        self.settings = saved;
    }

    fn current_settings(&self, cx: &App) -> AppSettings {
        AppSettings {
            theme: if cx.theme().is_dark() {
                ThemePreference::Dark
            } else {
                ThemePreference::Light
            },
            default_model: Some(self.selected_model.clone()),
            permission_mode: self.permission_mode.into(),
            terminal_open: self.is_terminal_open,
            extra: self.settings.extra.clone(),
        }
    }

    /// Persists preferences if they differ from what was last saved.
    pub fn save_settings(&mut self, cx: &mut Context<Self>) {
        let next = self.current_settings(cx);
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

    pub fn set_permission_mode(&mut self, mode: PermissionMode, cx: &mut Context<Self>) {
        self.permission_mode = mode;
        self.save_settings(cx);
        cx.notify();
    }

    pub fn set_terminal_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.is_terminal_open = open;
        self.save_settings(cx);
        cx.notify();
    }

    pub fn set_theme_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        Theme::set_mode(mode, cx);
        self.save_settings(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_modes_round_trip_through_preferences() {
        for mode in [
            PermissionMode::Auto,
            PermissionMode::Confirm,
            PermissionMode::ReadOnly,
        ] {
            let pref: PermissionPreference = mode.into();
            assert_eq!(PermissionMode::from(pref), mode);
        }
        assert_eq!(theme_mode(ThemePreference::Light), Mode::Light);
    }
}
