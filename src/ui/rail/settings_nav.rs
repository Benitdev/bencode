//! MonoCode `SettingsNav`: the rail's body while Settings is open, its
//! sections by group and Back at the foot.

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px, relative,
};

use crate::app::BenCodeApp;
use crate::ui::settings_modal::{SETTINGS_GROUPS, SettingsTab};

impl BenCodeApp {
    pub(super) fn render_settings_nav(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let current = self.settings_tab;
        let groups = SETTINGS_GROUPS.iter().map(|(label, tabs)| {
            // `flex flex-col gap-px`, its `px-2 pb-1 text-xs font-semibold
            // text-content/35` heading.
            div()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg.opacity(0.35))
                        .child(*label),
                )
                .children(tabs.iter().map(|tab| {
                    let (name, icon) = tab.label_icon();
                    let tab = *tab;
                    nav_row(
                        SharedString::from(format!("settings-nav-{name}")),
                        name,
                        icon,
                        tab == current,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select_settings_tab(tab, cx)))
                }))
        });
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .flex_col()
            .child(
                // `flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto px-2 py-3`
                div()
                    .id("settings-nav")
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .flex_col()
                    .gap_5()
                    .overflow_y_scroll()
                    .px_2()
                    .py_3()
                    .children(groups),
            )
            .child(
                // `flex shrink-0 flex-col gap-px p-2`
                div().flex().flex_none().flex_col().gap(px(1.0)).p_2().child(
                    nav_row("settings-nav-back", "Back", IconName::ArrowLeft, false, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.close_surface(cx))),
                ),
            )
    }

    pub(crate) fn select_settings_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        self.settings_tab = tab;
        if tab == SettingsTab::Worktrees {
            // MonoCode's page starts on the open project, freshly listed.
            self.open_worktrees_page(cx);
        }
        cx.notify();
    }
}

/// MonoCode `NavRow`: `flex items-center gap-2 rounded-md px-2 py-1.5`,
/// `bg-selection text-content` when current, else `text-content/50
/// hover:bg-content/5 hover:text-content`; a `size-4` icon at 70%.
fn nav_row(
    id: impl Into<SharedString>,
    label: &'static str,
    icon: IconName,
    active: bool,
    cx: &Context<BenCodeApp>,
) -> gpui::Stateful<gpui::Div> {
    let theme = cx.theme();
    let fg = theme.colors.fg;
    let selection = fg.opacity(if theme.is_dark() { 0.10 } else { 0.06 });
    let id: SharedString = id.into();
    let ink = if active { fg } else { fg.opacity(0.5) };
    div()
        .id(id.clone())
        .group(id.clone())
        .flex()
        .w_full()
        .items_center()
        .gap_2()
        .rounded(px(6.0))
        .px_2()
        .py(px(6.0))
        .cursor_pointer()
        .map(|el| {
            if active {
                el.bg(selection).text_color(fg)
            } else {
                el.text_color(fg.opacity(0.5))
                    .hover(move |s| s.bg(fg.opacity(0.05)).text_color(fg))
            }
        })
        .child(
            Icon::new(icon)
                .size(IconSize::Md)
                .color(ink.opacity(0.7))
                .group_hover_color(id, fg.opacity(0.7)),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_size(px(14.0))
                .font_weight(FontWeight::MEDIUM)
                .line_height(relative(1.25))
                .child(label),
        )
}
