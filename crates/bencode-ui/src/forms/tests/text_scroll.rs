use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement, Render, ScrollHandle, Styled,
    TestAppContext, Window, div, prelude::*, px,
};

use super::popup::wheel;
use super::setup;
use crate::forms::TextInput;

/// A multi-line input holding more lines than it shows, at the top of a
/// page that scrolls.
struct Notes {
    page: ScrollHandle,
    input: Entity<TextInput>,
}

impl Render for Notes {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("page")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.page)
            .child(
                div()
                    .debug_selector(|| "notes".into())
                    .w(px(300.0))
                    .child(self.input.clone()),
            )
            .child(div().h(px(2000.0)))
    }
}

fn long_notes(
    cx: &mut TestAppContext,
) -> (
    ScrollHandle,
    Entity<TextInput>,
    &mut gpui::VisualTestContext,
) {
    setup(cx);
    let page = ScrollHandle::new();
    let (view, cx) = cx.add_window_view({
        let page = page.clone();
        move |window, cx| {
            let input = cx.new(|cx| TextInput::new(window, cx).multi_line(3, 6));
            let text = (1..=40).map(|n| format!("line {n}")).collect::<Vec<_>>();
            input.update(cx, |input, cx| input.set_text(text.join("\n"), cx));
            Notes { page, input }
        }
    });
    cx.run_until_parked();
    let input = view.read_with(cx, |view, _| view.input.clone());
    (page, input, cx)
}

#[gpui::test]
fn a_long_input_shows_its_start_until_scrolled(cx: &mut TestAppContext) {
    let (_, input, cx) = long_notes(cx);
    assert_eq!(input.read_with(cx, |input, _| input.scroll.y), px(0.0));
}

#[gpui::test]
fn a_wheel_over_a_long_input_scrolls_it_not_the_page(cx: &mut TestAppContext) {
    let (page, input, cx) = long_notes(cx);
    let notes = cx.debug_bounds("notes").expect("the input is drawn");
    cx.simulate_event(wheel(notes.center()));
    cx.run_until_parked();
    assert!(
        input.read_with(cx, |input, _| input.scroll.y) > px(0.0),
        "the input scrolled, and the next frame kept it"
    );
    assert_eq!(page.offset().y, px(0.0), "the page stayed");
}
