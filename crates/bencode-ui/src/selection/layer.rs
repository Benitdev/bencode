//! The window's listeners for its selection (gpui-kit's
//! `TextSelectionLayer`): mounted once, first, so its bubble-phase
//! listeners run after every element's, and anything that answers a press
//! (`claim_press`) has done so by then.

use std::time::Duration;

use gpui::{
    App, Bounds, Element, ElementId, Entity, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Position,
    ScrollEvent, SharedString, Style, TextLayout, Window, px,
};

use super::geometry::{Reach, Spot, drag_reach, nearest};
use super::model::{Caret, SegKey};
use super::{AutoScroll, Claimed, TextSelection};

/// How often a drag held past a list's edge scrolls it.
const TICK: Duration = Duration::from_millis(16);

/// Mount once per window, ahead of the text it selects.
pub struct SelectionLayer(Entity<TextSelection>);

impl SelectionLayer {
    pub fn new(selection: &Entity<TextSelection>) -> Self {
        Self(selection.clone())
    }
}

impl IntoElement for SelectionLayer {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectionLayer {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            position: Position::Absolute,
            ..Style::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        listen(&self.0, window, cx);
    }
}

fn caret_at(layout: &TextLayout, key: SegKey, at: Point<Pixels>) -> Caret {
    let ix = match layout.index_for_position(at) {
        Ok(ix) | Err(ix) => ix,
    };
    Caret { key, ix }
}

fn claimed(cx: &App) -> bool {
    cx.try_global::<Claimed>().is_some_and(|claimed| claimed.0)
}

fn listen(selection: &Entity<TextSelection>, window: &mut Window, cx: &mut App) {
    schedule_finish(selection, window, cx);

    let sel = selection.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if event.button != MouseButton::Left {
            return;
        }
        if phase.capture() {
            // Before the press lands: a press anywhere drops the selection,
            // so one on text starts a new one; shift extends it and its
            // menu's buttons keep it.
            if claimed(cx) {
                cx.set_global(Claimed(false));
            }
            let state = sel.read(cx);
            state.beside.borrow_mut().take();
            let kept = (state.kept.get()).is_some_and(|kept| kept.contains(&event.position));
            if !event.modifiers.shift && !kept {
                TextSelection::clear(&sel, cx);
            }
        } else if !claimed(cx) {
            press(&sel, event, window, cx);
        }
    });

    let sel = selection.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if !phase.bubble() || !sel.read(cx).model.borrow().is_dragging() {
            return;
        }
        if event.pressed_button == Some(MouseButton::Left) {
            drag_at(&sel, event.position, cx);
        } else {
            // The button came up outside the window.
            release(&sel, cx);
        }
    });

    let sel = selection.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase.bubble() && event.button == MouseButton::Left {
            release(&sel, cx);
        }
    });

    // A scroll during a drag moves the text under the pointer; where the
    // drag reaches is known once the frame shows the text where it went.
    let sel = selection.clone();
    window.on_mouse_event(move |_: &ScrollEvent, phase, _, cx| {
        let state = sel.read(cx);
        if phase.bubble() && state.model.borrow().is_dragging() {
            state.frame.borrow_mut().refresh = true;
        }
    });
}

/// A press the text took, or that the empty space of a document took
/// (`TextSelection::press_beside`).
fn press(sel: &Entity<TextSelection>, event: &MouseDownEvent, window: &mut Window, cx: &mut App) {
    let at = event.position;
    let landed = {
        let state = sel.read(cx);
        let frame = state.frame.borrow();
        // The innermost text under the pointer.
        let hit = (frame.painted.iter())
            .filter(|(_, painted)| painted.hitbox.is_hovered(window))
            .min_by(|(_, a), (_, b)| {
                let area = |b: &Bounds<Pixels>| f32::from(b.size.width) * f32::from(b.size.height);
                area(&a.bounds).total_cmp(&area(&b.bounds))
            })
            .map(|((doc, _), painted)| (doc.clone(), caret_at(&painted.layout, painted.key, at)));
        let beside = || {
            let doc = state.beside.borrow_mut().take()?;
            let (spots, layouts) = painted_in(&frame, &doc);
            let (spot, point) = nearest(&spots, at)?;
            Some((doc, caret_at(&layouts[spot], spots[spot].key, point)))
        };
        hit.or_else(beside)
            .and_then(|(doc, caret)| Some((frame.docs.get(&doc)?.clone(), caret)))
    };
    let Some((doc, caret)) = landed else {
        return;
    };
    let (previous, selected) = {
        let state = sel.read(cx);
        let mut model = state.model.borrow_mut();
        let previous = model.doc().cloned();
        let (clicks, extend) = (event.click_count, event.modifiers.shift);
        model.press(doc.id(), doc.0.lines, caret, clicks, extend);
        (previous, model.has_selection())
    };
    if let Some(focus) = &doc.0.focus {
        window.focus(focus, cx);
    }
    if previous.as_ref().is_some_and(|p| p != doc.id()) {
        TextSelection::changed(sel, previous.as_ref(), cx);
    }
    if selected {
        doc.changed(cx);
    }
}

/// The segments of `doc` painted, in no order, and their layouts.
fn painted_in(frame: &super::Frame, doc: &SharedString) -> (Vec<Spot>, Vec<TextLayout>) {
    (frame.painted.iter())
        .filter(|((id, _), _)| id == doc)
        .map(|(_, painted)| {
            let spot = Spot {
                key: painted.key,
                bounds: painted.bounds,
            };
            (spot, painted.layout.clone())
        })
        .unzip()
}

/// The drag is at `position`: the selection's head follows, and past its
/// list's edges the list scrolls.
fn drag_at(sel: &Entity<TextSelection>, position: Point<Pixels>, cx: &mut App) {
    let reached = {
        let state = sel.read(cx);
        let frame = state.frame.borrow();
        let Some(doc) = state.model.borrow().doc().cloned() else {
            return;
        };
        let Some(document) = frame.docs.get(&doc).cloned() else {
            return;
        };
        let view = (document.0.list.as_ref())
            .map(|list| list.viewport_bounds())
            .filter(|view| view.size.height > px(1.0));
        // Past the list's edges, the drag selects to the edge row.
        let mut at = position;
        if let Some(view) = view {
            at.y = at.y.clamp(view.top(), view.bottom() - px(1.0));
        }
        let (spots, layouts) = painted_in(&frame, &doc);
        let caret = drag_reach(&spots, at).map(|reach| match reach {
            Reach::At { spot, point } => caret_at(&layouts[spot], spots[spot].key, point),
            Reach::End { spot } => Caret {
                key: spots[spot].key,
                ix: layouts[spot].len(),
            },
            Reach::Start { spot } => Caret {
                key: spots[spot].key,
                ix: 0,
            },
        });
        (caret, document, view)
    };
    let (caret, document, view) = reached;
    let moved = caret.is_some_and(|caret| sel.read(cx).model.borrow_mut().drag_to(caret));
    if moved {
        document.changed(cx);
    }
    let delta = view.and_then(|view| scroll_delta(position.y, view));
    steer(sel, delta, cx);
}

/// The press ended.
fn release(sel: &Entity<TextSelection>, cx: &mut App) {
    let (released, doc) = {
        let state = sel.read(cx);
        state.scroll.borrow_mut().take();
        let mut model = state.model.borrow_mut();
        let released = model.release() && model.has_selection();
        (released, model.doc().cloned())
    };
    if released {
        TextSelection::changed(sel, doc.as_ref(), cx);
    }
}

/// How far a drag at `y` scrolls a list in `view` each tick (gpui-kit's
/// `AutoScroll::compute_delta`): from 16px inside its edges, faster the
/// further past them; positive toward the end.
fn scroll_delta(y: Pixels, view: Bounds<Pixels>) -> Option<Pixels> {
    const MIN_SPEED: f32 = 12.0;
    const MAX_SPEED: f32 = 64.0;
    const INNER_ZONE: f32 = 16.0;
    const OUTER_RAMP: f32 = 80.0;
    let speed = |past: Pixels| {
        let t = (past / px(INNER_ZONE + OUTER_RAMP)).min(1.0);
        px(MIN_SPEED + t * (MAX_SPEED - MIN_SPEED))
    };
    let (top, bottom) = (view.top() + px(INNER_ZONE), view.bottom() - px(INNER_ZONE));
    if y > bottom {
        Some(speed(y - bottom))
    } else if y < top {
        Some(-speed(top - y))
    } else {
        None
    }
}

/// Starts, steers or stops the scroll of a drag held at its list's edge.
fn steer(sel: &Entity<TextSelection>, delta: Option<Pixels>, cx: &mut App) {
    let Some(delta) = delta else {
        sel.read(cx).scroll.borrow_mut().take();
        return;
    };
    if let Some(scroll) = sel.read(cx).scroll.borrow_mut().as_mut() {
        scroll.delta = delta;
        return;
    }
    let weak = sel.downgrade();
    let task = cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            let ticked = cx.update(|cx| weak.upgrade().is_some_and(|sel| tick(&sel, cx)));
            if !ticked {
                break;
            }
        }
    });
    *sel.read(cx).scroll.borrow_mut() = Some(AutoScroll { delta, _task: task });
}

/// One step of a drag's scroll; `false` once it is over.
fn tick(sel: &Entity<TextSelection>, cx: &mut App) -> bool {
    let (delta, document) = {
        let state = sel.read(cx);
        let Some(delta) = state.scroll.borrow().as_ref().map(|scroll| scroll.delta) else {
            return false;
        };
        let doc = state.model.borrow().doc().cloned();
        let document = doc.and_then(|doc| state.frame.borrow().docs.get(&doc).cloned());
        (delta, document)
    };
    let Some((document, list)) =
        document.and_then(|d| d.0.list.clone().map(|list| (d.clone(), list)))
    else {
        return false;
    };
    let before = list.logical_scroll_top();
    list.scroll_by(delta);
    // At the list's end there is nowhere to go: nothing to redraw.
    if list.logical_scroll_top() != before {
        sel.read(cx).frame.borrow_mut().refresh = true;
        document.changed(cx);
    }
    true
}

/// Once per frame, after it is drawn: what no element painted or kept
/// goes, and a drag whose text moved finds its end again.
fn schedule_finish(sel: &Entity<TextSelection>, window: &mut Window, cx: &mut App) {
    {
        let state = sel.read(cx);
        let mut frame = state.frame.borrow_mut();
        if frame.finishing {
            return;
        }
        frame.finishing = true;
    }
    let sel = sel.clone();
    window.defer(cx, move |window, cx| finish(&sel, window, cx));
}

fn finish(sel: &Entity<TextSelection>, window: &mut Window, cx: &mut App) {
    let refresh = {
        let state = sel.read(cx);
        let mut frame = state.frame.borrow_mut();
        let generation = frame.generation;
        (frame.painted).retain(|_, painted| {
            painted.generation == generation || painted.alive.strong_count() > 0
        });
        let drawn: std::collections::HashSet<SharedString> =
            frame.painted.keys().map(|(doc, _)| doc.clone()).collect();
        frame.docs.retain(|doc, _| drawn.contains(doc));
        state
            .model
            .borrow_mut()
            .retain_docs(|doc| drawn.contains(doc));
        frame.generation = generation.wrapping_add(1);
        frame.finishing = false;
        std::mem::take(&mut frame.refresh)
    };
    if refresh && sel.read(cx).model.borrow().is_dragging() {
        drag_at(sel, window.mouse_position(), cx);
    }
}

#[cfg(test)]
mod tests {
    use gpui::{point, size};

    use super::*;

    #[test]
    fn a_drag_scrolls_from_inside_the_edges_faster_past_them() {
        let view = Bounds::new(point(px(0.), px(100.)), size(px(200.), px(100.)));
        assert_eq!(scroll_delta(px(150.), view), None);
        let top = scroll_delta(px(80.), view).unwrap();
        let bottom = scroll_delta(px(220.), view).unwrap();
        assert_eq!(top, -bottom);
        assert!(top < px(-12.));
        assert!(scroll_delta(px(195.), view).unwrap() < bottom);
    }
}
