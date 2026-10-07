//! MonoCode "Add to chat" (`quoteDraft.ts`, `editorSelection.ts`): a code
//! reference like `@src/app.rs (lines 4-9)` joins the prompt as its own
//! paragraph, and text selected in the transcript joins it quoted with `> `.

use gpui::Context;

use crate::app::BenCodeApp;

/// MonoCode `joinComposerInsert`: a blank line between the draft and the
/// block, and one after it for the next words.
pub fn join_insert(draft: &str, block: &str) -> String {
    let separator = if draft.is_empty() || draft.ends_with("\n\n") {
        ""
    } else if draft.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    format!("{draft}{separator}{block}\n\n")
}

/// MonoCode `appendComposerInsert`.
pub fn append_to_draft(draft: &str, text: &str) -> String {
    let selected = text.replace("\r\n", "\n").replace('\r', "\n");
    match selected.trim() {
        "" => draft.to_string(),
        block => join_insert(draft, block),
    }
}

/// MonoCode `appendSelectionQuote`: every line behind `> ` (a blank one
/// as a bare `>`).
pub fn append_quote(draft: &str, text: &str) -> String {
    let selected = text.replace("\r\n", "\n").replace('\r', "\n");
    let selected = selected.trim();
    if selected.is_empty() {
        return draft.to_string();
    }
    let quote = selected
        .split('\n')
        .map(|line| {
            if line.is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    join_insert(draft, &quote)
}

/// MonoCode `isMentionablePath`: short, relative, plain segments.
fn is_mentionable(path: &str) -> bool {
    !path.is_empty()
        && path.chars().count() <= 120
        && !path.starts_with('/')
        && !path
            .chars()
            .any(|c| c.is_whitespace() || c == '@' || c.is_control())
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// MonoCode `formatEditorSelectionReference`: `@path (line 4)` or
/// `@path (lines 4-9)`; other paths go in backticks.
pub fn selection_reference(path: &str, start_line: usize, end_line: usize) -> String {
    let path = path.replace('\\', "/");
    let file = if is_mentionable(&path) {
        format!("@{path}")
    } else {
        format!("`{}`", path.replace(['`', '\r', '\n'], "'"))
    };
    if start_line == end_line {
        format!("{file} (line {start_line})")
    } else {
        format!("{file} (lines {start_line}-{end_line})")
    }
}

impl BenCodeApp {
    /// MonoCode `requestAddToChat`: into the focused thread's prompt.
    pub fn add_to_chat(&mut self, text: &str, cx: &mut Context<Self>) {
        self.insert_into_prompt(text, append_to_draft, cx);
    }

    /// MonoCode `addSelectionToChat`: transcript text, quoted.
    pub fn add_quote_to_chat(&mut self, text: &str, cx: &mut Context<Self>) {
        self.insert_into_prompt(text, append_quote, cx);
    }

    fn insert_into_prompt(
        &mut self,
        text: &str,
        join: fn(&str, &str) -> String,
        cx: &mut Context<Self>,
    ) {
        let draft = self.prompt_input.read(cx).text().to_string();
        let next = join(&draft, text);
        if next == draft {
            return;
        }
        self.prompt_input
            .update(cx, |input, cx| input.set_text(next, cx));
        self.refocus_prompt(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_join_the_draft_as_paragraphs() {
        assert_eq!(join_insert("", "x"), "x\n\n");
        assert_eq!(join_insert("hi", "x"), "hi\n\nx\n\n");
        assert_eq!(join_insert("hi\n", "x"), "hi\n\nx\n\n");
        assert_eq!(join_insert("hi\n\n", "x"), "hi\n\nx\n\n");
    }

    #[test]
    fn inserts_are_trimmed_and_blank_ones_ignored() {
        assert_eq!(
            append_to_draft("", " @a.rs (line 1)\r\n"),
            "@a.rs (line 1)\n\n"
        );
        assert_eq!(append_to_draft("keep", "  "), "keep");
    }

    #[test]
    fn selections_join_as_quotes() {
        assert_eq!(append_quote("", " one\r\n\ntwo \n"), "> one\n>\n> two\n\n");
        assert_eq!(append_quote("hi", "x"), "hi\n\n> x\n\n");
        assert_eq!(append_quote("keep", " \n "), "keep");
    }

    #[test]
    fn references_mention_plain_paths_only() {
        assert_eq!(
            selection_reference("src/app.rs", 4, 9),
            "@src/app.rs (lines 4-9)"
        );
        assert_eq!(
            selection_reference("src/app.rs", 4, 4),
            "@src/app.rs (line 4)"
        );
        assert_eq!(
            selection_reference("/abs/my file.rs", 1, 2),
            "`/abs/my file.rs` (lines 1-2)"
        );
        assert_eq!(selection_reference("../x.rs", 1, 1), "`../x.rs` (line 1)");
    }
}
