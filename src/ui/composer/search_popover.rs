//! The frame MonoCode's composer pickers share (`McpServerPicker`,
//! `SessionFolderPicker`): a box the width of the composer, just above it,
//! with a search row (icon, field, close button), a scrolling list and an
//! optional footer. A click outside closes it.

use ely_gpui_component::forms::TextInput;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, ElementId, Entity, InteractiveElement, IntoElement, ParentElement, Pixels,
    SharedString, Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;

type Action = fn(&mut BenCodeApp, &mut Context<BenCodeApp>);

/// What a picker puts in the shared frame.
pub struct SearchPopover<'a> {
    pub id: &'static str,
    pub icon: IconName,
    pub input: &'a Entity<TextInput>,
    /// Kept on the highlighted row as the keys move it.
    pub scroll: &'a gpui::ScrollHandle,
    /// The close button: its icon and tooltip, and what it does.
    pub close: (IconName, &'static str, Action),
    /// A click outside.
    pub dismiss: Action,
    pub list_max_height: Pixels,
    /// Shown instead of rows when there are none.
    pub empty: Option<SharedString>,
    pub rows: Vec<AnyElement>,
    pub footer: Option<AnyElement>,
}

impl SearchPopover<'_> {
    pub fn render(self, cx: &Context<BenCodeApp>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let (close_icon, close_tip, close) = self.close;
        let dismiss = self.dismiss;
        let search = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py(px(6.0))
            .border_b_1()
            .border_color(colors.border)
            .child(
                Icon::new(self.icon)
                    .size(IconSize::Xs)
                    .color(fg.opacity(0.45)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.0))
                    .child(self.input.clone()),
            )
            .child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "{}-close",
                        self.id
                    ))))
                    .size(px(28.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(fg.opacity(0.08)))
                    .tooltip(Tooltip::text(close_tip))
                    .on_click(cx.listener(move |this, _, _, cx| close(this, cx)))
                    .child(
                        Icon::new(close_icon)
                            .size(IconSize::Sm)
                            .color(fg.opacity(0.45)),
                    ),
            );
        let list = div()
            .id(ElementId::from(SharedString::from(format!(
                "{}-list",
                self.id
            ))))
            .max_h(self.list_max_height)
            .overflow_y_scroll()
            .track_scroll(self.scroll)
            .p_1()
            .when_some(self.empty, |el, text| {
                el.child(
                    div()
                        .px_2()
                        .py_2()
                        .text_size(px(12.0))
                        .text_color(fg.opacity(0.5))
                        .child(text),
                )
            })
            .children(self.rows);
        div()
            .absolute()
            .bottom_full()
            .left_0()
            .right_0()
            .mb_1()
            .overflow_hidden()
            .rounded(px(8.0))
            .border_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .shadow_xl()
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| dismiss(this, cx)))
            .child(search)
            .child(list)
            .children(self.footer)
            .into_any_element()
    }
}

/// A picker footer action (MonoCode "Manage MCP Servers…").
pub fn footer_action(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    action: Action,
    cx: &Context<BenCodeApp>,
) -> AnyElement {
    let colors = &cx.theme().colors;
    let fg = colors.fg;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(colors.border)
        .text_size(px(12.0))
        .text_color(fg.opacity(0.65))
        .cursor_pointer()
        .hover(move |s| s.bg(fg.opacity(0.05)).text_color(fg))
        .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
        .child(Icon::new(icon).size(IconSize::Xs))
        .child(label)
        .into_any_element()
}
