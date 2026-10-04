//! MonoCode's sidebar folders (`Sidebar.tsx` `FolderRow`): a tinted shell
//! with the folder's row (icon, name, count) over its threads; a click folds
//! it, its menu renames or ungroups it, and each thread's menu files it.

use ely_gpui_component::menus::{ContextMenu, Menu, MenuItem};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::BenCodeApp;
use crate::app::session_folders::{SessionFolder, folder_of};
use crate::db::SessionRow;
use crate::ui::app_callback::app_callback;
use crate::ui::sidebar::SessionDialog;

impl BenCodeApp {
    /// A folder and, unless folded, its threads (`cards`).
    pub(crate) fn render_session_folder(
        &self,
        folder: &SessionFolder,
        cards: Vec<AnyElement>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let group = SharedString::from(format!("folder-{}", folder.id));
        let open = !folder.collapsed;
        let toggle_id = folder.id.clone();
        // MonoCode swaps the folder icon for a chevron on hover and while open.
        let glyph = div()
            .relative()
            .size(px(14.0))
            .flex_none()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .when(open, |el| el.opacity(0.0))
                    .group_hover(group.clone(), |s| s.opacity(0.0))
                    .child(
                        Icon::new(IconName::Folder)
                            .size(IconSize::Sm)
                            .color(fg.opacity(0.55)),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .when(!open, |el| el.opacity(0.0))
                    .group_hover(group.clone(), |s| s.opacity(1.0))
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(IconSize::Sm)
                        .color(fg.opacity(0.55)),
                    ),
            );
        let header = div()
            .id(SharedString::from(format!("folder-row-{}", folder.id)))
            .group(group)
            .flex()
            .items_center()
            .gap_1p5()
            .h(px(32.0))
            .px_2()
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(move |s| s.bg(fg.opacity(0.05)))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_folder(&toggle_id, cx)))
            .child(glyph)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg)
                    .child(SharedString::from(folder.name.clone())),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.0))
                    .text_color(fg.opacity(0.45))
                    .child(folder.session_ids.len().to_string()),
            );
        div()
            .flex()
            .flex_col()
            .rounded(px(6.0))
            .bg(fg.opacity(0.05))
            .child(
                ContextMenu::new(
                    SharedString::from(format!("folder-menu-{}", folder.id)),
                    self.folder_menu(folder, cx),
                )
                .child(header),
            )
            .when(open, |el| el.children(cards))
            .into_any_element()
    }

    /// MonoCode's folder menu: Rename, Ungroup.
    fn folder_menu(&self, folder: &SessionFolder, cx: &Context<Self>) -> Menu {
        let (rename, ungroup) = (folder.id.clone(), folder.id.clone());
        let name = folder.name.clone();
        Menu::new()
            .item(
                MenuItem::new("Rename…")
                    .icon(IconName::Pencil)
                    .on_click(app_callback(cx, move |this, cx| {
                        this.rename_input
                            .update(cx, |input, cx| input.set_text(name.clone(), cx));
                        this.session_dialog = Some(SessionDialog::RenameFolder(rename.clone()));
                        cx.notify();
                    })),
            )
            .item(
                MenuItem::new("Ungroup")
                    .icon(IconName::FolderOpen)
                    .on_click(app_callback(cx, move |this, cx| {
                        this.ungroup_folder(&ungroup, cx)
                    })),
            )
    }

    /// The thread menu's folder items: New folder, Add to each folder,
    /// Remove from folder.
    pub(crate) fn session_folder_items(
        &self,
        menu: Menu,
        session: &SessionRow,
        cx: &Context<Self>,
    ) -> Menu {
        let folders = self.project_folders();
        let current = folder_of(folders, &session.id).map(|f| f.id.clone());
        let new_id = session.id.clone();
        let mut menu = menu.separator().item(
            MenuItem::new("New folder")
                .icon(IconName::FolderPlus)
                .on_click(app_callback(cx, move |this, cx| {
                    this.new_folder_with_session(&new_id, cx)
                })),
        );
        for folder in folders {
            let (sid, target) = (session.id.clone(), folder.id.clone());
            let checked = current.as_deref() == Some(folder.id.as_str());
            menu = menu.item(
                MenuItem::check(format!("Add to {}", folder.name), checked).on_click(app_callback(
                    cx,
                    move |this, cx| {
                        let target =
                            crate::app::session_folders::FolderTarget::Existing(target.clone());
                        this.place_session_in_folder(&sid, &target, cx)
                    },
                )),
            );
        }
        if current.is_some() {
            let sid = session.id.clone();
            menu = menu.item(
                MenuItem::new("Remove from folder").on_click(app_callback(cx, move |this, cx| {
                    this.remove_session_from_folder(&sid, cx)
                })),
            );
        }
        menu
    }
}
