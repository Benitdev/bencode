//! The pieces the Automations and Notes pages share: MonoCode's `content/N`
//! tints, `SectionTitle`, `SettingsRow`, `PageTab` and the bordered box
//! sections sit in.

use gpui::{Div, FontWeight, Hsla, IntoElement, ParentElement, Stateful, Styled, div, prelude::*};

use crate::ui::scale::px;

/// MonoCode's `content/N` tints: how much of the foreground colour a
/// line, a fill or a quieter text takes.
pub mod tint {
    /// `border-content/10`: a box's outline.
    pub const STROKE: f32 = 0.10;
    pub const STROKE_HOVER: f32 = 0.16;
    /// `border-dashed border-content/15`.
    pub const DASH: f32 = 0.15;
    pub const DASH_HOVER: f32 = 0.25;
    /// `border-content/8`: the hairline between rows.
    pub const RULE: f32 = 0.08;
    /// `bg-content/15`: the short divider in a toolbar.
    pub const DIVIDER: f32 = 0.15;
    /// `bg-content/8`: icon discs and hovered pills.
    pub const FILL: f32 = 0.08;
    /// `hover:bg-content/5`.
    pub const HOVER: f32 = 0.05;
    /// `bg-content/3`: the instructions box.
    pub const WELL: f32 = 0.03;
    /// A status pill's fill, of its own colour.
    pub const PILL: f32 = 0.12;
    /// An alert's outline, of its own colour.
    pub const ALERT_STROKE: f32 = 0.2;
    // Text, from a row's label down to an aside.
    pub const STRONG: f32 = 0.75;
    pub const BODY: f32 = 0.7;
    pub const SOFT: f32 = 0.55;
    pub const QUIET: f32 = 0.45;
    pub const HINT: f32 = 0.4;
    pub const FAINT: f32 = 0.35;
    /// `text-content/85`: markdown source.
    pub const SOURCE: f32 = 0.85;
}

/// The width a row's control gets.
const SELECT_WIDTH: f32 = 168.0;

/// MonoCode `rounded-md border border-content/10`.
pub fn panel(fg: Hsla) -> Div {
    div()
        .rounded(px(6.0))
        .border_1()
        .border_color(fg.opacity(tint::STROKE))
}

/// The hairline between a panel's rows.
pub fn rule(fg: Hsla) -> Div {
    div().h(px(1.0)).bg(fg.opacity(tint::RULE))
}

/// MonoCode `SectionTitle`.
pub fn section_title(title: &'static str, muted: Hsla) -> Div {
    div()
        .px_1()
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(muted)
        .child(title)
}

/// MonoCode `SettingsRow`: a label and hint with the control at the right.
pub fn settings_row(
    label: &'static str,
    hint: &'static str,
    control: impl IntoElement,
    fg: Hsla,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .px_4()
        .py(px(10.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .min_w_0()
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(fg.opacity(tint::STRONG))
                        .child(label),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(fg.opacity(tint::HINT))
                        .child(hint),
                ),
        )
        .child(div().flex_none().w(px(SELECT_WIDTH)).child(control))
}

/// MonoCode `PageTab`: a label with a 2px line under the open page.
pub fn page_tab(
    id: &'static str,
    label: &'static str,
    selected: bool,
    fg: Hsla,
    muted: Hsla,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .h(px(36.0))
        .text_size(px(12.0))
        .cursor_pointer()
        .text_color(if selected { fg } else { muted })
        .hover(|style| style.text_color(fg))
        .child(label)
        .when(selected, |el| {
            el.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(2.0))
                    .bg(fg),
            )
        })
}
