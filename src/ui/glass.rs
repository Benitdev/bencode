//! MonoCode's native glass (`src-tauri/src/macos.rs`, `.sidebar-glass` and
//! `.body-glass` in `src/styles/index.css`, Appearance › Translucency).
//!
//! In dark mode on macOS the window is transparent and blurs the desktop
//! behind it. MonoCode tints the NSWindow itself with the base background at
//! the sidebar opacity (`prepare_glass`), so one tint covers the whole window,
//! title bar included, and its glass panes go clear over it
//! (`has-native-glass-tint`); with "Main pane glass" off the main pane stays
//! opaque. Here the root plays the NSWindow. Light mode and other platforms
//! stay opaque.
//!
//! GPUI's `WindowBackgroundAppearance::Blurred` draws an `NSVisualEffectView`
//! under the window's content, so the blur strength is the system's: MonoCode's
//! "Blur radius" slider has no counterpart.

use ely_gpui_component::theme::ActiveTheme;
use gpui::{App, Hsla, Window, WindowBackgroundAppearance};

use crate::app::BenCodeApp;

/// MonoCode `SIDEBAR_OPACITY_MIN` / `MAX` / `DEFAULT`.
pub const OPACITY_MIN: f32 = 0.15;
pub const OPACITY_MAX: f32 = 1.0;
pub const OPACITY_DEFAULT: f32 = 0.85;

/// MonoCode `HAS_NATIVE_GLASS`: the window can blur what lies behind it.
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// The glass in effect this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glass {
    /// The window is transparent and blurred (dark mode, macOS).
    pub on: bool,
    /// MonoCode `--sidebar-opacity`.
    pub opacity: f32,
    /// MonoCode `glass-body`: the main pane is tinted too.
    pub body: bool,
}

impl Glass {
    /// The window's own fill: under glass, MonoCode's NSWindow tint, the base
    /// background at the opacity over the blur.
    pub fn root(&self, bg: Hsla) -> Hsla {
        if self.on {
            bg.opacity(self.opacity)
        } else {
            bg
        }
    }

    /// `.sidebar-glass` (the project rail): the base background darkened
    /// 10% in dark mode, or clear over the window's tint.
    pub fn sidebar(&self, bg: Hsla, dark: bool) -> Hsla {
        if self.on {
            gpui::transparent_black()
        } else if dark {
            bg.blend(gpui::black().opacity(0.1))
        } else {
            bg
        }
    }

    /// `.body-glass` (the sidebar and the main pane): the base background,
    /// or clear over the window's tint when "Main pane glass" is on.
    pub fn body(&self, bg: Hsla) -> Hsla {
        if self.on && self.body {
            gpui::transparent_black()
        } else {
            bg
        }
    }

    /// A pane inside a glass one: clear while the glass is on, so the tint
    /// under it is not painted twice.
    pub fn fill(&self, bg: Hsla) -> Hsla {
        if self.on {
            gpui::transparent_black()
        } else {
            bg
        }
    }

    fn window_background(&self) -> WindowBackgroundAppearance {
        if self.on {
            WindowBackgroundAppearance::Blurred
        } else {
            WindowBackgroundAppearance::Opaque
        }
    }
}

/// `value` within MonoCode's opacity range.
pub fn clamp_opacity(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(OPACITY_MIN, OPACITY_MAX)
    } else {
        OPACITY_DEFAULT
    }
}

impl BenCodeApp {
    /// The glass for this frame (MonoCode `has-native-glass:not(.theme-light)`).
    pub fn glass(&self, cx: &App) -> Glass {
        Glass {
            on: SUPPORTED && cx.theme().is_dark(),
            opacity: self.sidebar_opacity,
            body: self.body_glass,
        }
    }

    /// Turns the window's blur on or off when the glass changes (MonoCode
    /// `set_window_glass_enabled`, which also waits for the first paint).
    pub fn sync_window_glass(&mut self, window: &mut Window, cx: &App) {
        let wanted = self.glass(cx).window_background();
        if self.window_background != Some(wanted) {
            window.set_background_appearance(wanted);
            self.window_background = Some(wanted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_stays_in_range() {
        assert_eq!(clamp_opacity(0.0), OPACITY_MIN);
        assert_eq!(clamp_opacity(2.0), OPACITY_MAX);
        assert_eq!(clamp_opacity(0.5), 0.5);
        assert_eq!(clamp_opacity(f32::NAN), OPACITY_DEFAULT);
    }

    #[test]
    fn panes_clear_only_under_glass() {
        let bg = gpui::white();
        let off = Glass {
            on: false,
            opacity: 0.5,
            body: true,
        };
        assert_eq!(off.root(bg), bg);
        assert_eq!(off.body(bg), bg);
        assert_eq!(off.fill(bg), bg);
        // One tint, on the window; the glass panes over it stay clear so it
        // is not compounded.
        let on = Glass { on: true, ..off };
        assert_eq!(on.root(bg).a, 0.5);
        assert_eq!(on.body(bg).a, 0.0);
        assert_eq!(on.sidebar(bg, true).a, 0.0);
        assert_eq!(on.fill(bg).a, 0.0);
        assert_eq!(Glass { body: false, ..on }.body(bg), bg);
    }
}
