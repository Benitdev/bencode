//! MonoCode `AutomationPicker`: the New automation page, with Start from
//! scratch and the example automations by category.

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{AnyElement, Context, FontWeight, Hsla, IntoElement, ParentElement, Styled, div, prelude::*};

use super::PAGE_WIDTH;
use super::parts::tint;
use super::templates::{AutomationTemplate, TemplateCategory, templates_for};
use crate::app::BenCodeApp;
use crate::ui::scale::px;
use crate::ui::scrollbar::Scrolled;

/// MonoCode `min-h-37`.
const CARD_MIN_HEIGHT: f32 = 148.0;

/// The round icon, name and description every card starts with.
fn card_head(icon: IconName, name: &'static str, description: &'static str, fg: Hsla, muted: Hsla) -> impl IntoElement {
    div()
        .flex()
        .gap_3()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(36.0))
                .rounded_full()
                .bg(fg.opacity(tint::FILL))
                .child(Icon::new(icon).size(IconSize::Sm).color(fg.opacity(tint::BODY))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .min_w_0()
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg)
                        .child(name),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .line_height(px(17.0))
                        .text_color(muted)
                        .child(description),
                ),
        )
}

impl BenCodeApp {
    pub(super) fn render_automation_picker(&self, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let (fg, muted) = (colors.fg, colors.fg_muted);
        let category = self.automations.category;
        let pills = TemplateCategory::ALL.into_iter().enumerate().map(|(ix, option)| {
            div()
                .id(("automation-category", ix))
                .flex()
                .items_center()
                .h(px(28.0))
                .px_3()
                .rounded_full()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .map(|el| {
                    if option == category {
                        el.bg(fg).text_color(colors.bg)
                    } else {
                        el.text_color(muted)
                            .hover(|style| style.bg(fg.opacity(tint::FILL)).text_color(fg))
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.automations.category = option;
                    cx.notify();
                }))
                .child(option.label())
        });
        let blank = div()
            .id("automation-blank")
            .flex()
            .flex_col()
            .min_h(px(CARD_MIN_HEIGHT))
            .p_4()
            .rounded(px(12.0))
            .border_1()
            .border_dashed()
            .border_color(fg.opacity(tint::DASH))
            .cursor_pointer()
            .hover(|style| style.bg(fg.opacity(tint::HOVER)).border_color(fg.opacity(tint::DASH_HOVER)))
            .on_click(cx.listener(|this, _, _, cx| this.begin_blank_automation(cx)))
            .child(card_head(
                IconName::Plus,
                "Start from scratch",
                "Write your own instructions and choose a trigger.",
                fg,
                muted,
            ));
        let cards = templates_for(category)
            .enumerate()
            .map(|(ix, template)| self.render_template_card(ix, template, cx));
        let page = div()
            .id("automation-picker")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .mx_auto()
                    .w_full()
                    .max_w(px(PAGE_WIDTH))
                    .px_8()
                    .pt_5()
                    .pb_10()
                    .child(
                        div()
                            .text_size(px(20.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child("New automation"),
                    )
                    .child(
                        div()
                            .mt(px(6.0))
                            .text_size(px(13.0))
                            .text_color(muted)
                            .child("Pick an example or start from scratch."),
                    )
                    .child(div().flex().flex_wrap().gap(px(6.0)).mt_4().children(pills))
                    .child(div().grid().grid_cols(2).gap_3().mt_4().child(blank).children(cards)),
            );
        Scrolled::new("automation-picker-scrollbar", page).into_any_element()
    }

    fn render_template_card(
        &self,
        ix: usize,
        template: &'static AutomationTemplate,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let (fg, muted) = (colors.fg, colors.fg_muted);
        div()
            .id(("automation-template", ix))
            .flex()
            .flex_col()
            .min_h(px(CARD_MIN_HEIGHT))
            .p_4()
            .rounded(px(12.0))
            .border_1()
            .border_color(fg.opacity(tint::STROKE))
            .cursor_pointer()
            .hover(|style| style.bg(fg.opacity(tint::HOVER)).border_color(fg.opacity(tint::STROKE_HOVER)))
            .on_click(cx.listener(move |this, _, _, cx| this.begin_automation_from(template, cx)))
            .child(card_head(template.icon, template.name, template.description, fg, muted))
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .pt_3()
                    .text_size(px(11.0))
                    .text_color(muted)
                    .child(Icon::new(IconName::Clock).size(IconSize::Xs).color(muted))
                    .child(template.trigger_label),
            )
    }
}
