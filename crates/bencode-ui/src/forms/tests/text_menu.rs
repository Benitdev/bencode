use gpui::{
    AppContext as _, Context, Entity, InteractiveElement, IntoElement, KeyUpEvent, Keystroke,
    Modifiers, MouseButton, ParentElement, Render, Styled, TestAppContext, VisualTestContext,
    Window, div, point, px,
};

use super::setup;
use crate::forms::TextInput;

struct Field(Entity<TextInput>);

impl Render for Field {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "field".into())
            .w(px(300.0))
            .child(self.0.clone())
    }
}

/// A field holding "hello", all selected, right clicked.
fn right_clicked(cx: &mut TestAppContext) -> (Entity<TextInput>, &mut VisualTestContext) {
    setup(cx);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let input = cx.new(|cx| TextInput::new(window, cx));
        input.update(cx, |input, cx| {
            input.set_text("hello", cx);
            input.select(0..5, cx);
        });
        Field(input)
    });
    let input = view.read_with(cx, |field, _| field.0.clone());
    cx.run_until_parked();
    // The box runs the window's height; the input is its first line.
    let top = cx.debug_bounds("field").expect("the field drew").origin;
    let at = point(top.x + px(12.0), top.y + px(8.0));
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
    (input, cx)
}

fn press(key: &'static str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(key);
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse(key).unwrap(),
    });
    cx.run_until_parked();
}

#[gpui::test]
fn a_right_click_keeps_the_selection_and_cuts_from_the_menu(cx: &mut TestAppContext) {
    let (input, cx) = right_clicked(cx);
    input.read_with(cx, |input, _| {
        assert!(input.menu.is_some(), "the click opens the menu");
        assert_eq!(input.selection(), 0..5, "it lands in the selection");
    });
    cx.simulate_keystrokes("down");
    press("enter", cx);
    input.read_with(cx, |input, _| {
        assert_eq!(input.text(), "", "Cut is the first row");
        assert!(input.menu.is_none(), "picking a row closes it");
    });
    let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(copied.as_deref(), Some("hello"));
    let focused = cx.update(|window, cx| input.read(cx).focus().is_focused(window));
    assert!(focused, "the field has the keyboard back");
}

#[gpui::test]
fn escape_closes_the_menu_and_changes_nothing(cx: &mut TestAppContext) {
    let (input, cx) = right_clicked(cx);
    press("escape", cx);
    input.read_with(cx, |input, _| {
        assert!(input.menu.is_none());
        assert_eq!(input.text(), "hello");
    });
}

#[gpui::test]
fn a_masked_field_gives_nothing_away(cx: &mut TestAppContext) {
    let (input, cx) = right_clicked(cx);
    input.update(cx, |input, cx| input.set_masked(true, cx));
    let menu = input.read_with(cx, |input, _| input.editing_menu(true));
    assert_eq!(
        menu.enabled_labels(),
        ["Paste", "Select All"],
        "no Cut or Copy of a password"
    );
}
