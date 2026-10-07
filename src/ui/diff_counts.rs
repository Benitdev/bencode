//! `+N -M` line counts in the chosen diff palette (MonoCode's diff stat:
//! `text-diff-add-fg` / `text-diff-del-fg`).

use gpui::{Div, FontWeight, ParentElement, Styled, div, prelude::*};

use crate::ui::appearance::DiffColors;

/// Zero counts are omitted, so an all-zero stat renders nothing.
pub fn diff_counts(added: usize, removed: usize, diff: DiffColors) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .font_weight(FontWeight::SEMIBOLD)
        .when(added > 0, |el| {
            el.child(div().text_color(diff.add_fg).child(format!("+{added}")))
        })
        .when(removed > 0, |el| {
            el.child(div().text_color(diff.del_fg).child(format!("-{removed}")))
        })
}
