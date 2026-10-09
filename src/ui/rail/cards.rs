//! The rail's lists (MonoCode `ProjectSection`, `ProjectGroupSection`)
//! and their rows (MonoCode `ProjectCard`).

use std::cell::Cell;
use std::rc::Rc;

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use ely_gpui_component::typography::ShimmerText;
use gpui::{
    AnyElement, Bounds, Context, DragMoveEvent, FontWeight, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Pixels, Point, SharedString, Styled, canvas, div,
    prelude::*, relative, rgb,
};

use crate::ui::scale::px;
use crate::ui::thumbnail::{LOGO, thumbnail};

use super::group_color;
use super::model::{ProjectGroup, mute_status, notification_id, path_key};
use super::reorder::{DraggedRailProject, preview_order};
use super::widgets::{
    AMBER_400, ROW_HEIGHT, hover_control, project_card_title, project_diff_stat, project_mascot_icon,
    rail_label, section_button,
};
use crate::app::{BenCodeApp, same_project_path};
use crate::ui::mascot::mascot_for;

/// The rows' `px-2` on each side, for the held row's width.
const LIST_INSET: f32 = 16.0;

type Anchor = Rc<Cell<Option<Bounds<Pixels>>>>;

/// One project row's data.
struct ProjectCard {
    path: String,
    /// The list it reorders within (`pinned`, `projects`, `group:<id>`).
    list: String,
    draggable: bool,
    selected: bool,
    busy: bool,
    pinned: bool,
    stats: (usize, usize),
    mute: Option<String>,
}

impl BenCodeApp {
    fn rail_card(&self, path: &str, list: &str, draggable: bool) -> ProjectCard {
        // No project is highlighted while a view is open.
        let current = same_project_path(path, &self.current_cwd);
        let busy = self
            .runs
            .keys()
            .filter_map(|id| self.sessions.iter().find(|s| &s.id == id))
            .any(|s| crate::app::is_path_in_project(&s.cwd, path));
        ProjectCard {
            path: path.to_string(),
            list: list.to_string(),
            draggable,
            selected: self.surface.is_none() && current,
            busy,
            pinned: self.settings.pinned_projects.iter().any(|p| same_project_path(p, path)),
            stats: if current {
                self.current_diff_stats()
            } else {
                self.project_diff_stats(path)
                    .or_else(|| self.workspace.cached_diff_stats(path))
                    .unwrap_or((0, 0))
            },
            mute: mute_status(
                self.settings.rail.project_notifications.get(&notification_id(path)),
                crate::app::now_ms(),
            ),
        }
    }

    /// Uncommitted lines in the open project (MonoCode `useProjectDiffStats`).
    fn current_diff_stats(&self) -> (usize, usize) {
        let files = self.git_status.staged.iter().chain(&self.git_status.unstaged);
        files.fold((0, 0), |(add, del), f| (add + f.additions, del + f.deletions))
    }

    /// MonoCode `ProjectSectionHeader`: `flex items-center gap-1 px-3
    /// pb-1.5 pt-1`, its label `min-w-0 flex-1 truncate px-1 text-xs
    /// text-content/50`, then its buttons.
    pub(super) fn render_rail_section_header(&self, label: &'static str, button: Option<AnyElement>, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        div()
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .pb(px(6.0))
            .pt_1()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .px_1()
                    .text_size(px(12.0))
                    .text_color(fg.opacity(0.5))
                    .child(label),
            )
            .children(button)
    }

    /// MonoCode `ProjectSection`: `shrink-0 mb-2`, its header, then the cards.
    pub(super) fn render_project_section(
        &self,
        label: &'static str,
        list: &'static str,
        paths: &[String],
        can_add: bool,
        empty: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let add = can_add.then(|| self.render_add_project_button(cx).into_any_element());
        div()
            .flex_none()
            .mb_2()
            .child(self.render_rail_section_header(label, add, cx))
            .when(empty, |el| {
                // `px-4 pb-1 text-[11px] leading-tight text-content/40`
                el.child(
                    div()
                        .px_4()
                        .pb_1()
                        .text_size(px(11.0))
                        .line_height(relative(1.25))
                        .text_color(fg.opacity(0.4))
                        .child("No projects yet"),
                )
            })
            // `flex flex-col gap-px px-2`
            .child(div().px_2().child(self.render_card_list(list, paths, cx)))
    }

    /// A list's cards in `flex flex-col gap-px`, in the order a held row
    /// previews; the list follows the row while it is dragged over it.
    fn render_card_list(&self, list: &str, paths: &[String], cx: &Context<Self>) -> impl IntoElement {
        let shown = preview_order(list, paths, self.rail_ui.reorder.as_ref());
        let draggable = paths.len() > 1;
        let (track_list, track_ids) = (list.to_string(), paths.to_vec());
        div()
            .flex()
            .flex_col()
            .gap(px(1.0))
            .on_drag_move::<DraggedRailProject>(cx.listener(move |this, event: &DragMoveEvent<DraggedRailProject>, _, cx| {
                // Each list sees every move; only its own rows' count.
                if event.bounds.contains(&event.event.position) || this.rail_ui.reorder.is_some() {
                    this.track_rail_reorder(&track_list, &track_ids, event, cx);
                }
            }))
            .children(shown.iter().map(|path| {
                let card = self.rail_card(path, list, draggable);
                let row = self.render_project_card(card, cx);
                self.slide_rail_row(path, row)
            }))
    }

    /// MonoCode's "Groups" section: its header's "New project group"
    /// button, then each group.
    pub(super) fn render_groups_section(&self, groups: &[(ProjectGroup, Vec<String>)], cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let anchor: Anchor = Rc::default();
        let button_anchor = anchor.clone();
        let add = section_button("rail-new-group", IconName::FolderPlus, "New project group", false, fg)
            .child(canvas(move |bounds, _, _| button_anchor.set(Some(bounds)), |_, _, _, _| {}).absolute().size_full())
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                let at = anchor
                    .get()
                    .map_or(event.position(), |b| Point::new(b.left(), b.bottom()));
                this.create_rail_group(at, None, cx);
            }));
        div()
            .flex_none()
            .mb_2()
            .child(self.render_rail_section_header("Groups", Some(add.into_any_element()), cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .px_2()
                    .children(groups.iter().map(|(group, items)| self.render_group_section(group, items, cx))),
            )
    }

    /// MonoCode `ProjectGroupSection`: `rounded-md`, `mb-1.5 bg-content/5`
    /// while open; its header row toggles it and opens its menu.
    fn render_group_section(&self, group: &ProjectGroup, items: &[String], cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let id = format!("rail-group-{}", group.id);
        let header_group = SharedString::from(format!("{id}-row"));
        let expanded = !group.collapsed;
        let count = format!("{} {}", items.len(), if items.len() == 1 { "project" } else { "projects" });
        let (toggle_id, menu_id, options_id) = (group.id.clone(), group.id.clone(), group.id.clone());
        let color = group_color(group);
        let mascot = mascot_for(&group.id, group.mascot.as_deref());
        let chevron = |icon: IconName| Icon::new(icon).size(IconSize::Sm).color(fg);
        let glyph = if group.collapsed {
            div()
                .relative()
                .size_4()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .group_hover(header_group.clone(), |s| s.invisible())
                        .child(project_mascot_icon(mascot, color, false, &id)),
                )
                .child(
                    div()
                        .absolute()
                        .flex()
                        .invisible()
                        .group_hover(header_group.clone(), |s| s.visible())
                        .child(chevron(IconName::ChevronRight)),
                )
                .into_any_element()
        } else {
            chevron(IconName::ChevronDown).into_any_element()
        };
        let header = div()
            .id(SharedString::from(format!("{id}-row")))
            .group(header_group.clone())
            .relative()
            .flex()
            .items_stretch()
            .h(px(ROW_HEIGHT))
            .px_2()
            .rounded(px(6.0))
            .opacity(0.65)
            .hover(move |s| s.bg(fg.opacity(0.05)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_rail_group_menu(&menu_id, event.position, cx);
                }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{id}-toggle")))
                    .flex()
                    .min_w_0()
                    .flex_1()
                    .items_center()
                    .gap_2()
                    .group_hover(header_group.clone(), |s| s.pr(px(24.0)))
                    .tooltip(Tooltip::text(format!("{} · {count}", group.name)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_rail_prefs(
                            |prefs| {
                                prefs.with_group(&toggle_id, |g| ProjectGroup {
                                    collapsed: !g.collapsed,
                                    ..g.clone()
                                })
                            },
                            cx,
                        );
                    }))
                    .child(div().flex().flex_none().size_4().items_center().justify_center().child(glyph))
                    .child(rail_label(group.name.clone())),
            )
            .child({
                let anchor: Anchor = Rc::default();
                let button_anchor = anchor.clone();
                hover_control(
                    SharedString::from(format!("{id}-options")),
                    IconName::Ellipsis,
                    IconSize::Md,
                    "Group options",
                    &header_group,
                    fg,
                )
                .right_1()
                .top(px(4.0))
                .size_6()
                .child(canvas(move |bounds, _, _| button_anchor.set(Some(bounds)), |_, _, _, _| {}).absolute().size_full())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let at = anchor
                            .get()
                            .map_or(event.position, |b| Point::new(b.left(), b.bottom()));
                        this.open_rail_group_menu(&options_id, at, cx);
                    }),
                )
            });
        let list = format!("group:{}", group.id);
        div()
            .id(SharedString::from(id.clone()))
            .flex_none()
            .overflow_hidden()
            .rounded(px(6.0))
            .when(expanded, |el| el.mb(px(6.0)).bg(fg.opacity(0.05)))
            .child(header)
            .when(expanded, |el| {
                // `flex flex-col gap-px p-1`
                el.child(div().p_1().child(self.render_card_list(&list, items, cx)))
            })
            .into_any_element()
    }

    /// MonoCode `ProjectCard`.
    fn render_project_card(&self, card: ProjectCard, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let dark = theme.is_dark();
        let id = format!("rail-{}-{}", card.list, path_key(&card.path));
        let group = SharedString::from(id.clone());
        let name = self.rail_project_label(&card.path);
        let color = self.project_color(&card.path);
        let mascot = self.project_mascot(&card.path);
        let logo = self.settings.rail.tab_group_logos.get(&path_key(&card.path)).cloned();
        let (additions, deletions) = card.stats;
        let has_changes = additions > 0 || deletions > 0;
        let title = project_card_title(&name, &card.path, card.stats, card.busy);
        let title = match &card.mute {
            Some(status) => format!("{title}\n{status}"),
            None => title,
        };
        let held = self
            .rail_ui
            .reorder
            .as_ref()
            .is_some_and(|r| r.list == card.list && r.path == card.path);
        let drag = DraggedRailProject {
            list: card.list.clone(),
            path: card.path.clone(),
            label: name.clone(),
            color,
            mascot,
            width: self.rail_width() - LIST_INSET,
        };
        let (select_path, menu_path, options_path, pin_path) = (
            card.path.clone(),
            card.path.clone(),
            card.path.clone(),
            card.path.clone(),
        );
        // `bg-selection-strong` (content 12%, 7% in light), else `opacity-65`
        // with the `content/5` hover from `.project-reorder-item`.
        let strong = fg.opacity(if dark { 0.12 } else { 0.07 });
        let hover = fg.opacity(0.05);
        let label = if card.busy {
            ShimmerText::new(SharedString::from(format!("{id}-shimmer")), name).into_any_element()
        } else {
            name.into_any_element()
        };
        let icon = match logo.filter(|_| !card.busy) {
            // `ProjectLogoIcon className="size-4 rounded-sm"`
            Some(file) => thumbnail(file, LOGO)
                .size_4()
                .rounded(px(4.0))
                .into_any_element(),
            None => project_mascot_icon(mascot, color, card.busy, &id),
        };
        div()
            .id(SharedString::from(id.clone()))
            .group(group.clone())
            .relative()
            .flex()
            .items_stretch()
            .h(px(ROW_HEIGHT))
            .px_2()
            .rounded(px(6.0))
            .map(|el| {
                if card.selected {
                    el.bg(strong)
                } else {
                    el.opacity(0.65).hover(move |s| s.bg(hover))
                }
            })
            // The held row's slot stays open while it rides the pointer.
            .when(held, |el| el.opacity(0.0))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.switch_project(select_path.clone(), cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_rail_project_menu(&menu_path, event.position, cx);
                }),
            )
            .when(card.draggable, |el| {
                el.on_drag(drag, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
            })
            .child(
                // The card's button: `flex min-w-0 flex-1 items-center gap-2
                // group-hover:pr-6`, titled with the path and its changes.
                div()
                    .id(SharedString::from(format!("{id}-main")))
                    .flex()
                    .min_w_0()
                    .flex_1()
                    .items_center()
                    .gap_2()
                    .group_hover(group.clone(), |s| s.pr(px(24.0)))
                    .tooltip(Tooltip::text(title))
                    .child(
                        // `project-card-logo grid size-4 group-hover:opacity-0`
                        div()
                            .flex()
                            .flex_none()
                            .size_4()
                            .items_center()
                            .justify_center()
                            .group_hover(group.clone(), |s| s.opacity(0.0))
                            .child(icon),
                    )
                    .child(
                        // `min-w-0 flex-1 truncate text-sm font-medium leading-tight`
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .line_height(relative(1.25))
                            .child(label),
                    )
                    .when(has_changes, |el| {
                        // `project-card-stats shrink-0 group-hover:hidden`
                        el.child(
                            div()
                                .flex_none()
                                .group_hover(group.clone(), |s| s.invisible())
                                .child(project_diff_stat(additions, deletions, crate::ui::appearance::diff_colors(cx))),
                        )
                    })
                    .when_some(card.mute, |el, status| {
                        // `grid size-4 shrink-0 place-items-center text-amber-400`, `BellOff size-3.5`
                        el.child(
                            div()
                                .id(SharedString::from(format!("{id}-muted")))
                                .flex()
                                .flex_none()
                                .size_4()
                                .items_center()
                                .justify_center()
                                .tooltip(Tooltip::text(status))
                                .child(
                                    crate::ui::icons::ExtraIcon::BellOff
                                        .icon()
                                        .size(IconSize::Sm)
                                        .color(rgb(AMBER_400)),
                                ),
                        )
                    }),
            )
            .child(
                // "Project options": `absolute right-1 top-1/2 hidden size-6
                // rounded-md text-content/55 hover:bg-content/8 group-hover:grid`.
                hover_control(
                    SharedString::from(format!("{id}-options")),
                    IconName::Ellipsis,
                    IconSize::Md,
                    "Project options",
                    &group,
                    fg,
                )
                .right_1()
                .top(px(4.0))
                .size_6()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_rail_project_menu(&options_path, event.position, cx);
                    }),
                ),
            )
            .child(
                // Pin: `absolute left-2 top-1/2 size-4 rounded-sm opacity-0
                // group-hover:opacity-100`, over the logo it replaces.
                hover_control(
                    SharedString::from(format!("{id}-pin")),
                    if card.pinned { IconName::PinOff } else { IconName::Pin },
                    IconSize::Sm,
                    if card.pinned { "Unpin project" } else { "Pin project" },
                    &group,
                    fg,
                )
                .left_2()
                .top(px(8.0))
                .size_4()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_project_pin(&pin_path, cx);
                })),
            )
            .into_any_element()
    }
}
