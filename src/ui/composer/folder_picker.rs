//! MonoCode `/add-to-folder` (`SessionFolderPicker`): files the focused
//! thread in one of the project's sidebar folders, or a new one named by
//! the search.

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, Focusable, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use super::focus_later;
use super::search_popover::SearchPopover;
use crate::app::BenCodeApp;
use crate::app::session_folders::{FolderTarget, picker_rows};

/// MonoCode `max-h-[min(240px,40vh)]`.
const LIST_MAX_HEIGHT: gpui::Pixels = px(240.0);

impl BenCodeApp {
    fn folder_rows(&self, cx: &Context<Self>) -> Vec<FolderTarget> {
        picker_rows(
            self.project_folders(),
            self.folder_search_input.read(cx).text(),
        )
    }

    /// `/add-to-folder`: the token leaves the prompt and the picker opens.
    pub fn start_folder_command(&mut self, cx: &mut Context<Self>) {
        self.remove_prompt_token(cx);
        self.open_folder_picker(cx);
    }

    pub fn open_folder_picker(&mut self, cx: &mut Context<Self>) {
        if self.selected_session_id.is_none() {
            return;
        }
        self.is_skill_picker_open = false;
        self.is_mention_picker_open = false;
        self.folder_picker = Some(0);
        self.folder_search_input
            .update(cx, |input, cx| input.set_text("", cx));
        focus_later(self.folder_search_input.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    pub fn close_folder_picker(&mut self, refocus: bool, cx: &mut Context<Self>) -> bool {
        if self.folder_picker.take().is_none() {
            return false;
        }
        if refocus {
            self.refocus_prompt(cx);
        }
        cx.notify();
        true
    }

    pub fn on_folder_query_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(active) = &mut self.folder_picker {
            *active = 0;
        }
        cx.notify();
    }

    /// ↑/↓ (wrapping, as MonoCode), Enter and Esc in the picker's search.
    pub fn folder_picker_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(active) = self.folder_picker else {
            return false;
        };
        let len = self.folder_rows(cx).len().max(1);
        match key {
            "up" => self.folder_picker = Some((active + len - 1) % len),
            "down" => self.folder_picker = Some((active + 1) % len),
            "enter" => self.pick_folder(active, cx),
            "escape" => {
                self.close_folder_picker(true, cx);
            }
            _ => return false,
        }
        if let Some(active) = self.folder_picker {
            self.picker_scroll.scroll_to_item(active);
        }
        cx.notify();
        true
    }

    fn pick_folder(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(target) = self.folder_rows(cx).into_iter().nth(ix) else {
            return;
        };
        let Some(session_id) = self.selected_session_id.clone() else {
            return;
        };
        self.close_folder_picker(true, cx);
        self.place_session_in_folder(&session_id, &target, cx);
    }

    pub(super) fn render_folder_picker(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let active = self.folder_picker?;
        let rows = self.folder_rows(cx);
        let empty = rows
            .is_empty()
            .then(|| SharedString::from("Type a name to create the first folder"));
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(ix, target)| self.render_folder_row(ix, target, active == ix, cx))
            .collect();
        Some(
            SearchPopover {
                scroll: &self.picker_scroll,
                id: "folder-picker",
                icon: IconName::Folder,
                input: &self.folder_search_input,
                close: (IconName::X, "Cancel", |this, cx| {
                    this.close_folder_picker(true, cx);
                }),
                dismiss: |this, cx| {
                    this.close_folder_picker(true, cx);
                },
                list_max_height: LIST_MAX_HEIGHT,
                empty,
                rows,
                footer: None,
            }
            .render(cx),
        )
    }

    /// A folder with its thread count, or MonoCode's "Create “name”".
    fn render_folder_row(
        &self,
        ix: usize,
        target: FolderTarget,
        active: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let (fg, skill) = (colors.fg, colors.warning);
        let row = div()
            .id(("folder-row", ix))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_2()
            .rounded(px(6.0))
            .text_size(px(13.0))
            .cursor_pointer()
            .when(active, |el| el.bg(skill.opacity(0.15)))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.folder_picker != Some(ix) {
                    this.folder_picker = Some(ix);
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.pick_folder(ix, cx)));
        match target {
            FolderTarget::Existing(id) => {
                let Some(folder) = self.project_folders().iter().find(|f| f.id == id) else {
                    return row.into_any_element();
                };
                row.when(!active, |el| el.hover(move |s| s.bg(fg.opacity(0.05))))
                    .child(
                        Icon::new(IconName::Folder)
                            .size(IconSize::Xs)
                            .color(fg.opacity(if active { 0.7 } else { 0.45 })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(if active { fg } else { fg.opacity(0.8) })
                            .child(SharedString::from(folder.name.clone())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.0))
                            .text_color(fg.opacity(0.45))
                            .child(folder.session_ids.len().to_string()),
                    )
                    .into_any_element()
            }
            FolderTarget::New(name) => row
                .child(Icon::new(IconName::Plus).size(IconSize::Xs).color(skill))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(fg)
                        .child(format!("Create “{name}”")),
                )
                .into_any_element(),
        }
    }
}
