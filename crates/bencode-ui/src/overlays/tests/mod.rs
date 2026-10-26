use std::time::Duration;

use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, Styled, TestAppContext,
    VisualTestContext, Window, div, px,
};

use super::Dialog;
use crate::theme::Theme;

pub(super) fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// A dialog open from the first frame.
struct Rising;

impl Render for Rising {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            Dialog::new("rising", "Rising", |_, _| {})
                .child(div().debug_selector(|| "rising-body".into()).h(px(40.0))),
        )
    }
}

#[gpui::test]
fn an_opening_dialog_rises_in_the_box_its_scrim_centers(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let (_, cx) = cx.add_window_view(|_, _| Rising);
    cx.run_until_parked();
    let top = |cx: &mut VisualTestContext| cx.debug_bounds("rising-body").expect("the body").top();
    let first = top(cx);
    std::thread::sleep(Duration::from_millis(250));
    settle(cx);
    let lift = first - top(cx);
    assert!(
        lift > px(0.0) && lift < crate::motion::NUDGE * 0.75,
        "the box grows with the rise, so the card starts half a nudge low: {lift:?}"
    );
}
