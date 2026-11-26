use std::time::Duration;

use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, Styled, TestAppContext,
    VisualTestContext, Window, div, prelude::FluentBuilder, px,
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
    cx.executor().advance_clock(Duration::from_millis(250));
    settle(cx);
    let lift = first - top(cx);
    assert!(
        lift > px(0.0) && lift < crate::motion::NUDGE * 0.75,
        "the box grows with the rise, so the card starts half a nudge low: {lift:?}"
    );
}

/// A dialog whose owner will not close it yet (a submit in flight).
struct Busy {
    behind: gpui::FocusHandle,
    field: gpui::FocusHandle,
    open: bool,
}

impl Render for Busy {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(div().track_focus(&self.behind).h(px(20.0)))
            .when(self.open, |root| {
                root.child(
                    Dialog::new("busy", "Busy", |_, _| {})
                        .focus_first(self.field.clone())
                        .child(div().track_focus(&self.field).h(px(20.0))),
                )
            })
    }
}

#[gpui::test]
fn a_dialog_that_stays_open_keeps_the_focus(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let (view, cx) = cx.add_window_view(|_, cx| Busy {
        behind: cx.focus_handle(),
        field: cx.focus_handle(),
        open: false,
    });
    cx.update(|window, cx| {
        let behind = view.read(cx).behind.clone();
        window.focus(&behind, cx);
    });
    view.update(cx, |busy, cx| {
        busy.open = true;
        cx.notify();
    });
    settle(cx);
    let field = cx.update(|_, cx| view.read(cx).field.clone());
    assert!(
        cx.update(|window, _| field.is_focused(window)),
        "it opens on its field"
    );
    cx.simulate_keystrokes("escape");
    settle(cx);
    let (behind, field) = cx.update(|window, cx| {
        let busy = view.read(cx);
        (
            busy.behind.is_focused(window),
            busy.field.contains_focused(window, cx),
        )
    });
    assert!(
        !behind,
        "the page behind an open dialog must not take the keys"
    );
    assert!(field, "focus stays where it was in the dialog");
}
