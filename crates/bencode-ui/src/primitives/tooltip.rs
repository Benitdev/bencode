use std::rc::Rc;

use gpui::{
    Animation, AnimationExt, AnyElement, AnyView, App, AppContext, Context, IntoElement,
    ParentElement, Render, SharedString, Styled, Window, div, px, relative,
};

use crate::{
    motion,
    primitives::frost,
    theme::{ActiveTheme, Elevation, Radius, TextSize},
};

type Content = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// A tooltip's card: frosted glass like the other popups
/// (`primitives::frost`), the tooltip fill where there is no blur.
fn surface(content: AnyElement, window: &Window, cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let colors = &theme.colors;
    let radius = theme.radius(Radius::Md).to_pixels(window.rem_size());
    let rise = motion::NUDGE;
    let body = div()
        .relative()
        .max_w(theme.tooltip_max_width())
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
            .child(surface(content, window, cx))
    }
}
