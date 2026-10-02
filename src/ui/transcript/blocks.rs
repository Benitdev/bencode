//! One transcript row per MonoCode block role.

use ely_gpui_component::agent::ToolCallCard;
use ely_gpui_component::chat::{CodeBlock, StepState, StreamingMarkdown};
use ely_gpui_component::documents::MarkdownRenderer;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize, TextSize};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, FontWeight, IntoElement, ParentElement, SharedString, Styled, div, px, rgb,
};
use jiff::Timestamp;
use serde_json::Value;

use crate::db::{Block, SessionRow};
use crate::harness::catalog;
use crate::ui::HarnessIcon;

/// Readable column width for message content, aligned with composer card.
pub const MESSAGE_MAX_WIDTH: gpui::Pixels = gpui::px(840.0);
const USER_BUBBLE_MAX_WIDTH: gpui::Pixels = gpui::px(580.0);

fn tool_field<'a>(block: &'a Block, key: &str) -> Option<&'a str> {
    block.tool.as_ref()?.get(key).and_then(Value::as_str)
}

/// Maps MonoCode's tool `status` onto Ely's step marks.
pub fn step_state(status: &str) -> StepState {
    match status {
        "completed" | "success" => StepState::Done,
        "failed" | "error" => StepState::Failed,
        "pending" => StepState::Waiting,
        _ => StepState::Working,
    }
}

/// Display name for MonoCode's tool `kind`.
pub fn tool_label(kind: &str) -> &'static str {
    match kind {
        "execute" => "Run",
        "edit" => "Edit",
        "read" => "Read",
        "search" => "Search",
        "agent" => "Agent",
        "skill" => "Skill",
        _ => "Tool",
    }
}

fn markdown(id: SharedString, text: &str, live: bool) -> AnyElement {
    if live {
        return StreamingMarkdown::new(id, text.to_string(), true).into_any_element();
    }
    MarkdownRenderer::new(id, text.to_string())
        .code(|id, language, code| {
            let block = CodeBlock::new(id, code);
            match language {
                Some(language) => block.language(language),
                None => block,
            }
            .into_any_element()
        })
        .into_any_element()
}

/// Renders block `ix` of `session`; `live` marks the block still streaming.
pub fn render_block(session: &SessionRow, ix: usize, live: bool, cx: &App) -> AnyElement {
    let block = &session.blocks[ix];
    let id = SharedString::from(format!("{}-{ix}", session.id));
    let text = block.text.as_deref().unwrap_or("");
    match block.role.as_str() {
        "user" => user_bubble(text, cx),
        "tool" => tool_card(id, block),
        "reasoning" => reasoning(text, cx),
        "system" => system_notice(text, cx),
        _ => assistant(session, block, id, text, live, cx),
    }
}

fn system_notice(text: &str, _cx: &App) -> AnyElement {
    let lower = text.to_lowercase();
    let is_err = lower.contains("stopped")
        || lower.contains("fail")
        || lower.contains("error")
        || lower.contains("terminated")
        || lower.contains("kill");
    div()
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .py_1p5()
        .px_3()
        .rounded(px(6.0))
        .bg(if is_err {
            gpui::rgba(0xef444415)
        } else {
            gpui::rgba(0xffffff0a)
        })
        .border_1()
        .border_color(if is_err {
            gpui::rgba(0xef444425)
        } else {
            gpui::rgba(0xffffff10)
        })
        .child(
            Icon::new(if is_err {
                IconName::CircleAlert
            } else {
                IconName::Info
            })
            .size(IconSize::Xs)
            .color(if is_err { rgb(0xf87171) } else { rgb(0x8e8a9d) }),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(if is_err { rgb(0xfca5a5) } else { rgb(0xdedce6) })
                .child(text.to_string()),
        )
        .into_any_element()
}

fn user_bubble(text: &str, _cx: &App) -> AnyElement {
    let is_single_line = !text.contains('\n') && text.chars().count() < 80;
    div()
        .w_full()
        .min_w_0()
        .flex()
        .justify_end()
        .child(
            div()
                .min_w_0()
                .max_w(USER_BUBBLE_MAX_WIDTH)
                .px_3p5()
                .py_2()
                .when(is_single_line, |el| el.rounded(px(18.0)))
                .when(!is_single_line, |el| el.rounded(px(12.0)))
                .bg(rgb(0x232030))
                .border_1()
                .border_color(gpui::rgba(0xffffff10))
                .text_size(px(13.0))
                .text_color(rgb(0xe2e0ea))
                .child(text.to_string()),
        )
        .into_any_element()
}

fn tool_card(id: SharedString, block: &Block) -> AnyElement {
    let status = tool_field(block, "status").unwrap_or("completed");
    let name = tool_label(tool_field(block, "kind").unwrap_or_default());
    let mut card = ToolCallCard::new(id, name, step_state(status));
    if let Some(title) = tool_field(block, "title").filter(|t| !t.is_empty()) {
        card = card.summary(title.to_string());
    }
    if let Some(detail) = tool_field(block, "detail").filter(|d| !d.is_empty()) {
        card = card.result(detail.to_string());
    }
    if let Some(ms) = block.duration_ms.and_then(|ms| u64::try_from(ms).ok()) {
        card = card.took(std::time::Duration::from_millis(ms));
    }
    card.into_any_element()
}

fn reasoning(text: &str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .pl_3()
        .border_l_2()
        .border_color(theme.colors.border_strong)
        .text_size(theme.text_size(TextSize::Xs))
        .italic()
        .text_color(theme.colors.fg_muted)
        .child(text.to_string())
        .into_any_element()
}

fn assistant(
    session: &SessionRow,
    block: &Block,
    id: SharedString,
    text: &str,
    live: bool,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let model = block
        .turn_model
        .as_ref()
        .and_then(|m| m.name.clone())
        .unwrap_or_else(|| catalog::label_for(&session.model));
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .text_size(theme.text_size(TextSize::Xs))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.colors.fg_subtle)
                .child(HarnessIcon::new(&session.harness).size(px(14.0)))
                .child(div().min_w_0().truncate().child(model)),
        )
        .child(
            div()
                .text_size(px(13.0))
                .text_color(theme.colors.fg)
                .child(markdown(id.clone(), text, live)),
        )
        .when(!live && !text.is_empty(), |el| {
            let clock_time = block
                .started_at
                .or(Some(session.updated_at))
                .and_then(|ms| Timestamp::from_millisecond(ms).ok())
                .map(|ts| {
                    let zdt = ts.to_zoned(jiff::tz::TimeZone::system());
                    format!("{}:{:02}", zdt.hour(), zdt.minute())
                })
                .unwrap_or_else(|| "9:56".to_string());

            el.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_1()
                    .pb_2()
                    .text_color(theme.colors.fg_muted)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .id(SharedString::from(format!("{id}-act-copy")))
                                    .size(px(22.0))
                                    .rounded(px(4.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme.colors.hover))
                                    .child(
                                        Icon::new(IconName::Copy)
                                            .size(IconSize::Xs)
                                            .color(theme.colors.fg_muted),
                                    ),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("{id}-act-link")))
                                    .size(px(22.0))
                                    .rounded(px(4.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme.colors.hover))
                                    .child(
                                        Icon::new(IconName::Link)
                                            .size(IconSize::Xs)
                                            .color(theme.colors.fg_muted),
                                    ),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("{id}-act-retry")))
                                    .size(px(22.0))
                                    .rounded(px(4.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme.colors.hover))
                                    .child(
                                        Icon::new(IconName::RotateCcw)
                                            .size(IconSize::Xs)
                                            .color(theme.colors.fg_muted),
                                    ),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("{id}-act-comment")))
                                    .size(px(22.0))
                                    .rounded(px(4.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme.colors.hover))
                                    .child(
                                        Icon::new(IconName::MessageSquare)
                                            .size(IconSize::Xs)
                                            .color(theme.colors.fg_muted),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.colors.fg_subtle)
                            .child("·"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.colors.fg_subtle)
                            .child(clock_time),
                    ),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_status_maps_to_step_marks() {
        assert_eq!(step_state("completed"), StepState::Done);
        assert_eq!(step_state("failed"), StepState::Failed);
        assert_eq!(step_state("in_progress"), StepState::Working);
        assert_eq!(step_state("pending"), StepState::Waiting);
    }

    #[test]
    fn tool_kinds_have_labels() {
        assert_eq!(tool_label("execute"), "Run");
        assert_eq!(tool_label("unknown"), "Tool");
    }
}
