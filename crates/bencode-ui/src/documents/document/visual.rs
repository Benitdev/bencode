use gpui::{App, KeyBinding, actions};

use crate::key_binding;

actions!(markdown_document, [Undo, Redo, Indent, Outdent, Paste]);

const CONTEXT: &str = "MarkdownDocument";

/// Undo and redo; Tab shifts an open block's lines.
pub(crate) fn bind_keys(cx: &mut App) {
    let context = Some(CONTEXT);
    let block = Some("MarkdownDocument > ElyInput");
    let mut bindings = vec![
        key_binding("secondary-z", Undo, context, cx),
        key_binding("secondary-shift-z", Redo, context, cx),
        key_binding("secondary-v", Paste, context, cx),
        KeyBinding::new("tab", Indent, block),
        KeyBinding::new("shift-tab", Outdent, block),
    ];
    if !cfg!(target_os = "macos") {
        bindings.push(KeyBinding::new("ctrl-y", Redo, context));
    }
    cx.bind_keys(bindings);
}
