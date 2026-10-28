use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement, IntoElement, Modifiers, MouseButton,
    ParentElement, Render, Styled, TestAppContext, TextRun, VisualTestContext, Window, div, point,
    px,
};

use gpui::prelude::FluentBuilder as _;

use super::{Document, SegKey, SelectableText, SelectionLayer, TextSelection, claim_press};
use crate::theme::Theme;

/// Two paragraphs of one document, 40px apart, then a button that claims
/// its presses; the first paragraph can be taken away.
struct Page {
    selection: Entity<TextSelection>,
    first: bool,
    changes: Rc<Cell<usize>>,
}

const FIRST: &str = "alpha beta";
const SECOND: &str = "gamma delta";

fn runs(text: &str, window: &Window) -> Vec<TextRun> {
    let style = window.text_style();
    vec![TextRun {
        len: text.len(),
        font: style.font(),
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }]
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (changes, page) = (self.changes.clone(), cx.entity().downgrade());
        let doc = Document::new("doc", &self.selection).on_change(move |cx| {
            changes.set(changes.get() + 1);
            if let Err(err) = page.update(cx, |_, cx| cx.notify()) {
                panic!("page dropped: {err}");
            }
        });
        let text = |block: usize, text: &'static str| {
            let key = SegKey { block, seq: 0 };
            div().id(("row", block)).h(px(20.)).w(px(300.)).child(
                SelectableText::new("run", &doc, key, text, runs(text, window)).quotable(true),
            )
        };
        div()
            .size_full()
            .child(SelectionLayer::new(&self.selection))
            .child(
                div()
                    .h(px(20.))
                    .when(self.first, |el| el.child(text(0, FIRST))),
            )
            .child(div().h(px(40.)))
            .child(text(1, SECOND))
            .child(div().id("button").h(px(20.)).w(px(300.)).on_mouse_down(
                MouseButton::Left,
                |_, window, cx| {
                    window.prevent_default();
                    claim_press(cx);
                },
            ))
    }
}

fn open(cx: &mut TestAppContext) -> (Entity<Page>, &mut VisualTestContext) {
    cx.update(Theme::init);
    let (page, cx) = cx.add_window_view(|_, cx| Page {
        selection: cx.new(|_| TextSelection::default()),
        first: true,
        changes: Rc::default(),
    });
    draw(cx);
    (page, cx)
}

fn drag(from: (f32, f32), to: (f32, f32), cx: &mut VisualTestContext) {
    let (from, to) = (point(px(from.0), px(from.1)), point(px(to.0), px(to.1)));
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    draw(cx);
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.run_until_parked();
}

fn selection(page: &Entity<Page>, cx: &mut VisualTestContext) -> Entity<TextSelection> {
    cx.update(|_, cx| page.read(cx).selection.clone())
}

#[gpui::test]
fn a_drag_across_paragraphs_selects_and_copies_both(cx: &mut TestAppContext) {
    let (page, cx) = open(cx);
    let sel = selection(&page, cx);
    // From the start of the first paragraph to past the end of the second.
    drag((0., 10.), (290., 70.), cx);
    cx.update(|_, cx| {
        let sel = sel.read(cx);
        assert_eq!(
            sel.selected_text(&[]).as_deref(),
            Some("alpha beta\n\ngamma delta")
        );
        // Across two blocks: nothing to quote.
        assert_eq!(sel.quote("doc"), None);
    });
    assert!(page.read_with(cx, |page, _| page.changes.get()) > 0);
}

#[gpui::test]
fn a_drag_into_the_gap_ends_the_paragraph_above(cx: &mut TestAppContext) {
    let (page, cx) = open(cx);
    let sel = selection(&page, cx);
    drag((0., 10.), (100., 40.), cx);
    cx.update(|_, cx| {
        let sel = sel.read(cx);
        assert_eq!(sel.selected_text(&[]).as_deref(), Some(FIRST));
        // Inside one block, let go: it can be quoted, over its first line.
        let (quote, line) = sel.quote("doc").expect("a quote");
        assert_eq!(quote, FIRST);
        assert_eq!(line.top(), px(0.));
    });
}

#[gpui::test]
fn a_selection_outlives_its_start_leaving_the_window(cx: &mut TestAppContext) {
    let (page, cx) = open(cx);
    let sel = selection(&page, cx);
    drag((0., 10.), (290., 70.), cx);
    // The first paragraph scrolls out of a virtual list: its element goes.
    page.update(cx, |page, cx| {
        page.first = false;
        cx.notify();
    });
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(
            sel.read(cx).selected_text(&[]).as_deref(),
            Some("alpha beta\n\ngamma delta")
        );
    });
}

#[gpui::test]
fn a_claimed_press_starts_nothing_and_any_press_clears(cx: &mut TestAppContext) {
    let (page, cx) = open(cx);
    let sel = selection(&page, cx);
    drag((0., 10.), (290., 10.), cx);
    assert!(cx.update(|_, cx| sel.read(cx).has_selection()));
    // A drag from the button over the text selects nothing.
    drag((10., 90.), (100., 70.), cx);
    cx.update(|_, cx| {
        assert!(!sel.read(cx).has_selection());
        assert!(!sel.read(cx).is_dragging_in("doc"));
    });
}

#[gpui::test]
fn a_double_click_takes_a_word_and_select_all_the_document(cx: &mut TestAppContext) {
    let (page, cx) = open(cx);
    let sel = selection(&page, cx);
    let at = point(px(2.), px(70.));
    cx.simulate_click(at, Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(sel.read(cx).selected_text(&[]).as_deref(), Some("gamma")));
    cx.update(|_, cx| TextSelection::select_all(&sel, "doc", cx));
    cx.update(|_, cx| {
        assert_eq!(
            sel.read(cx).selected_text(&[]).as_deref(),
            Some("alpha beta\n\ngamma delta")
        );
        assert!(TextSelection::clear(&sel, cx));
        assert!(!sel.read(cx).has_selection());
    });
}
