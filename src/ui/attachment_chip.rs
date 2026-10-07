//! MonoCode `AttachmentChip`, shared by the composer (with a remove
//! button) and sent messages: an image as a 36px thumbnail that opens the
//! preview, any other file as a small chip with its file icon.

use std::path::PathBuf;
use std::rc::Rc;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ObjectFit, ParentElement, SharedString,
    Styled, StyledImage, div, img, prelude::*, px,
};
use serde_json::Value;

use crate::app::BenCodeApp;
use crate::harness::Attachment;
use crate::ui::file_tree::resolve_entry_icon;

/// What a chip shows of an attached file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChipFile {
    pub id: String,
    pub name: String,
    pub path: String,
    pub image: bool,
    pub folder: bool,
}

impl From<&Attachment> for ChipFile {
    fn from(file: &Attachment) -> Self {
        Self {
            id: file.id.clone(),
            name: file.name.clone(),
            path: file.path.clone(),
            image: file.is_image(),
            folder: file.mime_type == "inode/directory",
        }
    }
}

impl ChipFile {
    /// A file as a sent message keeps it (`Attachment::to_block_json`).
    pub fn from_block_json(value: &Value) -> Option<Self> {
        let field = |k: &str| value.get(k).and_then(Value::as_str).map(String::from);
        Some(Self {
            id: field("id")?,
            name: field("name")?,
            path: field("path").unwrap_or_default(),
            image: field("kind").as_deref() == Some("image"),
            folder: field("mimeType").as_deref() == Some("inode/directory"),
        })
    }
}

/// Removes a chip's file; absent on sent messages.
pub type OnRemove = Rc<dyn Fn(&mut BenCodeApp, &mut Context<BenCodeApp>)>;

pub fn attachment_chip(
    file: &ChipFile,
    on_remove: Option<OnRemove>,
    cx: &Context<BenCodeApp>,
) -> AnyElement {
    let colors = &cx.theme().colors;
    let remove = |size: f32, badge: bool| {
        on_remove.clone().map(|remove| {
            let hover = colors.fg.opacity(if badge { 0.3 } else { 0.15 });
            div()
                .id(SharedString::from(format!("att-remove-{}", file.id)))
                .size(px(size))
                .flex_none()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(badge, |el| {
                    el.absolute()
                        .top(px(-4.0))
                        .right(px(-4.0))
                        .bg(colors.fg.opacity(0.2))
                        .shadow_sm()
                })
                .hover(move |s| s.bg(hover))
                .tooltip(Tooltip::text("Remove"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    remove(this, cx);
                }))
                .child(Icon::new(IconName::X).size(IconSize::Xs).color(if badge {
                    colors.fg
                } else {
                    colors.fg.opacity(0.4)
                }))
        })
    };
    let id = SharedString::from(format!("att-{}", file.id));
    if file.image {
        let preview = PathBuf::from(&file.path);
        return div()
            .id(id)
            .relative()
            .size(px(36.0))
            .flex_none()
            .cursor_pointer()
            .tooltip(Tooltip::text(file.path.clone()))
            .on_click(cx.listener(move |this, _, _, cx| this.open_lightbox(preview.clone(), cx)))
            .child(
                div().size_full().rounded(px(8.0)).overflow_hidden().child(
                    img(PathBuf::from(&file.path))
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                ),
            )
            .children(remove(20.0, true))
            .into_any_element();
    }
    let icon = resolve_entry_icon(&file.name, file.folder, false);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .py_0p5()
        .px_1()
        .rounded(px(6.0))
        .bg(colors.fg.opacity(0.10))
        .tooltip(Tooltip::text(file.path.clone()))
        .child(icon.size(IconSize::Sm))
        .child(
            div()
                .max_w(px(140.0))
                .truncate()
                .text_size(px(11.0))
                .text_color(colors.fg.opacity(0.8))
                .child(file.name.clone()),
        )
        .children(remove(16.0, false))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sent_files_read_back_from_the_block() {
        let value = serde_json::json!({
            "id": "a", "name": "shot.png", "path": "/tmp/shot.png",
            "mimeType": "image/png", "kind": "image", "size": 3
        });
        let chip = ChipFile::from_block_json(&value).unwrap();
        assert!(chip.image && !chip.folder);
        assert_eq!(chip.path, "/tmp/shot.png");
        assert!(ChipFile::from_block_json(&serde_json::json!({ "name": "x" })).is_none());
    }
}
