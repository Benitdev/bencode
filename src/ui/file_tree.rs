//! Files sidebar: the workspace as an Ely `FileTree` with git status marks.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::layout::Sidebar;
use ely_gpui_component::lists::FileTree;
use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, Radius, TextSize};
use gpui::prelude::*;
use gpui::{Context, FontWeight, IntoElement, ParentElement, SharedString, Styled, div};

use crate::app::BenCodeApp;
use crate::ui::git_changes_panel::to_ely_status;

impl BenCodeApp {
    /// Opens a repo-relative `path` in the native code editor.
    pub fn select_tree_file(
        &mut self,
        path: &str,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file_in_editor(path, window, cx);
    }

    pub fn render_file_tree(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let root = std::path::Path::new(&self.workspace.cwd)
            .file_name()
            .map_or_else(
                || "workspace".to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
        let changes = self
            .git_status
            .staged
            .iter()
            .chain(&self.git_status.unstaged);
        let tree = changes.fold(
            FileTree::new("workspace-files", self.workspace.files.iter().cloned()),
            |tree, change| tree.status(change.path.clone(), to_ely_status(&change.status)),
        );

        let mut sidebar = Sidebar::new("files-sidebar", false)
            .child(self.render_sidebar_mode_tabs(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_1()
                    .text_size(theme.text_size(TextSize::Xs))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.colors.fg_muted)
                    .child(root.to_uppercase())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                IconButton::new("tree-open-editor", IconName::ExternalLink)
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .tooltip("Open workspace in external editor")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_in_external_editor(None, None, cx);
                                    })),
                            )
                            .child(
                                IconButton::new("tree-refresh", IconName::RotateCw)
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .tooltip("Refresh files")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.refresh_workspace(cx)),
                                    ),
                            ),
                    ),
            );

        if !self.git_status.staged.is_empty() || !self.git_status.unstaged.is_empty() {
            let changes: Vec<String> = self
                .git_status
                .staged
                .iter()
                .chain(&self.git_status.unstaged)
                .map(|c| c.path.clone())
                .collect();
            let theme = cx.theme();
            sidebar = sidebar.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_3()
                    .py_1p5()
                    .border_b_1()
                    .border_color(theme.colors.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.colors.fg_muted)
                            .child("DRAG TO ATTACH")
                            .child(format!("{} modified", changes.len())),
                    )
                    .children(changes.into_iter().take(6).map(|path| {
                        let file_name = std::path::Path::new(&path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.clone());
                        let drag_path = path.clone();
                        let drag_name = file_name.clone();
                        let el_id = format!("drag-file-{}", path);

                        div()
                            .id(SharedString::from(el_id))
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .px_2()
                            .py_1()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(theme.colors.surface)
                            .hover(|s| s.bg(theme.colors.hover))
                            .cursor_grab()
                            .tooltip(Tooltip::text("Drag into chat to attach file"))
                            .on_drag(
                                crate::ui::drag_drop::DraggedFile {
                                    path: drag_path,
                                    name: drag_name,
                                },
                                |dragged, _, _, cx| cx.new(|_| dragged.clone()),
                            )
                            .child(
                                Icon::new(IconName::FileText)
                                    .size(IconSize::Xs)
                                    .color(theme.colors.accent),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .child(file_name),
                            )
                    })),
            );
        }

        sidebar.child(
            tree.on_open(cx.listener(|this, path: &SharedString, window, cx| {
                this.select_tree_file(path, window, cx)
            }))
            .flex_1()
            .h_full(),
        )
    }
}
