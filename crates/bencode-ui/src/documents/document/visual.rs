use gpui::{App, KeyBinding, actions};

actions!(markdown_document, [Undo, Redo, Indent, Outdent, Paste]);

const CONTEXT: &str = "MarkdownDocument";

/// Undo and redo; Tab shifts an open block's lines.
pub(crate) fn bind_keys(cx: &mut App) {
    let context = Some(CONTEXT);
    let block = Some("MarkdownDocument > ElyInput");
    let mut bindings = vec![
        KeyBinding::new("secondary-z", Undo, context),
        KeyBinding::new("secondary-shift-z", Redo, context),
        KeyBinding::new("secondary-v", Paste, context),
        KeyBinding::new("tab", Indent, block),
        KeyBinding::new("shift-tab", Outdent, block),
    ];
    if !cfg!(target_os = "macos") {
        bindings.push(KeyBinding::new("ctrl-y", Redo, context));
    }
    cx.bind_keys(bindings);
}
