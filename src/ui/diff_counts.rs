//! `+N -M` line counts in the success/danger colours, MonoCode's diff stat.

use ely_gpui_component::theme::Palette;
use gpui::{Div, FontWeight, ParentElement, Styled, div, prelude::*};

/// Zero counts are omitted, so an all-zero stat renders nothing.
pub fn diff_counts(added: usize, removed: usize, colors: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .font_weight(FontWeight::SEMIBOLD)
        .when(added > 0, |el| {
            el.child(div().text_color(colors.success).child(format!("+{added}")))
        })
        .when(removed > 0, |el| {
            el.child(div().text_color(colors.danger).child(format!("-{removed}")))
        })
}
