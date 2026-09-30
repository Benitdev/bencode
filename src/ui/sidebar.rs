use ely_gpui_component::{
    layout::on_axis,
    primitives::{Icon, IconName},
    theme::{ActiveTheme, IconSize, Radius, TextSize},
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::{BenCodeApp, FilterMode};
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected_id = self.selected_session_id.clone();
        let search_query = self.search_query.to_lowercase();
        let active_filter = self.filter_mode;

        // Count per filter tab
        let all_count = self.sessions.len();
        let pinned_count = self.sessions.iter().filter(|s| s.pinned).count();
        let archived_count = self.sessions.iter().filter(|s| s.archived).count();
        let active_count = self.sessions.iter().filter(|s| !s.archived).count();

        // Filter sessions by search query and filter mode
        let filtered_sessions: Vec<_> = self.sessions
            .iter()
            .filter(|s| {
                match active_filter {
                    FilterMode::All => true,
                    FilterMode::Active => !s.archived,
                    FilterMode::Pinned => s.pinned,
                    FilterMode::Archived => s.archived,
                }
            })
            .filter(|s| {
                if search_query.is_empty() {
                    true
                } else {
                    s.title.to_lowercase().contains(&search_query)
                        || s.cwd.to_lowercase().contains(&search_query)
                        || s.branch.as_deref().unwrap_or("").to_lowercase().contains(&search_query)
                }
            })
            .cloned()
            .collect();

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(280.0))
            .h_full()
            .border_r_1()
            .border_color(MonoTheme::border_stroke())
            .bg(MonoTheme::bg_surface())
            // 1. Search Bar (with ⌘K shortcut hint)
            .child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(MonoTheme::border_stroke())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(32.0))
                            .px_3()
                            .rounded(theme.radius(Radius::Md))
                            .bg(MonoTheme::bg_base())
                            .border_1()
                            .border_color(MonoTheme::border_stroke())
                            .child(
                                div()
                                    .text_color(MonoTheme::fg_muted())
                                    .child(Icon::new(IconName::Search).size(IconSize::Xs)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.search_input.clone()),
                            )
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::bg_hover())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_subtle())
                                    .child("⌘K"),
                            ),
                    ),
            )
            // 2. Filter Pills matching MonoCode (All, Active, Pinned, Archived)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(MonoTheme::border_stroke())
                    .child(
                        self.render_filter_pill("All", all_count, active_filter == FilterMode::All, FilterMode::All, cx)
                    )
                    .child(
                        self.render_filter_pill("Active", active_count, active_filter == FilterMode::Active, FilterMode::Active, cx)
                    )
                    .child(
                        self.render_filter_pill("Pinned", pinned_count, active_filter == FilterMode::Pinned, FilterMode::Pinned, cx)
                    )
                    .child(
                        self.render_filter_pill("Archived", archived_count, active_filter == FilterMode::Archived, FilterMode::Archived, cx)
                    ),
            )
            // 3. Action Bar (Header with count & + New Thread button)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(MonoTheme::fg_muted())
                                    .child("RECENT THREADS"),
                            )
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::bg_hover())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(MonoTheme::fg_subtle())
                                    .child(SharedString::from(format!("{}", filtered_sessions.len()))),
                            ),
                    )
                    .child(
                        div()
                            .id("sidebar-new-session-pill")
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(MonoTheme::accent())
                            .text_color(MonoTheme::on_accent())
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.9))
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(Icon::new(IconName::Plus).size(IconSize::Xs))
                            .child("New")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.create_new_session(cx);
                            })),
                    ),
            )
            // 4. Thread List Items with Pin & Delete Actions
            .child(
                on_axis(div().id("sidebar-threads-scroll"))
                    .flex_1()
                    .overflow_y_scroll()
                    .children(filtered_sessions.into_iter().map(|session| {
                        let is_active = selected_id.as_deref() == Some(&session.id);
                        let id = session.id.clone();
                        let pin_id = session.id.clone();
                        let del_id = session.id.clone();
                        let title = if session.title.trim().is_empty() {
                            "Untitled Session"
                        } else {
                            &session.title
                        };
                        let harness = session.harness.to_lowercase();
                        let branch = session.branch.clone().unwrap_or_else(|| "main".into());
                        let relative_time = format_relative_time(session.updated_at);
                        let (harness_color, harness_label) = match harness.as_str() {
                            "claude" => (MonoTheme::claude_orange(), "Claude"),
                            "antigravity" => (MonoTheme::antigravity_blue(), "Agy"),
                            "codex" => (MonoTheme::codex_green(), "Codex"),
                            "cursor" => (MonoTheme::accent(), "Cursor"),
                            _ => (MonoTheme::accent(), "Agent"),
                        };

                        div()
                            .id(SharedString::from(format!("session-row-{}", session.id)))
                            .relative()
                            .flex()
                            .flex_col()
                            .px_3()
                            .py_2p5()
                            .mx_1p5()
                            .my_0p5()
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .when(is_active, |el| el.bg(MonoTheme::bg_active()))
                            .when(!is_active, |el| el.hover(|s| s.bg(MonoTheme::bg_hover())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_session(id.clone(), cx);
                            }))
                            // Left vertical bar on active
                            .child(
                                if is_active {
                                    div()
                                        .absolute()
                                        .left(px(0.0))
                                        .top(px(6.0))
                                        .bottom(px(6.0))
                                        .w(px(3.0))
                                        .rounded(theme.radius(Radius::Sm))
                                        .bg(MonoTheme::accent())
                                } else {
                                    div()
                                },
                            )
                            // Title Row
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .flex_1()
                                            .overflow_hidden()
                                            .text_size(theme.text_size(TextSize::Sm))
                                            .font_weight(if is_active {
                                                FontWeight::SEMIBOLD
                                            } else {
                                                FontWeight::NORMAL
                                            })
                                            .text_color(if is_active {
                                                MonoTheme::fg_primary()
                                            } else {
                                                MonoTheme::fg_muted()
                                            })
                                            .child(title.to_string()),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            // Pin Toggle Button
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!("pin-btn-{}", session.id)))
                                                    .p_1()
                                                    .rounded(theme.radius(Radius::Sm))
                                                    .cursor_pointer()
                                                    .text_color(if session.pinned {
                                                        MonoTheme::skill_gold()
                                                    } else {
                                                        MonoTheme::fg_subtle()
                                                    })
                                                    .hover(|s| s.bg(MonoTheme::bg_hover()).text_color(MonoTheme::skill_gold()))
                                                    .child(Icon::new(IconName::Pin).size(IconSize::Xs))
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.toggle_pin_session(&pin_id, cx);
                                                    })),
                                            )
                                            // Delete / Close Button
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!("del-btn-{}", session.id)))
                                                    .p_1()
                                                    .rounded(theme.radius(Radius::Sm))
                                                    .text_color(MonoTheme::fg_subtle())
                                                    .hover(|s| s.bg(MonoTheme::bg_hover()).text_color(MonoTheme::danger()))
                                                    .child(Icon::new(IconName::Trash2).size(IconSize::Xs))
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.delete_session(&del_id, cx);
                                                    })),
                                            ),
                                    ),
                            )
                            // Meta Row: Harness Badge + Branch Pill + Relative Time
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .pt_1p5()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .size(px(6.0))
                                                            .rounded_full()
                                                            .bg(harness_color),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .text_color(harness_color)
                                                            .child(harness_label),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_1()
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .text_color(MonoTheme::fg_subtle())
                                                    .child(Icon::new(IconName::GitBranch).size(IconSize::Xs))
                                                    .child(branch),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(MonoTheme::fg_subtle())
                                            .child(relative_time),
                                    ),
                            )
                    })),
            )
    }

    fn render_filter_pill(
        &self,
        label: &'static str,
        count: usize,
        is_active: bool,
        mode: FilterMode,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .id(SharedString::from(format!("filter-pill-{}", label)))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .rounded(theme.radius(Radius::Sm))
            .cursor_pointer()
            .when(is_active, |el| {
                el.bg(MonoTheme::bg_active()).text_color(MonoTheme::fg_primary())
            })
            .when(!is_active, |el| {
                el.text_color(MonoTheme::fg_muted()).hover(|s| s.text_color(MonoTheme::fg_primary()))
            })
            .text_size(theme.text_size(TextSize::Xs))
            .font_weight(FontWeight::MEDIUM)
            .child(label)
            .child(
                div()
                    .text_size(theme.text_size(TextSize::Xs))
                    .text_color(if is_active {
                        MonoTheme::accent()
                    } else {
                        MonoTheme::fg_subtle()
                    })
                    .child(format!("{}", count)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.filter_mode = mode;
                cx.notify();
            }))
    }
}

fn format_relative_time(timestamp_ms: i64) -> String {
    let now = jiff::Timestamp::now().as_millisecond();
    let diff = now.saturating_sub(timestamp_ms);

    if diff < 60_000 {
        "just now".to_string()
    } else if diff < 3_600_000 {
        format!("{}m ago", diff / 60_000)
    } else if diff < 86_400_000 {
        format!("{}h ago", diff / 3_600_000)
    } else {
        format!("{}d ago", diff / 86_400_000)
    }
}
