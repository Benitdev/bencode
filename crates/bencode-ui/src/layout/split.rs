use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, App, Axis, CursorStyle, DragMoveEvent, ElementId, EmptyView, EntityId, Hsla,
    InteractiveElement, IntoElement, ParentElement, Pixels, RenderOnce, Stateful,
    StatefulInteractiveElement, Styled, Window, div, prelude::*,
};

use super::seeded::use_seeded;
use crate::theme::ActiveTheme;

/// How close, in pixels, a dragged divider must come to a snap spot to take
/// it (MonoCode `SASH_SNAP_PX`).
const SNAP_PX: f32 = 12.0;

/// Thin divider with a wide grab area. `axis` is the split's direction.
pub fn resize_handle(id: impl Into<ElementId>, axis: Axis, cx: &App) -> Stateful<gpui::Div> {
    handle(id, axis, cx.theme().colors.border, false, cx)
}

/// `snapped`: the divider rests on a snap spot mid-drag, and says so in the
/// accent, a pixel wider.
fn handle(
    id: impl Into<ElementId>,
    axis: Axis,
    line: Hsla,
    snapped: bool,
    cx: &App,
) -> Stateful<gpui::Div> {
    let theme = cx.theme();
    let (lit, hit) = (theme.colors.focus, theme.handle_hit());
    let (line, thickness) = if snapped {
        (lit, gpui::px(2.0))
    } else {
        (line, gpui::px(1.0))
    };
    let base = div()
        .id(id)
        .group("resize")
        .flex_none()
        .flex()
        .justify_center()
        .items_center();
    // A filled line: gpui paints a border no wider than half its box.
    match axis {
        Axis::Horizontal => base
            .w(hit)
            .h_full()
            .cursor(CursorStyle::ResizeLeftRight)
            .child(
                div()
                    .w(thickness)
                    .h_full()
                    .bg(line)
                    .group_hover("resize", |s| s.bg(lit)),
            ),
        Axis::Vertical => base
            .h(hit)
            .w_full()
            .cursor(CursorStyle::ResizeUpDown)
            .child(
                div()
                    .h(thickness)
                    .w_full()
                    .bg(line)
                    .group_hover("resize", |s| s.bg(lit)),
            ),
    }
}

struct DividerDrag {
    owner: EntityId,
    ix: usize,
    start: Vec<f32>,
    anchor: Rc<Cell<Option<Pixels>>>,
}

/// Moves divider `ix` by `delta` of the whole, keeping each pane at `floor` or more.
fn shift(start: &[f32], ix: usize, delta: f32, floor: f32) -> Vec<f32> {
    let pair = start[ix] + start[ix + 1];
    let floor = floor.min(pair / 2.0);
    let left = (start[ix] + delta).clamp(floor, pair - floor);
    let mut next = start.to_vec();
    next[ix] = left;
    next[ix + 1] = pair - left;
    next
}

/// Where divider `ix` likes to rest, as a fraction of the whole (MonoCode
/// `sash_snaps`): halfway between its two panes, and where it sits when the
/// split is evenly spaced; only spots that keep each pane at `floor`.
fn snap_spots(start: &[f32], ix: usize, floor: f32) -> Vec<f32> {
    let before: f32 = start[..ix].iter().sum();
    let pair = start[ix] + start[ix + 1];
    let middle = before + pair / 2.0;
    let even = (ix + 1) as f32 / start.len() as f32;
    let mut spots = vec![middle];
    if (even - middle).abs() > 0.001 {
        spots.push(even);
    }
    let floor = floor.min(pair / 2.0);
    spots.retain(|at| (before + floor..=before + pair - floor).contains(at));
    spots
}

/// `next` with divider `ix` moved onto the snap spot within `reach` of
/// it, if there is one; whether it snapped.
fn snap(start: &[f32], next: &mut [f32], ix: usize, floor: f32, reach: f32) -> bool {
    let before: f32 = start[..ix].iter().sum();
    let pair = start[ix] + start[ix + 1];
    let at = before + next[ix];
    let Some(spot) = snap_spots(start, ix, floor)
        .into_iter()
        .map(|spot| (spot, (spot - at).abs()))
        .filter(|(_, off)| *off <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(spot, _)| spot)
    else {
        return false;
    };
    next[ix] = spot - before;
    next[ix + 1] = pair - next[ix];
    true
}

/// Divider `ix` halfway between its two panes (a double-click on it).
fn even_out(shares: &[f32], ix: usize) -> Vec<f32> {
    let half = (shares[ix] + shares[ix + 1]) / 2.0;
    let mut next = shares.to_vec();
    next[ix] = half;
    next[ix + 1] = half;
    next
}

/// Weights are finite and non-negative, with a finite, positive sum.
pub(crate) fn check_weights(weights: &[f32]) -> anyhow::Result<f32> {
    anyhow::ensure!(
        weights
            .iter()
            .all(|weight| weight.is_finite() && *weight >= 0.0),
        "split weights {weights:?} must be finite and non-negative"
    );
    let total: f32 = weights.iter().sum();
    anyhow::ensure!(
        total.is_finite() && total > 0.0,
        "split weights {weights:?} need a finite, positive sum"
    );
    Ok(total)
}

type OnResize = Rc<dyn Fn(&[f32], &mut Window, &mut App)>;

/// Panes side by side or stacked, split by draggable handles.
#[derive(IntoElement)]
pub struct SplitPane {
    id: ElementId,
    axis: Axis,
    panes: Vec<AnyElement>,
    sizes: Option<Vec<f32>>,
    min: Pixels,
    on_resize: Option<OnResize>,
}

impl SplitPane {
    /// `min` is the smallest a pane may shrink to.
    pub fn new(id: impl Into<ElementId>, axis: Axis, min: Pixels) -> Self {
        Self {
            id: id.into(),
            axis,
            panes: Vec::new(),
            sizes: None,
            min,
            on_resize: None,
        }
    }

    pub fn pane(mut self, pane: impl IntoElement) -> Self {
        self.panes.push(pane.into_any_element());
        self
    }

    /// Shares, one per pane, normalized. A new value replaces dragged ones.
    pub fn sizes(mut self, sizes: &[f32]) -> Self {
        self.sizes = Some(sizes.to_vec());
        self
    }

    /// Reports shares after each drag step.
    pub fn on_resize(mut self, handler: impl Fn(&[f32], &mut Window, &mut App) + 'static) -> Self {
        self.on_resize = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for SplitPane {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let count = self.panes.len();
        assert!(count >= 2, "split pane {:?} needs two panes", self.id);
        let even = vec![1.0 / count as f32; count];
        let normalized = match self.sizes.clone() {
            None => even,
            Some(weights) if weights.len() != count => {
                log::error!(
                    "split pane {:?}: {} sizes for {count} panes; even shares",
                    self.id,
                    weights.len()
                );
                even
            }
            Some(weights) => match check_weights(&weights) {
                Ok(total) => weights.iter().map(|weight| weight / total).collect(),
                Err(error) => {
                    log::error!("split pane {:?}: {error:#}; even shares", self.id);
                    even
                }
            },
        };
        let seed = (self.sizes, normalized);
        let fractions = use_seeded(self.id.clone(), seed, window, cx);
        let owner = fractions.entity_id();
        // The divider resting on a snap spot, while a drag lasts.
        let snapped = window.use_keyed_state((self.id.clone(), "snapped"), cx, |_, _| None);
        let lit = (*snapped.read(cx)).filter(|_| cx.has_active_drag());
        let on_resize = self.on_resize;
        let shares = fractions.read(cx).value.1.clone();
        let (axis, min) = (self.axis, self.min);
        let (dragged, report_drag) = (fractions.clone(), on_resize.clone());
        let mut root = div().id(self.id).size_full().flex().on_drag_move(
            move |event: &DragMoveEvent<DividerDrag>, window, cx| {
                let (fractions, on_resize) = (&dragged, &report_drag);
                let drag = event.drag(cx);
                if drag.owner != owner {
                    return;
                }
                if drag.start.len() != count {
                    log::error!(
                        "split pane: a drag over {} panes ends at {count}",
                        drag.start.len()
                    );
                    return;
                }
                let (along, extent) = match axis {
                    Axis::Horizontal => (event.event.position.x, event.bounds.size.width),
                    Axis::Vertical => (event.event.position.y, event.bounds.size.height),
                };
                let anchor = match drag.anchor.get() {
                    Some(anchor) => anchor,
                    None => {
                        drag.anchor.set(Some(along));
                        along
                    }
                };
                let mut next = shift(
                    &drag.start,
                    drag.ix,
                    (along - anchor) / extent,
                    min / extent,
                );
                let on_spot = snap(
                    &drag.start,
                    &mut next,
                    drag.ix,
                    min / extent,
                    gpui::px(SNAP_PX) / extent,
                );
                let at = on_spot.then_some(drag.ix);
                if *snapped.read(cx) != at {
                    snapped.update(cx, |snapped, cx| {
                        *snapped = at;
                        cx.notify();
                    });
                }
                if let Some(report) = on_resize {
                    report(&next, window, cx);
                }
                fractions.update(cx, |shares, cx| {
                    shares.value.1 = next;
                    cx.notify();
                });
            },
        );
        root = match axis {
            Axis::Horizontal => root.flex_row(),
            Axis::Vertical => root.flex_col(),
        };
        let mut children = Vec::with_capacity(count * 2);
        for (ix, pane) in self.panes.into_iter().enumerate() {
            if ix > 0 {
                let drag = DividerDrag {
                    owner,
                    ix: ix - 1,
                    start: shares.clone(),
                    anchor: Rc::new(Cell::new(None)),
                };
                let line = cx.theme().colors.border;
                let (even, report, fractions) = (
                    even_out(&shares, ix - 1),
                    on_resize.clone(),
                    fractions.clone(),
                );
                children.push(
                    handle(("divider", ix), axis, line, lit == Some(ix - 1), cx)
                        .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyView))
                        // A double-click evens out the two panes.
                        .on_click(move |event, window, cx| {
                            if event.click_count() != 2 {
                                return;
                            }
                            if let Some(report) = &report {
                                report(&even, window, cx);
                            }
                            let even = even.clone();
                            fractions.update(cx, |shares, cx| {
                                shares.value.1 = even;
                                cx.notify();
                            });
                        })
                        .into_any_element(),
                );
            }
            let mut cell = div()
                .relative()
                .overflow_hidden()
                .flex_basis(gpui::relative(0.0));
            cell.style().flex_grow = Some(shares[ix]);
            children.push(cell.child(pane).into_any_element());
        }
        root.children(children)
    }
}

#[cfg(test)]
mod tests {
    use super::{check_weights, even_out, shift, snap, snap_spots};

    #[test]
    fn weights_must_be_finite_non_negative_and_sum_positive() {
        assert_eq!(check_weights(&[1.0, 3.0]).unwrap(), 4.0);
        assert_eq!(check_weights(&[0.0, 1.0]).unwrap(), 1.0);
        assert!(check_weights(&[0.0, 0.0]).is_err());
        assert!(check_weights(&[-1.0, 2.0]).is_err());
        assert!(check_weights(&[f32::NAN, 1.0]).is_err());
        assert!(check_weights(&[f32::MAX, f32::MAX]).is_err());
    }

    #[test]
    fn dividers_snap_to_their_pair_middle_and_to_even_spacing() {
        // Three panes, the second divider dragged out to 0.8.
        let start = [0.2, 0.6, 0.2];
        let spots = snap_spots(&start, 0, 0.05);
        // Halfway between panes 1 and 2 (0 to 0.8), and an even third.
        assert_eq!(spots.len(), 2);
        assert!((spots[0] - 0.4).abs() < 1e-6);
        assert!((spots[1] - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(snap_spots(&[0.7, 0.3], 0, 0.1), vec![0.5]);

        let mut near = shift(&[0.7, 0.3], 0, -0.19, 0.1);
        assert!(snap(&[0.7, 0.3], &mut near, 0, 0.1, 0.02));
        assert!((near[0] - 0.5).abs() < 1e-6 && (near[1] - 0.5).abs() < 1e-6);
        let mut far = shift(&[0.7, 0.3], 0, -0.1, 0.1);
        assert!(!snap(&[0.7, 0.3], &mut far, 0, 0.1, 0.02));
        assert!((far[0] - 0.6).abs() < 1e-6);

        let even = even_out(&[0.2, 0.6, 0.2], 1);
        assert!((even[1] - 0.4).abs() < 1e-6 && (even[2] - 0.4).abs() < 1e-6);
        assert!((even[0] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn shift_moves_one_boundary_and_respects_the_floor() {
        let moved = shift(&[0.5, 0.5], 0, 0.1, 0.1);
        assert!((moved[0] - 0.6).abs() < 1e-6 && (moved[1] - 0.4).abs() < 1e-6);
        let floored = shift(&[0.5, 0.5], 0, -0.9, 0.2);
        assert!((floored[0] - 0.2).abs() < 1e-6);
        let three = shift(&[0.2, 0.3, 0.5], 1, 0.1, 0.05);
        assert!((three[1] - 0.4).abs() < 1e-6 && (three[2] - 0.4).abs() < 1e-6);
        assert!((three[0] - 0.2).abs() < 1e-6);
    }
}
