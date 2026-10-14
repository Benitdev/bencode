//! The busy marks that show while agents run, stepped by the app's clock
//! (`start_clock`) rather than by per-frame animation: `with_animation`
//! asks for every frame and notifies the view it is drawn in, which for
//! these is the whole app, so one busy mascot on the rail kept BenCode
//! redrawing at the display's rate for as long as an agent ran anywhere.
//! Each mark reads where it is from one epoch, so they move in step and the
//! clock's tick is the only redraw they need.
//!
//! MonoCode `TerminalSpinner` (a braille spinner, one frame every 80ms),
//! and in place of their frame-driven forms: the rail's mascot beat
//! (`--mascot-beat`) and pulse (`animate-pulse`), Ely's `ShimmerText` and
//! the ring of Ely's `Spinner`.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::ops::Range;
use std::time::{Duration, Instant};

use ely_gpui_component::theme::{ActiveTheme, IconSize, Mix};
use gpui::{
    AnyElement, App, Bounds, HighlightStyle, Hsla, IntoElement, ParentElement, PathBuilder, Pixels,
    Point, Styled, StyledText, Window, canvas, div, point,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::ui::scale::px;

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// How long each glyph shows; the app's clock redraws this often while
/// agents run.
pub const FRAME: Duration = Duration::from_millis(80);
/// The clock's tick while a running thread is in view: its ring and
/// shimmer turn smoothly where they are watched.
pub const WATCHED_FRAME: Duration = Duration::from_millis(33);
/// Ely `ShimmerText`'s sweep.
const SHIMMER: Duration = Duration::from_millis(1800);
/// Ely `Spinner`'s ring: one turn.
const RING: Duration = Duration::from_millis(900);
/// Where a still ring rests in its turn (Ely `STILL`).
const RING_STILL: f32 = 0.3;
/// Segments in a drawn arc (Ely `ARC`).
const ARC: usize = 32;

/// One start for every spinner, so they turn together.
fn epoch() -> Instant {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// How far through a `period`-long cycle the clock is now, 0..1.
pub fn cycle(period: Duration) -> f32 {
    cycle_at(epoch().elapsed(), period)
}

fn cycle_at(elapsed: Duration, period: Duration) -> f32 {
    let period = period.as_millis().max(1);
    (elapsed.as_millis() % period) as f32 / period as f32
}

/// Motion held still: the system's reduced motion, or Ely's.
pub fn still(cx: &App) -> bool {
    cx.reduce_motion() || cx.theme().reduced_motion
}

/// Which of the clock's frames it is now: a view drawn outside the app
/// (the floating chat) redraws its marks when this moves on.
pub fn frame_count() -> u64 {
    (epoch().elapsed().as_millis() / FRAME.as_millis()) as u64
}

/// The glyph shown now: the first one when motion is reduced.
fn frame_at(elapsed: Duration, reduced: bool) -> &'static str {
    if reduced {
        return FRAMES[0];
    }
    FRAMES[(elapsed.as_millis() / FRAME.as_millis()) as usize % FRAMES.len()]
}

/// The spinner in `color`.
pub fn terminal_spinner(color: Hsla, cx: &App) -> impl IntoElement {
    div()
        .w(px(12.0))
        .flex_none()
        .text_size(px(11.0))
        .line_height(px(11.0))
        .text_color(color)
        .child(frame_at(epoch().elapsed(), cx.reduce_motion()))
}

/// Ely `ShimmerText`: a band of light sweeping `text` while it waits.
pub fn shimmer(text: impl Into<gpui::SharedString>, cx: &App) -> AnyElement {
    let colors = &cx.theme().colors;
    let (dim, lit) = (colors.fg_subtle, colors.fg);
    let text = text.into();
    if still(cx) {
        return div().text_color(dim).child(text).into_any_element();
    }
    let band = -0.3 + 1.6 * cycle(SHIMMER);
    let runs = shimmer_runs(&text, band, |at| dim.mix(&lit, at));
    div()
        .child(StyledText::new(text).with_highlights(runs))
        .into_any_element()
}

/// Each grapheme of `text` lit by how near it sits to `band` (0 at the
/// first, 1 at the last).
fn shimmer_runs(
    text: &str,
    band: f32,
    ink: impl Fn(f32) -> Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let spans: Vec<Range<usize>> = text
        .grapheme_indices(true)
        .map(|(start, grapheme)| start..start + grapheme.len())
        .collect();
    let last = spans.len().saturating_sub(1).max(1) as f32;
    spans
        .into_iter()
        .enumerate()
        .map(|(ix, span)| {
            let x = ix as f32 / last;
            let lit = (1.0 - (x - band).abs() / 0.25).max(0.0);
            let style = HighlightStyle {
                color: Some(ink(lit)),
                ..HighlightStyle::default()
            };
            (span, style)
        })
        .collect()
}

/// Ely `Spinner`'s ring at `size` in muted ink: an arc running around a
/// faint circle.
pub fn ring(size: IconSize, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let color = theme.colors.fg_muted;
    let t = if still(cx) { RING_STILL } else { cycle(RING) };
    div()
        .flex_none()
        .size(theme.icon_size(size))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| paint_ring(bounds, t, color, window),
            )
            .size_full(),
        )
        .into_any_element()
}

/// Ely `paint` for `SpinnerStyle::Ring` at `t` in its turn.
fn paint_ring(bounds: Bounds<Pixels>, t: f32, color: Hsla, window: &mut Window) {
    let side = bounds.size.width.min(bounds.size.height);
    let center = bounds.center();
    let stroke = side / 8.0;
    let radius = side / 2.0 - stroke;
    arc(center, radius, 0.0, TAU, stroke, color.opacity(0.2), window);
    // Ely `bump(t, 0)`: the arc grows and shrinks once a turn.
    let sweep = TAU * (0.2 + 0.3 * (0.5 - 0.5 * (TAU * t).cos()));
    arc(
        center,
        radius,
        -FRAC_PI_2 + TAU * t,
        sweep,
        stroke,
        color,
        window,
    );
}

/// Ely `arc`.
fn arc(
    center: Point<Pixels>,
    radius: Pixels,
    from: f32,
    sweep: f32,
    width: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    let steps = ((ARC as f32 * sweep / TAU).ceil() as usize).max(2);
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
        Err(err) => log::error!("spinner: an arc failed to build: {err:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_lasts_80ms_and_the_ring_repeats() {
        assert_eq!(frame_at(Duration::ZERO, false), FRAMES[0]);
        assert_eq!(frame_at(Duration::from_millis(79), false), FRAMES[0]);
        assert_eq!(frame_at(Duration::from_millis(80), false), FRAMES[1]);
        assert_eq!(frame_at(Duration::from_millis(800), false), FRAMES[0]);
    }

    #[test]
    fn reduced_motion_holds_the_first_frame() {
        assert_eq!(frame_at(Duration::from_millis(240), true), FRAMES[0]);
    }

    #[test]
    fn a_cycle_runs_from_0_to_1_and_starts_over() {
        let period = Duration::from_millis(400);
        assert_eq!(cycle_at(Duration::ZERO, period), 0.0);
        assert_eq!(cycle_at(Duration::from_millis(100), period), 0.25);
        assert_eq!(cycle_at(Duration::from_millis(500), period), 0.25);
    }

    #[test]
    fn the_band_lights_the_graphemes_near_it() {
        // "ab👍" is three graphemes: the band on the middle one.
        let runs = shimmer_runs("ab👍", 0.5, |lit| gpui::hsla(0.0, 0.0, lit, 1.0));
        let lit: Vec<f32> = runs.iter().map(|(_, s)| s.color.unwrap().l).collect();
        assert_eq!(
            runs.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(),
            [0..1, 1..2, 2..6]
        );
        assert_eq!(lit, [0.0, 1.0, 0.0]);
    }
}
