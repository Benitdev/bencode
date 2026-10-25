use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div, prelude::*,
};
use smallvec::SmallVec;

use crate::{
    buttons::{ButtonVariant, IconButton},
    forms::Run,
    primitives::{Icon, IconName, Severity},
    theme::{ActiveTheme, ControlSize, IconSize, Radius, TextSize},
};

/// A ghost close button that keeps its row's height.
fn dismiss(id: ElementId, run: Run) -> impl IntoElement {
    div().my_neg_1().child(
        IconButton::new(id, IconName::X)
            .variant(ButtonVariant::Ghost)
            .size(ControlSize::Sm)
            .on_click(move |_, window, cx| run(window, cx)),
    )
}

/// A message in a page's flow: the severity's icon on its tint, a title, a line, actions and a close button.
#[derive(IntoElement)]
pub struct Alert {
    id: ElementId,
    severity: Severity,
    title: SharedString,
    body: Option<SharedString>,
    actions: SmallVec<[AnyElement; 2]>,
    on_dismiss: Option<Run>,
}

impl Alert {
    pub fn new(
        id: impl Into<ElementId>,
        severity: Severity,
        title: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            severity,
            title: title.into(),
            body: None,
            actions: SmallVec::new(),
            on_dismiss: None,
        }
    }

    pub fn body(mut self, text: impl Into<SharedString>) -> Self {
        self.body = Some(text.into());
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Alert {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let close = (self.id.clone(), "dismiss");
        div()
            .id(self.id)
            .flex()
            .items_start()
            .gap_3()
            .p_3()
            .rounded(theme.radius(Radius::Lg))
            .bg(self.severity.subtle(colors))
            .text_size(theme.text_size(TextSize::Base))
            .child(
                div().mt_0p5().child(
                    Icon::new(self.severity.icon())
                        .size(IconSize::Md)
                        .color(self.severity.color(colors)),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(colors.fg)
                            .child(self.title),
                    )
                    .when_some(self.body, |text, body| {
                        text.child(div().text_color(colors.fg_muted).child(body))
                    })
                    .when(!self.actions.is_empty(), |text| {
                        text.child(div().flex().gap_2().pt_1().children(self.actions))
                    }),
            )
            .when_some(self.on_dismiss, |alert, run| {
                alert.child(dismiss(close.into(), run))
            })
    }
}
