//! Files sidebar: the workspace as an Ely `FileTree` with git status marks.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::layout::Sidebar;
use ely_gpui_component::lists::FileTree;
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, ControlSize, TextSize};
use gpui::{Context, FontWeight, IntoElement, ParentElement, SharedString, Styled, div};

use crate::app::{BenCodeApp, ViewMode};
use crate::ui::git_changes_panel::to_ely_status;

impl BenCodeApp {
    /// Opens a repo-relative `path` in the Changes view.
    pub fn select_tree_file(&mut self, path: &str, cx: &mut Context<Self>) {
        self.active_view_mode = ViewMode::Changes;
        self.select_diff_path(path.to_string(), cx);
    }

    pub fn render_file_tree(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let root = std::path::Path::new(&self.workspace.cwd)
            .file_name()
            .map_or_else(|| "workspace".to_string(), |name| name.to_string_lossy().into_owned());
        let changes = self.git_status.staged.iter().chain(&self.git_status.unstaged);
        let tree = changes.fold(
            FileTree::new("workspace-files", self.workspace.files.iter().cloned()),
            |tree, change| tree.status(change.path.clone(), to_ely_status(&change.status)),
        );

        Sidebar::new("files-sidebar", false)
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
                        IconButton::new("tree-refresh", IconName::RotateCw)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("Refresh files")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_workspace(cx))),
                    ),
            )
            .child(
                tree.on_open(cx.listener(|this, path: &SharedString, _, cx| this.select_tree_file(path, cx)))
                    .flex_1()
                    .min_h_0()
                    .px_2()
                    .pb_2(),
            )
    }
}
