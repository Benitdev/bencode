use ely_gpui_component::{
    layout::on_axis,
    theme::{ActiveTheme, Radius, TextSize},
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::ui::theme::MonoTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Providers,
    Mcp,
    Appearance,
    About,
}

impl BenCodeApp {
    pub fn render_settings_modal(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let current_tab = self.settings_tab;

        div()
            .absolute()
            .inset_0()
            .bg(gpui::rgba(0x00000099))
            .flex()
            .items_center()
            .justify_center()
            .child(
                // Modal Card
                div()
                    .w(px(680.0))
                    .h(px(480.0))
                    .rounded(theme.radius(Radius::Lg))
                    .border_1()
                    .border_color(MonoTheme::border_stroke())
                    .bg(MonoTheme::bg_surface())
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    // Header Bar
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .h(px(44.0))
                            .px_4()
                            .border_b_1()
                            .border_color(MonoTheme::border_stroke())
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Sm))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(MonoTheme::fg_primary())
                                    .child("Settings & Configuration"),
                            )
                            .child(
                                div()
                                    .id("settings-close-x-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(24.0))
                                    .rounded(theme.radius(Radius::Sm))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(MonoTheme::bg_hover()))
                                    .text_color(MonoTheme::fg_muted())
                                    .child("×")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.is_settings_open = false;
                                        cx.notify();
                                    })),
                            ),
                    )
                    // Body: Left Tab Sidebar + Right Content Pane
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .overflow_hidden()
                            // Left Tab Buttons
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .w(px(180.0))
                                    .h_full()
                                    .p_2()
                                    .gap_1()
                                    .border_r_1()
                                    .border_color(MonoTheme::border_stroke())
                                    .bg(MonoTheme::bg_base())
                                    .child(self.render_settings_tab_btn("🔑 Providers & Keys", SettingsTab::Providers, current_tab, cx))
                                    .child(self.render_settings_tab_btn("🔌 MCP Servers", SettingsTab::Mcp, current_tab, cx))
                                    .child(self.render_settings_tab_btn("🎨 Appearance", SettingsTab::Appearance, current_tab, cx))
                                    .child(self.render_settings_tab_btn("ℹ️ About BenCode", SettingsTab::About, current_tab, cx)),
                            )
                            // Right Content Area
                            .child(
                                on_axis(div().id("settings-right-pane-scroll"))
                                    .flex_1()
                                    .p_5()
                                    .overflow_y_scroll()
                                    .bg(MonoTheme::bg_surface())
                                    .child(match current_tab {
                                        SettingsTab::Providers => self.render_settings_providers(cx).into_any_element(),
                                        SettingsTab::Mcp => self.render_settings_mcp(cx).into_any_element(),
                                        SettingsTab::Appearance => self.render_settings_appearance(cx).into_any_element(),
                                        SettingsTab::About => self.render_settings_about(cx).into_any_element(),
                                    }),
                            ),
                    ),
            )
    }

    fn render_settings_tab_btn(
        &self,
        label: &'static str,
        tab: SettingsTab,
        active_tab: SettingsTab,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let is_active = tab == active_tab;

        div()
            .id(SharedString::from(format!("tab-btn-{:?}", tab)))
            .flex()
            .items_center()
            .px_3()
            .py_2()
            .rounded(theme.radius(Radius::Md))
            .cursor_pointer()
            .when(is_active, |el| el.bg(MonoTheme::bg_active()).text_color(MonoTheme::fg_primary()))
            .when(!is_active, |el| el.text_color(MonoTheme::fg_muted()).hover(|s| s.bg(MonoTheme::bg_hover())))
            .text_size(theme.text_size(TextSize::Xs))
            .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::NORMAL })
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings_tab = tab;
                cx.notify();
            }))
    }

    fn render_settings_providers(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_size(theme.text_size(TextSize::Sm))
                    .font_weight(FontWeight::BOLD)
                    .text_color(MonoTheme::fg_primary())
                    .child("Model Provider Authentication"),
            )
            .child(
                div()
                    .text_size(theme.text_size(TextSize::Xs))
                    .text_color(MonoTheme::fg_muted())
                    .child("BenCode connects directly to Anthropic, Google Antigravity, and OpenAI CLI/APIs."),
            )
            // Claude Row
            .child(
                div()
                    .p_3()
                    .rounded(theme.radius(Radius::Md))
                    .border_1()
                    .border_color(MonoTheme::border_stroke())
                    .bg(MonoTheme::bg_base())
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().size(px(8.0)).rounded_full().bg(MonoTheme::claude_orange()))
                            .child(div().text_size(theme.text_size(TextSize::Xs)).font_weight(FontWeight::BOLD).text_color(MonoTheme::fg_primary()).child("Anthropic (Claude Code)"))
                    )
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(MonoTheme::success_bg())
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::success())
                            .child("Connected (CLI)"),
                    ),
            )
            // Antigravity Row
            .child(
                div()
                    .p_3()
                    .rounded(theme.radius(Radius::Md))
                    .border_1()
                    .border_color(MonoTheme::border_stroke())
                    .bg(MonoTheme::bg_base())
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().size(px(8.0)).rounded_full().bg(MonoTheme::antigravity_blue()))
                            .child(div().text_size(theme.text_size(TextSize::Xs)).font_weight(FontWeight::BOLD).text_color(MonoTheme::fg_primary()).child("Google Antigravity"))
                    )
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(MonoTheme::success_bg())
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::success())
                            .child("Active (ACP Socket)"),
                    ),
            )
    }

    fn render_settings_mcp(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_size(theme.text_size(TextSize::Sm))
                    .font_weight(FontWeight::BOLD)
                    .text_color(MonoTheme::fg_primary())
                    .child("Model Context Protocol (MCP) Servers"),
            )
            .child(
                div()
                    .text_size(theme.text_size(TextSize::Xs))
                    .text_color(MonoTheme::fg_muted())
                    .child("Configured tool servers providing extended capabilities to all agent harnesses:"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.render_mcp_item("sqlite", "Query and inspect project databases", "Running", cx))
                    .child(self.render_mcp_item("chrome-devtools", "DOM inspection, console logs & web auditing", "Running", cx))
                    .child(self.render_mcp_item("github", "Fetch PRs, issues and repo metadata", "Connected", cx)),
            )
    }

    fn render_mcp_item(&self, name: &'static str, desc: &'static str, status: &'static str, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .p_3()
            .rounded(theme.radius(Radius::Md))
            .border_1()
            .border_color(MonoTheme::border_stroke())
            .bg(MonoTheme::bg_base())
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(div().font_family(theme.mono_family.clone()).text_size(theme.text_size(TextSize::Xs)).font_weight(FontWeight::BOLD).text_color(MonoTheme::fg_primary()).child(name))
                    .child(div().text_size(theme.text_size(TextSize::Xs)).text_color(MonoTheme::fg_subtle()).child(desc)),
            )
            .child(
                div()
                    .px_2()
                    .py_0p5()
                    .rounded(theme.radius(Radius::Sm))
                    .bg(MonoTheme::success_bg())
                    .text_size(theme.text_size(TextSize::Xs))
                    .text_color(MonoTheme::success())
                    .child(status),
            )
    }

    fn render_settings_appearance(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_size(theme.text_size(TextSize::Sm))
                    .font_weight(FontWeight::BOLD)
                    .text_color(MonoTheme::fg_primary())
                    .child("Appearance & Display"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_3()
                    .rounded(theme.radius(Radius::Md))
                    .bg(MonoTheme::bg_base())
                    .child(div().text_size(theme.text_size(TextSize::Xs)).text_color(MonoTheme::fg_primary()).child("Color Theme"))
                    .child(div().text_size(theme.text_size(TextSize::Xs)).text_color(MonoTheme::accent()).child("Dark Slate (MonoCode)")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_3()
                    .rounded(theme.radius(Radius::Md))
                    .bg(MonoTheme::bg_base())
                    .child(div().text_size(theme.text_size(TextSize::Xs)).text_color(MonoTheme::fg_primary()).child("Hardware Acceleration"))
                    .child(div().text_size(theme.text_size(TextSize::Xs)).text_color(MonoTheme::success()).child("Apple Metal GPU (Active)")),
            )
    }

    fn render_settings_about(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(theme.text_size(TextSize::Md))
                    .font_weight(FontWeight::BOLD)
                    .text_color(MonoTheme::accent())
                    .child("⚡ BenCode Native Control Plane"),
            )
            .child(
                div()
                    .text_size(theme.text_size(TextSize::Xs))
                    .text_color(MonoTheme::fg_muted())
                    .child("Comprehensive 100% Rust + GPUI migration of MonoCode.\nBuilt with high UI/UX parity, instant Apple Metal rendering, and embedded PTY terminal."),
            )
            .child(
                div()
                    .pt_2()
                    .text_size(theme.text_size(TextSize::Xs))
                    .text_color(MonoTheme::fg_subtle())
                    .child("Repository: https://github.com/Kozocom-ThienPV/bencode\nVersion: 0.1.0-alpha"),
            )
    }
}
