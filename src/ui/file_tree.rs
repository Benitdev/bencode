//! Workspace file tree explorer: directory hierarchy with automatic file icons,
//! expandable folders, and direct file selection.

use std::path::{Path, PathBuf};
use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::files::FileIcon;
use ely_gpui_component::layout::on_axis;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    div, prelude::*, px,
};

use crate::app::{BenCodeApp, ViewMode};
use crate::workspace::FsNode;

impl BenCodeApp {
    pub fn toggle_folder_expanded(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.expanded_folders.contains(path) {
            self.expanded_folders.remove(path);
        } else {
            self.expanded_folders.insert(path.to_string());
        }
        cx.notify();
    }

    pub fn select_tree_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let cwd = self.workspace_cwd();
        let relative = Path::new(path)
            .strip_prefix(&cwd)
            .map_or(path.to_string(), |p| p.to_string_lossy().to_string());
        self.active_view_mode = ViewMode::Changes;
        self.select_diff_path(relative, cx);
    }

    pub fn render_file_tree(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors.clone();
        let session = self.sessions.iter().find(|s| self.selected_session_id.as_deref() == Some(&s.id));
        let cwd_str = session.map(|s| s.cwd.as_str()).unwrap_or(".");
        let cwd_path = PathBuf::from(cwd_str);
        let root_name = cwd_path.file_name().and_then(|n| n.to_str()).unwrap_or("workspace");

        let tree_nodes = self.workspace.tree.clone();

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(280.0))
            .h_full()
            .border_r_1()
            .border_color(colors.border)
            .bg(colors.surface)
            // 0. Top Segmented Mode Switcher (Sessions | Files | Changes)
            .child(self.render_sidebar_mode_tabs(cx))
            // 1. Header Toolbar
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .h(px(40.0))
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(FileIcon::folder(true).size(IconSize::Xs))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(cx.theme().text_size(TextSize::Xs))
                                    .text_color(colors.fg)
                                    .child(root_name.to_string()),
                            ),
                    )
                    .child(
                        IconButton::new("tree-refresh-btn", IconName::RotateCw)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip("Refresh files")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_workspace(cx);
                            })),
                    ),
            )
            // 2. Tree Content List
            .child(
                on_axis(div().id("file-tree-scroll"))
                    .flex_1()
                    .overflow_y_scroll()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .children(tree_nodes.into_iter().map(|node| {
                        self.render_tree_node(&node, 0, cx)
                    })),
            )
    }

    fn render_tree_node(&self, node: &FsNode, depth: usize, cx: &Context<Self>) -> gpui::AnyElement {
        let colors = &cx.theme().colors;
        let indent = depth as f32 * 14.0;
        let is_expanded = self.expanded_folders.contains(&node.path);
        let node_path = node.path.clone();
        let is_dir = node.is_dir;

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(SharedString::from(format!("node-{}", node.path.replace('/', "-").replace('.', "_"))))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .px_1p5()
                    .py_1()
                    .pl(px(indent + 6.0))
                    .rounded(cx.theme().radius(Radius::Sm))
                    .cursor_pointer()
                    .hover(|s| s.bg(colors.hover))
                    .on_click(cx.listener({
                        let path = node_path.clone();
                        move |this, _, _, cx| {
                            if is_dir {
                                this.toggle_folder_expanded(&path, cx);
                            } else {
                                this.select_tree_file(&path, cx);
                            }
                        }
                    }))
                    .when(is_dir, |el| {
                        el.child(
                            Icon::new(if is_expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                                .size(IconSize::Xs)
                                .color(colors.fg_subtle),
                        )
                    })
                    .when(!is_dir, |el| {
                        el.child(div().w(px(12.0)))
                    })
                    .child(if is_dir {
                        FileIcon::folder(is_expanded).size(IconSize::Xs)
                    } else {
                        FileIcon::file(&node.name).size(IconSize::Xs)
                    })
                    .child(
                        div()
                            .text_size(cx.theme().text_size(TextSize::Xs))
                            .text_color(if is_dir { colors.fg } else { colors.fg_muted })
                            .font_weight(if is_dir { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                            .child(node.name.clone()),
                    ),
            )
            .when(is_dir && is_expanded, |el| {
                el.children(
                    node.children
                        .iter()
                        .map(|child| self.render_tree_node(child, depth + 1, cx)),
                )
            })
            .into_any_element()
    }
}
