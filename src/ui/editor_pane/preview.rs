//! MonoCode `MarkdownViewShell` and `MarkdownDocumentPreview`
//! (`src/features/sessions/ui/`) with `fileViewMode.ts`: a markdown file
//! opens as Preview or Source. Each open file keeps its own choice; the
//! last one picked is what the next file opens in. The preview draws the
//! buffer, so unsaved edits show, and a leading YAML frontmatter folds
//! into "Properties".

use std::path::Path;
use std::rc::Rc;

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::ui::scale::px;
use crate::ui::transcript::markdown::AgentMarkdown;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// How a previewable file is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileViewMode {
    #[default]
    Preview,
    Source,
}

impl FileViewMode {
    pub fn key(self) -> &'static str {
        match self {
            FileViewMode::Preview => "preview",
            FileViewMode::Source => "source",
        }
    }

    pub fn from_key(key: &str) -> Self {
        if key == "source" {
            FileViewMode::Source
        } else {
            FileViewMode::Preview
        }
    }
}

/// MonoCode `isMarkdownPath`.
pub fn is_markdown_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ["md", "mdx", "markdown"]
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
}

/// MonoCode `splitMarkdownFrontmatter`: a leading `---` block, uninterpreted,
/// and the body after it. An unclosed block stays in the body.
pub fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let rest = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let Some(after) = rest.strip_prefix("---") else {
        return (None, text);
    };
    let Some(newline) = after.find('\n') else {
        return (None, text);
    };
    if !after[..newline].trim_end_matches('\r').trim().is_empty() {
        return (None, text);
    }
    let block = &after[newline + 1..];
    let mut at = 0;
    for line in block.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        let closes = ["---", "..."].iter().any(|fence| {
            bare.strip_prefix(fence)
                .is_some_and(|t| t.trim().is_empty())
        });
        if closes {
            let metadata = block[..at].trim_end_matches('\n').trim_end_matches('\r');
            return (Some(metadata), &block[at + line.len()..]);
        }
        at += line.len();
    }
    (None, text)
}

impl BenCodeApp {
    /// The view `path` is shown in: its own choice, else the last one made.
    pub(crate) fn file_view_mode(&self, path: &str) -> FileViewMode {
        self.editor
            .view_modes
            .get(path)
            .copied()
            .unwrap_or(self.editor.default_view_mode)
    }

    pub(crate) fn set_file_view_mode(
        &mut self,
        path: &str,
        mode: FileViewMode,
        cx: &mut Context<Self>,
    ) {
        self.editor.view_modes.insert(path.to_string(), mode);
        if self.editor.default_view_mode != mode {
            self.editor.default_view_mode = mode;
            self.save_settings(cx);
        }
        cx.notify();
    }

    /// The rendered document, scrolled on its own.
    pub(super) fn render_markdown_preview(
        &self,
        path: &str,
        text: &str,
        cx: &Context<Self>,
    ) -> AnyElement {
        let (metadata, body) = split_frontmatter(text);
        let app = cx.entity().downgrade();
        let markdown = AgentMarkdown::new(
            SharedString::from(format!("file-preview:{path}")),
            body.to_string(),
        )
        .on_open_file(Rc::new(move |path, _line, window, cx| {
            let opened = app.update(cx, |this, cx| this.open_file_in_editor(path, window, cx));
            if let Err(err) = opened {
                log::debug!("preview file link after app drop: {err:#}");
            }
        }));
        div()
            .id(SharedString::from(format!("file-preview-scroll:{path}")))
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .px(px(24.0))
                    .py(px(32.0))
                    .children(metadata.map(|m| self.render_properties(path, m, cx)))
                    .child(markdown),
            )
            .into_any_element()
    }

    /// MonoCode's "Properties" `<details>`: the frontmatter as written.
    fn render_properties(&self, path: &str, metadata: &str, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let open = self.editor.properties_open.contains(path);
        let key = path.to_string();
        let summary = div()
            .id(SharedString::from(format!(
                "file-preview-properties:{path}"
            )))
            .flex()
            .items_center()
            .gap(px(6.0))
            .px_3()
            .py_2()
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(Text::Base.px())
            .text_color(fg.ink(Ink::Secondary))
            .hover(|s| s.text_color(fg))
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(IconSize::Xs)
                .color(fg.ink(Ink::Secondary)),
            )
            .child("Properties")
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.editor.properties_open.remove(&key) {
                    this.editor.properties_open.insert(key.clone());
                }
                cx.notify();
            }));
        div()
            .mb(px(24.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(fg.fill(Fill::Strong))
            .bg(fg.fill(Fill::Faint))
            .child(summary)
            .when(open, |el| {
                el.child(
                    div()
                        .border_t_1()
                        .border_color(theme.colors.border)
                        .px_3()
                        .py_2()
                        .font_family(theme.mono_family.clone())
                        .text_size(Text::Base.px())
                        .line_height(px(20.0))
                        .text_color(fg.ink(Ink::Body))
                        .child(metadata.to_string()),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{is_markdown_path, split_frontmatter};

    #[test]
    fn markdown_paths() {
        assert!(is_markdown_path("docs/README.md"));
        assert!(is_markdown_path("a.MDX"));
        assert!(is_markdown_path("notes.markdown"));
        assert!(!is_markdown_path("main.rs"));
        assert!(!is_markdown_path("md"));
    }

    #[test]
    fn frontmatter_splits_off() {
        let text = "---\ntitle: A\ntags: [x]\n---\n# Body\n";
        assert_eq!(
            split_frontmatter(text),
            (Some("title: A\ntags: [x]"), "# Body\n")
        );
        let crlf = "\u{FEFF}---\r\nname: b\r\n...\r\nrest";
        assert_eq!(split_frontmatter(crlf), (Some("name: b"), "rest"));
        assert_eq!(split_frontmatter("---\n---"), (Some(""), ""));
    }

    #[test]
    fn no_frontmatter_keeps_the_text() {
        for text in [
            "# Title\n---\n",
            "---\nunclosed: yes\n",
            "--- x\na\n---\n",
            "",
        ] {
            assert_eq!(split_frontmatter(text), (None, text));
        }
    }
}
