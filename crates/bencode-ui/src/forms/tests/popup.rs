use gpui::{
    Context, IntoElement, KeyUpEvent, Keystroke, Modifiers, ParentElement, Pixels, Point, Render,
    ScrollDelta, ScrollEvent, ScrollHandle, ScrollSource, Styled, TestAppContext, TouchPhase,
    VisualTestContext, Window, div, point, prelude::*, px,
};
use jiff::civil::time;

use super::setup;
use crate::forms::{Choice, Select, TimePicker};

/// A wheel turned down by a few lines at `at`.
pub(super) fn wheel(at: Point<Pixels>) -> ScrollEvent {
    ScrollEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
        source: ScrollSource::Pointer,
    }
}

/// Opens the focused field's popup from the keyboard.
fn open_with_enter(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.focus_next(cx));
    cx.simulate_keystrokes("enter");
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("enter").unwrap(),
    });
    cx.run_until_parked();
}

/// A long select at the top of a page that scrolls.
struct Page(ScrollHandle);

impl Render for Page {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let choices = (0..40).map(|ix| Choice::new(format!("{ix}"), format!("Choice {ix}")));
        div()
            .id("page")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.0)
            .child(div().w(px(200.0)).child(Select::new("long", choices)))
            .child(div().h(px(2000.0)))
    }
}

#[gpui::test]
fn a_wheel_over_an_open_list_leaves_the_page(cx: &mut TestAppContext) {
    setup(cx);
    let page = ScrollHandle::new();
    let (_, cx) = cx.add_window_view({
        let page = page.clone();
        |_, _| Page(page)
    });
    open_with_enter(cx);
    let list = cx.debug_bounds("option-list").expect("the list opens");
    cx.simulate_event(wheel(list.center()));
    cx.run_until_parked();
    assert_eq!(page.offset().y, px(0.0), "the page stayed");
}

/// A time picker at the top of a page that scrolls.
struct Clock(ScrollHandle);

impl Render for Clock {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("page")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.0)
            .child(
                div()
                    .w(px(200.0))
                    .child(TimePicker::new("at", time(8, 0, 0, 0))),
            )
            .child(div().h(px(2000.0)))
    }
}

#[gpui::test]
fn a_wheel_over_an_open_time_picker_leaves_the_page(cx: &mut TestAppContext) {
    setup(cx);
    let page = ScrollHandle::new();
    let (_, cx) = cx.add_window_view({
        let page = page.clone();
        |_, _| Clock(page)
    });
    open_with_enter(cx);
    let minutes = cx.debug_bounds("time-minutes").expect("the picker opens");
    cx.simulate_event(wheel(minutes.center()));
    cx.run_until_parked();
    assert_eq!(page.offset().y, px(0.0), "the page stayed");
}
