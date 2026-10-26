use gpui::{
    Axis, Context, InteractiveElement, IntoElement, Modifiers, MouseButton, ParentElement, Pixels,
    Point, Render, Styled, TestAppContext, VisualTestContext, Window, div, point, px,
};

use super::SplitPane;
use crate::theme::Theme;

/// Three panes given two sizes.
struct Lagging;

impl Render for Lagging {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let pane = |name: &'static str| div().debug_selector(move || name.into()).size_full();
        div().w(px(300.0)).h(px(100.0)).child(
            SplitPane::new("lagging", Axis::Horizontal, px(10.0))
                .sizes(&[1.0, 3.0])
                .pane(pane("first"))
                .pane(pane("second"))
                .pane(pane("third")),
        )
    }
}

#[gpui::test]
fn sizes_that_lag_their_panes_split_evenly(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let (_, cx) = cx.add_window_view(|_, _| Lagging);
    cx.run_until_parked();
    let widths =
        ["first", "second", "third"].map(|name| cx.debug_bounds(name).expect(name).size.width);
    assert_eq!(widths[0], widths[2]);
    assert_eq!(widths[1], widths[2]);
}

/// Two panes that may grow a third, with sizes that may turn invalid.
struct Changing {
    grown: bool,
    invalid: bool,
}

impl Render for Changing {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let pane = |name: &'static str| div().debug_selector(move || name.into()).size_full();
        let sizes = if self.invalid { [0.0, 0.0] } else { [1.0, 1.0] };
        let mut split = SplitPane::new("changing", Axis::Horizontal, px(10.0))
            .sizes(&sizes)
            .pane(pane("first"))
            .pane(pane("second"));
        if self.grown {
            split = split.pane(pane("third"));
        }
        div().w(px(300.0)).h(px(100.0)).child(split)
    }
}

fn drag_divider(cx: &mut VisualTestContext) -> Point<Pixels> {
    cx.run_until_parked();
    let first = cx.debug_bounds("first").expect("first");
    let second = cx.debug_bounds("second").expect("second");
    let grab = point((first.right() + second.left()) / 2.0, px(50.0));
    cx.simulate_mouse_move(grab, None, Modifiers::none());
    cx.simulate_mouse_down(grab, MouseButton::Left, Modifiers::none());
    for step in [8.0, 8.0, 16.0] {
        cx.simulate_mouse_move(
            grab + point(px(step), px(0.0)),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.run_until_parked();
    }
    let moved = cx.debug_bounds("first").expect("first").size.width;
    assert!(moved > first.size.width, "the drag moved the divider");
    grab
}

#[gpui::test]
fn a_pane_added_mid_drag_drops_the_drag(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let (host, cx) = cx.add_window_view(|_, _| Changing {
        grown: false,
        invalid: false,
    });
    let grab = drag_divider(cx);
    host.update(cx, |host, cx| {
        host.grown = true;
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_mouse_move(
        grab + point(px(24.0), px(0.0)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("third").is_some());
}

#[gpui::test]
fn invalid_sizes_after_a_drag_split_evenly(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let (host, cx) = cx.add_window_view(|_, _| Changing {
        grown: false,
        invalid: false,
    });
    let grab = drag_divider(cx);
    cx.simulate_mouse_up(
        grab + point(px(16.0), px(0.0)),
        MouseButton::Left,
        Modifiers::none(),
    );
    host.update(cx, |host, cx| {
        host.invalid = true;
        cx.notify();
    });
    cx.run_until_parked();
    let first = cx.debug_bounds("first").expect("first").size.width;
    let second = cx.debug_bounds("second").expect("second").size.width;
    assert_eq!(first, second);
}
