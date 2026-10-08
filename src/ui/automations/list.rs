//! MonoCode `AutomationsContent`'s `<aside>` and `AutomationCard`: the
//! filter, New automation, and one card per automation.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::forms::Switch;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{AnyElement, Context, FontWeight, IntoElement, ParentElement, Styled, div, prelude::*};

use crate::app::automations::automation_matches;
use crate::app::{BenCodeApp, now_ms};
use crate::db::AutomationRow;
use crate::harness::catalog;
use crate::schedule::schedule_label;
use crate::ui::mascot::pixel_sprite;
use crate::ui::relative_time;
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::scrollbar::Scrolled;

impl BenCodeApp {
    pub(super) fn render_automation_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let query = self.automations.filter_input.read(cx).text().trim().to_lowercase();
        let now = now_ms();
        let cards: Vec<AnyElement> = self
            .automations
            .items
            .iter()
            .enumerate()
            .filter(|(_, auto)| {
                automation_matches(auto, &self.rail_project_label(&auto.cwd), &query)
            })
            .map(|(ix, auto)| self.render_automation_card(ix, auto, now, cx))
            .collect();
        let list = if cards.is_empty() {
            div()
                .px_3()
                .py_8()
                .text_center()
                .text_size(px(12.0))
                .text_color(colors.fg_muted)
                .child(if query.is_empty() {
                    "No automations yet"
                } else {
                    "No matching automations"
                })
                .into_any_element()
        } else {
            let cards = div()
                .id("automation-cards")
                .flex()
                .flex_col()
                .gap(px(2.0))
                .size_full()
                .p(px(6.0))
                .overflow_y_scroll()
                .children(cards);
            Scrolled::new("automation-cards-scrollbar", cards).into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .h(px(36.0))
                    .pl(px(10.0))
                    .pr_2()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(Icon::new(IconName::Search).size(IconSize::Xs).color(colors.fg_muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.0))
                            .child(self.automations.filter_input.clone()),
                    )
                    .child(
                        IconButton::new("automation-new", IconName::Plus)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("New automation")
                            .on_click(cx.listener(|this, _, _, cx| this.show_automation_picker(cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(list))
    }

    fn render_automation_card(
        &self,
        ix: usize,
        auto: &AutomationRow,
        now: i64,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let editing = self.automations.draft.as_ref().map(|d| d.id.as_str());
        let active = !self.automations.picker_open && editing == Some(auto.id.as_str());
        let (open_id, toggle_id) = (auto.id.clone(), auto.id.clone());
        let toggle = cx.listener(move |this, on: &bool, _, cx| {
            this.set_automation_enabled(&toggle_id, *on, cx)
        });
        let muted = colors.fg_muted;
        let meta = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_w_0()
            .text_size(px(11.0))
            .text_color(muted)
            .child(pixel_sprite(
                &self.project_mascot(&auto.cwd).rest,
                px(12.0),
                self.project_color(&auto.cwd),
                false,
            ))
            .child(div().min_w_0().truncate().child(self.rail_project_label(&auto.cwd)))
            .when_some(auto.last_run_at, |el, at| {
                el.child(div().flex_none().child("·"))
                    .child(div().flex_none().child(relative_time::since(at, now)))
            })
            .child(div().flex_1())
            .child(HarnessIcon::new(auto.harness.clone()).size(px(14.0)))
            .child(
                div()
                    .max_w(px(112.0))
                    .truncate()
                    .child(catalog::display_label(&auto.harness, &auto.model)),
            );
        div()
            .id(("automation-card", ix))
            .relative()
            .flex_none()
            .rounded(px(6.0))
            .map(|el| {
                if active {
                    el.bg(colors.active)
                } else {
                    el.hover(|style| style.bg(colors.hover))
                }
            })
            .child(
                div()
                    .id(("automation-card-open", ix))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .px(px(10.0))
                    .py_2()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.select_automation(&open_id, cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            // Room for the switch.
                            .pr(px(40.0))
                            .text_size(px(10.0))
                            .text_color(muted)
                            .child(Icon::new(IconName::Clock).size(IconSize::Xs).color(muted))
                            .child(div().min_w_0().truncate().child(schedule_label(auto))),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colors.fg)
                            .child(auto.name.clone()),
                    )
                    .child(meta),
            )
            .child(
                // Above the card, so a press on the switch does not open it.
                div().absolute().top(px(6.0)).right(px(8.0)).occlude().child(
                    Switch::new(("automation-card-enabled", ix), auto.enabled)
                        .on_change(move |on, window, cx| toggle(&on, window, cx)),
                ),
            )
            .into_any_element()
    }
}
