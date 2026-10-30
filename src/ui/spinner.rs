//! The busy marks that show while agents run, stepped by the app's clock
//! (`start_clock`) rather than by per-frame animation: `with_animation`
//! asks for every frame and notifies the view it is drawn in, which for
//! these is the whole app, so one busy figure on the rail kept BenCode
//! redrawing at the display's rate for as long as an agent ran anywhere.
//! Each mark reads where it is from one epoch, so they move in step and the
//! clock's tick is the only redraw they need.
//!
//! MonoCode `TerminalSpinner` (a braille spinner, one frame every 80ms),
//! and in place of their frame-driven forms: the rail's figure beat
//! (`--mascot-beat`) and pulse (`animate-pulse`, [`pulse`]), bencode-ui's `ShimmerText` and
//! the ring of bencode-ui's `Spinner`.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::ops::Range;
use std::time::{Duration, Instant};

use bencode_ui::theme::{ActiveTheme, IconSize, Mix};
use gpui::{
    AnyElement, App, Bounds, DecorationRun, Hsla, IntoElement, ParentElement, PathBuilder, Pixels,
    Point, Styled, StyledText, TextAlign, Window, canvas, div, point,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::ui::scale::px;
use crate::ui::tokens::Text;

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// How long each glyph shows; the app's clock redraws this often while
/// agents run.
pub const FRAME: Duration = Duration::from_millis(80);
/// The clock's tick while a running thread is in view: its ring and
/// shimmer turn smoothly where they are watched.
pub const WATCHED_FRAME: Duration = Duration::from_millis(33);
/// bencode-ui `ShimmerText`'s sweep.
const SHIMMER: Duration = Duration::from_millis(1800);
/// bencode-ui `Spinner`'s ring: one turn.
const RING: Duration = Duration::from_millis(900);
/// Tailwind `animate-pulse`: one breath.
const PULSE: Duration = Duration::from_secs(2);
/// Where a still ring rests in its turn (bencode-ui `STILL`).
const RING_STILL: f32 = 0.3;
/// Segments in a drawn arc (bencode-ui `ARC`).
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

/// How long until the next `step` boundary of the shared epoch: a clock
/// that wakes then redraws a stepped mark the moment it moves.
pub fn until_step(step: Duration) -> Duration {
    until_step_at(epoch().elapsed(), step)
}

/// How far through a `period`-long cycle the clock is, `offset` ahead of
/// the shared epoch (a mark that should not move in unison with the rest).
pub fn cycle_offset(period: Duration, offset: Duration) -> f32 {
    cycle_at(epoch().elapsed() + offset, period)
}

/// How long until the cycle (`period`, `offset` ahead) next crosses one of
/// `edges` (fractions of the period): when a mark stepped at those edges
/// changes next.
pub fn until_edge(period: Duration, offset: Duration, edges: &[f32]) -> Duration {
    until_edge_at(epoch().elapsed() + offset, period, edges)
}

fn until_edge_at(elapsed: Duration, period: Duration, edges: &[f32]) -> Duration {
    let period_ms = period.as_millis().max(1) as f32;
    let at = (elapsed.as_millis() as f32) % period_ms;
    let left = edges
        .iter()
        .map(|edge| {
            let edge = edge * period_ms;
            if edge > at {
                edge - at
            } else {
                edge + period_ms - at
            }
        })
        .fold(period_ms, f32::min);
    Duration::from_millis(left.ceil() as u64 + 1)
}

fn until_step_at(elapsed: Duration, step: Duration) -> Duration {
    let step = step.as_millis().max(1);
    let left = step - elapsed.as_millis() % step;
    // Just past the boundary, so the mark has moved when it is drawn.
    Duration::from_millis(left as u64 + 1)
}

fn cycle_at(elapsed: Duration, period: Duration) -> f32 {
    let period = period.as_millis().max(1);
    (elapsed.as_millis() % period) as f32 / period as f32
}

/// Motion held still: the system's reduced motion, or bencode-ui's.
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

/// The spinner in `color`, a busy mark (`ui/marks.rs`).
pub fn terminal_spinner(color: Hsla, cx: &App) -> impl IntoElement {
    crate::ui::marks::mark(
        move |cx| {
            div()
                .w(px(12.0))
                .flex_none()
                .text_size(Text::Sm.px())
                .line_height(px(11.0))
                .text_color(color)
                .child(frame_at(epoch().elapsed(), cx.reduce_motion()))
                .into_any_element()
        },
        cx,
    )
}

/// Tailwind `animate-pulse` as an opacity: full, down to half at the
/// midpoint, and back; full while motion is held still.
pub fn pulse(cx: &App) -> f32 {
    if still(cx) {
        return 1.0;
    }
    pulse_at(cycle(PULSE))
}

/// A round dot `size` across in `color`, pulsing: a busy mark
/// (`ui/marks.rs`).
pub fn pulse_dot(color: Hsla, size: f32, cx: &App) -> AnyElement {
    crate::ui::marks::mark(
        move |cx| {
            div()
                .flex_none()
                .size(px(size))
                .rounded_full()
                .bg(color)
                .opacity(pulse(cx))
                .into_any_element()
        },
        cx,
    )
}

fn pulse_at(t: f32) -> f32 {
    (t * TAU).cos() * 0.25 + 0.75
}

/// bencode-ui `ShimmerText`: a band of light sweeping `text` while it waits, a
/// busy mark (`ui/marks.rs`).
pub fn shimmer(text: impl Into<gpui::SharedString>, cx: &App) -> AnyElement {
    let text = text.into();
    crate::ui::marks::mark(move |cx| shimmer_now(text.clone(), cx), cx)
}

/// The text is laid out once in one (clear) colour and painted over with
/// a colour per grapheme. Colouring it through text runs instead would
/// shape each lit grapheme on its own (GPUI splits shaping where the
/// colour changes), dropping the kerning between them: "5.5" spread
/// apart as the band passed.
fn shimmer_now(text: gpui::SharedString, cx: &App) -> AnyElement {
    let colors = &cx.theme().colors;
    let (dim, lit) = (colors.fg_subtle, colors.fg);
    if still(cx) {
        return div().text_color(dim).child(text).into_any_element();
    }
    let band = -0.3 + 1.6 * cycle(SHIMMER);
    let styled = StyledText::new(text);
    let layout = styled.layout().clone();
    let paint = move |bounds: Bounds<Pixels>, (), window: &mut Window, cx: &mut App| {
        // One line: callers truncate, and the text never wraps.
        let Some(line) = layout.line_layouts().into_iter().next() else {
            return;
        };
        let shown = layout.text();
        let runs: Vec<DecorationRun> = shimmer_runs(&shown, band, |at| dim.mix(&lit, at))
            .into_iter()
            .map(|(range, color)| DecorationRun {
                len: range.len() as u32,
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            })
            .collect();
        let painted = line.unwrapped_layout.paint(
            bounds.origin,
            layout.line_height(),
            TextAlign::Left,
            None,
            &runs,
            window,
            cx,
        );
        if let Err(err) = painted {
            log::debug!("shimmer paint failed: {err:#}");
        }
    };
    div()
        .relative()
        .whitespace_nowrap()
        .text_color(gpui::transparent_black())
        .child(styled)
        .child(
            canvas(|_, _, _| (), paint)
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        )
        .into_any_element()
}

/// Each grapheme of `text` lit by how near it sits to `band` (0 at the
/// first, 1 at the last).
fn shimmer_runs(text: &str, band: f32, ink: impl Fn(f32) -> Hsla) -> Vec<(Range<usize>, Hsla)> {
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
            (span, ink(lit))
        })
        .collect()
}

/// bencode-ui `Spinner`'s ring at `size` in muted ink: an arc running around a
/// faint circle; a busy mark (`ui/marks.rs`).
pub fn ring(size: IconSize, cx: &App) -> AnyElement {
    crate::ui::marks::mark(move |cx| ring_now(size, cx), cx)
}

fn ring_now(size: IconSize, cx: &App) -> AnyElement {
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

/// bencode-ui `paint` for `SpinnerStyle::Ring` at `t` in its turn.
fn paint_ring(bounds: Bounds<Pixels>, t: f32, color: Hsla, window: &mut Window) {
    let side = bounds.size.width.min(bounds.size.height);
    let center = bounds.center();
    let stroke = side / 8.0;
    let radius = side / 2.0 - stroke;
    arc(center, radius, 0.0, TAU, stroke, color.opacity(0.2), window);
    // bencode-ui `bump(t, 0)`: the arc grows and shrinks once a turn.
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

/// bencode-ui `arc`.
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
    fn the_clock_wakes_just_past_the_next_step() {
        let step = Duration::from_millis(1800);
        assert_eq!(
            until_step_at(Duration::from_millis(300), step),
            Duration::from_millis(1501)
        );
        assert_eq!(
            until_step_at(Duration::from_millis(1800), step),
            Duration::from_millis(1801)
        );
    }

    #[test]
    fn a_pulse_dips_to_half_and_comes_back() {
        assert_eq!(pulse_at(0.0), 1.0);
        assert!((pulse_at(0.5) - 0.5).abs() < 1e-6);
        assert!((pulse_at(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_band_lights_the_graphemes_near_it() {
        // "ab👍" is three graphemes: the band on the middle one.
        let runs = shimmer_runs("ab👍", 0.5, |lit| gpui::hsla(0.0, 0.0, lit, 1.0));
        let lit: Vec<f32> = runs.iter().map(|(_, color)| color.l).collect();
        assert_eq!(
            runs.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(),
            [0..1, 1..2, 2..6]
        );
        assert_eq!(lit, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn the_next_edge_is_the_nearest_ahead() {
        let period = Duration::from_millis(1000);
        let at = |ms| until_edge_at(Duration::from_millis(ms), period, &[0.5, 0.9]);
        assert_eq!(at(100), Duration::from_millis(401));
        assert_eq!(at(600), Duration::from_millis(301));
        assert_eq!(at(950), Duration::from_millis(551));
    }
}
