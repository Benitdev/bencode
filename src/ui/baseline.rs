//! Lining icons and smaller text up with a line of text, to the pixel.
//!
//! GPUI lays elements out on whole pixels but draws a glyph at its line's
//! baseline, which falls between them, and rounds each on its own. An icon
//! centred beside text therefore drifts a pixel against it from one
//! interface scale to the next. Here the baseline is rounded as GPUI rounds
//! a glyph, and what lines up with it is kept a whole number of pixels from
//! it, so both round alike at every scale.

use gpui::{App, Pixels, SharedString};

/// GPUI's default line height (`phi`).
pub const PHI: f32 = 1.618_034;

/// An icon's middle sits this far above its text's baseline, in ems: 5px
/// beside 14px text, between the middles of the capitals and of the
/// lowercase.
const ICON_RISE: f32 = 5.0 / 14.0;

/// `round_half_toward_zero`, as `Window::paint_glyph` places a glyph.
pub fn round(value: f32) -> f32 {
    (value.abs() - 0.5).ceil().copysign(value)
}

/// A line of text: its font, size and line height (all at the interface
/// scale).
#[derive(Clone, Copy)]
pub struct Line<'a> {
    pub family: &'a SharedString,
    pub size: Pixels,
    pub height: Pixels,
}

impl<'a> Line<'a> {
    /// At GPUI's default line height.
    pub fn new(family: &'a SharedString, size: Pixels) -> Self {
        Self {
            family,
            size,
            height: size * PHI,
        }
    }

    /// The baseline's distance below the line box's top, unrounded, as a
    /// laid-out line places it.
    pub fn baseline(&self, cx: &App) -> f32 {
        let system = cx.text_system();
        let font = system.resolve_font(&gpui::font(self.family.clone()));
        let ascent = system.ascent(font, self.size);
        // A font's descent is negative; a laid-out line's is not.
        let descent = system.descent(font, self.size).abs();
        f32::from((self.height - ascent - descent) / 2.0 + ascent)
    }

    /// Where this line's box starts, below a box top, so its glyphs sit on
    /// `baseline` (whole pixels below that top).
    pub fn top_for(&self, baseline: f32, cx: &App) -> Pixels {
        gpui::px((baseline - round(self.baseline(cx))).max(0.0))
    }

    /// Where an icon `height` tall starts, below a box top, so its middle
    /// sits on this line's optical centre for a baseline at `baseline`.
    pub fn icon_top(&self, baseline: f32, height: Pixels) -> Pixels {
        let above = (ICON_RISE * f32::from(self.size) + f32::from(height) / 2.0).round();
        gpui::px((baseline - above).max(0.0))
    }
}
