use std::rc::Rc;

use gpui::{
    Animation, AnimationExt, AnyElement, AnyView, App, AppContext, Context, FontWeight, Hsla,
    IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
    prelude::FluentBuilder as _, px, relative,
};

use crate::{
    motion,
    primitives::frost,
    theme::{ActiveTheme, Elevation, Radius, TextSize},
};

type Content = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// A tooltip's card: frosted glass like the other popups
/// (`primitives::frost`), the tooltip fill where there is no blur. Text
/// wraps at the tooltip's width; a card that `fits` (columns that never
/// wrap) takes its content's.
fn surface(content: AnyElement, fits: bool, window: &Window, cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let colors = &theme.colors;
    let radius = theme.radius(Radius::Md).to_pixels(window.rem_size());
    let rise = motion::NUDGE;
    let body = div()
        .relative()
        .when(!fits, |el| el.max_w(theme.tooltip_max_width()))
        .px_2p5()
        .py_1p5()
        .rounded(radius)
        .border_1()
        .border_color(colors.border)
        .text_color(colors.tooltip_fg)
        .text_size(theme.text_size(TextSize::Sm))
        .line_height(relative(1.45))
        .shadow(theme.elevation(Elevation::Floating))
        .child(frost::popover(radius - px(1.0), colors.tooltip_bg, cx))
        .child(div().relative().child(content));
    body.with_animation(
        "tooltip-in",
        Animation::new(motion::duration(motion::FAST, cx)).with_easing(motion::ease_out_cubic),
        move |body, t| body.opacity(t).mt(rise * (1.0 - t)),
    )
}

/// What a tooltip says a step below its first line.
fn quiet(cx: &App) -> Hsla {
    cx.theme().colors.tooltip_fg.opacity(0.62)
}

/// The first line of a tooltip that says more than one thing.
fn heading(text: SharedString) -> impl IntoElement {
    div().font_weight(FontWeight::MEDIUM).child(text)
}

/// A row of [`Tooltip::details`]: a label, its value, and a note after.
#[derive(Clone, Debug, PartialEq)]
pub struct TipRow {
    pub label: SharedString,
    pub value: SharedString,
    pub note: Option<SharedString>,
}

/// Tooltip body for gpui's `.tooltip()`.
pub struct Tooltip {
    content: Content,
    fits: bool,
}

impl Tooltip {
    /// One line as it is; several (split on `\n`) as a heading over the
    /// rest, a step quieter.
    pub fn text(title: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
        let title: SharedString = title.into();
        let mut lines = title
            .lines()
            .map(|line| SharedString::from(line.to_string()));
        let head = lines.next().unwrap_or_default();
        let rest: Vec<SharedString> = lines.collect();
        Self::rich(move |_, cx| {
            if rest.is_empty() {
                return title.clone().into_any_element();
            }
            let quiet = quiet(cx);
            div()
                .child(heading(head.clone()))
                .children(
                    rest.iter()
                        .map(|line| div().mt(px(2.0)).text_color(quiet).child(line.clone())),
                )
                .into_any_element()
        })
    }

    /// An optional heading over rows whose labels, values and notes line
    /// up in columns, a faint `·` between them: the label and the note
    /// quiet, the value in full.
    pub fn details(
        title: Option<SharedString>,
        rows: Vec<TipRow>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView {
        Self::build(true, move |_, cx| {
            let quiet = quiet(cx);
            let column = |cells: Vec<Option<SharedString>>| {
                div().flex_none().whitespace_nowrap().children(
                    cells
                        .into_iter()
                        .map(|cell| div().child(cell.unwrap_or_default())),
                )
            };
            let notes: Vec<Option<SharedString>> = rows.iter().map(|r| r.note.clone()).collect();
            let has_notes = notes.iter().any(Option::is_some);
            // A dot on each row that has the column after it.
            let dots = |shown: Vec<bool>| {
                column(
                    shown
                        .into_iter()
                        .map(|shown| shown.then(|| SharedString::from("·")))
                        .collect(),
                )
                .text_color(cx.theme().colors.tooltip_fg.opacity(0.3))
            };
            div()
                .when_some(title.clone(), |el, title| el.child(heading(title)))
                .child(
                    div()
                        .when(title.is_some(), |el| el.mt(px(4.0)))
                        .flex()
                        .gap_1p5()
                        .child(
                            column(rows.iter().map(|r| Some(r.label.clone())).collect())
                                .text_color(quiet),
                        )
                        .child(dots(vec![true; rows.len()]))
                        .child(
                            column(rows.iter().map(|r| Some(r.value.clone())).collect())
                                .font_weight(FontWeight::MEDIUM),
                        )
                        .when(has_notes, |el| {
                            el.child(dots(notes.iter().map(Option::is_some).collect()))
                                .child(column(notes.clone()).text_color(quiet))
                        }),
                )
                .into_any_element()
        })
    }

    pub fn rich(
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView {
        Self::build(false, content)
    }

    fn build(
        fits: bool,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView {
        let content: Content = Rc::new(content);
        move |_, cx| {
            let content = content.clone();
            cx.new(|_| Tooltip { content, fits }).into()
        }
    }
}

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = (self.content)(window, cx);
        div()
            .pt(cx.theme().cursor_offset())
            .child(surface(content, self.fits, window, cx))
    }
}
