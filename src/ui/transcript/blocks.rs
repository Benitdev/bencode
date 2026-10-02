//! One transcript row per MonoCode block role.

use ely_gpui_component::agent::ToolCallCard;
use ely_gpui_component::chat::{CodeBlock, StepState, StreamingMarkdown};
use ely_gpui_component::documents::MarkdownRenderer;
use ely_gpui_component::feedback::Alert;
use ely_gpui_component::primitives::Severity;
use ely_gpui_component::theme::{ActiveTheme, Radius, TextSize};
use gpui::{AnyElement, App, FontWeight, IntoElement, ParentElement, SharedString, Styled, div};
use serde_json::Value;

use crate::db::{Block, SessionRow};
use crate::harness::catalog;

/// Readable column width for message content.
pub const MESSAGE_MAX_WIDTH: gpui::Pixels = gpui::px(760.0);
const USER_BUBBLE_MAX_WIDTH: gpui::Pixels = gpui::px(640.0);

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
        "system" => Alert::new(id, Severity::Danger, "Notice").body(text.to_string()).into_any_element(),
        _ => assistant(session, block, id, text, live, cx),
    }
}

fn user_bubble(text: &str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .flex()
        .justify_end()
        .child(
            div()
                .max_w(USER_BUBBLE_MAX_WIDTH)
                .px_4()
                .py_3()
                .rounded(theme.radius(Radius::Lg))
                .bg(theme.colors.active)
                .text_size(theme.text_size(TextSize::Sm))
                .text_color(theme.colors.fg)
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

fn assistant(session: &SessionRow, block: &Block, id: SharedString, text: &str, live: bool, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let model = block
        .turn_model
        .as_ref()
        .and_then(|m| m.name.clone())
        .unwrap_or_else(|| catalog::label_for(&session.model));
    let dot = crate::ui::theme::harness_color(&session.harness, &theme.colors);
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(theme.text_size(TextSize::Xs))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.colors.fg_subtle)
                .child(div().size_1p5().rounded_full().bg(dot))
                .child(model),
        )
        .child(
            div()
                .text_size(theme.text_size(TextSize::Sm))
                .text_color(theme.colors.fg)
                .child(markdown(id, text, live)),
        )
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
