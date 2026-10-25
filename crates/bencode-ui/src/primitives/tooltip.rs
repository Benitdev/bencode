use std::rc::Rc;

use gpui::{
    Animation, AnimationExt, AnyElement, AnyView, App, AppContext, Context, IntoElement,
    ParentElement, Render, SharedString, Styled, Window, div,
};

use crate::{
    motion,
    theme::{ActiveTheme, Elevation, Radius, TextSize},
};

type Content = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

fn surface(content: AnyElement, cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let rise = motion::NUDGE;
    let body = div()
        .max_w(theme.tooltip_max_width())
        .px_2()
        .py_1()
        .rounded(theme.radius(Radius::Md))
        .bg(theme.colors.tooltip_bg)
        .text_color(theme.colors.tooltip_fg)
        .text_size(theme.text_size(TextSize::Sm))
        .shadow(theme.elevation(Elevation::Floating))
        .child(content);
    body.with_animation(
        "tooltip-in",
        Animation::new(motion::duration(motion::FAST, cx)).with_easing(motion::ease_out_cubic),
        move |body, t| body.opacity(t).mt(rise * (1.0 - t)),
    )
}

/// Tooltip body for gpui's `.tooltip()`.
pub struct Tooltip {
    content: Content,
}

impl Tooltip {
    pub fn text(title: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
        let title = title.into();
        Self::rich(move |_, _| title.clone().into_any_element())
    }

    pub fn rich(
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView {
        let content: Content = Rc::new(content);
        move |_, cx| {
            let content = content.clone();
            cx.new(|_| Tooltip { content }).into()
        }
    }
}

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = (self.content)(window, cx);
        div()
            .pt(cx.theme().cursor_offset())
            .child(surface(content, cx))
    }
}
