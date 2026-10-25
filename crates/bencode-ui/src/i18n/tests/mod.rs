#[cfg(feature = "test-support")]
mod shown {
    use gpui::{
        Context, InteractiveElement, IntoElement, ParentElement, Render, Styled, TestAppContext,
        Window, div, px,
    };

    use super::super::{Direction, I18n};

    /// A row of two marks, laid out the way `I18n`'s locale reads.
    struct Row;

    impl Render for Row {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let i18n = cx.global::<I18n>();
            let direction = i18n.locale().direction;
            direction
                .row(div().w(px(280.0)).justify_between())
                .child(
                    div()
                        .debug_selector(|| "first".into())
                        .w(px(40.0))
                        .child(i18n.text("hi", &[])),
                )
                .child(div().debug_selector(|| "second".into()).w(px(40.0)))
        }
    }

    /// A new locale reaches every window, and a right-to-left one starts the row at the right.
    #[gpui::test]
    fn a_right_to_left_locale_mirrors_the_row(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(
                I18n::new("en-US")
                    .catalog("en-US", &[("hi", "Hi")])
                    .catalog("he-IL", &[("hi", "שלום")]),
            )
        });
        let (_, cx) = cx.add_window_view(|_, _| Row);
        cx.run_until_parked();
        let first = cx.debug_bounds("first").expect("the first mark");
        let second = cx.debug_bounds("second").expect("the second mark");
        assert!(first.left() < second.left(), "left to right in en-US");
        cx.update(|_, cx| I18n::set_locale("he-IL", cx));
        cx.run_until_parked();
        assert_eq!(
            cx.read(|cx| cx.global::<I18n>().text("hi", &[])),
            "\u{200f}שלום"
        );
        let first = cx.debug_bounds("first").expect("the first mark");
        let second = cx.debug_bounds("second").expect("the second mark");
        assert!(
            first.left() > second.left(),
            "right to left in he-IL: {first:?} {second:?}"
        );
    }

    #[gpui::test]
    #[should_panic(expected = "no catalog for de-DE")]
    fn a_locale_without_a_catalog_fails(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(I18n::new("en-US").catalog("en-US", &[("hi", "Hi")]));
            I18n::set_locale("de-DE", cx);
        });
    }

    #[test]
    fn text_sits_against_the_reading_side() {
        let side = |direction: Direction| direction.align(div()).style().text.text_align;
        assert_eq!(side(Direction::Ltr), Some(gpui::TextAlign::Left));
        assert_eq!(side(Direction::Rtl), Some(gpui::TextAlign::Right));
    }
}
