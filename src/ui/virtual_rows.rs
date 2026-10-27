//! Builds only the rows of a long list that are in view. The scroll pane
//! keeps every row's height in a spacer above and below the built rows, so
//! the scrollbar and the rows' layout look the same as building them all.
//! GPUI's `uniform_list` needs equal heights; these lists mix headers,
//! notes and rows of different (but known) heights.

use std::ops::Range;

use gpui::ScrollHandle;

/// Pixels built past each edge of the viewport, so a fast scroll does not
/// show a gap before the next frame.
const OVERSCAN: f32 = 360.0;
/// Viewport assumed before the scroll pane has been laid out once.
const FIRST_FRAME_VIEWPORT: f32 = 1200.0;

/// The rows to build and the space taken by the rows skipped around them.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub range: Range<usize>,
    pub above: f32,
    pub below: f32,
}

impl Window {
    pub fn all(len: usize) -> Self {
        Self {
            range: 0..len,
            above: 0.0,
            below: 0.0,
        }
    }
}

/// The window for a pane scrolled by `scroll`, whose rows start `lead`
/// pixels into its content (its top padding).
pub fn for_scroll(heights: &[Option<f32>], scroll: &ScrollHandle, lead: f32) -> Window {
    let viewport = crate::ui::scale::logical(scroll.bounds().size.height);
    let viewport = if viewport > 0.0 {
        viewport
    } else {
        FIRST_FRAME_VIEWPORT
    };
    visible_window(
        heights,
        crate::ui::scale::logical(-scroll.offset().y) - lead,
        viewport,
    )
}

/// The rows overlapping `scroll_top..scroll_top + viewport`, plus
/// [`OVERSCAN`] each side. Every row is built when any height is unknown.
pub fn visible_window(heights: &[Option<f32>], scroll_top: f32, viewport: f32) -> Window {
    let Some(heights) = heights.iter().copied().collect::<Option<Vec<f32>>>() else {
        return Window::all(heights.len());
    };
    let total: f32 = heights.iter().sum();
    // A stale offset past the end (the list just shrank) still shows the tail.
    let scroll_top = scroll_top.min(total - viewport).max(0.0);
    let top = (scroll_top - OVERSCAN).max(0.0);
    let bottom = scroll_top + viewport + OVERSCAN;
    let (mut start, mut end) = (heights.len(), heights.len());
    let (mut y, mut above) = (0.0, 0.0);
    for (ix, height) in heights.iter().enumerate() {
        if start == heights.len() && y + height > top {
            start = ix;
            above = y;
        }
        if y >= bottom {
            end = ix;
            break;
        }
        y += height;
    }
    let start = start.min(end);
    let below = total - above - heights[start..end].iter().sum::<f32>();
    Window {
        range: start..end,
        above,
        below,
    }
}

/// Scrolls `scroll` the least that brings row `ix` into view, so a row
/// the keyboard moves to is built; `lead` as in [`for_scroll`]. Nothing
/// moves while a height is unknown (every row is built then).
pub fn reveal(heights: &[Option<f32>], ix: usize, scroll: &ScrollHandle, lead: f32) {
    let Some(heights) = heights.iter().copied().collect::<Option<Vec<f32>>>() else {
        return;
    };
    let Some(height) = heights.get(ix) else {
        return;
    };
    let top = lead + heights[..ix].iter().sum::<f32>();
    let viewport = crate::ui::scale::logical(scroll.bounds().size.height);
    let scroll_top = crate::ui::scale::logical(-scroll.offset().y);
    if let Some(to) = reveal_top((top, top + height), scroll_top, viewport) {
        let offset = scroll.offset();
        scroll.set_offset(gpui::point(offset.x, -crate::ui::scale::px(to)));
    }
}

/// The scroll top that shows `item` (its top and bottom in the content)
/// in a viewport of `viewport` scrolled to `scroll_top`, moving the least:
/// `None` when it already shows. An item taller than the viewport shows
/// its top.
pub fn reveal_top(item: (f32, f32), scroll_top: f32, viewport: f32) -> Option<f32> {
    let (top, bottom) = item;
    if top < scroll_top || bottom - top > viewport {
        (top != scroll_top).then_some(top)
    } else if bottom > scroll_top + viewport {
        Some(bottom - viewport)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: f32 = 30.0;

    #[test]
    fn builds_only_rows_in_view() {
        let heights = vec![Some(ROW); 1000];
        let window = visible_window(&heights, 3000.0, 600.0);
        let overscan = (OVERSCAN / ROW) as usize;
        assert_eq!(window.range, (100 - overscan)..(120 + overscan));
        assert_eq!(window.above, window.range.start as f32 * ROW);
        assert_eq!(window.below, (1000 - window.range.end) as f32 * ROW);
    }

    #[test]
    fn mixed_heights_keep_the_total() {
        let heights: Vec<Option<f32>> = (0..500)
            .map(|ix| Some(if ix % 7 == 0 { 28.0 } else { 18.0 }))
            .collect();
        let window = visible_window(&heights, 2000.0, 400.0);
        let built: f32 = heights[window.range.clone()].iter().flatten().sum();
        let total: f32 = heights.iter().flatten().sum();
        assert!((window.above + built + window.below - total).abs() < 0.01);
        assert!(window.range.len() < 100);
    }

    #[test]
    fn shows_the_tail_for_a_stale_offset() {
        let heights = vec![Some(ROW); 10];
        assert_eq!(visible_window(&heights, 9000.0, 600.0), Window::all(10));
    }

    #[test]
    fn builds_everything_when_a_height_is_unknown() {
        let heights = vec![Some(ROW), None, Some(18.0)];
        assert_eq!(visible_window(&heights, 0.0, 10.0), Window::all(3));
    }

    #[test]
    fn empty_list() {
        assert_eq!(visible_window(&[], 0.0, 600.0), Window::all(0));
    }

    #[test]
    fn reveal_moves_the_least() {
        // Already in view: nothing moves.
        assert_eq!(reveal_top((100.0, 130.0), 50.0, 200.0), None);
        // Below: its bottom meets the viewport's.
        assert_eq!(reveal_top((300.0, 330.0), 50.0, 200.0), Some(130.0));
        // Above: its top meets the viewport's.
        assert_eq!(reveal_top((20.0, 50.0), 50.0, 200.0), Some(20.0));
        // Taller than the viewport: its top.
        assert_eq!(reveal_top((300.0, 600.0), 50.0, 200.0), Some(300.0));
        assert_eq!(reveal_top((300.0, 600.0), 300.0, 200.0), None);
    }
}
