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
//! On macOS 26 and later the blur is Liquid Glass under GPUI's view
//! (`ui/native_glass.rs`); before that, GPUI's
//! `WindowBackgroundAppearance::Blurred` draws an `NSVisualEffectView` under
//! the window's content. Either way the blur strength is the system's:
//! MonoCode's "Blur radius" slider has no counterpart.

use bencode_ui::theme::ActiveTheme;
use gpui::{App, Hsla, Styled, Window, WindowBackgroundAppearance};

use crate::app::BenCodeApp;
use crate::ui::native_glass::NativeGlass;

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
    /// The window's outer canvas behind the navigator, title bar and floating
    /// workspace cards: translucent sunken tint under glass, solid sunken otherwise.
    pub fn canvas(&self, sunken: Hsla) -> Hsla {
        if self.on {
            sunken.opacity(self.opacity)
        } else {
            sunken
        }
    }

    /// Surface fill of a floating workspace card (Chat, File/Browser, Terminal,
    /// Surface, Bot):
    /// - Light mode: crisp lifted `colors.surface` over the `sunken` canvas.
    /// - Dark mode with "Main pane glass" on: a soft luminous veil (`fg` at 4%)
    ///   over the window's glass canvas — gently lifted regardless of what lies
    ///   behind the window, never compounding dark alpha into a dark pit.
    /// - Dark mode without main-pane glass: `colors.bg` lifted over `colors.sunken`.
    pub fn panel(&self, colors: &bencode_ui::theme::Palette, dark: bool) -> Hsla {
        if !dark {
            colors.surface
        } else if self.on && self.body {
            colors.fg.opacity(0.04)
        } else {
            colors.bg
        }
    }

    /// Styles `el` as a floating workspace card: rounded corners, glass-aware
    /// surface fill, whisper-soft rim, and ambient shadow in light mode.
    pub fn card<E: Styled>(&self, el: E, cx: &App) -> E {
        let theme = cx.theme();
        let colors = &theme.colors;
        let dark = theme.is_dark();
        el.rounded(crate::ui::scale::px(crate::ui::tokens::PANEL_RADIUS))
            .overflow_hidden()
            .bg(self.panel(colors, dark))
            .border_1()
            .border_color(crate::ui::tokens::panel_card_ring(colors.fg, dark))
            .shadow(crate::ui::tokens::panel_card_shadow(dark))
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
    /// `set_window_glass_enabled`, which also waits for the first paint):
    /// Liquid Glass where macOS has it, else GPUI's blur.
    pub fn sync_window_glass(&mut self, window: &mut Window, cx: &App) {
        let on = self.glass(cx).on;
        if !on {
            self.native_glass = None;
        } else if self.native_glass.is_none()
            && !self.native_glass_failed
            && NativeGlass::available()
        {
            self.native_glass = NativeGlass::install(window);
            if self.native_glass.is_none() {
                log::warn!("could not put Liquid Glass under the window; using GPUI's blur");
                self.native_glass_failed = true;
            }
        }
        if let Some(glass) = &mut self.native_glass {
            glass.sync(window);
        }
        let wanted = match (on, self.native_glass.is_some()) {
            (false, _) => WindowBackgroundAppearance::Opaque,
            // GPUI clears the window and drops its own blur view; the glass
            // under its view does the blurring.
            (true, true) => WindowBackgroundAppearance::Transparent,
            (true, false) => WindowBackgroundAppearance::Blurred,
        };
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
    fn canvas_and_panels_follow_glass_and_mode() {
        let mut colors = bencode_ui::theme::Palette::dark(false);
        colors.bg = gpui::white();
        colors.sunken = gpui::black();
        colors.surface = gpui::white();
        colors.fg = gpui::white();
        let off = Glass {
            on: false,
            opacity: 0.5,
            body: true,
        };
        let on = Glass { on: true, ..off };
        assert_eq!(off.canvas(colors.sunken), colors.sunken);
        assert_eq!(on.canvas(colors.sunken).a, 0.5);
        assert_eq!(off.panel(&colors, false), colors.surface);
        assert_eq!(Glass { body: false, ..on }.panel(&colors, true), colors.bg);
        assert_eq!(on.panel(&colors, true), colors.fg.opacity(0.04));
    }
}
