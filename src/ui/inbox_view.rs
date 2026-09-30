use ely_gpui_component::{
    layout::on_axis,
    primitives::{Icon, IconName},
    theme::{ActiveTheme, IconSize, Radius, TextSize},
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::{BenCodeApp, ViewMode};
use crate::ui::theme::MonoTheme;

#[derive(Clone, Debug)]
pub enum CiCheckState {
    Passing(usize),
    Failing {
        total: usize,
        failed: usize,
        test_name: String,
    },
}

#[derive(Clone, Debug)]
pub struct InboxPrItem {
    pub id: String,
    pub number: usize,
    pub title: String,
    pub repo: String,
    pub branch: String,
    pub author: String,
    pub ci_state: CiCheckState,
    pub comments_count: usize,
    pub updated_time: String,
}

impl BenCodeApp {
    pub fn open_inbox_modal(&mut self, cx: &mut Context<Self>) {
        self.is_inbox_open = true;
        cx.notify();
    }

    pub fn close_inbox_modal(&mut self, cx: &mut Context<Self>) {
        self.is_inbox_open = false;
        cx.notify();
    }

    pub fn trigger_ci_repair(&mut self, pr_title: &str, test_name: &str, cx: &mut Context<Self>) {
        let repair_prompt = format!(
            "Inspect and repair failing CI check \"{}\" on PR \"{}\". Analyze test failure, run reproduction script, and apply code fix.",
            test_name, pr_title
        );

        // Open a fresh thread and actually run the repair prompt through the agent.
        self.create_new_session(cx);
        if let Some(session) = self.selected_session_mut() {
            session.title = format!("Repair CI: {test_name}");
            session.pinned = true;
        }
        self.active_view_mode = ViewMode::Chat;
        self.prompt_input.update(cx, |input, cx| input.set_text(repair_prompt, cx));
        self.submit_prompt(cx);
        self.close_inbox_modal(cx);
    }

    pub fn render_inbox_modal(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        // Sample PR items matching MonoCode's Inbox data structure
        let pr_items = vec![
            InboxPrItem {
                id: "pr-101".to_string(),
                number: 101,
                title: "feat(ui): migrate to 100% native Rust GPUI vector icons".to_string(),
                repo: "bencode".to_string(),
                branch: "feat/lucide-vector-icons".to_string(),
                author: "thienpv".to_string(),
                ci_state: CiCheckState::Passing(14),
                comments_count: 3,
                updated_time: "12m ago".to_string(),
            },
            InboxPrItem {
                id: "pr-102".to_string(),
                number: 102,
                title: "fix(core): resolve race condition in prompt queue handler".to_string(),
                repo: "bencode".to_string(),
                branch: "fix/queue-race".to_string(),
                author: "kozocom".to_string(),
                ci_state: CiCheckState::Failing {
                    total: 16,
                    failed: 1,
                    test_name: "test_concurrent_queue_drain".to_string(),
                },
                comments_count: 5,
                updated_time: "45m ago".to_string(),
            },
            InboxPrItem {
                id: "pr-103".to_string(),
                number: 103,
                title: "feat(db): add automated routine scheduled runs index".to_string(),
                repo: "bencode".to_string(),
                branch: "feat/db-automations".to_string(),
                author: "thienpv".to_string(),
                ci_state: CiCheckState::Passing(18),
                comments_count: 1,
                updated_time: "2h ago".to_string(),
            },
        ];

        div()
            .absolute()
            .inset_0()
            .bg(gpui::rgba(0x000000aa))
            .flex()
            .items_start()
            .justify_center()
            .pt(px(60.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(720.0))
                    .max_h(px(580.0))
                    .rounded(theme.radius(Radius::Lg))
                    .bg(MonoTheme::bg_surface())
                    .border_1()
                    .border_color(MonoTheme::border_stroke())
                    .shadow_lg()
                    .overflow_hidden()
                    // 1. Header Row
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(MonoTheme::border_stroke())
                            .bg(MonoTheme::bg_base())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        Icon::new(IconName::Inbox)
                                            .size(IconSize::Sm)
                                            .color(MonoTheme::accent()),
                                    )
                                    .child(
                                        div()
                                            .font_weight(FontWeight::BOLD)
                                            .text_size(theme.text_size(TextSize::Md))
                                            .text_color(MonoTheme::fg_primary())
                                            .child("Inbox & Pull Requests"),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded(theme.radius(Radius::Sm))
                                            .bg(MonoTheme::bg_hover())
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_muted())
                                            .child("GitHub Connected"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("close-inbox-modal-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .px_2p5()
                                    .py_1()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::bg_hover())
                                    .text_color(MonoTheme::fg_muted())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .cursor_pointer()
                                    .child("ESC")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_inbox_modal(cx);
                                    })),
                            ),
                    )
                    // 2. PR List
                    .child(
                        on_axis(div().id("inbox-prs-scroll"))
                            .flex_1()
                            .overflow_y_scroll()
                            .p_3()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .children(pr_items.into_iter().map(|pr| {
                                let pr_title = pr.title.clone();
                                div()
                                    .id(SharedString::from(format!("inbox-pr-row-{}", pr.id)))
                                    .flex()
                                    .flex_col()
                                    .p_3()
                                    .rounded(theme.radius(Radius::Md))
                                    .border_1()
                                    .border_color(MonoTheme::border_stroke())
                                    .bg(MonoTheme::bg_base())
                                    .hover(|s| s.bg(MonoTheme::bg_hover()))
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex()
                                            .items_start()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(
                                                        Icon::new(IconName::GitPullRequest)
                                                            .size(IconSize::Sm)
                                                            .color(MonoTheme::accent()),
                                                    )
                                                    .child(
                                                        div()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_size(theme.text_size(TextSize::Sm))
                                                            .text_color(MonoTheme::fg_primary())
                                                            .child(format!("#{} {}", pr.number, pr.title)),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .text_color(MonoTheme::fg_subtle())
                                                    .child(pr.updated_time),
                                            ),
                                    )
                                    // Status & CI Checks Row
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .child(
                                                        div()
                                                            .text_color(MonoTheme::fg_muted())
                                                            .child(format!("Branch: {}", pr.branch)),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_color(MonoTheme::fg_muted())
                                                            .child(format!("By: @{}", pr.author)),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap_1()
                                                            .text_color(MonoTheme::fg_muted())
                                                            .child(Icon::new(IconName::MessageSquare).size(IconSize::Xs))
                                                            .child(pr.comments_count.to_string()),
                                                    ),
                                            )
                                            .child(
                                                match pr.ci_state {
                                                    CiCheckState::Passing(checks) => {
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap_1()
                                                            .px_2()
                                                            .py_0p5()
                                                            .rounded(theme.radius(Radius::Sm))
                                                            .bg(MonoTheme::success_bg())
                                                            .text_color(MonoTheme::success())
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .child(Icon::new(IconName::Check).size(IconSize::Xs))
                                                            .child(format!("{} checks passing", checks))
                                                            .into_any_element()
                                                    }
                                                    CiCheckState::Failing { total, failed, test_name } => {
                                                        let test_name_clone = test_name.clone();
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap_2()
                                                            .child(
                                                                div()
                                                                    .px_2()
                                                                    .py_0p5()
                                                                    .rounded(theme.radius(Radius::Sm))
                                                                    .bg(MonoTheme::status_error_bg())
                                                                    .text_color(MonoTheme::status_error())
                                                                    .text_size(theme.text_size(TextSize::Xs))
                                                                    .font_weight(FontWeight::MEDIUM)
                                                                    .child(format!("{}/{} failed ({})", failed, total, test_name)),
                                                            )
                                                            .child(
                                                                div()
                                                                    .id(SharedString::from(format!("repair-pr-{}", pr.number)))
                                                                    .flex()
                                                                    .items_center()
                                                                    .gap_1()
                                                                    .px_2p5()
                                                                    .py_1()
                                                                    .rounded(theme.radius(Radius::Sm))
                                                                    .bg(MonoTheme::accent())
                                                                    .text_color(MonoTheme::on_accent())
                                                                    .text_size(theme.text_size(TextSize::Xs))
                                                                    .font_weight(FontWeight::MEDIUM)
                                                                    .cursor_pointer()
                                                                    .hover(|s| s.opacity(0.9))
                                                                    .child(Icon::new(IconName::WandSparkles).size(IconSize::Xs))
                                                                    .child("Repair with Agent")
                                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                                        this.trigger_ci_repair(&pr_title, &test_name_clone, cx);
                                                                    })),
                                                            )
                                                            .into_any_element()
                                                    }
                                                }
                                            ),
                                    )
                            })),
                    ),
            )
    }
}
