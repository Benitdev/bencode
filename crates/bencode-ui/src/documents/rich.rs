use gpui::{App, KeyBinding, actions};

actions!(ely_rich_text, [ToggleBold, ToggleItalic, EditLink]);

const CONTEXT: &str = "ElyRichText";

/// Binds the rich text keys. `init` calls it.
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-b", ToggleBold, Some(CONTEXT)),
        KeyBinding::new("secondary-i", ToggleItalic, Some(CONTEXT)),
        KeyBinding::new("secondary-k", EditLink, Some(CONTEXT)),
    ]);
}
