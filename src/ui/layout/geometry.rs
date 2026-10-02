//! Layout geometry: unit-square rectangles, neighbour focus, drop edges and
//! render-ready split shares.

use super::{FocusDir, LayoutNode, PaneEdge, SplitDir, equal_sizes, normalize};

/// Tolerance for treating two rectangle edges as touching.
const EDGE_EPSILON: f32 = 1e-5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Default for LayoutRect {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutLeaf {
    pub id: String,
    pub rect: LayoutRect,
}

/// The shares a split should render with: its stored sizes when they fit
/// (one finite, non-negative value per child, positive sum), else equal.
pub fn split_shares(children: usize, sizes: &[f32]) -> Vec<f32> {
    let valid = sizes.len() == children
        && sizes.iter().all(|s| s.is_finite() && *s >= 0.0)
        && sizes.iter().sum::<f32>() > 0.0;
    if valid {
        normalize(sizes)
    } else {
        equal_sizes(children)
    }
}

/// Every leaf with its rectangle inside `rect`.
pub fn layout_leaves(node: &LayoutNode, rect: LayoutRect) -> Vec<LayoutLeaf> {
    match node {
        LayoutNode::Leaf { id } => vec![LayoutLeaf {
            id: id.clone(),
            rect,
        }],
        LayoutNode::Split {
            dir,
            children,
            sizes,
            ..
        } => {
            let shares = split_shares(children.len(), sizes);
            let mut offset = 0.0f32;
            let mut out = Vec::new();
            for (child, size) in children.iter().zip(shares) {
                let child_rect = match dir {
                    SplitDir::Right => LayoutRect {
                        x: rect.x + offset * rect.w,
                        w: size * rect.w,
                        ..rect
                    },
                    SplitDir::Down => LayoutRect {
                        y: rect.y + offset * rect.h,
                        h: size * rect.h,
                        ..rect
                    },
                };
                offset += size;
                out.extend(layout_leaves(child, child_rect));
            }
            out
        }
    }
}

/// Which edge of a pane the point `(x, y)` is nearest, measured from centre.
pub fn pane_edge_from_point(
    x: f32,
    y: f32,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
) -> PaneEdge {
    let nx = if width <= 0.0 {
        0.0
    } else {
        (x - left) / width - 0.5
    };
    let ny = if height <= 0.0 {
        0.0
    } else {
        (y - top) / height - 0.5
    };
    if nx.abs() > ny.abs() {
        if nx < 0.0 {
            PaneEdge::Left
        } else {
            PaneEdge::Right
        }
    } else if ny < 0.0 {
        PaneEdge::Top
    } else {
        PaneEdge::Bottom
    }
}

fn range_overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    0.0f32.max(a1.min(b1) - a0.max(b0))
}

/// `(gap, perpendicular overlap)` from `c` to `r` in `dir`, if `r` lies that way.
fn directional_gap(c: LayoutRect, r: LayoutRect, dir: FocusDir) -> Option<(f32, f32)> {
    let vertical_overlap = || range_overlap(c.y, c.y + c.h, r.y, r.y + r.h);
    let horizontal_overlap = || range_overlap(c.x, c.x + c.w, r.x, r.x + r.w);
    match dir {
        FocusDir::Left if r.x + r.w <= c.x + EDGE_EPSILON => {
            Some((c.x - (r.x + r.w), vertical_overlap()))
        }
        FocusDir::Right if r.x >= c.x + c.w - EDGE_EPSILON => {
            Some((r.x - (c.x + c.w), vertical_overlap()))
        }
        FocusDir::Up if r.y + r.h <= c.y + EDGE_EPSILON => {
            Some((c.y - (r.y + r.h), horizontal_overlap()))
        }
        FocusDir::Down if r.y >= c.y + c.h - EDGE_EPSILON => {
            Some((r.y - (c.y + c.h), horizontal_overlap()))
        }
        _ => None,
    }
}

/// The pane next to `focused_id` in `dir`: overlapping panes first, then the
/// nearest, then the widest overlap.
pub fn neighbor_leaf_id(node: &LayoutNode, focused_id: &str, dir: FocusDir) -> Option<String> {
    let panes = layout_leaves(node, LayoutRect::default());
    let current = panes.iter().find(|p| p.id == focused_id)?.rect;

    let mut best: Option<(&str, usize, f32, f32)> = None;
    for pane in panes.iter().filter(|p| p.id != focused_id) {
        let Some((gap, perp)) = directional_gap(current, pane.rect, dir) else {
            continue;
        };
        let miss = usize::from(perp <= 0.0);
        let is_better = best.is_none_or(|(_, b_miss, b_gap, b_perp)| {
            miss < b_miss
                || (miss == b_miss && gap < b_gap - EDGE_EPSILON)
                || (miss == b_miss && (gap - b_gap).abs() < EDGE_EPSILON && perp > b_perp)
        });
        if is_better {
            best = Some((&pane.id, miss, gap, perp));
        }
    }
    best.map(|(id, ..)| id.to_string())
}
