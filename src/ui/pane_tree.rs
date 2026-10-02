//! Pane tree view: the active tab's split chat panes, resized with Ely
//! `SplitPane`, docked by drag-and-drop, each streaming its own transcript.

use ely_gpui_component::buttons::{ButtonVariant, IconButton};
use ely_gpui_component::data_display::{Badge, Tone};
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::layout::SplitPane;
use ely_gpui_component::primitives::{DragGhost, Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize, Radius, TextSize};
use gpui::prelude::*;
use gpui::{
    AnyElement, Axis, Context, DragMoveEvent, FollowMode, FontWeight, IntoElement, ListState,
    ParentElement, SharedString, Stateful, Styled, div, list,
};

use crate::app::BenCodeApp;
use crate::db::SessionRow;
use crate::harness::catalog;
use crate::ui::drag_drop::{
    DraggedFile, DraggedPane, render_file_drop_hint, render_pane_drop_hint,
};
use crate::ui::layout::{LayoutNode, SplitDir, leaf_count, pane_edge_from_point, split_shares};
use crate::ui::theme;
use crate::ui::transcript::context_percent;

const UNTITLED: &str = "Untitled thread";

fn display_title(session: &SessionRow) -> &str {
    if session.title.trim().is_empty() {
        UNTITLED
    } else {
        session.title.as_str()
    }
}

/// Edge of `bounds` nearest to the drag position, as a docking edge.
fn drop_edge<T>(event: &DragMoveEvent<T>) -> crate::ui::layout::PaneEdge {
    let (bounds, pos) = (event.bounds, event.event.position);
    pane_edge_from_point(
        pos.x.into(),
        pos.y.into(),
        bounds.origin.x.into(),
        bounds.origin.y.into(),
        bounds.size.width.into(),
        bounds.size.height.into(),
    )
}

impl BenCodeApp {
    /// Renders the active tab's layout of chat panes.
    pub fn render_pane_tree(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // A drag released anywhere ends without a drop on a pane; GPUI then
        // redraws with no active drag, so stale hints are cleared here.
        if !cx.has_active_drag() {
            self.clear_drop_hints();
        }
        let shell = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(cx.theme().colors.bg);

        // The tree is a handful of ids; cloning it frees `self` for rendering.
        let Some(layout) = self.active_layout().cloned() else {
            return shell.justify_center().items_center().child(
                EmptyState::new("no-thread", IconName::MessageSquare, "No thread selected")
                    .body("Pick a thread from the sidebar or start a new one."),
            );
        };
        let in_split = leaf_count(&layout) > 1;
        shell.child(self.render_layout_node(&layout, in_split, cx))
    }

    fn render_layout_node(
        &mut self,
        node: &LayoutNode,
        in_split: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            LayoutNode::Leaf { id } => self.render_session_pane(id, in_split, cx),
            LayoutNode::Split {
                id,
                dir,
                children,
                sizes,
            } => self.render_split(id, *dir, children, sizes, in_split, cx),
        }
    }

    /// A split node as an Ely `SplitPane`; dragged shares go back into the tree.
    fn render_split(
        &mut self,
        split_id: &str,
        dir: SplitDir,
        children: &[LayoutNode],
        sizes: &[f32],
        in_split: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let [only] = children {
            return self.render_layout_node(only, in_split, cx);
        }
        if children.is_empty() {
            return div().into_any_element();
        }
        let axis = match dir {
            SplitDir::Right => Axis::Horizontal,
            SplitDir::Down => Axis::Vertical,
        };
        let theme = cx.theme();
        let min = theme.pane_min().to_pixels(theme.base_rem());
        let weak = cx.entity().downgrade();
        let owned_id = split_id.to_string();
        let mut split = SplitPane::new(SharedString::from(owned_id.clone()), axis, min)
            .sizes(&split_shares(children.len(), sizes))
            .on_resize(move |shares, _window, cx| {
                if let Err(err) = weak.update(cx, |this, _| this.resize_split(&owned_id, shares)) {
                    log::debug!("split resize after app drop: {err:#}");
                }
            });
        for child in children {
            let pane = self.render_layout_node(child, in_split, cx);
            split = split.pane(div().size_full().child(pane));
        }
        split.into_any_element()
    }

    fn render_session_pane(
        &mut self,
        session_id: &str,
        in_split: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.sessions.iter().any(|s| s.id == session_id) {
            return EmptyState::new(
                "missing-thread",
                IconName::MessageSquare,
                "Thread not found",
            )
            .into_any_element();
        }
        self.sync_transcript_list_for(session_id);
        let list_state = self.transcript_view_for(session_id).list.clone();

        // Borrowed, never cloned: a session's blocks can be huge.
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return div().into_any_element();
        };
        let is_focused = self.selected_session_id.as_deref() == Some(session_id);
        let drop_hint = self
            .active_pane_drop
            .as_ref()
            .filter(|target| target.over_id == session_id)
            .map(|target| target.edge);
        let file_hint = self.active_file_drop_target.as_deref() == Some(session_id);
        let border = match (in_split, is_focused) {
            (true, true) => cx.theme().colors.accent,
            (true, false) => cx.theme().colors.border,
            (false, _) => gpui::transparent_black(),
        };
        let composer = if is_focused {
            self.render_composer(Some(session), cx).into_any_element()
        } else {
            self.render_inactive_composer(session, cx)
                .into_any_element()
        };

        let frame = div()
            .id(SharedString::from(format!("pane-session-{session_id}")))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .border_1()
            .border_color(border);
        self.with_pane_listeners(frame, session_id, cx)
            .child(self.render_pane_header(session, in_split, is_focused, cx))
            .child(self.render_pane_body(session, list_state, cx))
            .child(composer)
            .when_some(drop_hint, |el, edge| {
                el.child(render_pane_drop_hint(edge, cx))
            })
            .when(file_hint, |el| el.child(render_file_drop_hint(cx)))
            .into_any_element()
    }

    /// Focus on click, pane docking and file attachment drops. Drag-move
    /// listeners fire for every pane, so each checks the pointer is inside.
    fn with_pane_listeners(
        &self,
        frame: Stateful<gpui::Div>,
        session_id: &str,
        cx: &Context<Self>,
    ) -> Stateful<gpui::Div> {
        let sid = session_id.to_string();
        let (focus_id, pane_id, drop_id, file_id, attach_id, ext_id) = (
            sid.clone(),
            sid.clone(),
            sid.clone(),
            sid.clone(),
            sid.clone(),
            sid,
        );
        frame
            .on_click(cx.listener(move |this, _, _, cx| this.focus_pane(focus_id.clone(), cx)))
            .on_drag_move::<DraggedPane>(cx.listener(
                move |this, event: &DragMoveEvent<DraggedPane>, _, cx| {
                    let inside = event.bounds.contains(&event.event.position);
                    let own = event.drag(cx).session_id == pane_id;
                    if inside && !own {
                        this.set_active_pane_drop(pane_id.clone(), drop_edge(event), cx);
                    } else {
                        this.clear_pane_drop(&pane_id, cx);
                    }
                },
            ))
            .on_drop(cx.listener(move |this, dragged: &DraggedPane, _, cx| {
                this.handle_pane_drop(&dragged.session_id, &drop_id, cx);
            }))
            .on_drag_move::<DraggedFile>(cx.listener(
                move |this, event: &DragMoveEvent<DraggedFile>, _, cx| {
                    if event.bounds.contains(&event.event.position) {
                        this.set_active_file_drop(Some(file_id.clone()), cx);
                    } else if this.active_file_drop_target.as_deref() == Some(file_id.as_str()) {
                        this.set_active_file_drop(None, cx);
                    }
                },
            ))
            .on_drop(cx.listener(move |this, file: &DraggedFile, _, cx| {
                this.attach_file_to_composer(&attach_id, &file.path, cx);
            }))
            .on_drop(
                cx.listener(move |this, paths: &gpui::ExternalPaths, _, cx| {
                    this.attach_external_paths_to_composer(&ext_id, paths.paths(), cx);
                }),
            )
    }

    /// The transcript list (or welcome), with "Jump to latest" while the
    /// list is scrolled away from its tail.
    fn render_pane_body(
        &self,
        session: &SessionRow,
        list_state: ListState,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let scrolled_up = list_state.is_scrolled_to_end() == Some(false);
        let content = if session.blocks.is_empty() {
            self.render_welcome(session, cx)
        } else {
            let sid = session.id.clone();
            list(
                list_state,
                cx.processor(move |this, ix: usize, window, cx| {
                    this.render_transcript_row_for(&sid, ix, window, cx)
                }),
            )
            .size_full()
            .into_any_element()
        };
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .justify_center()
            .child(content)
            .when(scrolled_up, |el| {
                el.child(self.render_jump_to_latest(&session.id, cx))
            })
    }

    fn render_jump_to_latest(&self, session_id: &str, cx: &Context<Self>) -> impl IntoElement {
        let sid = session_id.to_string();
        div()
            .absolute()
            .bottom_2()
            .left_0()
            .w_full()
            .flex()
            .justify_center()
            .child(
                IconButton::new(
                    SharedString::from(format!("jump-latest-{session_id}")),
                    IconName::ChevronDown,
                )
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Secondary)
                .tooltip("Jump to latest")
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(view) = this.transcripts.get(&sid) {
                        // Tail mode scrolls to the end and keeps following.
                        view.list.set_follow_mode(FollowMode::Tail);
                        cx.notify();
                    }
                })),
            )
    }

    /// Pane header: drag grip, harness dot, title and cwd on the left;
    /// badges and split / close actions on the right.
    fn render_pane_header(
        &self,
        session: &SessionRow,
        in_split: bool,
        is_focused: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap_2()
            .px_4()
            .py_1p5()
            .border_b_1()
            .border_color(colors.border)
            .bg(if is_focused {
                colors.surface
            } else {
                colors.bg
            })
            .child(self.render_pane_title(session, is_focused, cx))
            .child(self.render_pane_actions(session, in_split, cx))
    }

    fn render_pane_title(
        &self,
        session: &SessionRow,
        is_focused: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let title = display_title(session);
        let ghost_title = SharedString::from(title.to_string());
        let weight = if is_focused {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .child(
                div()
                    .id(SharedString::from(format!("pane-grip-{}", session.id)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_1()
                    .rounded(theme.radius(Radius::Sm))
                    .cursor_grab()
                    .hover(|s| s.bg(colors.hover))
                    .tooltip(Tooltip::text("Drag to split, move or detach pane"))
                    .on_drag(
                        DraggedPane {
                            session_id: session.id.clone(),
                        },
                        move |_, _, _, cx| {
                            DragGhost::new(ghost_title.clone(), Some(IconName::GripVertical), cx)
                        },
                    )
                    .child(
                        Icon::new(IconName::GripVertical)
                            .size(IconSize::Xs)
                            .color(colors.fg_muted),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .size(theme.status_dot())
                    .rounded_full()
                    .bg(theme::harness_color(&session.harness, colors)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(theme.text_size(TextSize::Sm))
                    .font_weight(weight)
                    .child(title.to_string()),
            )
            .when(!session.cwd.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .min_w_0()
                        .text_size(theme.text_size(TextSize::Xs))
                        .text_color(colors.fg_muted)
                        .child(Icon::new(IconName::Folder).size(IconSize::Xs))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .max_w(theme.label_width())
                                .child(session.cwd.clone()),
                        ),
                )
            })
    }

    fn render_pane_actions(
        &self,
        session: &SessionRow,
        in_split: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let action = |name: &str, icon: IconName, tooltip: &'static str| {
            IconButton::new(SharedString::from(format!("{name}-{}", session.id)), icon)
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .tooltip(tooltip)
        };
        let (right_id, down_id, close_id) =
            (session.id.clone(), session.id.clone(), session.id.clone());
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .when_some(context_percent(session), |el, pct| {
                el.child(Badge::new(format!("Context {pct}%")).tone(Tone::Info))
            })
            .when_some(session.branch.as_ref(), |el, branch| {
                el.child(Badge::new(branch.clone()))
            })
            .child(Badge::new(catalog::label_for(&session.model)))
            .child(
                action("split-r", IconName::Columns2, "Split right (⌘D)").on_click(cx.listener(
                    move |this, _, _, cx| this.split_pane_from(&right_id, SplitDir::Right, cx),
                )),
            )
            .child(
                action("split-d", IconName::Rows2, "Split down (⇧⌘D)").on_click(cx.listener(
                    move |this, _, _, cx| this.split_pane_from(&down_id, SplitDir::Down, cx),
                )),
            )
            .when(in_split, |el| {
                el.child(
                    action("close-pane", IconName::X, "Close split pane").on_click(
                        cx.listener(move |this, _, _, cx| this.close_pane(&close_id, cx)),
                    ),
                )
            })
    }

    /// Composer stand-in for unfocused panes; clicking focuses the pane.
    fn render_inactive_composer(
        &self,
        session: &SessionRow,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let sid = session.id.clone();
        div().flex_none().px_6().pb_4().pt_2().child(
            div()
                .id(SharedString::from(format!(
                    "inactive-composer-{}",
                    session.id
                )))
                .flex()
                .items_center()
                .justify_between()
                .px_4()
                .py_2()
                .rounded(theme.radius(Radius::Lg))
                .border_1()
                .border_color(colors.border)
                .bg(colors.surface)
                .cursor_pointer()
                .hover(|s| s.border_color(colors.accent))
                .on_click(cx.listener(move |this, _, _, cx| this.focus_pane(sid.clone(), cx)))
                .child(
                    div()
                        .text_size(theme.text_size(TextSize::Sm))
                        .text_color(colors.fg_muted)
                        .child("Click to activate thread and chat…"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(theme.text_size(TextSize::Xs))
                        .text_color(colors.fg_muted)
                        .child(Icon::new(IconName::MessageSquare).size(IconSize::Xs))
                        .child(catalog::label_for(&session.model)),
                ),
        )
    }
}
