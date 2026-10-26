//! What BenCode adds around bencode-ui's buttons: a tooltip on a labelled
//! `Button`, which bencode-ui leaves to its caller.

use bencode_ui::primitives::Tooltip;
use gpui::{Div, ElementId, IntoElement, SharedString, Stateful, div, prelude::*};

/// `button` with `tip` on hover; `id` names the box and must be unique
/// beside the button's own.
pub fn with_tooltip(
    id: impl Into<ElementId>,
    tip: impl Into<SharedString>,
    button: impl IntoElement,
) -> Stateful<Div> {
    let tip = tip.into();
    div()
        .id(id)
        .flex_none()
        .tooltip(Tooltip::text(tip))
        .child(button)
}
