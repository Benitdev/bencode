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

/// MonoCode electric blue, the accent in dark mode.
const ACCENT: u32 = 0x388bfd;

/// Registers MonoCode's palettes and applies the starting mode.
pub fn install(mode: Mode, cx: &mut App) {
    Theme::set_palette(Mode::Dark, Some(monocode_dark()), cx);
    Theme::set_palette(Mode::Light, Some(monocode_light()), cx);
    Theme::set_mode_now(mode, cx);
}

fn monocode_dark() -> Palette {
    let mut p = Palette::dark(false);
    p.bg = c(0x0e1015);
    p.surface = c(0x13161c);
    p.sunken = c(0x0b0d11);
    p.overlay = c(0x161922);
    p.hover = c(0x1c202a);
    p.active = c(0x222734);
    p.border = a(0xffffff14);
    p.border_strong = a(0xffffff26);
    p.fg = c(0xf1f3f7);
    p.fg_muted = c(0x8b949e);
    p.fg_subtle = c(0x6e7681);
    p.accent = c(ACCENT);
    p.accent_hover = c(0x4f9aff);
    p.on_accent = c(0xffffff);
    p.focus = c(ACCENT);
    p.link = c(0x58a6ff);
    p.selection = a(0x388bfd4d);
    p.success = c(0x3fb950);
    p.warning = c(0xd29922);
    p.danger = c(0xf85149);
    p.info = c(0x38bdf8);
    p.success_subtle = a(0x2ea04326);
    p.warning_subtle = a(0xd2992226);
    p.danger_subtle = a(0xf8514926);
    p.info_subtle = a(0x38bdf826);
    p
}

fn monocode_light() -> Palette {
    let mut p = Palette::light(false);
    p.accent = c(0x0969da);
    p.accent_hover = c(0x0550ae);
    p.on_accent = c(0xffffff);
    p.focus = c(0x0969da);
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
