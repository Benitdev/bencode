//! Left pane: the user's automations, then the starter templates.

use ely_gpui_component::buttons::Button;
use ely_gpui_component::data_display::{Badge, Tag, Tone};
use ely_gpui_component::layout::{ScrollArea, Section};
use ely_gpui_component::lists::ListItem;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use ely_gpui_component::typography::Caption;
use gpui::{Context, IntoElement, ParentElement, Styled, div};

use super::schedule_label;
use super::templates::{AutomationTemplate, BUILTIN_TEMPLATES};
use crate::app::BenCodeApp;
use crate::db::AutomationRow;

impl BenCodeApp {
    pub(super) fn render_automation_master(&self, cx: &Context<Self>) -> impl IntoElement {
        let mine = if self.automations.is_empty() {
            Caption::new("No automations yet. Start from a template below.").into_any_element()
        } else {
            column(self.automations.iter().enumerate().map(|(ix, auto)| self.render_automation_item(ix, auto, cx)))
                .into_any_element()
        };
        let new = Button::new("automation-new", "New")
            .icon(IconName::Plus)
            .size(ControlSize::Sm)
            .on_click(cx.listener(|this, _, _, cx| this.create_new_automation(cx)));
        let templates = column(BUILTIN_TEMPLATES.iter().enumerate().map(|(ix, tpl)| render_template_item(ix, tpl, cx)));
        ScrollArea::new("automations-master").size_full().child(
            div()
                .flex()
                .flex_col()
                .gap_6()
                .pr_4()
                .child(Section::new("Your automations").action(new).child(mine))
                .child(Section::new("Starter templates").description("Pick one to create an automation from it.").child(templates)),
        )
    }

    fn render_automation_item(&self, ix: usize, auto: &AutomationRow, cx: &Context<Self>) -> ListItem {
        let id = auto.id.clone();
        let status = if auto.enabled {
            Badge::new("Active").tone(Tone::Success)
        } else {
            Badge::new("Paused")
        };
        ListItem::new(("automation", ix), auto.name.clone())
            .description(schedule_label(auto))
            .trailing(status)
            .selected(self.selected_automation_id.as_deref() == Some(auto.id.as_str()))
            .on_click(cx.listener(move |this, _, _, cx| this.select_automation(&id, cx)))
    }
}

fn render_template_item(ix: usize, tpl: &'static AutomationTemplate, cx: &Context<BenCodeApp>) -> ListItem {
    let accent = cx.theme().colors.warning;
    ListItem::new(("automation-template", ix), tpl.name)
        .description(tpl.description)
        .leading(Icon::new(tpl.icon).size(IconSize::Sm).color(accent))
        .trailing(Tag::new(("automation-template-tag", ix), tpl.category))
        .on_click(cx.listener(move |this, _, _, cx| this.insert_automation(tpl, cx)))
}

fn column(rows: impl IntoIterator<Item = ListItem>) -> impl IntoElement {
    div().flex().flex_col().gap_1().children(rows)
}
