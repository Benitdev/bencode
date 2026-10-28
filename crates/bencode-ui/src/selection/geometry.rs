//! Where the pointer is among the painted segments of one document, and
//! the bands a selection paints. Pure: bounds in, choices out.

use gpui::{Bounds, Pixels, Point, point, px};

use super::model::SegKey;

/// Where a segment was painted, as the hit tests see it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Spot {
    pub key: SegKey,
    pub bounds: Bounds<Pixels>,
}

/// Where a drag at a point lands in the segment `spot` (an index).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Reach {
    /// At the character nearest to the point.
    At { spot: usize, point: Point<Pixels> },
    /// After the segment's last character.
    End { spot: usize },
    /// Before its first.
    Start { spot: usize },
}

/// How far `v` is outside `from..to`.
fn gap(from: Pixels, to: Pixels, v: Pixels) -> Pixels {
    if v < from {
        from - v
    } else if v >= to {
        v - to
    } else {
        Pixels::ZERO
    }
}

/// `at` moved inside `bounds`, a pixel short of their far edges.
fn inside(bounds: Bounds<Pixels>, at: Point<Pixels>) -> Point<Pixels> {
    let right = (bounds.right() - px(1.0)).max(bounds.left());
    let bottom = (bounds.bottom() - px(1.0)).max(bounds.top());
    point(
        at.x.clamp(bounds.left(), right),
        at.y.clamp(bounds.top(), bottom),
    )
}

/// Where a drag at `at` reaches. On a segment's rows it takes the nearest
/// character of the one nearest across (a table's cells by column);
/// between segments (the gap between
/// paragraphs) it ends the last one above in reading order, as WebKit
/// does, and above them all it starts the first.
pub(crate) fn drag_reach(spots: &[Spot], at: Point<Pixels>) -> Option<Reach> {
    let across = spots
        .iter()
        .enumerate()
        .filter(|(_, s)| at.y >= s.bounds.top() && at.y < s.bounds.bottom())
        .min_by(|(_, a), (_, b)| {
            let (a, b) = (
                gap(a.bounds.left(), a.bounds.right(), at.x),
                gap(b.bounds.left(), b.bounds.right(), at.x),
            );
            f32::from(a).total_cmp(&f32::from(b))
        });
    if let Some((spot, s)) = across {
        return Some(Reach::At {
            spot,
            point: inside(s.bounds, at),
        });
    }
    let above = spots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.bounds.bottom() <= at.y)
        .max_by_key(|(_, s)| s.key);
    if let Some((spot, _)) = above {
        return Some(Reach::End { spot });
    }
    let first = spots.iter().enumerate().min_by_key(|(_, s)| s.key)?;
    Some(Reach::Start { spot: first.0 })
}

/// The segment nearest to a press beside the text (a margin, the gap
/// between messages), rows away first, then columns, and the point inside
/// it that press is nearest to.
pub(crate) fn nearest(spots: &[Spot], at: Point<Pixels>) -> Option<(usize, Point<Pixels>)> {
    let (spot, s) = spots.iter().enumerate().min_by(|(_, a), (_, b)| {
        let key = |s: &Spot| {
            (
                gap(s.bounds.top(), s.bounds.bottom(), at.y),
                gap(s.bounds.left(), s.bounds.right(), at.x),
            )
        };
        let (a, b) = (key(a), key(b));
        let cmp = |a: Pixels, b: Pixels| f32::from(a).total_cmp(&f32::from(b));
        cmp(a.0, b.0).then(cmp(a.1, b.1))
    })?;
    Some((spot, inside(s.bounds, at)))
}

/// The bands that paint a selection from `start` to `end` (the tops of
/// their rows) in text laid out in `bounds`: the first row from `start`
/// to the right edge, whole rows between, the last to `end`, or to the
/// right edge too when the selection goes on past this text.
pub(crate) fn bands(
    start: Point<Pixels>,
    end: Point<Pixels>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    continues: bool,
) -> Vec<Bounds<Pixels>> {
    let last_right = if continues { bounds.right() } else { end.x };
    if start.y == end.y {
        return vec![Bounds::from_corners(
            start,
            point(last_right.max(start.x), end.y + line_height),
        )];
    }
    let mut out = vec![Bounds::from_corners(
        start,
        point(bounds.right(), start.y + line_height),
    )];
    if end.y > start.y + line_height {
        out.push(Bounds::from_corners(
            point(bounds.left(), start.y + line_height),
            point(bounds.right(), end.y),
        ));
    }
    out.push(Bounds::from_corners(
        point(bounds.left(), end.y),
        point(last_right, end.y + line_height),
    ));
    out
}

#[cfg(test)]
mod tests {
    use gpui::size;

    use super::*;

    fn spot(block: usize, x: f32, y: f32, w: f32, h: f32) -> Spot {
        Spot {
            key: SegKey { block, seq: 0 },
            bounds: Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
        }
    }

    #[test]
    fn a_drag_on_a_row_takes_its_nearest_character() {
        let spots = [spot(0, 10., 0., 100., 20.), spot(1, 10., 40., 100., 20.)];
        // Right of the text on its row: the row's last character.
        assert_eq!(
            drag_reach(&spots, point(px(300.), px(45.))),
            Some(Reach::At {
                spot: 1,
                point: point(px(109.), px(45.))
            })
        );
    }

    #[test]
    fn a_drag_between_paragraphs_ends_the_one_above() {
        let spots = [spot(0, 10., 0., 100., 20.), spot(1, 10., 40., 100., 20.)];
        assert_eq!(
            drag_reach(&spots, point(px(50.), px(30.))),
            Some(Reach::End { spot: 0 })
        );
        // Above them all: the start of the first.
        assert_eq!(
            drag_reach(&spots, point(px(50.), px(-30.))),
            Some(Reach::Start { spot: 0 })
        );
        // Below them all: the end of the last in reading order.
        let cells = [spot(3, 10., 0., 40., 20.), spot(4, 60., 0., 40., 20.)];
        assert_eq!(
            drag_reach(&cells, point(px(20.), px(90.))),
            Some(Reach::End { spot: 1 })
        );
        assert_eq!(drag_reach(&[], point(px(0.), px(0.))), None);
    }

    #[test]
    fn side_by_side_segments_go_by_column() {
        let cells = [spot(0, 10., 0., 40., 20.), spot(1, 60., 0., 40., 20.)];
        let reached = drag_reach(&cells, point(px(70.), px(5.)));
        assert!(matches!(reached, Some(Reach::At { spot: 1, .. })));
        let reached = drag_reach(&cells, point(px(52.), px(5.)));
        assert!(matches!(reached, Some(Reach::At { spot: 0, .. })));
    }

    #[test]
    fn a_press_beside_the_text_takes_the_nearest_row() {
        let spots = [spot(0, 10., 0., 100., 20.), spot(1, 10., 40., 100., 20.)];
        // In the left margin of the second paragraph.
        assert_eq!(
            nearest(&spots, point(px(0.), px(45.))),
            Some((1, point(px(10.), px(45.))))
        );
        // Below the first, nearer the second.
        assert_eq!(
            nearest(&spots, point(px(50.), px(35.))).map(|n| n.0),
            Some(1)
        );
    }

    #[test]
    fn wrapped_selection_paints_whole_middle_rows() {
        let bounds = Bounds::new(point(px(10.), px(20.)), size(px(100.), px(100.)));
        let (start, end) = (point(px(40.), px(20.)), point(px(30.), px(80.)));
        assert_eq!(
            bands(start, end, bounds, px(20.), false),
            vec![
                Bounds::from_corners(point(px(40.), px(20.)), point(px(110.), px(40.))),
                Bounds::from_corners(point(px(10.), px(40.)), point(px(110.), px(80.))),
                Bounds::from_corners(point(px(10.), px(80.)), point(px(30.), px(100.))),
            ]
        );
        // Going on past the text: its last row reaches the right edge.
        let last = *bands(start, end, bounds, px(20.), true).last().unwrap();
        assert_eq!(last.right(), px(110.));
        let one = bands(start, point(px(60.), px(20.)), bounds, px(20.), false);
        assert_eq!(
            one,
            vec![Bounds::from_corners(
                point(px(40.), px(20.)),
                point(px(60.), px(40.))
            )]
        );
    }
}
