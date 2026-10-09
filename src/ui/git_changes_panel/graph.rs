//! MonoCode `GitHistoryGraph`: the commit graph under the Changes list, its
//! sash and row painting.

use super::*;

impl BenCodeApp {
    /// MonoCode `GraphResizeSash`.
    pub(super) fn render_graph_sash(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let dragging = self.changes_ui.graph_drag.is_some();
        let hover = fg.opacity(0.10);
        div()
            .id("git-graph-sash")
            .h(px(6.0))
            .flex_none()
            .cursor_row_resize()
            .when(dragging, |el| el.bg(fg.opacity(0.15)))
            .when(!dragging, |el| el.hover(move |s| s.bg(hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if event.click_count >= 2 {
                        this.changes_ui.graph_height = GRAPH_DEFAULT;
                    } else {
                        this.changes_ui.graph_drag = Some((
                            crate::ui::scale::logical(event.position.y),
                            this.changes_ui.graph_height,
                        ));
                    }
                    cx.notify();
                }),
            )
    }

    /// MonoCode `GitHistoryGraph`: "GRAPH" and its chevron over the rows.
    /// The graph layout of `git_history`, laid out once per history.
    fn graph_rows(&self) -> Rc<Vec<git_graph::Row>> {
        self.changes_ui
            .graph_rows
            .borrow_mut()
            .get_or_insert_with(|| Rc::new(git_graph::layout(&self.git_history)))
            .clone()
    }

    pub(super) fn render_graph(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let open = !self.changes_ui.graph_closed;
        let max = (self.changes_ui.panel_height.get() - 160.0).max(GRAPH_MIN);
        let height = self.changes_ui.graph_height.min(max);
        let hover = fg.opacity(0.05);
        let commits = &self.git_history;
        let rows = self.graph_rows();
        let selected = match self.file_pane.active() {
            Some(PaneTab::Commit { cwd, sha, .. }) if *cwd == self.workspace.cwd => {
                Some(sha.clone())
            }
            _ => None,
        };
        div()
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .border_t_1()
            .border_color(stroke(fg))
            .map(|el| {
                if open {
                    el.h(px(height))
                } else {
                    el.h(px(28.0))
                }
            })
            .child(
                div()
                    .id("git-graph-toggle")
                    // `flex h-7 items-center gap-1 px-3 leading-none
                    // hover:bg-content/5`, `h-full` while collapsed.
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .map(|el| if open { el.h(px(28.0)) } else { el.flex_1() })
                    .px_3()
                    .line_height(relative(1.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.changes_ui.graph_closed = !this.changes_ui.graph_closed;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg.opacity(0.55))
                            .child("GRAPH"),
                    )
                    // `ml-auto size-3.5`
                    .child(div().flex_1())
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(IconSize::Sm)
                        .color(fg.opacity(0.5)),
                    ),
            )
            .when(open, |el| {
                el.child(scrollbar::framed(
                    "git-graph-scrollbar",
                    &self.changes_ui.graph_scroll,
                    div()
                        .id("git-graph-rows")
                        .track_scroll(&self.changes_ui.graph_scroll)
                        .flex_1()
                        .min_h_0()
                        .pr(scrollbar::gutter(&self.changes_ui.graph_scroll))
                        .overflow_y_scroll()
                        .overflow_x_hidden()
                        .map(|el| {
                            if commits.is_empty() {
                                el.child(
                                    div()
                                        .px_3()
                                        .py_2()
                                        .text_size(px(12.0))
                                        .text_color(fg.opacity(0.45))
                                        .child("No commits yet"),
                                )
                            } else {
                                // Rows are all one lane tall; only those in
                                // view are built.
                                let heights = vec![Some(git_graph::SWIMLANE_HEIGHT); commits.len()];
                                let visible = virtual_rows::for_scroll(
                                    &heights,
                                    &self.changes_ui.graph_scroll,
                                    0.0,
                                );
                                let range = visible.range.clone();
                                el.when(visible.above > 0.0, |el| {
                                    el.child(div().flex_none().h(px(visible.above)))
                                })
                                .children(commits[range.clone()].iter().zip(&rows[range]).map(
                                    |(commit, row)| {
                                        let active =
                                            selected.as_deref() == Some(commit.sha.as_str());
                                        self.render_history_row(commit, row, active, cx)
                                    },
                                ))
                                .when(visible.below > 0.0, |el| {
                                    el.child(div().flex_none().h(px(visible.below)))
                                })
                            }
                        }),
                ))
            })
    }

    /// MonoCode `HistoryRow`: the graph, the subject (bold at HEAD), the
    /// author, and the first ref as a pill.
    fn render_history_row(
        &self,
        commit: &HistoryCommit,
        row: &git_graph::Row,
        active: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let bg = colors.bg;
        let shape = git_graph::row_graph(row);
        let width = shape.width;
        let badge = row
            .refs
            .iter()
            .find(|r| r.color.is_some())
            .or(row.refs.first())
            .cloned();
        // MonoCode `bg-selection`.
        let selection = colors.active;
        let hover = fg.opacity(0.05);
        let hovered = self.changes_ui.hovered_commit.as_deref() == Some(commit.sha.as_str());
        let node = node_look(bg, fg, hovered, active);
        let open = commit.clone();
        let hover_sha = commit.sha.clone();
        let tip = if commit.author.is_empty() {
            format!("{} {}", commit.short_sha, commit.subject)
        } else {
            format!(
                "{} {} — {}",
                commit.short_sha, commit.subject, commit.author
            )
        };
        div()
            .id(SharedString::from(format!("graph-{}", commit.sha)))
            .flex()
            .items_center()
            .h(px(git_graph::SWIMLANE_HEIGHT))
            .min_w_0()
            .pr_2()
            .cursor_pointer()
            .when(active, |el| el.bg(selection))
            .when(!active, |el| el.hover(move |s| s.bg(hover)))
            .tooltip(Tooltip::text(tip))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                let next = hovered.then(|| hover_sha.clone());
                if *hovered || this.changes_ui.hovered_commit.as_deref() == Some(hover_sha.as_str())
                {
                    this.changes_ui.hovered_commit = next;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                let tab = PaneTab::Commit {
                    cwd: this.workspace.cwd.clone(),
                    sha: open.sha.clone(),
                    short_sha: open.short_sha.clone(),
                    subject: open.subject.clone(),
                };
                this.open_pane_tab(tab, event.click_count() == 2, cx);
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        paint_row_graph(&shape, bounds.origin, node, window)
                    },
                )
                .w(px(width))
                .h(px(git_graph::SWIMLANE_HEIGHT))
                .flex_none(),
            )
            .child(
                div()
                    .ml_1()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .overflow_hidden()
                    // `text-[12px] leading-[22px]`, `font-semibold` at HEAD.
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.0))
                            .line_height(px(git_graph::SWIMLANE_HEIGHT))
                            .text_color(fg)
                            .when(commit.head, |el| el.font_weight(FontWeight::SEMIBOLD))
                            .child(if commit.subject.is_empty() {
                                commit.short_sha.clone()
                            } else {
                                commit.subject.clone()
                            }),
                    )
                    .when(!commit.author.is_empty(), |el| {
                        el.child(
                            div()
                                .ml_2()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.0))
                                .line_height(px(git_graph::SWIMLANE_HEIGHT))
                                .text_color(fg.opacity(0.45))
                                .child(commit.author.clone()),
                        )
                    }),
            )
            .children(badge.map(|r| {
                let local = r.kind == "local";
                let (fill, ink) = match r.color {
                    Some(color) => (rgb(color).into(), bg),
                    None => (fg.opacity(0.10), fg.opacity(0.55)),
                };
                // MonoCode `RefPill`: `ml-1 h-3.5 max-w-[6.5rem] gap-0.5
                // rounded-full px-1.5 text-[10px] leading-none`, a
                // `GitBranch size-2.5` on local branches.
                div()
                    .ml_1()
                    .flex()
                    .flex_none()
                    .max_w(px(104.0))
                    .h(px(14.0))
                    .items_center()
                    .gap_0p5()
                    .px_1p5()
                    .rounded_full()
                    .bg(fill)
                    .text_size(px(10.0))
                    .line_height(relative(1.0))
                    .text_color(ink)
                    .when(local, |el| {
                        el.child(
                            gpui::svg()
                                .path(IconName::GitBranch.path())
                                .size(px(10.0))
                                .flex_none()
                                .text_color(ink),
                        )
                    })
                    .child(div().min_w_0().truncate().child(r.name))
            }))
            .into_any_element()
    }
}

/// The colours MonoCode's `.git-history-item` CSS gives a node's circles.
#[derive(Clone, Copy)]
struct NodeLook {
    /// The outer circle's `stroke: background-base` halo, which masks the
    /// lanes around the node; `transparent` on hover.
    halo: Option<Hsla>,
    /// The second circle's stroke and HEAD's centre: `background-base`,
    /// mixed with content at 5% on hover and 10% when selected.
    inner: Hsla,
}

fn node_look(bg: Hsla, fg: Hsla, hovered: bool, selected: bool) -> NodeLook {
    let inner = if selected {
        bg.blend(fg.opacity(0.10))
    } else if hovered {
        bg.blend(fg.opacity(0.05))
    } else {
        bg
    };
    NodeLook {
        halo: (!hovered).then_some(bg),
        inner,
    }
}

/// MonoCode `CIRCLE_STROKE_WIDTH`: each circle's 2px stroke straddles its
/// radius by 1px.
const NODE_STROKE_HALF: f32 = 1.0;

/// Paints one row of the graph at `origin`.
fn paint_row_graph(
    shape: &git_graph::RowGraph,
    origin: gpui::Point<gpui::Pixels>,
    look: NodeLook,
    window: &mut gpui::Window,
) {
    let at = |x: f32, y: f32| point(origin.x + px(x), origin.y + px(y));
    for path in &shape.paths {
        let mut builder = PathBuilder::stroke(px(1.0));
        for cmd in &path.cmds {
            match *cmd {
                Cmd::Move(x, y) => builder.move_to(at(x, y)),
                Cmd::Line(x, y) => builder.line_to(at(x, y)),
                Cmd::Arc { r, sweep, x, y } => {
                    builder.arc_to(point(px(r), px(r)), px(0.0), false, sweep, at(x, y))
                }
            }
        }
        if let Ok(built) = builder.build() {
            window.paint_path(built, rgb(path.color));
        }
    }
    let color: Hsla = rgb(shape.color).into();
    let disc = |r: f32, fill: Hsla, window: &mut gpui::Window| {
        let center = at(shape.cx, shape.cy);
        let bounds = gpui::Bounds::new(
            point(center.x - px(r), center.y - px(r)),
            gpui::size(px(r * 2.0), px(r * 2.0)),
        );
        window.paint_quad(gpui::fill(bounds, fill).corner_radii(px(r)));
    };
    // The outer circle (`r` = 5, 6 or 7): filled in the lane colour, its
    // stroke a background ring from `r - 1` to `r + 1`.
    let outer = git_graph::node_radius(shape.node);
    match look.halo {
        Some(halo) => {
            disc(outer + NODE_STROKE_HALF, halo, window);
            disc(outer - NODE_STROKE_HALF, color, window);
        }
        None => disc(outer, color, window),
    }
    match shape.node {
        // A merge's second circle (`r = 3`, filled): a ring from 2 to 4,
        // the lane colour inside.
        Node::Merge => {
            disc(3.0 + NODE_STROKE_HALF, look.inner, window);
            disc(3.0 - NODE_STROKE_HALF, color, window);
        }
        // HEAD's second circle (`r = 2`, `stroke-width: 4`) fills to 4.
        Node::Head => disc(4.0, look.inner, window),
        Node::Commit => {}
    }
}
