//! Settings & configuration modal: provider auth (Claude, Antigravity, OpenAI),
//! MCP server configurations, skills catalog, appearance, and system status.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::layout::on_axis;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::harness::HarnessKind;
use crate::ui::theme;
use crate::workspace::BUILTIN_SKILLS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SettingsTab {
    General,
    #[default]
    Providers,
    Mcp,
    Skills,
    Appearance,
    About,
}

impl BenCodeApp {
    pub fn render_settings_modal(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors.clone();
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
                    .w(px(720.0))
                    .h(px(520.0))
                    .rounded(cx.theme().radius(Radius::Lg))
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.surface)
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    // Header Bar
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .h(px(46.0))
                            .px_4()
                            .border_b_1()
                            .border_color(colors.border)
                            .child(
                                div()
                                    .text_size(cx.theme().text_size(TextSize::Sm))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(colors.fg)
                                    .child("Settings & Configuration"),
                            )
                            .child(
                                IconButton::new("settings-close-x-btn", IconName::X)
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .tooltip("Close (Esc)")
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
                                    .w(px(190.0))
                                    .h_full()
                                    .p_2()
                                    .gap_1()
                                    .border_r_1()
                                    .border_color(colors.border)
                                    .bg(colors.bg)
                                    .child(self.render_settings_tab_btn(IconName::Settings, "General", SettingsTab::General, current_tab, cx))
                                    .child(self.render_settings_tab_btn(IconName::Key, "Providers & Keys", SettingsTab::Providers, current_tab, cx))
                                    .child(self.render_settings_tab_btn(IconName::SlidersHorizontal, "MCP Servers", SettingsTab::Mcp, current_tab, cx))
                                    .child(self.render_settings_tab_btn(IconName::Sparkles, "Skills Catalog", SettingsTab::Skills, current_tab, cx))
                                    .child(self.render_settings_tab_btn(IconName::Palette, "Appearance", SettingsTab::Appearance, current_tab, cx))
                                    .child(self.render_settings_tab_btn(IconName::Info, "About BenCode", SettingsTab::About, current_tab, cx)),
                            )
                            // Right Content Area
                            .child(
                                on_axis(div().id("settings-right-pane-scroll"))
                                    .flex_1()
                                    .p_5()
                                    .overflow_y_scroll()
                                    .bg(colors.surface)
                                    .child(match current_tab {
                                        SettingsTab::General => self.render_settings_general(cx).into_any_element(),
                                        SettingsTab::Providers => self.render_settings_providers(cx).into_any_element(),
                                        SettingsTab::Mcp => self.render_settings_mcp(cx).into_any_element(),
                                        SettingsTab::Skills => self.render_settings_skills(cx).into_any_element(),
                                        SettingsTab::Appearance => self.render_settings_appearance(cx).into_any_element(),
                                        SettingsTab::About => self.render_settings_about(cx).into_any_element(),
                                    }),
                            ),
                    ),
            )
    }

    fn render_settings_tab_btn(
        &self,
        icon: IconName,
        label: &'static str,
        tab: SettingsTab,
        active_tab: SettingsTab,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let is_active = tab == active_tab;

        div()
            .id(SharedString::from(format!("tab-btn-{:?}", tab)))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(cx.theme().radius(Radius::Md))
            .cursor_pointer()
            .when(is_active, |el| el.bg(colors.active).text_color(colors.fg))
            .when(!is_active, |el| el.text_color(colors.fg_muted).hover(|s| s.bg(colors.hover)))
            .child(
                Icon::new(icon)
                    .size(IconSize::Xs)
                    .color(if is_active { colors.accent } else { colors.fg_muted }),
            )
            .child(
                div()
                    .text_size(cx.theme().text_size(TextSize::Xs))
                    .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::NORMAL })
                    .child(label),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings_tab = tab;
                cx.notify();
            }))
    }

    fn render_settings_providers(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_size(cx.theme().text_size(TextSize::Sm))
                    .font_weight(FontWeight::BOLD)
                    .text_color(colors.fg)
                    .child("Model Provider Authentication"),
            )
            .child(
                div()
                    .text_size(cx.theme().text_size(TextSize::Xs))
                    .text_color(colors.fg_muted)
                    .child("BenCode connects directly to Anthropic, Google Antigravity, OpenAI Codex, and OpenCode harnesses."),
            )
            .children([HarnessKind::Claude, HarnessKind::Antigravity, HarnessKind::Codex, HarnessKind::OpenCode].into_iter().map(|kind| {
                let dot_color = theme::harness_color(kind.id(), colors);
                let available = self.harnesses.iter().any(|h| h.id == kind.id() && h.available);
                div()
                    .p_3()
                    .rounded(cx.theme().radius(Radius::Md))
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.bg)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().size(px(8.0)).rounded_full().bg(dot_color))
                            .child(
                                div()
                                    .text_size(cx.theme().text_size(TextSize::Xs))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(colors.fg)
                                    .child(kind.label()),
                            ),
                    )
                    .child(
                        if available {
                            Badge::new("Available (CLI)").tone(Tone::Success)
                        } else {
                            Badge::new("Not Installed").tone(Tone::Neutral)
                        }
                    )
            }))
    }

    fn render_settings_mcp(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_size(cx.theme().text_size(TextSize::Sm))
                    .font_weight(FontWeight::BOLD)
                    .text_color(colors.fg)
                    .child("Model Context Protocol (MCP) Servers"),
            )
            .child(
                div()
                    .text_size(cx.theme().text_size(TextSize::Xs))
                    .text_color(colors.fg_muted)
                    .child("Configured tool servers providing extended capabilities to all agent harnesses:"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.render_mcp_item("sqlite", "Query and inspect project databases", "Running", Tone::Success, cx))
                    .child(self.render_mcp_item("chrome-devtools", "DOM inspection, console logs & web auditing", "Running", Tone::Success, cx))
                    .child(self.render_mcp_item("github", "Fetch PRs, issues and repo metadata", "Connected", Tone::Info, cx)),
            )
    }

    fn render_mcp_item(&self, name: &'static str, desc: &'static str, status: &'static str, tone: Tone, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        div()
            .p_3()
            .rounded(cx.theme().radius(Radius::Md))
            .border_1()
            .border_color(colors.border)
            .bg(colors.bg)
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .font_family(cx.theme().mono_family.clone())
                            .text_size(cx.theme().text_size(TextSize::Xs))
                            .font_weight(FontWeight::BOLD)
                            .text_color(colors.fg)
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(cx.theme().text_size(TextSize::Xs))
                            .text_color(colors.fg_subtle)
                            .child(desc),
                    ),
            )
            .child(Badge::new(status).tone(tone))
    }

    fn render_settings_skills(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_size(cx.theme().text_size(TextSize::Sm))
                    .font_weight(FontWeight::BOLD)
                    .text_color(colors.fg)
                    .child("Built-in Agent Skills"),
            )
            .child(
                div()
                    .text_size(cx.theme().text_size(TextSize::Xs))
                    .text_color(colors.fg_muted)
                    .child("Reusable skills available in the composer via '/' prompt command:"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(BUILTIN_SKILLS.iter().map(|skill| {
                        div()
                            .p_3()
                            .rounded(cx.theme().radius(Radius::Md))
                            .border_1()
                            .border_color(colors.border)
                            .bg(colors.bg)
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::BOLD)
                                            .text_size(cx.theme().text_size(TextSize::Xs))
                                            .text_color(colors.accent)
                                            .child(format!("/{}", skill.name)),
                                    )
                                    .child(
                                        div()
                                            .text_size(cx.theme().text_size(TextSize::Xs))
                                            .text_color(colors.fg_muted)
                                            .child(skill.description),
                                    ),
                            )
                            .child(Badge::new("built-in").tone(Tone::Neutral))
                    })),
            )
    }

    fn render_settings_appearance(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_size(cx.theme().text_size(TextSize::Sm))
                    .font_weight(FontWeight::BOLD)
                    .text_color(colors.fg)
                    .child("Appearance & Display"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_3()
                    .rounded(cx.theme().radius(Radius::Md))
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.bg)
                    .child(
                        div()
                            .text_size(cx.theme().text_size(TextSize::Xs))
                            .text_color(colors.fg)
                            .child("Active Color Theme"),
                    )
                    .child(Badge::new(self.theme_name.as_str()).tone(Tone::Info)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_3()
                    .rounded(cx.theme().radius(Radius::Md))
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.bg)
                    .child(
                        div()
                            .text_size(cx.theme().text_size(TextSize::Xs))
                            .text_color(colors.fg)
                            .child("Hardware Acceleration"),
                    )
                    .child(Badge::new("Apple Metal GPU (120 FPS)").tone(Tone::Success)),
            )
    }

    fn render_settings_about(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::Zap)
                            .size(IconSize::Sm)
                            .color(colors.accent),
                    )
                    .child(
                        div()
                            .text_size(cx.theme().text_size(TextSize::Md))
                            .font_weight(FontWeight::BOLD)
                            .text_color(colors.accent)
                            .child("BenCode Native Control Plane"),
                    ),
            )
            .child(
                div()
                    .text_size(cx.theme().text_size(TextSize::Xs))
                    .text_color(colors.fg_muted)
                    .child("Comprehensive 100% Rust + GPUI + Ely migration of MonoCode.\nBuilt with high UI/UX parity, instant Apple Metal rendering, and embedded PTY terminal."),
            )
            .child(
                div()
                    .pt_2()
                    .text_size(cx.theme().text_size(TextSize::Xs))
                    .text_color(colors.fg_subtle)
                    .child("Repository: https://github.com/Kozocom-ThienPV/bencode\nVersion: 0.1.0-alpha"),
            )
    }

    fn render_settings_general(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_size(cx.theme().text_size(TextSize::Sm))
                    .text_color(colors.fg)
                    .child("General Settings"),
            )
            // Default Model Setting Row
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_3()
                    .rounded(cx.theme().radius(Radius::Md))
                    .bg(colors.bg)
                    .border_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(cx.theme().text_size(TextSize::Xs))
                                    .text_color(colors.fg)
                                    .child("Default Agent Model"),
                            )
                            .child(
                                div()
                                    .text_size(cx.theme().text_size(TextSize::Xs))
                                    .text_color(colors.fg_muted)
                                    .child("Model used when initiating new workspace threads"),
                            ),
                    )
                    .child(Badge::new(self.selected_model.clone()).tone(Tone::Neutral)),
            )
    }
}
