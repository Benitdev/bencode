use ely_gpui_component::{
    layout::on_axis,
    theme::{ActiveTheme, Radius, TextSize},
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;
use crate::ui::theme::MonoTheme;

impl BenCodeApp {
    pub fn render_transcript_panel(
        &mut self,
        session: Option<&SessionRow>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .bg(MonoTheme::bg_base())
            // 1. Session Sub-Header Bar
            .child(self.render_header(session, cx))
            // 2. Scrollable Messages Timeline
            .child(
                on_axis(div().id("transcript-scroll-area"))
                    .flex_1()
                    .px_6()
                    .py_4()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_5()
                            .max_w(px(840.0))
                            .mx_auto()
                            .children(if let Some(s) = session {
                                if s.blocks.is_empty() {
                                    vec![
                                        // Empty Session Welcome Screen matching MonoCode
                                        div()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .justify_center()
                                            .py_12()
                                            .child(
                                                div()
                                                    .text_size(theme.text_size(TextSize::Lg))
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(MonoTheme::fg_primary())
                                                    .child(format!("Thread: {}", s.title)),
                                            )
                                            .child(
                                                div()
                                                    .pt_2()
                                                    .text_size(theme.text_size(TextSize::Sm))
                                                    .text_color(MonoTheme::fg_muted())
                                                    .child(format!("Harness: {} • Path: {}", s.harness, s.cwd)),
                                            )
                                            // Suggestion Chips (Clicking fills the composer prompt!)
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_wrap()
                                                    .gap_2()
                                                    .pt_6()
                                                    .child(
                                                        div()
                                                            .id("chip-review-changes")
                                                            .px_3()
                                                            .py_1p5()
                                                            .rounded(theme.radius(Radius::Md))
                                                            .border_1()
                                                            .border_color(MonoTheme::border_stroke())
                                                            .bg(MonoTheme::bg_surface())
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .text_color(MonoTheme::fg_muted())
                                                            .cursor_pointer()
                                                            .hover(|s| s.bg(MonoTheme::bg_hover()).text_color(MonoTheme::fg_primary()))
                                                            .child("🔍 Review recent git changes")
                                                            .on_click(cx.listener(|this, _, _, cx| {
                                                                this.prompt_input.update(cx, |input, cx| {
                                                                    input.set_text("Review recent git changes in the workspace and explain differences", cx);
                                                                });
                                                            })),
                                                    )
                                                    .child(
                                                        div()
                                                            .id("chip-run-tests")
                                                            .px_3()
                                                            .py_1p5()
                                                            .rounded(theme.radius(Radius::Md))
                                                            .border_1()
                                                            .border_color(MonoTheme::border_stroke())
                                                            .bg(MonoTheme::bg_surface())
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .text_color(MonoTheme::fg_muted())
                                                            .cursor_pointer()
                                                            .hover(|s| s.bg(MonoTheme::bg_hover()).text_color(MonoTheme::fg_primary()))
                                                            .child("⚡ Run tests & fix failures")
                                                            .on_click(cx.listener(|this, _, _, cx| {
                                                                this.prompt_input.update(cx, |input, cx| {
                                                                    input.set_text("Run the test suite and investigate any failures", cx);
                                                                });
                                                            })),
                                                    )
                                                    .child(
                                                        div()
                                                            .id("chip-explain-arch")
                                                            .px_3()
                                                            .py_1p5()
                                                            .rounded(theme.radius(Radius::Md))
                                                            .border_1()
                                                            .border_color(MonoTheme::border_stroke())
                                                            .bg(MonoTheme::bg_surface())
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .text_color(MonoTheme::fg_muted())
                                                            .cursor_pointer()
                                                            .hover(|s| s.bg(MonoTheme::bg_hover()).text_color(MonoTheme::fg_primary()))
                                                            .child("📦 Explain project architecture")
                                                            .on_click(cx.listener(|this, _, _, cx| {
                                                                this.prompt_input.update(cx, |input, cx| {
                                                                    input.set_text("Explain project architecture, database models, and main entry points", cx);
                                                                });
                                                            })),
                                                    )
                                                    .child(
                                                        div()
                                                            .id("chip-add-feature")
                                                            .px_3()
                                                            .py_1p5()
                                                            .rounded(theme.radius(Radius::Md))
                                                            .border_1()
                                                            .border_color(MonoTheme::border_stroke())
                                                            .bg(MonoTheme::bg_surface())
                                                            .text_size(theme.text_size(TextSize::Xs))
                                                            .text_color(MonoTheme::fg_muted())
                                                            .cursor_pointer()
                                                            .hover(|s| s.bg(MonoTheme::bg_hover()).text_color(MonoTheme::fg_primary()))
                                                            .child("🛠 Add a new feature / refactor")
                                                            .on_click(cx.listener(|this, _, _, cx| {
                                                                this.prompt_input.update(cx, |input, cx| {
                                                                    input.set_text("Plan and implement the next feature step", cx);
                                                                });
                                                            })),
                                                    ),
                                            )
                                            .into_any_element()
                                    ]
                                } else {
                                    s.blocks.iter().map(|block| {
                                        let is_user = block.role == "user";
                                        let text = block.text.as_deref().unwrap_or("");
                                        let harness_label = block.turn_model
                                            .as_ref()
                                            .and_then(|tm| tm.name.as_deref())
                                            .unwrap_or(s.harness.as_str())
                                            .to_uppercase();

                                        let second_opinion = block.second_opinion.as_ref();

                                        if is_user {
                                            div()
                                                .flex()
                                                .flex_col()
                                                .items_end()
                                                .w_full()
                                                // Optional Handoff Mini-Card
                                                .when(second_opinion.is_some(), |parent| {
                                                    let so = second_opinion.unwrap();
                                                    let from = so.get("from").and_then(|v| v.as_str()).unwrap_or("agent");
                                                    let to = so.get("to").and_then(|v| v.as_str()).unwrap_or("agent");
                                                    let files = so.get("files").and_then(|v| v.as_u64()).unwrap_or(0);
                                                    let req = so.get("request").and_then(|v| v.as_str()).unwrap_or("");

                                                    parent.child(
                                                        div()
                                                            .max_w(px(680.0))
                                                            .mb_2()
                                                            .p_2p5()
                                                            .rounded(theme.radius(Radius::Md))
                                                            .bg(MonoTheme::bg_surface())
                                                            .border_1()
                                                            .border_color(MonoTheme::mention_cyan())
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .items_center()
                                                                    .gap_2()
                                                                    .child(
                                                                        div()
                                                                            .text_size(theme.text_size(TextSize::Xs))
                                                                            .font_weight(FontWeight::BOLD)
                                                                            .text_color(MonoTheme::mention_cyan())
                                                                            .child(format!("HANDOFF: {} ➔ {} ({} files)", from.to_uppercase(), to.to_uppercase(), files)),
                                                                    )
                                                            )
                                                            .when(!req.is_empty(), |el| {
                                                                el.child(
                                                                    div()
                                                                        .pt_1()
                                                                        .text_size(theme.text_size(TextSize::Xs))
                                                                        .text_color(MonoTheme::fg_muted())
                                                                        .child(req.to_string()),
                                                                )
                                                            })
                                                    )
                                                })
                                                .child(
                                                    div()
                                                        .max_w(px(680.0))
                                                        .p_4()
                                                        .rounded(theme.radius(Radius::Lg))
                                                        .bg(MonoTheme::bg_surface())
                                                        .border_1()
                                                        .border_color(MonoTheme::border_stroke())
                                                        .child(
                                                            div()
                                                                .flex()
                                                                .items_center()
                                                                .justify_between()
                                                                .pb_1()
                                                                .child(
                                                                    div()
                                                                        .text_size(theme.text_size(TextSize::Xs))
                                                                        .font_weight(FontWeight::BOLD)
                                                                        .text_color(MonoTheme::accent())
                                                                        .child("YOU"),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_size(theme.text_size(TextSize::Xs))
                                                                        .text_color(MonoTheme::fg_subtle())
                                                                        .child("prompt"),
                                                                ),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(theme.text_size(TextSize::Sm))
                                                                .text_color(MonoTheme::fg_primary())
                                                                .child(text.to_string()),
                                                        ),
                                                )
                                                .into_any_element()
                                        } else if block.role == "tool" {
                                            let tool_title = block.tool
                                                .as_ref()
                                                .and_then(|t| t.get("title").and_then(|v| v.as_str()))
                                                .unwrap_or(text);
                                            let tool_kind = block.tool
                                                .as_ref()
                                                .and_then(|t| t.get("kind").and_then(|v| v.as_str()))
                                                .unwrap_or("execute");
                                            let tool_status = block.tool
                                                .as_ref()
                                                .and_then(|t| t.get("status").and_then(|v| v.as_str()))
                                                .unwrap_or("completed");

                                            let (kind_icon, kind_label) = match tool_kind {
                                                "execute" | "shell" | "bash" => (">_", "EXEC"),
                                                "edit" | "write" => ("✍", "EDIT"),
                                                "read" => ("📖", "READ"),
                                                "search" => ("🔍", "SEARCH"),
                                                "agent" | "subagent" => ("🤖", "AGENT"),
                                                "skill" => ("⚡", "SKILL"),
                                                _ => ("🛠", "TOOL"),
                                            };

                                            div()
                                                .flex()
                                                .flex_col()
                                                .px_3()
                                                .py_2()
                                                .rounded(theme.radius(Radius::Md))
                                                .bg(MonoTheme::bg_surface())
                                                .border_1()
                                                .border_color(MonoTheme::border_stroke())
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .justify_between()
                                                        .child(
                                                            div()
                                                                .flex()
                                                                .items_center()
                                                                .gap_2()
                                                                .child(
                                                                    div()
                                                                        .size(px(6.0))
                                                                        .rounded_full()
                                                                        .bg(if tool_status == "completed" {
                                                                            MonoTheme::success()
                                                                        } else {
                                                                            MonoTheme::accent()
                                                                        }),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_size(theme.text_size(TextSize::Xs))
                                                                        .font_weight(FontWeight::BOLD)
                                                                        .text_color(MonoTheme::skill_gold())
                                                                        .child(format!("{} {}:", kind_icon, kind_label)),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_size(theme.text_size(TextSize::Xs))
                                                                        .font_family(theme.mono_family.clone())
                                                                        .text_color(MonoTheme::fg_primary())
                                                                        .child(tool_title.to_string()),
                                                                ),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(theme.text_size(TextSize::Xs))
                                                                .text_color(if tool_status == "completed" {
                                                                    MonoTheme::success()
                                                                } else {
                                                                    MonoTheme::accent()
                                                                })
                                                                .child(tool_status.to_string()),
                                                        ),
                                                )
                                                .into_any_element()
                                        } else {
                                            // Assistant Turn Card
                                            let duration_text = block.duration_ms
                                                .map(|d| format!("{:.1}s", d as f64 / 1000.0))
                                                .unwrap_or_else(|| "1.4s".to_string());

                                            div()
                                                .p_4()
                                                .rounded(theme.radius(Radius::Lg))
                                                .bg(MonoTheme::bg_surface())
                                                .border_1()
                                                .border_color(MonoTheme::border_stroke())
                                                // Header
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .justify_between()
                                                        .pb_2()
                                                        .border_b_1()
                                                        .border_color(MonoTheme::border_stroke())
                                                        .child(
                                                            div()
                                                                .flex()
                                                                .items_center()
                                                                .gap_2()
                                                                .child(
                                                                    div()
                                                                        .size(px(7.0))
                                                                        .rounded_full()
                                                                        .bg(MonoTheme::accent()),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_size(theme.text_size(TextSize::Xs))
                                                                        .font_weight(FontWeight::BOLD)
                                                                        .text_color(MonoTheme::accent())
                                                                        .child(harness_label),
                                                                ),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(theme.text_size(TextSize::Xs))
                                                                .text_color(MonoTheme::fg_subtle())
                                                                .child(duration_text),
                                                        ),
                                                )
                                                // Assistant Response Text
                                                .child(
                                                    div()
                                                        .pt_2p5()
                                                        .text_size(theme.text_size(TextSize::Sm))
                                                        .text_color(MonoTheme::fg_primary())
                                                        .child(text.to_string()),
                                                )
                                                .into_any_element()
                                        }
                                    }).collect()
                                }
                            } else {
                                vec![
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .h(px(320.0))
                                        .text_size(theme.text_size(TextSize::Sm))
                                        .text_color(MonoTheme::fg_muted())
                                        .child("Select a thread from the sidebar or click + New Session")
                                        .into_any_element()
                                ]
                            }),
                    ),
            )
            // 3. Composer Dock
            .child(self.render_composer(session, cx))
    }

    fn render_header(&self, session: Option<&SessionRow>, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        let branch = session.and_then(|s| s.branch.as_deref()).unwrap_or("main");
        let model = session.map(|s| s.model.as_str()).unwrap_or("Claude 3.7 Sonnet");

        let context_pct = match (session.and_then(|s| s.context_used), session.and_then(|s| s.context_window)) {
            (Some(used), Some(window)) if window > 0 => {
                let pct = (used as f64 / window as f64 * 100.0).round() as u64;
                format!("{}%", pct)
            }
            _ => "0%".to_string(),
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(46.0))
            .px_6()
            .border_b_1()
            .border_color(MonoTheme::border_stroke())
            .bg(MonoTheme::bg_surface())
            // Left Title & Cwd Breadcrumbs
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Sm))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(MonoTheme::fg_primary())
                            .child(
                                session
                                    .map(|s| s.title.as_str())
                                    .unwrap_or("BenCode Workspace")
                                    .to_string(),
                            ),
                    )
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::fg_subtle())
                            .child("•"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::fg_muted())
                            .child("📁")
                            .child(
                                session
                                    .map(|s| s.cwd.as_str())
                                    .unwrap_or("No workspace open")
                                    .to_string(),
                            ),
                    ),
            )
            // Right Status Badge: Branch + Model + Context
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(MonoTheme::bg_base())
                            .border_1()
                            .border_color(MonoTheme::border_stroke())
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::fg_muted())
                            .child(format!("⎇ {}", branch)),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(MonoTheme::bg_base())
                            .border_1()
                            .border_color(MonoTheme::border_stroke())
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::accent())
                            .child(model.to_string()),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(MonoTheme::bg_base())
                            .border_1()
                            .border_color(MonoTheme::border_stroke())
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(MonoTheme::fg_subtle())
                            .child(format!("context: {}", context_pct)),
                    ),
            )
    }
}
