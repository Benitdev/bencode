use gpui::{
    Context, IntoElement, KeyUpEvent, Keystroke, ParentElement, Render, Styled, TestAppContext,
    Window, div,
};
use jiff::civil::{Date, date};

use super::setup;
use crate::forms::{Calendar, DatePicker};

const TODAY: Date = date(2026, 9, 25);

struct Day {
    value: Option<Date>,
    disabled: bool,
}

impl Render for Day {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        div().size_full().child(
            DatePicker::new("day", self.value)
                .today(TODAY)
                .disabled(self.disabled)
                .on_change(move |picked, _, cx| {
                    view.update(cx, |view, cx| {
                        view.value = Some(picked);
                        cx.notify();
                    })
                }),
        )
    }
}

#[gpui::test]
fn a_date_picker_opens_and_picks_from_the_keyboard(cx: &mut TestAppContext) {
    setup(cx);
    let (view, cx) = cx.add_window_view(|_, _| Day {
        value: Some(date(2026, 9, 10)),
        disabled: false,
    });
    cx.update(|window, cx| window.focus_next(cx));
    cx.simulate_keystrokes("enter right enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.value),
        Some(date(2026, 9, 11))
    );
}

struct Bounded {
    picked: Option<Date>,
    max: Option<Date>,
}

impl Render for Bounded {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let calendar = Calendar::new("bounded").selected(TODAY).today(TODAY);
        let calendar = match self.max {
            Some(max) => calendar.max(max),
            None => calendar,
        };
        calendar.on_pick(move |picked, _, cx| {
            view.update(cx, |view, cx| {
                view.picked = Some(picked);
                cx.notify();
            })
        })
    }
}

#[gpui::test]
fn a_calendar_refuses_days_past_its_max(cx: &mut TestAppContext) {
    setup(cx);
    let (view, cx) = cx.add_window_view(|_, _| Bounded {
        picked: None,
        max: Some(TODAY),
    });
    cx.update(|window, cx| window.focus_next(cx));
    cx.simulate_keystrokes("right enter");
    assert_eq!(view.read_with(cx, |view, _| view.picked), None);
    cx.simulate_keystrokes("left enter");
    assert_eq!(view.read_with(cx, |view, _| view.picked), Some(TODAY));
}

#[gpui::test]
fn a_picker_disabled_while_open_stops_picking(cx: &mut TestAppContext) {
    setup(cx);
    let (view, cx) = cx.add_window_view(|_, _| Day {
        value: Some(date(2026, 9, 10)),
        disabled: false,
    });
    cx.update(|window, cx| window.focus_next(cx));
    cx.simulate_keystrokes("enter");
    view.update(cx, |view, cx| {
        view.disabled = true;
        cx.notify();
    });
    cx.simulate_keystrokes("right enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.value),
        Some(date(2026, 9, 10))
    );
}

#[gpui::test]
fn the_month_buttons_carry_the_cursor(cx: &mut TestAppContext) {
    setup(cx);
    let (view, cx) = cx.add_window_view(|_, _| Bounded {
        picked: None,
        max: None,
    });
    cx.update(|window, cx| {
        window.focus_next(cx);
        window.focus_next(cx);
        window.focus_next(cx);
    });
    cx.simulate_keystrokes("enter");
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("enter").unwrap(),
    });
    cx.update(|window, cx| {
        window.focus_prev(cx);
        window.focus_prev(cx);
    });
    cx.simulate_keystrokes("enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.picked),
        Some(date(2026, 10, 25))
    );
}
