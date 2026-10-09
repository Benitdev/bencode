//! Lucide icons MonoCode uses that Ely does not ship. [`Assets`] serves
//! their embedded SVGs beside Ely's own, so [`ExtraIcon::icon`] is an Ely
//! `Icon` with the theme's sizes, colours and hover.

use std::borrow::Cow;

use ely_gpui_component::primitives::Icon;
use gpui::{AssetSource, Hsla, IntoElement, Pixels, SharedString, Styled, svg};

macro_rules! extra_icons {
    ($($(#[$doc:meta])* $variant:ident => $file:literal,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum ExtraIcon {
            $($(#[$doc])* $variant,)*
        }

        impl ExtraIcon {
            const ALL: &[ExtraIcon] = &[$(ExtraIcon::$variant,)*];

            /// Where [`Assets`] serves it, clear of Ely's `icons/`.
            fn path(self) -> &'static str {
                match self {
                    $(Self::$variant => concat!("bencode/icons/", $file, ".svg"),)*
                }
            }

            fn data(self) -> &'static [u8] {
                match self {
                    $(Self::$variant => {
                        include_bytes!(concat!("../../assets/icons/", $file, ".svg"))
                    })*
                }
            }
        }
    };
}

extra_icons! {
    /// `folder-tree`: a worktree.
    FolderTree => "folder-tree",
    /// `replace`: a handoff.
    Replace => "replace",
    /// `file-diff`: Open All Changes.
    FileDiff => "file-diff",
    /// `list-filter`: Filter sessions.
    ListFilter => "list-filter",
    /// `fold-vertical`: the Explorer's Collapse All.
    FoldVertical => "fold-vertical",
    /// `unfold-vertical`: a review's Expand all files.
    UnfoldVertical => "unfold-vertical",
    /// `bell-off`: a project's muted notifications.
    BellOff => "bell-off",
    /// `image-plus`: Add project logo.
    ImagePlus => "image-plus",
    /// Hugeicons `comment-add-01`: a transcript selection's Add to chat.
    CommentAdd => "comment-add",
    /// Hugeicons `file-plus-corner`: a transcript selection's Add to notes.
    FilePlusCorner => "file-plus-corner",
    /// `panel-top`: the terminal docked at the top.
    PanelTop => "panel-top",
    /// `circle-arrow-down`: Update to (MonoCode `ArrowDownCircle`).
    CircleArrowDown => "circle-arrow-down",
}

impl ExtraIcon {
    pub fn icon(self) -> Icon {
        Icon::from_path(self.path())
    }

    #[allow(dead_code)]
    pub fn render(self, size: Pixels, color: Hsla) -> impl IntoElement {
        svg()
            .data(self.data())
            .size(size)
            .flex_none()
            .text_color(color)
    }
}

/// Ely's assets plus BenCode's icons. Pass to `Application::with_assets`.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        match ExtraIcon::ALL.iter().find(|icon| icon.path() == path) {
            Some(icon) => Ok(Some(Cow::Borrowed(icon.data()))),
            None => ely_gpui_component::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut names = ely_gpui_component::Assets.list(path)?;
        names.extend(
            ExtraIcon::ALL
                .iter()
                .map(|icon| icon.path())
                .filter(|name| name.starts_with(path))
                .map(SharedString::from),
        );
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use ely_gpui_component::primitives::IconName;
    use gpui::AssetSource;

    use super::{Assets, ExtraIcon};

    #[test]
    fn every_extra_icon_is_served() {
        for icon in ExtraIcon::ALL {
            let data = Assets.load(icon.path()).expect("the source loads");
            assert_eq!(data.as_deref(), Some(icon.data()), "{icon:?}");
        }
    }

    #[test]
    fn elys_icons_still_load() {
        let data = Assets
            .load(IconName::Check.path())
            .expect("the source loads");
        assert!(data.is_some());
        assert!(
            Assets
                .load("bencode/icons/missing.svg")
                .expect("the source loads")
                .is_none()
        );
    }
}
