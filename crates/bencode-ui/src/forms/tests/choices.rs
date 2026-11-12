use gpui::{
    Context, IntoElement, KeyUpEvent, Keystroke, ParentElement, Render, Styled, TestAppContext,
    VisualTestContext, Window, div, px,
};

use super::setup;
use crate::forms::{Checkbox, Choice, Select, Slider};

fn release(key: &'static str, cx: &mut VisualTestContext) {
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse(key).unwrap(),
    });
}

struct Level {
    value: f64,
}

impl Render for Level {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        Slider::new("level", self.value).on_change(move |value, _, cx| {
            view.update(cx, |view, cx| {
                view.value = value;
                cx.notify();
            })
        })
    }
}

#[gpui::test]
fn slider_keys_step_jump_and_reach_the_end(cx: &mut TestAppContext) {
    setup(cx);
    let (view, cx) = cx.add_window_view(|_, _| Level { value: 10.0 });
    cx.update(|window, cx| window.focus_next(cx));
    cx.simulate_keystrokes("right pageup");
    assert_eq!(view.read_with(cx, |view, _| view.value), 21.0);
    cx.simulate_keystrokes("end");
    assert_eq!(view.read_with(cx, |view, _| view.value), 100.0);
}

struct Agree {
    on: bool,
}

impl Render for Agree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        Checkbox::new("agree", self.on)
            .label("Agree")
            .on_change(move |on, _, cx| {
                view.update(cx, |view, cx| {
                    view.on = on;
                    cx.notify();
                })
            })
    }
}

#[gpui::test]
fn space_toggles_a_focused_checkbox(cx: &mut TestAppContext) {
    setup(cx);
    let (view, cx) = cx.add_window_view(|_, _| Agree { on: false });
    cx.update(|window, cx| window.focus_next(cx));
    cx.simulate_keystrokes("space");
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("space").unwrap(),
    });
    assert!(view.read_with(cx, |view, _| view.on));
}

/// A select with nothing to pick, open or shut.
struct Empty(bool);

impl Render for Empty {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            Select::new("empty", [])
                .placeholder("Nothing yet")
                .disabled(self.0),
        )
    }
}

#[gpui::test]
fn a_shut_select_may_be_empty(cx: &mut TestAppContext) {
    setup(cx);
    let (_, cx) = cx.add_window_view(|_, _| Empty(true));
    cx.run_until_parked();
    cx.update(|window, cx| window.focus_next(cx));
    assert!(cx.update(|window, cx| window.focused(cx).is_none()));
}

#[gpui::test]
#[should_panic(expected = "has no choices")]
fn an_open_select_with_nothing_to_pick_fails(cx: &mut TestAppContext) {
    setup(cx);
    cx.add_window_view(|_, _| Empty(false));
}

/// A select whose one choice runs long.
struct Named;

impl Render for Named {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let long = Choice::new("long", "A very long microphone name that runs past the box");
        div()
            .w(px(200.0))
            .child(Select::new("long", [long, Choice::new("b", "B")]))
    }
}

#[gpui::test]
fn a_long_choice_keeps_its_list_inside_the_window(cx: &mut TestAppContext) {
    setup(cx);
    let (_, cx) = cx.add_window_view(|_, _| Named);
    cx.simulate_resize(gpui::size(px(280.0), px(600.0)));
    cx.run_until_parked();
    cx.update(|window, cx| window.focus_next(cx));
    cx.simulate_keystrokes("enter");
    release("enter", cx);
    cx.run_until_parked();
    let list = cx.debug_bounds("option-list").expect("the list opens");
    assert!(
        list.left() >= px(0.0) && list.right() <= px(280.0),
        "{list:?}"
    );
}

/// A select long enough to filter, recording what was picked.
struct Fruits(Option<String>);

const FRUITS: [&str; 12] = [
    "Banana", "Cherry", "Date", "Elder", "Fig", "Grape", "Kiwi", "Lemon", "Mango", "Apricot",
    "Melon", "Apple",
];

impl Render for Fruits {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        Select::new("fruit", FRUITS.map(|name| Choice::new(name, name))).on_change(
            move |value, _, cx| {
                view.update(cx, |view, cx| {
                    view.0 = Some(value.to_string());
                    cx.notify();
                })
            },
        )
    }
}

#[gpui::test]
fn typing_filters_a_long_select_and_enter_picks_the_match(cx: &mut TestAppContext) {
    setup(cx);
    let (view, cx) = cx.add_window_view(|_, _| Fruits(None));
    cx.update(|window, cx| window.focus_next(cx));
    // Opens, filters to Apricot, takes a letter off (Grape, Apricot and
    // Apple match "ap") and steps down to the third match.
    cx.simulate_keystrokes("down a p r backspace down down enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.0.clone()),
        Some("Apple".into())
    );
}

#[test]
fn a_filter_matches_labels_ignoring_case() {
    let rows: Vec<Choice> = FRUITS
        .iter()
        .map(|name| Choice::new(*name, *name))
        .collect();
    let names = |query| {
        crate::forms::select::matching(&rows, query)
            .into_iter()
            .map(|ix| FRUITS[ix])
            .collect::<Vec<_>>()
    };
    assert_eq!(names("ON"), ["Lemon", "Melon"]);
    assert_eq!(names("  "), FRUITS.to_vec());
    assert!(names("zz").is_empty());
}

#[gpui::test]
fn escape_leaves_the_filter_and_reopening_starts_it_empty(cx: &mut TestAppContext) {
    setup(cx);
    let (view, cx) = cx.add_window_view(|_, _| Fruits(None));
    cx.update(|window, cx| window.focus_next(cx));
    let trigger = cx.update(|window, cx| window.focused(cx));
    // Nothing matches; Escape closes and gives the select back the keys.
    cx.simulate_keystrokes("down z z escape");
    cx.run_until_parked();
    assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger);
    cx.simulate_keystrokes("down enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.0.clone()),
        Some("Banana".into())
    );
}
