//! Active conversation transcript: message bubbles, assistant markdown with syntax highlighting,
//! tool execution cards, pending tool approvals, streaming cursors, and empty thread suggestion chips.

use ely_gpui_component::buttons::{Button, ButtonVariant};
use ely_gpui_component::chat::{CodeBlock, StreamingCursor, StreamingMarkdown};
use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::documents::MarkdownRenderer;
use ely_gpui_component::feedback::{Alert, ConfirmationCard, EmptyState};
use ely_gpui_component::layout::on_axis;
use ely_gpui_component::primitives::{Icon, IconName, Severity};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;

fn tool_tone(status: &str) -> Tone {
    match status {
        "completed" | "success" => Tone::Success,
        "failed" | "error" => Tone::Danger,
        "running" | "in_progress" => Tone::Warning,
        _ => Tone::Neutral,
    }
}

impl BenCodeApp {
    pub fn render_transcript_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.selected_session();
        let colors = cx.theme().colors.clone();
        let entity = cx.entity().clone();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .bg(colors.bg)
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
                            .gap_4()
                            .max_w(px(840.0))
                            .mx_auto()
                            .children(if let Some(s) = session {
                                if s.blocks.is_empty() {
                                    vec![
                                        div()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .justify_center()
                                            .py_12()
                                            .child(
                                                EmptyState::new(
                                                    "empty-transcript-state",
                                                    IconName::Sparkles,
                                                    format!("Thread: {}", s.title),
                                                )
                                                .body(format!("Harness: {} • Working dir: {}", s.harness, s.cwd)),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_wrap()
                                                    .gap_2()
                                                    .pt_6()
                                                    .child(
                                                        Button::new("chip-review-changes", "Review recent git changes")
                                                            .variant(ButtonVariant::Secondary)
                                                            .size(ControlSize::Sm)
                                                            .on_click(cx.listener(|this, _, _, cx| {
                                                                this.prompt_input.update(cx, |input, cx| {
                                                                    input.set_text("Review recent git changes in the workspace and explain differences", cx);
                                                                });
                                                            })),
                                                    )
                                                    .child(
                                                        Button::new("chip-plan-feature", "Plan next feature or refactor")
                                                            .variant(ButtonVariant::Secondary)
                                                            .size(ControlSize::Sm)
                                                            .on_click(cx.listener(|this, _, _, cx| {
                                                                this.prompt_input.update(cx, |input, cx| {
                                                                    input.set_text("Plan and implement the next feature step cleanly", cx);
                                                                });
                                                            })),
                                                    ),
                                            )
                                            .into_any_element()
                                    ]
                                } else {
                                    let is_running_here = self.is_agent_running_in(&s.id);
                                    let mut elements: Vec<gpui::AnyElement> = s.blocks.iter().enumerate().map(|(idx, block)| {
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
                                                .when_some(second_opinion, |parent, so| {
                                                    let from = so.get("from").and_then(|v| v.as_str()).unwrap_or("agent");
                                                    let to = so.get("to").and_then(|v| v.as_str()).unwrap_or("agent");
                                                    let files = so.get("files").and_then(|v| v.as_u64()).unwrap_or(0);
                                                    let req = so.get("request").and_then(|v| v.as_str()).unwrap_or("");

                                                    parent.child(
                                                        div()
                                                            .max_w(px(680.0))
                                                            .mb_2()
                                                            .p_2p5()
                                                            .rounded(cx.theme().radius(Radius::Md))
                                                            .bg(colors.surface)
                                                            .border_1()
                                                            .border_color(colors.accent)
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .items_center()
                                                                    .gap_2()
                                                                    .child(
                                                                        div()
                                                                            .text_size(cx.theme().text_size(TextSize::Xs))
                                                                            .font_weight(FontWeight::BOLD)
                                                                            .text_color(colors.accent)
                                                                            .child(format!("HANDOFF: {} ➔ {} ({files} files)", from.to_uppercase(), to.to_uppercase())),
                                                                    ),
                                                            )
                                                            .when(!req.is_empty(), |el| {
                                                                el.child(
                                                                    div()
                                                                        .pt_1()
                                                                        .text_size(cx.theme().text_size(TextSize::Xs))
                                                                        .text_color(colors.fg_muted)
                                                                        .child(req.to_string()),
                                                                )
                                                            }),
                                                    )
                                                })
                                                .child(
                                                    div()
                                                        .max_w(px(680.0))
                                                        .p_4()
                                                        .rounded(cx.theme().radius(Radius::Lg))
                                                        .bg(colors.surface)
                                                        .border_1()
                                                        .border_color(colors.border)
                                                        .child(
                                                            div()
                                                                .flex()
                                                                .items_center()
                                                                .justify_between()
                                                                .pb_1()
                                                                .child(
                                                                    div()
                                                                        .text_size(cx.theme().text_size(TextSize::Xs))
                                                                        .font_weight(FontWeight::BOLD)
                                                                        .text_color(colors.accent)
                                                                        .child("YOU"),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_size(cx.theme().text_size(TextSize::Xs))
                                                                        .text_color(colors.fg_subtle)
                                                                        .child("prompt"),
                                                                ),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(cx.theme().text_size(TextSize::Sm))
                                                                .text_color(colors.fg)
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
                                                "execute" | "shell" | "bash" => (IconName::Terminal, "EXEC"),
                                                "edit" | "write" => (IconName::FileText, "EDIT"),
                                                "read" => (IconName::FileText, "READ"),
                                                "search" => (IconName::Search, "SEARCH"),
                                                "agent" | "subagent" => (IconName::Zap, "AGENT"),
                                                "skill" => (IconName::Zap, "SKILL"),
                                                _ => (IconName::Settings, "TOOL"),
                                            };

                                            div()
                                                .flex()
                                                .flex_col()
                                                .px_3()
                                                .py_2()
                                                .rounded(cx.theme().radius(Radius::Md))
                                                .bg(colors.surface)
                                                .border_1()
                                                .border_color(colors.border)
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
                                                                    Icon::new(kind_icon)
                                                                        .size(IconSize::Xs)
                                                                        .color(colors.fg_muted),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_size(cx.theme().text_size(TextSize::Xs))
                                                                        .font_weight(FontWeight::BOLD)
                                                                        .text_color(colors.fg_muted)
                                                                        .child(format!("{kind_label}:")),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_size(cx.theme().text_size(TextSize::Xs))
                                                                        .font_family(cx.theme().mono_family.clone())
                                                                        .text_color(colors.fg)
                                                                        .child(tool_title.to_string()),
                                                                ),
                                                        )
                                                        .child(
                                                            Badge::new(tool_status.to_string())
                                                                .tone(tool_tone(tool_status))
                                                                .dot(),
                                                        ),
                                                )
                                                .into_any_element()
                                        } else if block.role == "reasoning" {
                                            div()
                                                .px_3()
                                                .py_2()
                                                .rounded(cx.theme().radius(Radius::Sm))
                                                .border_l_2()
                                                .border_color(colors.border_strong)
                                                .bg(colors.surface)
                                                .text_size(cx.theme().text_size(TextSize::Xs))
                                                .italic()
                                                .text_color(colors.fg_muted)
                                                .child(text.to_string())
                                                .into_any_element()
                                        } else if block.role == "system" {
                                            Alert::new(format!("block-sys-{idx}"), Severity::Danger, "System Notice")
                                                .body(text.to_string())
                                                .into_any_element()
                                        } else {
                                            // Assistant Turn Card
                                            let duration_text = block.duration_ms
                                                .map(|d| format!("{:.1}s", d as f64 / 1000.0))
                                                .unwrap_or_default();

                                            let is_last_block = idx + 1 == s.blocks.len();
                                            let is_live_turn = is_last_block && is_running_here;
                                            let block_id = SharedString::from(format!("block-md-{}-{}", s.id, idx));

                                            div()
                                                .p_4()
                                                .rounded(cx.theme().radius(Radius::Lg))
                                                .bg(colors.surface)
                                                .border_1()
                                                .border_color(colors.border)
                                                // Header
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .justify_between()
                                                        .pb_2()
                                                        .border_b_1()
                                                        .border_color(colors.border)
                                                        .child(
                                                            div()
                                                                .flex()
                                                                .items_center()
                                                                .gap_2()
                                                                .child(
                                                                    div()
                                                                        .size(px(7.0))
                                                                        .rounded_full()
                                                                        .bg(colors.accent),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_size(cx.theme().text_size(TextSize::Xs))
                                                                        .font_weight(FontWeight::BOLD)
                                                                        .text_color(colors.accent)
                                                                        .child(harness_label),
                                                                ),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(cx.theme().text_size(TextSize::Xs))
                                                                .text_color(colors.fg_subtle)
                                                                .child(duration_text),
                                                        ),
                                                )
                                                // Assistant Response Markdown
                                                .child(
                                                    div()
                                                        .pt_2p5()
                                                        .text_size(cx.theme().text_size(TextSize::Sm))
                                                        .text_color(colors.fg)
                                                        .child(if is_live_turn {
                                                            StreamingMarkdown::new(block_id, text.to_string(), true).into_any_element()
                                                        } else if !text.contains('\n') && !text.contains('`') && !text.contains('#') && !text.contains('*') && !text.contains('-') && !text.contains('>') {
                                                            div().child(text.to_string()).into_any_element()
                                                        } else {
                                                            MarkdownRenderer::new(block_id, text.to_string())
                                                                .code(|id, language, code| {
                                                                    let block = CodeBlock::new(id, code);
                                                                    match language {
                                                                        Some(language) => block.language(language),
                                                                        None => block,
                                                                    }
                                                                    .into_any_element()
                                                                })
                                                                .into_any_element()
                                                        }),
                                                )
                                                .into_any_element()
                                        }
                                    }).collect();

                                    // Pending Permission Request Inline Card
                                    if let Some(crate::harness::PermissionRequest { tool, description, .. }) = self.pending_permission_for(&s.id).cloned() {
                                        let e1 = entity.clone();
                                        let e2 = entity.clone();
                                        elements.push(
                                            ConfirmationCard::new(
                                                "perm-approval-card",
                                                format!("Authorize Tool Execution: {tool}"),
                                            )
                                            .body(description)
                                            .confirm("Approve (y)")
                                            .on_confirm(move |_, cx| {
                                                e1.update(cx, |this, cx| this.approve_permission(cx));
                                            })
                                            .on_cancel(move |_, cx| {
                                                e2.update(cx, |this, cx| this.deny_permission(cx));
                                            })
                                            .into_any_element(),
                                        );
                                    } else if self.is_agent_running_in(&s.id) {
                                        elements.push(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .py_2()
                                                .px_3()
                                                .rounded(cx.theme().radius(Radius::Md))
                                                .bg(colors.surface)
                                                .border_1()
                                                .border_color(colors.border)
                                                .child(StreamingCursor::new("agent-live-cursor"))
                                                .child(
                                                    div()
                                                        .text_size(cx.theme().text_size(TextSize::Xs))
                                                        .text_color(colors.fg_muted)
                                                        .child("Streaming tokens via Apple Metal GPU (120 FPS)..."),
                                                )
                                                .into_any_element(),
                                        );
                                    }

                                    elements
                                }
                            } else {
                                vec![
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .h(px(320.0))
                                        .child(
                                            EmptyState::new(
                                                "no-thread-selected",
                                                IconName::MessageSquare,
                                                "No Thread Selected",
                                            )
                                            .body("Select a thread from the sidebar or click + to start a new session"),
                                        )
                                        .into_any_element()
                                ]
                            }),
                    ),
            )
            // 3. Composer Dock
            .child(self.render_composer(session, cx))
    }

    fn render_header(&self, session: Option<&SessionRow>, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let text_xs = cx.theme().text_size(TextSize::Xs);
        let text_sm = cx.theme().text_size(TextSize::Sm);

        let branch = session.and_then(|s| s.branch.as_deref()).unwrap_or("main");
        let model = session.map(|s| s.model.as_str()).unwrap_or("Claude 3.7 Sonnet");

        let context_pct = match (session.and_then(|s| s.context_used), session.and_then(|s| s.context_window)) {
            (Some(used), Some(window)) if window > 0 => {
                let pct = (used as f64 / window as f64 * 100.0).round() as u64;
                format!("{pct}%")
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
            .border_color(colors.border)
            .bg(colors.surface)
            // Left Title & Cwd Breadcrumbs
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(text_sm)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colors.fg)
                            .child(
                                session
                                    .map(|s| s.title.as_str())
                                    .unwrap_or("BenCode Workspace")
                                    .to_string(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_size(text_xs)
                            .text_color(colors.fg_muted)
                            .child(Icon::new(IconName::Folder).size(IconSize::Xs))
                            .child(
                                session
                                    .map(|s| s.cwd.as_str())
                                    .unwrap_or("~")
                                    .to_string(),
                            ),
                    ),
            )
            // Right Status Chips
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Badge::new(branch)
                            .tone(Tone::Neutral)
                    )
                    .child(
                        Badge::new(model)
                            .tone(Tone::Neutral)
                    )
                    .child(
                        Badge::new(format!("Context: {context_pct}"))
                            .tone(Tone::Info)
                    ),
            )
    }
}
