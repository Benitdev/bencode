//! Scroll bars as MonoCode's WebView shows them on macOS: native ones
//! (`index.css` styles scroll bars only off the Mac), so they follow System
//! Settings › Appearance › "Show scroll bars". With a mouse, or "Always",
//! the bar is the legacy one: always there. With a
//! trackpad it is an overlay thumb that shows while the view scrolls, then
//! fades. GPUI draws no scroll bars, so scrolling views add a `ScrollBar`
//! over their right edge.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ely_gpui_component::theme::ActiveTheme;
use gpui::prelude::*;
use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, ElementId, ListState, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, ScrollHandle, UniformListScrollHandle, Window, canvas,
    div, point, px,
};

/// How the system draws scroll bars (`NSScroller.preferredScrollerStyle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Always there, beside the content.
    Legacy,
    /// Over the content while it scrolls.
    Overlay,
}

/// The system's scroll bar style now; it changes as a mouse is plugged in
/// or the setting moves, so it is read each frame (a cached class value).
// objc 0.2's `msg_send!` expands to a `cargo-clippy` feature check.
#[allow(unexpected_cfgs)]
pub fn system_style() -> Style {
    #[cfg(target_os = "macos")]
    {
        use objc::{class, msg_send, sel, sel_impl};
        // SAFETY: a class method of AppKit's `NSScroller` with no arguments,
        // returning `NSScrollerStyle` (an `NSInteger`); GPUI renders on the
        // main thread.
        let style: isize = unsafe { msg_send![class!(NSScroller), preferredScrollerStyle] };
        if style == 0 {
            return Style::Legacy;
        }
    }
    Style::Overlay
}

/// `NSScroller`'s legacy width, and the knob inside it.
const TRACK: f32 = 15.0;
const KNOB: f32 = 7.0;
/// An overlay knob under the pointer widens, as AppKit's does.
const KNOB_HOVER: f32 = 11.0;
const KNOB_MIN: f32 = 20.0;
/// Room between the knob and the track's ends.
const INSET: f32 = 2.0;
/// How long an overlay bar stays after the view stops, then fades.
const OVERLAY_IDLE: Duration = Duration::from_millis(1000);
const OVERLAY_FADE: Duration = Duration::from_millis(300);

/// Room a scroll view keeps at its right edge for its bar: WebKit
/// (`overflow: auto`) lays a legacy bar beside the content, and only while
/// the content overflows; an overlay bar goes over it. The extent is the
/// last layout's, so the room follows a change one frame later.
pub fn gutter(source: impl Into<ScrollSource>) -> Pixels {
    match system_style() {
        Style::Legacy if source.into().extent().reach > px(0.5) => px(TRACK),
        _ => Pixels::ZERO,
    }
}

/// What scrolls: a scrolling `div` (or `uniform_list`), or a `list`.
#[derive(Clone)]
pub enum ScrollSource {
    Div(ScrollHandle),
    List(ListState),
}

impl From<&ScrollHandle> for ScrollSource {
    fn from(handle: &ScrollHandle) -> Self {
        Self::Div(handle.clone())
    }
}

impl From<&ListState> for ScrollSource {
    fn from(list: &ListState) -> Self {
        Self::List(list.clone())
    }
}

impl From<&UniformListScrollHandle> for ScrollSource {
    fn from(handle: &UniformListScrollHandle) -> Self {
        Self::Div(handle.0.borrow().base_handle.clone())
    }
}

/// A view's extent along the bar: what shows, how far it can scroll, and
/// how far it has.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Extent {
    view: Pixels,
    reach: Pixels,
    scrolled: Pixels,
}

impl ScrollSource {
    fn extent(&self) -> Extent {
        match self {
            Self::Div(handle) => Extent {
                view: handle.bounds().size.height,
                reach: handle.max_offset().y,
                scrolled: -handle.offset().y,
            },
            Self::List(list) => Extent {
                view: list.viewport_bounds().size.height,
                reach: list.max_offset_for_scrollbar().y,
                scrolled: -list.scroll_px_offset_for_scrollbar().y,
            },
        }
    }

    fn scroll_to(&self, scrolled: Pixels) {
        match self {
            Self::Div(handle) => {
                let x = handle.offset().x;
                handle.set_offset(point(x, -scrolled));
            }
            Self::List(list) => list.set_offset_from_scrollbar(point(Pixels::ZERO, -scrolled)),
        }
    }

    /// A list keeps its measured height still while its knob is dragged.
    fn drag(&self, started: bool) {
        if let Self::List(list) = self {
            if started {
                list.scrollbar_drag_started();
            } else {
                list.scrollbar_drag_ended();
            }
        }
    }
}

/// Where the knob sits in a track `extent.view` long.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Knob {
    top: Pixels,
    length: Pixels,
    /// How far the knob can travel.
    travel: Pixels,
}

fn knob(extent: Extent) -> Option<Knob> {
    let Extent {
        view,
        reach,
        scrolled,
    } = extent;
    let room = view - px(INSET * 2.0);
    if reach <= px(0.5) || room <= px(KNOB_MIN) {
        return None;
    }
    let length = (room * (view / (view + reach))).clamp(px(KNOB_MIN), room);
    let travel = room - length;
    let top = px(INSET) + travel * (scrolled / reach).clamp(0.0, 1.0);
    Some(Knob {
        top,
        length,
        travel,
    })
}

/// How far the view scrolls with the knob's top at `top`.
fn scrolled_at(top: Pixels, knob: Knob, reach: Pixels) -> Pixels {
    if knob.travel <= Pixels::ZERO {
        return Pixels::ZERO;
    }
    reach * ((top - px(INSET)) / knob.travel).clamp(0.0, 1.0)
}

/// How much of an overlay bar shows, `since` it last moved.
fn overlay_presence(since: Option<Instant>, now: Instant) -> f32 {
    let Some(since) = since else {
        return 0.0;
    };
    let idle = now.saturating_duration_since(since);
    match idle.checked_sub(OVERLAY_IDLE) {
        None => 1.0,
        Some(fading) => 1.0 - (fading.as_secs_f32() / OVERLAY_FADE.as_secs_f32()).min(1.0),
    }
}

/// A bar's state between frames.
#[derive(Default)]
struct Bar {
    scrolled: Pixels,
    moved_at: Option<Instant>,
    hovered: bool,
    /// While the knob is dragged: where the pointer holds it.
    grab: Option<Pixels>,
    /// The track as last painted.
    track: Rc<Cell<Bounds<Pixels>>>,
}

/// `scroller`, a scrolling element that fills its frame, with its bar: the
/// frame takes the scroller's place in a flex column. The bar cannot be the
/// scroller's own child, which would scroll with the content.
pub fn framed(
    id: impl Into<ElementId>,
    source: impl Into<ScrollSource>,
    scroller: impl IntoElement,
) -> gpui::Div {
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .size_full()
        .min_h_0()
        .min_w_0()
        .child(scroller)
        .child(ScrollBar::new(id, source))
}

/// A scrolling `div` with its bar, for views that keep no scroll handle of
/// their own: the handle lives in the element's state.
#[derive(IntoElement)]
pub struct Scrolled {
    id: ElementId,
    scroller: gpui::Stateful<gpui::Div>,
    /// The scroller's own right padding, when its rows reach the edge and
    /// it keeps the bar's room beside them.
    gutter: Option<Pixels>,
}

impl Scrolled {
    /// `scroller` scrolls (`overflow_y_scroll`) and fills its frame.
    pub fn new(id: impl Into<ElementId>, scroller: gpui::Stateful<gpui::Div>) -> Self {
        Self {
            id: id.into(),
            scroller,
            gutter: None,
        }
    }

    /// Keeps `gutter()` at the scroller's right, on top of its own `padding`
    /// (which this sets, so the scroller must not set its right padding).
    pub fn gutter(mut self, padding: Pixels) -> Self {
        self.gutter = Some(padding);
        self
    }
}

impl RenderOnce for Scrolled {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let handle = window
            .use_keyed_state((self.id.clone(), "scroll"), cx, |_, _| ScrollHandle::new())
            .read(cx)
            .clone();
        let scroller = match self.gutter {
            Some(padding) => self.scroller.pr(padding + gutter(&handle)),
            None => self.scroller,
        };
        framed(self.id, &handle, scroller.track_scroll(&handle))
    }
}

/// The vertical scroll bar of `source`, along the right edge of the nearest
/// `relative()` ancestor, which should be the scroll view's frame.
#[derive(IntoElement)]
pub struct ScrollBar {
    id: ElementId,
    source: ScrollSource,
}

impl ScrollBar {
    pub fn new(id: impl Into<ElementId>, source: impl Into<ScrollSource>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
        }
    }
}

impl RenderOnce for ScrollBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| Bar::default());
        let extent = self.source.extent();
        let Some(knob) = knob(extent) else {
            return div().into_any_element();
        };
        let now = Instant::now();
        state.update(cx, |bar, _| {
            if (bar.scrolled - extent.scrolled).abs() > px(0.5) {
                bar.scrolled = extent.scrolled;
                bar.moved_at = Some(now);
            }
        });
        let (hovered, dragging, moved_at) = {
            let bar = state.read(cx);
            (bar.hovered, bar.grab.is_some(), bar.moved_at)
        };
        let style = system_style();
        let active = hovered || dragging;
        let presence = match style {
            Style::Legacy => 1.0,
            Style::Overlay if active => 1.0,
            Style::Overlay => overlay_presence(moved_at, now),
        };
        if presence <= 0.0 {
            return div().into_any_element();
        }
        if presence < 1.0 || (style == Style::Overlay && !active) {
            // Until the overlay bar has faded.
            window.request_animation_frame();
        }

        let fg = cx.theme().colors.fg;
        let knob_width = if style == Style::Overlay && active {
            KNOB_HOVER
        } else {
            KNOB
        };
        let ink = fg.opacity(match (style, active) {
            (_, true) => 0.5,
            (Style::Legacy, false) => 0.32,
            (Style::Overlay, false) => 0.4,
        });
        let source = self.source.clone();
        let press = {
            let (state, source) = (state.clone(), self.source.clone());
            move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                cx.stop_propagation();
                let at = event.position.y - state.read(cx).track.get().top();
                if at >= knob.top && at < knob.top + knob.length {
                    source.drag(true);
                    state.update(cx, |bar, cx| {
                        bar.grab = Some(at - knob.top);
                        cx.notify();
                    });
                    return;
                }
                // AppKit's default: a click in the track pages toward it.
                let page = extent.view - px(20.0);
                let scrolled = if at < knob.top {
                    extent.scrolled - page
                } else {
                    extent.scrolled + page
                };
                source.scroll_to(scrolled.clamp(Pixels::ZERO, extent.reach));
                state.update(cx, |_, cx| cx.notify());
            }
        };
        let listen = {
            let state = state.clone();
            let track = state.read(cx).track.clone();
            move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
                track.set(bounds);
                listen_while_dragging(&state, &source, knob, extent.reach, bounds, window)
            }
        };
        let hover = state.clone();
        div()
            .id(self.id)
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .w(px(TRACK))
            .opacity(presence)
            .cursor(CursorStyle::Arrow)
            // The content under the bar takes no press, but still scrolls.
            .block_mouse_except_scroll()
            .on_hover(move |inside, _, cx| {
                hover.update(cx, |bar, cx| {
                    bar.hovered = *inside;
                    cx.notify();
                })
            })
            .on_mouse_down(MouseButton::Left, press)
            .child(
                div()
                    .absolute()
                    .top(knob.top)
                    .right(px((TRACK - knob_width) / 2.0))
                    .w(px(knob_width))
                    .h(knob.length)
                    .rounded_full()
                    .bg(ink),
            )
            .child(canvas(|_, _, _| (), listen).absolute().size_full())
            .into_any_element()
    }
}

/// While the knob is held, follows the pointer until it is let go
/// (anywhere in the window).
fn listen_while_dragging(
    state: &gpui::Entity<Bar>,
    source: &ScrollSource,
    knob: Knob,
    reach: Pixels,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let (follow, release) = (state.clone(), state.clone());
    let (moved, ended) = (source.clone(), source.clone());
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let Some(grab) = follow.read(cx).grab else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            end_drag(&follow, &moved, cx);
            return;
        }
        let top = event.position.y - bounds.top() - grab;
        moved.scroll_to(scrolled_at(top, knob, reach));
        follow.update(cx, |_, cx| cx.notify());
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            end_drag(&release, &ended, cx);
        }
    });
}

fn end_drag(state: &gpui::Entity<Bar>, source: &ScrollSource, cx: &mut App) {
    state.update(cx, |bar, cx| {
        if bar.grab.take().is_some() {
            source.drag(false);
            cx.notify();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(view: f32, reach: f32, scrolled: f32) -> Extent {
        Extent {
            view: px(view),
            reach: px(reach),
            scrolled: px(scrolled),
        }
    }

    #[test]
    fn the_knob_shows_the_share_in_view_and_where_it_is() {
        // 100 of 400 in view: a quarter of the room, at the top.
        let at_top = knob(extent(100.0, 300.0, 0.0)).unwrap();
        assert_eq!(at_top.length, px(24.0));
        assert_eq!(at_top.top, px(INSET));
        let at_end = knob(extent(100.0, 300.0, 300.0)).unwrap();
        assert_eq!(at_end.top + at_end.length, px(100.0 - INSET));
    }

    #[test]
    fn a_long_view_keeps_a_knob_big_enough_to_grab() {
        let knob = knob(extent(200.0, 100_000.0, 0.0)).unwrap();
        assert_eq!(knob.length, px(KNOB_MIN));
    }

    #[test]
    fn nothing_to_scroll_draws_no_knob() {
        assert_eq!(knob(extent(100.0, 0.0, 0.0)), None);
        assert_eq!(knob(extent(10.0, 50.0, 0.0)), None);
    }

    #[test]
    fn dragging_the_knob_maps_back_to_the_offset() {
        let k = knob(extent(100.0, 300.0, 0.0)).unwrap();
        assert_eq!(scrolled_at(px(INSET), k, px(300.0)), px(0.0));
        assert_eq!(scrolled_at(px(INSET) + k.travel, k, px(300.0)), px(300.0));
        assert_eq!(scrolled_at(px(500.0), k, px(300.0)), px(300.0));
        let half = scrolled_at(px(INSET) + k.travel / 2.0, k, px(300.0));
        assert!((half - px(150.0)).abs() < px(0.01), "{half:?}");
    }

    #[test]
    fn an_overlay_bar_stays_then_fades() {
        let now = Instant::now();
        assert_eq!(overlay_presence(None, now), 0.0);
        assert_eq!(overlay_presence(Some(now), now), 1.0);
        let fading = now + OVERLAY_IDLE + OVERLAY_FADE / 2;
        assert!((overlay_presence(Some(now), fading) - 0.5).abs() < 0.01);
        assert_eq!(
            overlay_presence(Some(now), now + OVERLAY_IDLE + OVERLAY_FADE),
            0.0
        );
    }
}
