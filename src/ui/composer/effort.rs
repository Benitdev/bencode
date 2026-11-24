//! Circular Reasoning Effort gauge icon for the model picker and Bot composer.
//!
//! Draws a 240° dial (`canvas` + `PathBuilder`) whose swept arc, needle angle
//! and accent hue follow the chosen effort level (`None` -> `Low` -> `Medium`
//! -> `High` -> `Extra High` -> `Max` / `Ultrathink`). Committed changes on
//! the Bot effort pill glide with a one-shot transition and cost zero redraws
//! at rest.

use std::f32::consts::{PI, TAU};
use std::time::Duration;

use bencode_ui::theme::{ActiveTheme, Theme};
use gpui::{
    Animation, AnimationExt, AnyElement, Bounds, Context, Hsla, IntoElement, ParentElement,
    PathBuilder, Pixels, Point, Rgba, Styled, Window, canvas, div, fill, point, rgb, size,
};
use serde_json::{Map, Value};

use crate::app::BenCodeApp;
use crate::harness::catalog::ModelSetting;
use crate::ui::scale::px;
use crate::ui::tokens::{AMBER_400, Ink, SKY_400, Tone as _};

/// Tailwind's `orange-400`: peak tier (Max / Ultra / Ultrathink) on dark.
const ORANGE_400: u32 = 0xfb923c;
/// Deeper amber-orange for peak tiers on light backgrounds.
const ORANGE_600: u32 = 0xea580c;

/// Sweep of the dial track: 240° clockwise from 8 o'clock across 12 to 4 o'clock.
const START_ANGLE: f32 = 5.0 * PI / 6.0;
const SWEEP_ANGLE: f32 = 4.0 * PI / 3.0;
const ARC_SEGMENTS: f32 = 36.0;
const GAUGE_MOTION: Duration = Duration::from_millis(300);

/// One-shot transition state for the Effort icon when its level changes.
#[derive(Clone, Debug, Default)]
pub struct EffortMotion {
    pub session_id: Option<String>,
    pub from: f32,
    pub to: f32,
    pub generation: usize,
}

/// Normalized intensity (`0.0..=1.0`) for an effort option value.
pub fn effort_intensity(value: &str, index: usize, total: usize) -> f32 {
    match value.to_ascii_lowercase().as_str() {
        "none" | "off" => 0.06,
        "minimal" => 0.16,
        "low" => 0.25,
        "medium" | "normal" | "default" => 0.50,
        "high" => 0.72,
        "xhigh" | "extra-high" | "extra_high" => 0.85,
        "max" => 0.93,
        "ultra" | "ultracode" => 0.97,
        "ultrathink" => 1.0,
        _ => {
            if total <= 1 {
                0.5
            } else {
                0.20 + 0.80 * (index as f32 / (total - 1) as f32)
            }
        }
    }
}

/// Option index and normalized intensity for `value` within `setting`.
pub fn option_intensity(setting: &ModelSetting, value: &str) -> (usize, f32) {
    let total = setting.options.len();
    let ix = setting
        .options
        .iter()
        .position(|(v, _)| v == value)
        .unwrap_or(0);
    (ix, effort_intensity(value, ix, total))
}

fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.25;
    let c3 = c1 + 1.0;
    let u = t - 1.0;
    1.0 + c3 * u * u * u + c1 * u * u
}

/// Interpolates two colours in RGB space so transitions across complementary
/// hues (e.g. sky blue -> amber) never swing through green.
pub fn lerp_color(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    let a = from.to_rgb();
    let b = to.to_rgb();
    Hsla::from(Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    })
}

/// Gauge accent colour for a continuous intensity `level` (`0.0..=1.0`).
pub fn effort_accent(level: f32, theme: &Theme) -> Hsla {
    let level = level.clamp(0.0, 1.0);
    let colors = &theme.colors;
    let is_dark = theme.is_dark();
    let low = colors.fg.ink(Ink::Secondary);
    let mid = colors.fg.ink(Ink::Strong);
    let high = if is_dark {
        Hsla::from(rgb(SKY_400))
    } else {
        colors.accent
    };
    let xhigh = if is_dark {
        Hsla::from(rgb(AMBER_400))
    } else {
        colors.warning
    };
    let peak = Hsla::from(rgb(if is_dark { ORANGE_400 } else { ORANGE_600 }));
    if level <= 0.50 {
        lerp_color(low, mid, ((level - 0.15) / 0.35).clamp(0.0, 1.0))
    } else if level <= 0.72 {
        lerp_color(mid, high, ((level - 0.50) / 0.22).clamp(0.0, 1.0))
    } else if level <= 0.85 {
        lerp_color(high, xhigh, ((level - 0.72) / 0.13).clamp(0.0, 1.0))
    } else {
        lerp_color(xhigh, peak, ((level - 0.85) / 0.15).clamp(0.0, 1.0))
    }
}

#[derive(Clone, Copy)]
struct GaugeFrame {
    size_px: f32,
    arc_level: f32,
    needle_level: f32,
    fg: Hsla,
    accent: Hsla,
}

fn paint_dot(center: Point<Pixels>, radius: Pixels, color: Hsla, window: &mut Window) {
    let side = radius * 2.0;
    let origin = point(center.x - radius, center.y - radius);
    window.paint_quad(fill(Bounds::new(origin, size(side, side)), color).corner_radii(radius));
}

fn paint_arc(
    center: Point<Pixels>,
    radius: Pixels,
    from: f32,
    sweep: f32,
    width: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    if sweep <= 0.001 {
        return;
    }
    let steps = ((ARC_SEGMENTS * sweep / TAU).ceil() as usize).max(2);
    let mut path = PathBuilder::stroke(width);
    for step in 0..=steps {
        let angle = from + sweep * step as f32 / steps as f32;
        let at = point(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        );
        if step == 0 {
            path.move_to(at);
        } else {
            path.line_to(at);
        }
    }
    match path.build() {
        Ok(path) => window.paint_path(path, color),
        Err(err) => log::debug!("effort gauge arc: {err:?}"),
    }
    let cap = width / 2.0;
    let start = point(
        center.x + radius * from.cos(),
        center.y + radius * from.sin(),
    );
    let end_angle = from + sweep;
    let end = point(
        center.x + radius * end_angle.cos(),
        center.y + radius * end_angle.sin(),
    );
    paint_dot(start, cap, color, window);
    paint_dot(end, cap, color, window);
}

fn paint_gauge(frame: GaugeFrame, bounds: Bounds<Pixels>, window: &mut Window) {
    let side = bounds.size.width.min(bounds.size.height);
    let center = bounds.center();
    let radius = side * 0.42;
    let stroke = (side * 0.12).max(px(1.5));
    let arc_level = frame.arc_level.clamp(0.03, 1.0);
    let needle_level = frame.needle_level.clamp(0.0, 1.05);

    let glow = ((arc_level - 0.60) / 0.40).clamp(0.0, 1.0);
    if glow > 0.01 {
        paint_arc(
            center,
            radius,
            START_ANGLE,
            SWEEP_ANGLE * arc_level,
            stroke * 2.2,
            frame.accent.opacity(0.16 * glow),
            window,
        );
    }

    paint_arc(
        center,
        radius,
        START_ANGLE,
        SWEEP_ANGLE,
        stroke,
        frame.fg.opacity(0.16),
        window,
    );

    paint_arc(
        center,
        radius,
        START_ANGLE,
        SWEEP_ANGLE * arc_level,
        stroke,
        frame.accent,
        window,
    );

    let needle_angle = START_ANGLE + SWEEP_ANGLE * needle_level;
    let needle_len = radius * 0.66;
    let needle_w = (side * 0.105).max(px(1.35));
    let tail = point(
        center.x - radius * 0.08 * needle_angle.cos(),
        center.y - radius * 0.08 * needle_angle.sin(),
    );
    let tip = point(
        center.x + needle_len * needle_angle.cos(),
        center.y + needle_len * needle_angle.sin(),
    );
    let mut needle = PathBuilder::stroke(needle_w);
    needle.move_to(tail);
    needle.line_to(tip);
    if let Ok(path) = needle.build() {
        window.paint_path(path, frame.accent);
    }
    paint_dot(tip, needle_w / 2.0, frame.accent, window);

    let hub_r = (stroke * 0.72).max(px(1.3));
    paint_dot(center, hub_r, frame.accent, window);
}

fn gauge_canvas(frame: GaugeFrame) -> AnyElement {
    div()
        .size(px(frame.size_px))
        .flex_none()
        .child(
            canvas(|_, _, _| {}, move |bounds, _, window, _| {
                paint_gauge(frame, bounds, window);
            })
            .size_full(),
        )
        .into_any_element()
}

/// Static circular effort gauge icon at `level` (`0.0..=1.0`).
pub fn static_effort_gauge(size_px: f32, level: f32, cx: &gpui::App) -> AnyElement {
    let theme = cx.theme();
    let accent = effort_accent(level, theme);
    gauge_canvas(GaugeFrame {
        size_px,
        arc_level: level,
        needle_level: level,
        fg: theme.colors.fg,
        accent,
    })
}

fn animated_effort_gauge(
    id: &'static str,
    generation: usize,
    size_px: f32,
    from: f32,
    to: f32,
    cx: &gpui::App,
) -> AnyElement {
    if generation == 0 || (from - to).abs() < 0.001 {
        return static_effort_gauge(size_px, to, cx);
    }
    let theme = cx.theme();
    let fg = theme.colors.fg;
    let from_color = effort_accent(from, theme);
    let to_color = effort_accent(to, theme);
    div()
        .size(px(size_px))
        .flex_none()
        .with_animation(
            (id, generation),
            Animation::new(GAUGE_MOTION),
            move |el, t| {
                let eased = ease_out_cubic(t);
                let spring = ease_out_back(t);
                el.child(gauge_canvas(GaugeFrame {
                    size_px,
                    arc_level: from + (to - from) * eased,
                    needle_level: from + (to - from) * spring,
                    fg,
                    accent: lerp_color(from_color, to_color, eased),
                }))
            },
        )
        .into_any_element()
}

impl BenCodeApp {
    /// Records an effort change so the Bot effort pill icon animates smoothly.
    pub(super) fn record_effort_change(
        &mut self,
        setting: &ModelSetting,
        old_value: &str,
        new_value: &str,
    ) {
        let (_, from) = option_intensity(setting, old_value);
        let (_, to) = option_intensity(setting, new_value);
        if (from - to).abs() < 0.001 {
            return;
        }
        let motion = &mut self.composer_menus.effort_motion;
        motion.session_id = self.selected_session_id.clone();
        motion.from = from;
        motion.to = to;
        motion.generation = motion.generation.wrapping_add(1).max(1);
    }

    /// Animated effort gauge icon for the Bot composer's Effort pill.
    pub(super) fn render_bot_effort_gauge(
        &self,
        session_id: &str,
        setting: &ModelSetting,
        values: Option<&Map<String, Value>>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let (_, level) = option_intensity(setting, setting.value(values));
        let motion = &self.composer_menus.effort_motion;
        let matches = motion.generation > 0
            && motion.session_id.as_deref() == Some(session_id)
            && (motion.to - level).abs() < 0.01;
        if matches {
            animated_effort_gauge(
                "bot-effort-gauge",
                motion.generation,
                14.0,
                motion.from,
                motion.to,
                cx,
            )
        } else {
            static_effort_gauge(14.0, level, cx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_effort_levels_increase_monotonically() {
        let levels = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
        let intensities: Vec<f32> = levels
            .iter()
            .enumerate()
            .map(|(ix, v)| effort_intensity(v, ix, levels.len()))
            .collect();
        for pair in intensities.windows(2) {
            assert!(pair[0] < pair[1], "expected {} < {}", pair[0], pair[1]);
        }
        assert_eq!(
            effort_intensity("extra-high", 0, 1),
            effort_intensity("xhigh", 0, 1)
        );
    }

    #[test]
    fn unknown_effort_variants_spread_evenly() {
        assert!((effort_intensity("tier-a", 0, 3) - 0.20).abs() < 1e-5);
        assert!((effort_intensity("tier-b", 1, 3) - 0.60).abs() < 1e-5);
        assert!((effort_intensity("tier-c", 2, 3) - 1.00).abs() < 1e-5);
    }

    #[test]
    fn lerp_color_avoids_green_between_sky_and_amber() {
        let sky = Hsla::from(rgb(SKY_400));
        let amber = Hsla::from(rgb(AMBER_400));
        let mid = lerp_color(sky, amber, 0.5).to_rgb();
        assert!(mid.r > 0.55 && mid.b > 0.45);
    }
}
