//! Top titlebar: window drag area, open thread tabs, view mode segmented switcher, and settings trigger.

use ely_gpui_component::buttons::{ButtonVariant, IconButton, SegmentedControl};
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, ControlSize, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    WindowControlArea, div, prelude::*, px,
};

use crate::app::{BenCodeApp, ViewMode};
use crate::ui::theme;

fn view_mode_key(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Chat => "chat",
        ViewMode::Changes => "changes",
        ViewMode::Terminal => "terminal",
    }
}

impl BenCodeApp {
    pub fn render_titlebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors.clone();
        let active_tab_id = self.active_tab_id.clone();
        let changed_files = self.workspace.changes.len();
        let changes_label = if changed_files > 0 {
            format!("Changes ({changed_files})")
        } else {
            "Changes".to_string()
        };

        div()
            .window_control_area(WindowControlArea::Drag)
            .flex()
            .items_center()
            .justify_between()
            .h(px(40.0))
            .w_full()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.surface)
            // macOS traffic lights spacer
            .child(div().w(px(78.0)).h_full())
            // Middle Tab Strip
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .flex_1()
                    .h_full()
                    .px_2()
                    .overflow_x_hidden()
                    .children(self.open_tabs.iter().map(|tab_id| {
                        let is_active = active_tab_id.as_deref() == Some(tab_id.as_str());
                        let session = self.sessions.iter().find(|s| &s.id == tab_id);
                        let title = session
                            .map(|s| {
                                if s.title.trim().is_empty() {
                                    "Untitled thread"
                                } else {
                                    s.title.as_str()
                                }
                            })
                            .unwrap_or("New session");
                        let harness = session.map(|s| s.harness.as_str()).unwrap_or("claude");
                        let dot_color = theme::harness_color(harness, &colors);
                        let id = tab_id.clone();
                        let close_id = tab_id.clone();

                        div()
                            .id(SharedString::from(format!("tab-bar-item-{tab_id}")))
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(28.0))
                            .max_w(px(200.0))
                            .px_2p5()
                            .rounded(cx.theme().radius(Radius::Sm))
                            .cursor_pointer()
                            .when(is_active, |el| {
                                el.bg(colors.bg)
                                    .border_1()
                                    .border_color(colors.border)
                            })
                            .when(!is_active, |el| el.hover(|s| s.bg(colors.hover)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.switch_tab(id.clone(), cx);
                            }))
                            .child(div().size(px(6.0)).rounded_full().bg(dot_color))
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .text_size(cx.theme().text_size(TextSize::Xs))
                                    .font_weight(if is_active {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(if is_active { colors.fg } else { colors.fg_muted })
                                    .child(title.to_string()),
                            )
                            .child(
                                IconButton::new(SharedString::from(format!("close-tab-{close_id}")), IconName::X)
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.close_tab(&close_id, cx);
                                    })),
                            )
                    }))
                    .child(
                        IconButton::new("titlebar-new-tab", IconName::Plus)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("New thread")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.create_new_session(cx);
                            })),
                    ),
            )
            // Right Controls: ViewMode SegmentedControl & Settings
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pr_3()
                    .child(
                        SegmentedControl::new("view-mode-switcher", view_mode_key(self.active_view_mode))
                            .size(ControlSize::Sm)
                            .segment("chat", "Chat", Some(IconName::MessageSquare))
                            .segment("changes", changes_label, Some(IconName::GitPullRequest))
                            .segment("terminal", "Terminal", Some(IconName::Terminal))
                            .on_change(cx.listener(|this, key: &SharedString, _, cx| {
                                this.active_view_mode = match key.as_ref() {
                                    "changes" => ViewMode::Changes,
                                    "terminal" => ViewMode::Terminal,
                                    _ => ViewMode::Chat,
                                };
                                cx.notify();
                            })),
                    )
                    .child(
                        IconButton::new("titlebar-settings-btn", IconName::Settings)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("Settings (⌘,)")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.is_settings_open = true;
                                cx.notify();
                            })),
                    ),
            )
    }
}
