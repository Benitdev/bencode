//! MonoCode's look expressed as Ely palettes. Views read colours from
//! `cx.theme().colors` like every Ely component, so light/dark and Ely's
//! own widgets stay consistent.

use ely_gpui_component::theme::{Mode, Palette, Theme};
use gpui::{App, Hsla, rgb, rgba};

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn a(hex_rgba: u32) -> Hsla {
    rgba(hex_rgba).into()
}

/// MonoCode `--color-accent: hsl(211 92% 62%)`, the same in both modes.
const ACCENT: u32 = 0x459bf7;

/// Registers MonoCode's palettes and applies the starting mode.
pub fn install(mode: Mode, cx: &mut App) {
    Theme::set_palette(Mode::Dark, Some(monocode_dark()), cx);
    Theme::set_palette(Mode::Light, Some(monocode_light()), cx);
    Theme::set_mode_now(mode, cx);
}

/// MonoCode's dark theme: a neutral grey (`--theme-hue: 240`,
/// `--theme-saturation: 0%`) with the background at 9% lightness and text at
/// 92%. Every fill is text laid over the background at a set strength
/// (`--selection-strength: 10%`, hover 15%, stroke 7%…), so the solid
/// colours below are those mixes.
fn monocode_dark() -> Palette {
    let mut p = Palette::dark(false);
    p.bg = c(0x171717);
    p.surface = c(0x1f1f1f);
    p.sunken = c(0x121212);
    p.overlay = c(0x1f1f1f);
    p.hover = c(0x222222);
    p.active = c(0x2c2c2c);
    p.border = a(0xebebeb1a);
    p.border_strong = a(0xebebeb33);
    p.fg = c(0xebebeb);
    p.fg_muted = c(0x8a8a8a);
    p.fg_subtle = c(0x6c6c6c);
    p.fg_disabled = c(0x4a4a4a);
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
    p.glass = a(0x171717d9);
    p.tooltip_bg = c(0x2c2c2c);
    p.tooltip_fg = c(0xebebeb);
    p
}

/// MonoCode's light theme: background at 97%, text at 18%, gentler fills
/// (`--selection-strength: 6%`, hover 10%).
fn monocode_light() -> Palette {
    let mut p = Palette::light(false);
    p.bg = c(0xf7f7f7);
    p.surface = c(0xffffff);
    p.sunken = c(0xefefef);
    p.overlay = c(0xffffff);
    p.hover = c(0xededed);
    p.active = c(0xebebeb);
    p.border = a(0x2e2e2e1a);
    p.border_strong = a(0x2e2e2e33);
    p.fg = c(0x2e2e2e);
    p.fg_muted = c(0x8a8a8a);
    p.fg_subtle = c(0xa3a3a3);
    p.accent = c(ACCENT);
    p.accent_hover = c(0x2f86e6);
    p.on_accent = c(0xffffff);
    p.focus = c(ACCENT);
    p.link = c(0x0b67c9);
    p.success = c(0x059669);
    p.warning = c(0xd97706);
    p.danger = c(0xef4444);
    p.info = c(0x0284c7);
    p.glass = a(0xf7f7f7d9);
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
