//! MonoCode `.popover-backdrop` / `GlassBackdrop` for BenCode's own
//! popovers and dialogs: `bencode_ui::primitives::frost` (the vendored
//! renderer's backdrop blur) at BenCode's lengths and fills.

use gpui::{App, Hsla, IntoElement};

use crate::ui::scale::px;
use crate::ui::sidebar_popovers::popover_glass;

/// The inner radius of a `rounded-xl border` frame (12px less its border).
pub(crate) const POPOVER_RADIUS: f32 = 11.0;

/// The frosted surface of a popover: the first child of its frame, filling
/// the frame inside its border with corners of `radius`.
pub(crate) fn frost(radius: f32, cx: &App) -> impl IntoElement {
    bencode_ui::primitives::frost::popover(px(radius), popover_glass(cx), cx)
}

/// MonoCode `modal.rs` `surface`: a dialog card's frost, `opaque` being the
/// card's own fill where it cannot blur.
pub(crate) fn dialog_frost(radius: f32, opaque: Hsla, cx: &App) -> impl IntoElement {
    bencode_ui::primitives::frost::dialog(px(radius), opaque, cx)
}
