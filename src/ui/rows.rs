//! The list row views share: a tile, a title and a second line, the whole
//! row lit under the pointer, and buttons that show only then. A Mono's
//! Overview draws its board, documents, sessions and habits with it.
//!
//! The row owns its hover: callers pass what a click does and the buttons,
//! never a hover of their own (GPUI takes one hover per element).

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::primitives::{IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize};
use gpui::{
    AnyElement, App, ClickEvent, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::*,
};

use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct ListRow {
    id: SharedString,
    tile: Option<AnyElement>,
    title: SharedString,
    line: Option<SharedString>,
    dim: bool,
    tooltip: Option<SharedString>,
    on_click: Option<OnClick>,
    actions: Vec<AnyElement>,
}

impl ListRow {
    /// `id` must be unique among what the view draws.
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            tile: None,
            title: title.into(),
            line: None,
            dim: false,
            tooltip: None,
            on_click: None,
            actions: Vec::new(),
        }
    }

    /// What sits in the 28px tile at its left.
    pub fn tile(mut self, tile: impl IntoElement) -> Self {
        self.tile = Some(tile.into_any_element());
        self
    }

    /// The second line, under the title; left out when empty.
    pub fn line(mut self, line: impl Into<SharedString>) -> Self {
        let line = line.into();
        self.line = (!line.is_empty()).then_some(line);
        self
    }

    /// A title in muted ink: something paused or off.
    pub fn dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// A button shown while the pointer is on the row.
    pub fn action(mut self, button: impl IntoElement) -> Self {
        self.actions.push(button.into_any_element());
        self
    }
}

/// The small ghost button a row's actions use.
pub fn row_button(id: impl Into<SharedString>, icon: IconName, label: &'static str) -> IconButton {
    IconButton::new(id.into(), icon)
        .size(ControlSize::Sm)
        .variant(ButtonVariant::Ghost)
        .tooltip(label)
}

impl RenderOnce for ListRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let group = SharedString::from(format!("{}-row", self.id));
        let clickable = self.on_click.is_some();
        let body = div()
            .id(SharedString::from(format!("{}-body", self.id)))
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(10.0))
            .px_2()
            .py_2()
            .when(clickable, |el| el.cursor_pointer())
            .children(self.tile.map(|tile| {
                div()
                    .flex()
                    .flex_none()
                    .size(px(28.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .bg(fg.fill(Fill::Hover))
                    .child(tile)
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(Text::Base.px())
                            .line_height(px(20.0))
                            .text_color(fg.ink(if self.dim { Ink::Muted } else { Ink::Strong }))
                            .child(self.title),
                    )
                    .children(self.line.map(|line| {
                        div()
                            .truncate()
                            .text_size(Text::Sm.px())
                            .line_height(px(16.0))
                            .text_color(fg.ink(Ink::Muted))
                            .child(line)
                    })),
            )
            .when_some(self.tooltip, |el, text| el.tooltip(Tooltip::text(text)))
            .when_some(self.on_click, |el, handler| el.on_click(handler));
        let has_actions = !self.actions.is_empty();
        div()
            .id(group.clone())
            .group(group.clone())
            .flex()
            .w_full()
            .min_w_0()
            .items_center()
            .rounded(px(6.0))
            .hover(move |s| s.bg(fg.fill(Fill::Hover)))
            .child(body)
            .when(has_actions, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .pr_2()
                        .invisible()
                        .group_hover(group, |s| s.visible())
                        .children(self.actions),
                )
            })
    }
}
