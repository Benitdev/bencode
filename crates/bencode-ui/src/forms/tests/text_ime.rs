use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, Entity, EntityInputHandler as _, IntoElement, ParentElement, Render,
    TestAppContext, Window, div,
};

use super::setup;
use crate::forms::{InputEvent, TextInput};

struct Field(Entity<TextInput>);

impl Render for Field {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(self.0.clone())
    }
}

/// Select a word, start composing over it, then take the composition back
/// to nothing (Backspace or Esc in a Japanese IME): the word is gone, as
/// in any macOS field, but as an edit that undoes and is announced.
#[gpui::test]
fn a_composition_cancelled_over_a_selection_is_an_edit(cx: &mut TestAppContext) {
    setup(cx);
    let changes = Rc::new(Cell::new(0));
    let (view, cx) = cx.add_window_view(|window, cx| {
        let input = cx.new(|cx| TextInput::new(window, cx));
        input.update(cx, |input, cx| {
            input.set_text("hello world", cx);
            input.select(0..5, cx);
        });
        Field(input)
    });
    let input = view.read_with(cx, |field, _| field.0.clone());
    let counted = changes.clone();
    cx.update(|_, cx| {
        cx.subscribe(&input, move |_, event: &InputEvent, _| {
            if matches!(event, InputEvent::Changed) {
                counted.set(counted.get() + 1);
            }
        })
        .detach();
    });
    cx.update(|window, cx| {
        input.update(cx, |input, cx| {
            input.replace_and_mark_text_in_range(None, "あ", None, window, cx);
            input.replace_and_mark_text_in_range(None, "", None, window, cx);
        });
    });
    cx.run_until_parked();
    input.read_with(cx, |input, _| assert_eq!(input.text(), " world"));
    assert_eq!(changes.get(), 1, "the lost word is announced once");
    cx.update(|window, cx| {
        let focus = input.read(cx).focus().clone();
        window.focus(&focus, cx);
    });
    cx.simulate_keystrokes("cmd-z");
    input.read_with(cx, |input, _| assert_eq!(input.text(), "hello world"));
}
