use std::path::{Path, PathBuf};
use ely_gpui_component::forms::TextInput;
use ely_gpui_component::layout::on_axis;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize, Radius, TextSize};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, prelude::*, px,
};

use crate::app::{BenCodeApp, SidebarMode, ViewMode};
use crate::ui::theme::MonoTheme;

#[derive(Clone, Debug)]
pub struct FsNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub children: Vec<FsNode>,
}

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
        self.selected_diff_path = Some(path.to_string());
        self.active_view_mode = ViewMode::Changes;
        cx.notify();
    }

    pub fn render_file_tree(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let session = self.sessions.iter().find(|s| self.selected_session_id.as_deref() == Some(&s.id));
        let cwd_str = session.map(|s| s.cwd.as_str()).unwrap_or(".");
        let cwd_path = PathBuf::from(cwd_str);
        let root_name = cwd_path.file_name().and_then(|n| n.to_str()).unwrap_or("workspace");

        // Scan top-level directory entries
        let tree_nodes = scan_directory(&cwd_path, 2);

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(280.0))
            .h_full()
            .border_r_1()
            .border_color(MonoTheme::border_stroke())
            .bg(MonoTheme::bg_surface())
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
                    .border_color(MonoTheme::border_stroke())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(Icon::new(IconName::Folder).size(IconSize::Xs).color(MonoTheme::accent()))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_primary())
                                    .child(root_name.to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .id("tree-switch-to-sessions-btn")
                                    .px_2()
                                    .py_0p5()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::bg_hover())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_muted())
                                    .cursor_pointer()
                                    .hover(|s| s.bg(MonoTheme::bg_active()).text_color(MonoTheme::fg_primary()))
                                    .child("Threads")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.sidebar_mode = SidebarMode::Sessions;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("tree-refresh-btn")
                                    .size(px(24.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(theme.radius(Radius::Sm))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(MonoTheme::bg_hover()))
                                    .child(Icon::new(IconName::RotateCw).size(IconSize::Xs).color(MonoTheme::fg_muted()))
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.notify();
                                    })),
                            ),
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
        let theme = cx.theme();
        let indent = depth as f32 * 14.0;
        let is_expanded = self.expanded_folders.contains(&node.path);
        let node_path = node.path.clone();
        let is_dir = node.is_dir;

        let icon_name = if is_dir {
            IconName::Folder
        } else if node.name.ends_with(".rs") {
            IconName::FileText
        } else if node.name.ends_with(".sh") || node.name.ends_with(".zsh") {
            IconName::Terminal
        } else if node.name.ends_with(".toml") || node.name.ends_with(".json") || node.name.ends_with(".yaml") {
            IconName::Settings
        } else {
            IconName::FileText
        };

        let icon_color = if is_dir {
            MonoTheme::accent()
        } else if node.name.ends_with(".rs") {
            MonoTheme::skill_gold()
        } else if node.name.ends_with(".toml") || node.name.ends_with(".json") {
            MonoTheme::mention_cyan()
        } else {
            MonoTheme::fg_muted()
        };

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
                    .rounded(theme.radius(Radius::Sm))
                    .cursor_pointer()
                    .hover(|s| s.bg(MonoTheme::bg_hover()))
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
                                .color(MonoTheme::fg_subtle()),
                        )
                    })
                    .when(!is_dir, |el| {
                        el.child(div().w(px(12.0)))
                    })
                    .child(Icon::new(icon_name).size(IconSize::Xs).color(icon_color))
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(if is_dir { MonoTheme::fg_primary() } else { MonoTheme::fg_muted() })
                            .font_weight(if is_dir { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                            .truncate()
                            .child(node.name.clone()),
                    ),
            )
            .when(is_dir && is_expanded, |el| {
                el.children(node.children.iter().map(|child| {
                    self.render_tree_node(child, depth + 1, cx)
                }))
            })
            .into_any_element()
    }
}

fn scan_directory(dir: &Path, max_depth: usize) -> Vec<FsNode> {
    if max_depth == 0 {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }

        let path_str = entry.path().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

        if is_dir {
            let children = scan_directory(&entry.path(), max_depth - 1);
            dirs.push(FsNode {
                name,
                path: path_str,
                is_dir: true,
                children,
            });
        } else {
            files.push(FsNode {
                name,
                path: path_str,
                is_dir: false,
                children: Vec::new(),
            });
        }
    }

    dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    dirs.extend(files);
    dirs
}
