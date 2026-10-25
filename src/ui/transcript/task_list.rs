//! MonoCode `TaskListPreview`: the agent's task list as a card, one row per
//! task with its state.

use bencode_ui::primitives::{Icon, IconName};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{AnyElement, App, IntoElement, ParentElement, SharedString, Styled, div};

use crate::app::task_list::progress_label;
use crate::harness::{TaskItem, TaskStatus};
use crate::ui::scale::px;
use crate::ui::spinner::terminal_spinner;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// The row's leading mark (MonoCode `TaskState`).
fn state(status: TaskStatus, cx: &App) -> AnyElement {
    let colors = &cx.theme().colors;
    let slot = div()
        .mt(px(1.0))
        .size(px(16.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center();
    match status {
        TaskStatus::Completed => slot
            .rounded_full()
            .bg(colors.success.opacity(0.2))
            .child(
                Icon::new(IconName::Check)
                    .size(IconSize::Xs)
                    .color(colors.success),
            )
            .into_any_element(),
        TaskStatus::InProgress => slot
            .child(terminal_spinner(colors.accent, cx))
            .into_any_element(),
        TaskStatus::Cancelled => slot
            .rounded_full()
            .bg(colors.fg.fill(Fill::Selected))
            .child(
                Icon::new(IconName::Minus)
                    .size(IconSize::Xs)
                    .color(colors.fg.ink(Ink::Faint)),
            )
            .into_any_element(),
        TaskStatus::Pending => slot
            .rounded_full()
            .border_1()
            .border_color(colors.fg.ink(Ink::Faint))
            .bg(colors.fg.opacity(0.02))
            .into_any_element(),
    }
}

/// The card for `items`, under the line the agent explained them with.
pub(super) fn card(items: &[TaskItem], explanation: Option<&str>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let colors = &theme.colors;
    let header = div()
        .flex()
        .items_start()
        .gap_2()
        .px(px(10.0))
        .py_2()
        .border_b_1()
        .border_color(colors.border)
        .child(
            div().mt(px(2.0)).flex_none().child(
                Icon::new(IconName::ListTodo)
                    .size(IconSize::Sm)
                    .color(colors.fg.ink(Ink::Muted)),
            ),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .font_family(theme.mono_family.clone())
                                .text_size(Text::Base.px())
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.fg.ink(Ink::Strong))
                                .child("Tasks"),
                        )
                        .child(
                            div()
                                .flex_none()
                                .rounded_full()
                                .bg(colors.fg.fill(Fill::Line))
                                .px_2()
                                .py(px(2.0))
                                .font_family(theme.mono_family.clone())
                                .text_size(Text::Xs.px())
                                .text_color(colors.fg.ink(Ink::Secondary))
                                .child(progress_label(items)),
                        ),
                )
                .children(explanation.map(|line| {
                    div()
                        .mt(px(2.0))
                        .text_size(Text::Sm.px())
                        .line_height(px(16.0))
                        .text_color(colors.fg.ink(Ink::Secondary))
                        .line_clamp(2)
                        .child(SharedString::from(line.to_string()))
                })),
        );
    let rows = items.iter().map(|item| {
        let text = div()
            .min_w_0()
            .flex_1()
            .text_size(Text::Base.px())
            .line_height(px(18.0))
            .child(SharedString::from(item.text.clone()));
        let text = match item.status {
            TaskStatus::Completed => text.text_color(colors.fg.ink(Ink::Muted)).line_through(),
            TaskStatus::Cancelled => text.text_color(colors.fg.ink(Ink::Faint)).line_through(),
            TaskStatus::InProgress => text.text_color(colors.fg.ink(Ink::Strong)),
            TaskStatus::Pending => text.text_color(colors.fg.ink(Ink::Secondary)),
        };
        div()
            .flex()
            .items_start()
            .gap(px(10.0))
            .min_w_0()
            .px(px(10.0))
            .py(px(6.0))
            .child(state(item.status, cx))
            .child(text)
    });
    div()
        .px_4()
        .py_1()
        .child(
            div()
                .mb_2()
                .rounded(px(10.0))
                .border_1()
                .border_color(colors.fg.fill(Fill::Strong))
                .bg(colors.fg.opacity(0.035))
                .overflow_hidden()
                .child(header)
                .child(div().py_1().children(rows)),
        )
        .into_any_element()
}
