//! Drag payloads and the drop-target overlays drawn over panes.

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize, Radius, TextSize};
use gpui::{App, Context, IntoElement, ParentElement, Render, Styled, Window, div};

use crate::ui::layout::PaneEdge;
use crate::ui::scale::px;

/// Payload when dragging a split pane by its grip. The preview is an Ely
/// `DragGhost`.
#[derive(Clone, Debug, PartialEq)]
pub struct DraggedPane {
    pub session_id: String,
}

/// Payload when dragging a file from the file tree.
#[derive(Clone, Debug, PartialEq)]
pub struct DraggedFile {
    pub path: String,
    pub name: String,
}

impl Render for DraggedFile {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        div()
            .flex()
            .items_center()
            .min_w_0()
            .gap_1p5()
            .px_3()
            .py_1p5()
            .rounded(theme.radius(Radius::Md))
            .bg(colors.surface)
            .border_1()
            .border_color(colors.accent)
            .shadow_lg()
            .text_size(theme.text_size(TextSize::Xs))
            .text_color(colors.fg)
            .child(
                div().flex_none().child(
                    Icon::new(IconName::FileText)
                        .size(IconSize::Xs)
                        .color(colors.accent),
                ),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .max_w(theme.menu_width())
                    .child(self.name.clone()),
            )
    }
}

/// Payload when dragging a session card out of the sidebar: onto a folder
/// or card it groups, onto a pane it splits (MonoCode `onPlaceOnPane`).
#[derive(Clone, Debug, PartialEq)]
pub struct DraggedSession {
    pub session_id: String,
    pub title: String,
}

impl Render for DraggedSession {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        div()
            .max_w(px(240.0))
            .px(px(10.0))
            .py_1p5()
            .rounded(theme.radius(Radius::Md))
            .bg(colors.surface)
            .border_1()
            .border_color(colors.border)
            .shadow_lg()
            .opacity(0.9)
            .text_size(px(13.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(colors.fg)
            .child(div().truncate().child(self.title.clone()))
    }
}

/// Where a dragged pane would dock: an edge of the pane under the pointer.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneDropTarget {
    pub over_id: String,
    pub edge: PaneEdge,
}

/// Washes the half of the pane a dragged pane would take, with an accent
/// line on the docking edge.
pub fn render_pane_drop_hint(edge: PaneEdge, cx: &App) -> impl IntoElement {
    let colors = &cx.theme().colors;
    let wash = div()
        .absolute()
        .bg(colors.accent.opacity(0.18))
        .border_color(colors.accent);
    let wash = match edge {
        PaneEdge::Left => wash.top_0().bottom_0().left_0().w_1_2().border_l_2(),
        PaneEdge::Right => wash.top_0().bottom_0().right_0().w_1_2().border_r_2(),
        PaneEdge::Top => wash.left_0().right_0().top_0().h_1_2().border_t_2(),
        PaneEdge::Bottom => wash.left_0().right_0().bottom_0().h_1_2().border_b_2(),
    };
    div().absolute().inset_0().child(wash)
}
