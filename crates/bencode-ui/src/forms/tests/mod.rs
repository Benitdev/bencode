mod choices;
mod dates;
mod popup;
use gpui::{Context, IntoElement, Render, TestAppContext, Window};

use super::{NumberInput, bind_keys};
use crate::theme::Theme;

pub(super) fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        Theme::init(cx);
        bind_keys(cx);
    });
}

struct Stepped {
    value: f64,
}

impl Render for Stepped {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        NumberInput::new("stepped", self.value).on_change(move |value, _, cx| {
            view.update(cx, |view, cx| {
                view.value = value;
                cx.notify();
            })
        })
    }
}

#[gpui::test]
fn stepping_starts_from_the_typed_number(cx: &mut TestAppContext) {
    setup(cx);
    let (view, cx) = cx.add_window_view(|_, _| Stepped { value: 4.0 });
    cx.update(|window, cx| window.focus_next(cx));
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("20");
    cx.simulate_keystrokes("up");
    assert_eq!(view.read_with(cx, |view, _| view.value), 21.0);
}

#[gpui::test]
fn a_typed_number_reaches_the_owner_before_any_blur(cx: &mut TestAppContext) {
    setup(cx);
    let (view, cx) = cx.add_window_view(|_, _| Stepped { value: 100.0 });
    cx.update(|window, cx| window.focus_next(cx));
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("250");
    assert_eq!(
        view.read_with(cx, |view, _| view.value),
        250.0,
        "a press elsewhere reads the typed number"
    );
    cx.simulate_input(".");
    assert_eq!(
        view.read_with(cx, |view, _| view.value),
        250.0,
        "a half-typed number waits"
    );
}
