use gpui::{App, actions};

use crate::key_binding;

actions!(ely_rich_text, [ToggleBold, ToggleItalic, EditLink]);

const CONTEXT: &str = "ElyRichText";

/// Binds the rich text keys. `init` calls it.
pub(crate) fn bind_keys(cx: &mut App) {
    let bindings = [
        key_binding("secondary-b", ToggleBold, Some(CONTEXT), cx),
        key_binding("secondary-i", ToggleItalic, Some(CONTEXT), cx),
        key_binding("secondary-k", EditLink, Some(CONTEXT), cx),
    ];
    cx.bind_keys(bindings);
}
