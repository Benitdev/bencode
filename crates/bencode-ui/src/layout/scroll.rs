use gpui::{
    App, Axis, Bounds, Canvas, ElementId, FocusHandle, Pixels, ScrollHandle, Styled, Window,
    canvas, point,
};

/// Sets `scroll`'s offset along `axis` whole so `item`, as painted, sits inside the box, its start first when it is longer; true when it moved.
pub(crate) fn bring_into_view(scroll: &ScrollHandle, item: Bounds<Pixels>, axis: Axis) -> bool {
    let (frame, offset) = (scroll.bounds(), scroll.offset());
    let ((start, end), (first, last)) = match axis {
        Axis::Horizontal => ((item.left(), item.right()), (frame.left(), frame.right())),
        Axis::Vertical => ((item.top(), item.bottom()), (frame.top(), frame.bottom())),
    };
    let shift = if start < first {
        first - start
    } else if end > last {
        (last - end).max(first - start)
    } else {
        Pixels::ZERO
    };
    if shift != Pixels::ZERO {
        scroll.set_offset(match axis {
            Axis::Horizontal => point(offset.x + shift, offset.y),
            Axis::Vertical => point(offset.x, offset.y + shift),
        });
    }
    shift != Pixels::ZERO
}

/// A canvas over a Tab stop in a scroll box: when the stop takes focus, it brings its painted box into view along `axis`, once per focus. A new `id` reveals again.
pub(crate) fn reveal_when_focused(
    id: impl Into<ElementId>,
    scroll: &ScrollHandle,
    focus: &FocusHandle,
    axis: Axis,
    window: &mut Window,
    cx: &mut App,
) -> Canvas<()> {
    let shown = window.use_keyed_state(id, cx, |_, _| false);
    let (scroll, focus) = (scroll.clone(), focus.clone());
    canvas(
        move |bounds, window, cx| {
            let (focused, held) = (focus.is_focused(window), *shown.read(cx));
            if focused && !held {
                shown.update(cx, |shown, _| *shown = true);
                if bring_into_view(&scroll, bounds, axis) {
                    log::info!("scroll: a focused stop came into view");
                    window.request_animation_frame();
                }
            } else if !focused && held {
                shown.update(cx, |shown, _| *shown = false);
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Keeps a scroll box's wheel to its own axes; gpui turns a wheel along the other axis onto a box that scrolls one way, while the page scrolls too.
pub fn on_axis<E: Styled>(mut element: E) -> E {
    element.style().restrict_scroll_to_axis = Some(true);
    element
}
