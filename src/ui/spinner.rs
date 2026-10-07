//! MonoCode `TerminalSpinner`: a braille spinner, one frame every 80ms,
//! advanced by the app's clock (`start_clock`) rather than per-frame
//! animation requests, which would re-render the whole app every frame.

use std::time::{Duration, Instant};

use gpui::{App, Hsla, IntoElement, ParentElement, Styled, div};

use crate::ui::scale::px;

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// How long each glyph shows; the app's clock redraws this often while
/// agents run.
pub const FRAME: Duration = Duration::from_millis(80);

/// One start for every spinner, so they turn together.
fn epoch() -> Instant {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
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
}
