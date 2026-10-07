//! Provider and harness vector icons, matching MonoCode's HarnessIcon.
//!
//! Renders native vector SVG icons for all supported providers:
//! Claude, Codex (OpenAI), Antigravity, Cursor, OpenCode, Pi, Grok, Fx, Omp, Hermes.

use ely_gpui_component::theme::ActiveTheme;
use gpui::{App, Hsla, IntoElement, Pixels, RenderOnce, Styled, Window, svg};

use crate::ui::scale::px;

const CLAUDE_SVG: &[u8] = include_bytes!("../../assets/providers/claude.svg");
const CODEX_SVG: &[u8] = include_bytes!("../../assets/providers/codex.svg");
const ANTIGRAVITY_SVG: &[u8] = include_bytes!("../../assets/providers/antigravity.svg");
const CURSOR_SVG: &[u8] = include_bytes!("../../assets/providers/cursor.svg");
const OPENCODE_SVG: &[u8] = include_bytes!("../../assets/providers/opencode.svg");
const PI_SVG: &[u8] = include_bytes!("../../assets/providers/pi.svg");
const GROK_SVG: &[u8] = include_bytes!("../../assets/providers/grok.svg");
const FX_SVG: &[u8] = include_bytes!("../../assets/providers/fx.svg");
const OMP_SVG: &[u8] = include_bytes!("../../assets/providers/omp.svg");
const HERMES_SVG: &[u8] = include_bytes!("../../assets/providers/hermes.svg");

/// Returns the embedded SVG vector bytes for a given provider harness id.
pub fn harness_svg_data(harness: &str) -> &'static [u8] {
    let key = harness.split(':').next().unwrap_or(harness).trim();
    if key.eq_ignore_ascii_case("claude") {
        CLAUDE_SVG
    } else if key.eq_ignore_ascii_case("codex") {
        CODEX_SVG
    } else if key.eq_ignore_ascii_case("antigravity") {
        ANTIGRAVITY_SVG
    } else if key.eq_ignore_ascii_case("cursor") {
        CURSOR_SVG
    } else if key.eq_ignore_ascii_case("opencode") {
        OPENCODE_SVG
    } else if key.eq_ignore_ascii_case("pi") {
        PI_SVG
    } else if key.eq_ignore_ascii_case("grok") {
        GROK_SVG
    } else if key.eq_ignore_ascii_case("fx") {
        FX_SVG
    } else if key.eq_ignore_ascii_case("omp") {
        OMP_SVG
    } else if key.eq_ignore_ascii_case("hermes") {
        HERMES_SVG
    } else {
        CLAUDE_SVG
    }
}

/// Returns the default color for a harness icon, matching MonoCode's branding rules.
pub fn default_harness_color(harness: &str, theme: &ely_gpui_component::theme::Theme) -> Hsla {
    crate::ui::theme::harness_color(harness, &theme.colors)
}

/// A native vector provider icon element.
#[derive(IntoElement)]
pub struct HarnessIcon {
    harness: String,
    size: Pixels,
    color: Option<Hsla>,
}

impl HarnessIcon {
    pub fn new(harness: impl Into<String>) -> Self {
        Self {
            harness: harness.into(),
            size: px(14.0),
            color: None,
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    #[allow(dead_code)]
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for HarnessIcon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let color = self
            .color
            .unwrap_or_else(|| default_harness_color(&self.harness, theme));
        let data = harness_svg_data(&self.harness);
        svg()
            .data(data)
            .size(self.size)
            .flex_none()
            .text_color(color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_harness_svgs_are_valid() {
        for harness in [
            "claude",
            "codex",
            "antigravity",
            "cursor",
            "opencode",
            "pi",
            "grok",
            "fx",
            "omp",
            "hermes",
        ] {
            let data = harness_svg_data(harness);
            assert!(!data.is_empty(), "harness {harness} svg data is empty");
            let s = std::str::from_utf8(data).expect("valid utf8 svg");
            assert!(s.contains("<svg"), "harness {harness} missing <svg tag");
        }
    }
}
