use ely_gpui_component::theme::{ActiveTheme, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Styled, div,
    px,
};

use crate::app::BenCodeApp;
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn render_project_rail(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_between()
            .w(px(52.0))
            .h_full()
            .pt(px(48.0))
            .pb_4()
            .border_r_1()
            .border_color(MonoTheme::border_stroke())
            .bg(MonoTheme::bg_surface())
            // Top Rail Icons (Workspace / Projects)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    // Project Avatar Icon (Active)
                    .child(
                        div()
                            .id("rail-project-active")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .bg(MonoTheme::accent())
                            .text_color(MonoTheme::on_accent())
                            .font_weight(FontWeight::BOLD)
                            .text_size(theme.text_size(TextSize::Sm))
                            .cursor_pointer()
                            .child("BC"),
                    )
                    // Folder / Projects
                    .child(
                        div()
                            .id("rail-folder-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(MonoTheme::fg_muted())
                            .child("📁"),
                    )
                    // Automations / Zap
                    .child(
                        div()
                            .id("rail-automations-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(MonoTheme::fg_muted())
                            .child("⚡"),
                    )
                    // Inbox / Reminders
                    .child(
                        div()
                            .id("rail-inbox-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(MonoTheme::fg_subtle())
                            .child("📥"),
                    ),
            )
            // Bottom Rail Icons (Settings / Help)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("rail-settings-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .text_color(MonoTheme::fg_muted())
                            .child("⚙️"),
                    ),
            )
    }
}
