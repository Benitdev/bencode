//! MonoCode's look expressed as Ely palettes. Views read colours from
//! `cx.theme().colors` like every Ely component, so light/dark and Ely's
//! own widgets stay consistent.

use ely_gpui_component::theme::{ActiveTheme, Mode, Palette, Theme};
use gpui::{App, Hsla, rgb, rgba};

use crate::ui::appearance::{ThemeTint, UserAccent, mix};

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn a(hex_rgba: u32) -> Hsla {
    rgba(hex_rgba).into()
}

/// MonoCode `--color-accent: hsl(211 92% 62%)`, the same in both modes.
const ACCENT: u32 = 0x459bf7;

/// Registers MonoCode's palettes for `tint` and `accent` and applies the
/// starting mode.
pub fn install(mode: Mode, tint: &ThemeTint, accent: Option<UserAccent>, cx: &mut App) {
    set_palettes(tint, accent, cx);
    Theme::set_mode_now(mode, cx);
}

/// Rebuilds both palettes and shows them at once (a slider drag must not
/// cross-fade on every step).
pub fn set_palettes(tint: &ThemeTint, accent: Option<UserAccent>, cx: &mut App) {
    let dark = with_user_accent(monocode_dark(tint), accent, true);
    let light = with_user_accent(monocode_light(tint), accent, false);
    Theme::set_palette(Mode::Dark, Some(dark), cx);
    Theme::set_palette(Mode::Light, Some(light), cx);
    let mode = cx.theme().mode();
    Theme::set_mode_now(mode, cx);
}

/// MonoCode's dark theme: the background at `--theme-dark-lightness` (9%
/// by default) and text at 92%, both in the tint's hue and saturation.
/// Every fill is text laid over the background at a set strength
/// (`--selection-strength: 10%`, hover 15%, stroke 7%…), so the solid
/// colours below are those mixes; at the default tint they are the
/// neutral greys `#171717`, `#1f1f1f`, `#2c2c2c`….
fn monocode_dark(tint: &ThemeTint) -> Palette {
    let bg = tint.color(tint.dark_lightness);
    let fg = tint.color(92.0);
    let ink = |amount: f32| mix(bg, fg, amount);
    let mut p = Palette::dark(false);
    p.bg = bg;
    p.surface = ink(0.04);
    p.sunken = mix(bg, gpui::black(), 0.22);
    p.overlay = p.surface;
    p.hover = ink(0.05);
    p.active = ink(0.10);
    p.border = fg.opacity(0.10);
    p.border_strong = fg.opacity(0.20);
    p.fg = fg;
    p.fg_muted = ink(0.5425);
    p.fg_subtle = ink(0.40);
    p.fg_disabled = ink(0.24);
    p.accent = c(ACCENT);
    p.accent_hover = c(0x5ea9f8);
    p.on_accent = c(0xffffff);
    p.focus = c(ACCENT);
    p.link = c(0x7dd3fc);
    p.selection = a(0x459bf74d);
    p.success = c(0x34d399);
    p.warning = c(0xfbbf24);
    p.danger = c(0xf87171);
    p.info = c(0x38bdf8);
    p.success_subtle = a(0x34d39926);
    p.warning_subtle = a(0xfbbf2426);
    p.danger_subtle = a(0xf8717126);
    p.info_subtle = a(0x38bdf826);
    p.glass = bg.opacity(0.85);
    p.tooltip_bg = p.active;
    p.tooltip_fg = fg;
    p
}

/// MonoCode's light theme: background at 97%, text at 18%, gentler fills
/// (`--selection-strength: 6%`, hover 10%). Lightness is the dark theme's
/// alone; hue and saturation tint both.
fn monocode_light(tint: &ThemeTint) -> Palette {
    let bg = tint.color(97.0);
    let fg = tint.color(18.0);
    let ink = |amount: f32| mix(bg, fg, amount);
    let mut p = Palette::light(false);
    p.bg = bg;
    p.surface = tint.color(100.0);
    p.sunken = ink(0.04);
    p.overlay = p.surface;
    p.hover = ink(0.05);
    p.active = ink(0.06);
    p.border = fg.opacity(0.10);
    p.border_strong = fg.opacity(0.20);
    p.fg = fg;
    p.fg_muted = ink(0.542);
    p.fg_subtle = ink(0.418);
    p.accent = c(ACCENT);
    p.accent_hover = c(0x2f86e6);
    p.on_accent = c(0xffffff);
    p.focus = c(ACCENT);
    p.link = c(0x0b67c9);
    p.success = c(0x059669);
    p.warning = c(0xd97706);
    p.danger = c(0xef4444);
    p.info = c(0x0284c7);
    p.glass = bg.opacity(0.85);
    p
}

/// The accent the user picked, everywhere the palette's accent shows:
/// buttons, badges, dots, the caret, focus rings and selections. MonoCode
/// only tints its primary buttons, the user's bubble and the composer's
/// selection with it and keeps its blue elsewhere; BenCode lets the choice
/// colour the whole app. Links keep their blue so they still read as links.
fn with_user_accent(mut p: Palette, accent: Option<UserAccent>, dark: bool) -> Palette {
    let Some(accent) = accent else {
        return p;
    };
    let color = accent.color;
    p.accent = color;
    p.accent_hover = mix(color, if dark { gpui::white() } else { gpui::black() }, 0.12);
    p.on_accent = accent.foreground;
    p.focus = color;
    p.selection = color.opacity(if dark { 0.30 } else { 0.22 });
    p
}

/// Brand color for a `sessions.harness` id matching MonoCode.
pub fn harness_color(id: &str, colors: &Palette) -> Hsla {
    let key = id.split(':').next().unwrap_or(id).trim();
    if key.eq_ignore_ascii_case("claude") {
        c(0xd97757)
    } else if key.eq_ignore_ascii_case("codex") {
        c(0x10a37f)
    } else if key.eq_ignore_ascii_case("antigravity") {
        c(0x3186ff)
    } else if key.eq_ignore_ascii_case("omp") {
        c(0xa855f7)
    } else {
        colors.fg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(color: Hsla) -> u32 {
        let c = gpui::Rgba::from(color);
        let byte = |v: f32| (v * 255.0).round() as u32;
        (byte(c.r) << 16) | (byte(c.g) << 8) | byte(c.b)
    }

    /// The default tint gives the greys BenCode has always drawn.
    #[test]
    fn default_tint_keeps_monocode_greys() {
        let tint = ThemeTint::default();
        let dark = monocode_dark(&tint);
        let greys = [dark.bg, dark.surface, dark.sunken, dark.hover, dark.active, dark.fg, dark.fg_muted, dark.fg_subtle, dark.fg_disabled];
        assert_eq!(
            greys.map(hex),
            [0x171717, 0x1f1f1f, 0x121212, 0x222222, 0x2c2c2c, 0xebebeb, 0x8a8a8a, 0x6c6c6c, 0x4a4a4a]
        );
        let light = monocode_light(&tint);
        let greys = [light.bg, light.surface, light.sunken, light.hover, light.active, light.fg, light.fg_muted, light.fg_subtle];
        assert_eq!(
            greys.map(hex),
            [0xf7f7f7, 0xffffff, 0xefefef, 0xededed, 0xebebeb, 0x2e2e2e, 0x8a8a8a, 0xa3a3a3]
        );
    }

    #[test]
    fn user_accent_replaces_the_blue_but_not_links() {
        let tint = ThemeTint::default();
        let accent = UserAccent::parse("#ec4899");
        let dark = with_user_accent(monocode_dark(&tint), accent, true);
        assert_eq!([dark.accent, dark.focus].map(hex), [0xec4899, 0xec4899]);
        assert_eq!(hex(dark.link), 0x7dd3fc);
        assert_eq!(dark.on_accent, accent.unwrap().foreground);
        assert_eq!(with_user_accent(monocode_dark(&tint), None, true), monocode_dark(&tint));
    }

    #[test]
    fn lightness_moves_only_the_dark_theme() {
        let tint = ThemeTint { dark_lightness: 0.0, ..ThemeTint::default() };
        assert_eq!(hex(monocode_dark(&tint).bg), 0x000000);
        assert_eq!(hex(monocode_light(&tint).bg), 0xf7f7f7);
    }
}

