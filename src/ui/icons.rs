//! Lucide icons MonoCode uses that Ely does not ship, drawn from embedded
//! SVGs in the text colour like Ely's own.

use gpui::{Hsla, IntoElement, Pixels, Styled, svg};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtraIcon {
    /// `folder-tree`: a worktree.
    FolderTree,
    /// `replace`: a handoff.
    Replace,
    /// `file-diff`: Open All Changes.
    FileDiff,
    /// `list-filter`: Filter sessions.
    ListFilter,
    /// `fold-vertical`: the Explorer's Collapse All.
    FoldVertical,
}

impl ExtraIcon {
    fn data(self) -> &'static [u8] {
        match self {
            Self::FolderTree => include_bytes!("../../assets/icons/folder-tree.svg"),
            Self::Replace => include_bytes!("../../assets/icons/replace.svg"),
            Self::FileDiff => include_bytes!("../../assets/icons/file-diff.svg"),
            Self::ListFilter => include_bytes!("../../assets/icons/list-filter.svg"),
            Self::FoldVertical => include_bytes!("../../assets/icons/fold-vertical.svg"),
        }
    }

    pub fn render(self, size: Pixels, color: Hsla) -> impl IntoElement {
        svg()
            .data(self.data())
            .size(size)
            .flex_none()
            .text_color(color)
    }
}
