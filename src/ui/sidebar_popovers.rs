//! The Sessions tab's popovers: the worktree switcher (MonoCode
//! `SidebarWorktreeSwitcher`), the filter menu (`SessionFiltersMenu`) and
//! the folder menu's colour swatches (`FolderColorSwatches`).

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, App, Context, Div, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels,
    Point, SharedString, Styled, anchored, deferred, div, prelude::*, relative,
};

use crate::app::BenCodeApp;
use crate::app::session_folders::{FOLDER_COLORS, palette_color, parse_hex, to_hex};
use crate::app::session_list::{SessionFilters, TimeFilter, harnesses_in};
use crate::harness::HarnessKind;
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::sidebar_menus::{SidebarMenu, SidebarMenuKind};

/// MonoCode's filter popover is 228px wide.
const MENU_WIDTH: f32 = 228.0;

/// MonoCode `.popover-backdrop`: `content` at 2% over a `backdrop-blur-xl`
/// (the base background alone in the light theme). GPUI cannot blur what
/// lies behind an element, so the tint is laid on the opaque background.
pub(crate) fn popover_glass(cx: &App) -> Hsla {
    let theme = cx.theme();
    let colors = &theme.colors;
    if theme.is_dark() {
        colors.bg.blend(colors.fg.opacity(0.02))
    } else {
        colors.bg
    }
}

/// MonoCode `Popover`'s frame: `rounded-xl border border-content/10
/// shadow-xl` over its glass (`content` at 2% on the base background; the
/// base alone in the light theme). Popovers draw outside the sidebar, so
/// the web's 1.5 line height and text colour are set here too.
pub(crate) fn popover_frame(cx: &App) -> Div {
    let colors = &cx.theme().colors;
    div()
        .rounded(px(12.0))
        .border_1()
        .border_color(colors.fg.opacity(0.10))
        .bg(popover_glass(cx))
        .shadow_xl()
        .text_color(colors.fg)
        .line_height(relative(1.5))
}

impl BenCodeApp {
    pub(crate) fn sidebar_menu_is_worktrees(&self) -> bool {
        matches!(
            self.sidebar_menu.as_ref().map(|m| &m.kind),
            Some(SidebarMenuKind::Worktrees)
        )
    }

    /// The title button: opens the working copies under it, or closes them.
    pub(crate) fn toggle_worktree_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        if self.sidebar_menu_is_worktrees() {
            self.close_sidebar_menu(cx);
            return;
        }
        self.close_sidebar_menu(cx);
        self.sidebar_menu = Some(SidebarMenu {
            kind: SidebarMenuKind::Worktrees,
            position: Point::new(at.x - px(12.0), at.y + px(14.0)),
            active: 0,
        });
        cx.notify();
    }

    /// MonoCode's rows: the project folder ("all sessions"), then each
    /// worktree by branch with its folder, the focused one checked.
    pub(crate) fn render_worktree_menu(&self, position: Point<Pixels>, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let focus = self.worktree_focus().map(|f| f.path.clone());
        let trees = &self.workspace.worktrees;
        let main = trees.iter().find(|t| t.is_main);
        let row = |id: SharedString,
                   icon: AnyElement,
                   label: String,
                   detail: String,
                   selected: bool,
                   pick: Option<crate::app::WorktreeFocus>| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(move |s| s.bg(fg.opacity(0.05)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.close_sidebar_menu(cx);
                    this.select_workspace(pick.clone(), cx);
                }))
                .child(icon)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().text_size(px(12.0)).text_color(fg).child(label))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(10.0))
                                .text_color(fg.opacity(0.4))
                                .child(detail),
                        ),
                )
                .when(selected, |el| {
                    el.child(Icon::new(IconName::Check).size(IconSize::Sm).color(fg))
                })
        };
        let muted = fg.opacity(0.5);
        let mut list = div().flex().flex_col().child(row(
            "worktree-default".into(),
            Icon::new(IconName::GitBranch).size(IconSize::Sm).color(muted).into_any_element(),
            main.and_then(|m| m.branch.clone())
                .unwrap_or_else(|| "Project folder".into()),
            "Project folder · all sessions".into(),
            focus.is_none(),
            None,
        ));
        for tree in trees.iter().filter(|t| !t.is_main && !t.missing) {
            let label = tree
                .branch
                .clone()
                .unwrap_or_else(|| format!("Detached {}", tree.head.chars().take(7).collect::<String>()));
            let selected = focus
                .as_deref()
                .is_some_and(|f| crate::app::same_project_path(f, &tree.path));
            list = list.child(row(
                SharedString::from(format!("worktree-{}", tree.path)),
                crate::ui::icons::ExtraIcon::FolderTree
                    .icon()
                    .size(IconSize::Sm)
                    .color(muted)
                    .into_any_element(),
                label,
                pretty_path(&tree.path),
                selected,
                Some(crate::app::WorktreeFocus {
                    path: tree.path.clone(),
                    branch: tree.branch.clone(),
                }),
            ));
        }
        let menu = popover_frame(cx)
            .id("worktree-switcher-menu")
            .w(px(280.0))
            .max_h(px(360.0))
            .overflow_y_scroll()
            .p_1()
            .on_mouse_down_out(cx.listener(|_, _, window, cx| {
                cx.defer_in(window, |this, _, cx| {
                    if !std::mem::take(&mut this.sessions_ui.filter_button_hit) {
                        this.close_sidebar_menu(cx);
                    }
                });
            }))
            .child(list);
        deferred(anchored().position(position).snap_to_window().child(menu))
            .with_priority(3)
            .into_any_element()
    }

    /// MonoCode `FolderColorSwatches`: the palette (the first clears the
    /// tint), a custom colour button, and its picker once opened.
    pub(crate) fn render_folder_swatches(&self, folder_id: &str, cx: &Context<Self>) -> Option<AnyElement> {
        let folder = self.project_folders().iter().find(|f| f.id == folder_id)?;
        let fg = cx.theme().colors.fg;
        let custom = folder.custom_color.as_deref().and_then(parse_hex);
        let picker_open = self.sessions_ui.folder_color_picker;
        let dot = |id: SharedString, color: gpui::Hsla, selected: bool| {
            div()
                .id(id)
                .size(px(20.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .child(
                    div()
                        .size(px(14.0))
                        .rounded_full()
                        .bg(color)
                        .when(selected, |el| el.border_2().border_color(fg.opacity(0.8))),
                )
        };
        let mut row = div().flex().items_center().justify_between().gap_1().px(px(2.0));
        for (ix, _) in FOLDER_COLORS.iter().enumerate() {
            let selected = custom.is_none()
                && (folder.color_index == Some(ix) || (folder.color_index.is_none() && ix == 0));
            let target = folder.id.clone();
            row = row.child(
                dot(
                    SharedString::from(format!("folder-color-{ix}")),
                    palette_color(ix).unwrap_or(fg),
                    selected,
                )
                .tooltip(Tooltip::text(format!("Color {}", ix + 1)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.sessions_ui.folder_color_picker = false;
                    this.set_folder_color(&target, (ix > 0).then_some(ix), cx);
                })),
            );
        }
        let custom_dot = div()
            .id("folder-color-custom")
            .size(px(20.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            .tooltip(Tooltip::text("Custom color"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.sessions_ui.folder_color_picker = !this.sessions_ui.folder_color_picker;
                cx.notify();
            }))
            .child(
                div()
                    .size(px(14.0))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_color(fg.opacity(0.3))
                    .when_some(custom, |el, color| {
                        el.bg(color).border_2().border_color(fg.opacity(0.8))
                    })
                    .when(custom.is_none(), |el| {
                        el.child(Icon::new(IconName::Pipette).size(IconSize::Xs).color(fg.opacity(0.6)))
                    }),
            );
        row = row.child(custom_dot);
        let value = custom
            .or_else(|| folder.color_index.and_then(palette_color))
            .or_else(|| palette_color(0))
            .unwrap_or(fg);
        let target = folder.id.clone();
        Some(
            div()
                .px_1()
                .py_1()
                .flex()
                .flex_col()
                .gap_2()
                .child(row)
                .when(picker_open, |el| {
                    el.child(
                        ely_gpui_component::forms::ColorPicker::new("folder-color-picker", value)
                            .opaque()
                            .on_change(crate::ui::app_callback::app_callback_with(cx, move |this, color: gpui::Hsla, cx| {
                                this.preview_folder_custom_color(&target, to_hex(color), cx);
                            })),
                    )
                })
                .child(div().my_1().h(px(1.0)).bg(fg.opacity(0.1)))
                .into_any_element(),
        )
    }

    fn set_session_filters(&mut self, filters: SessionFilters, cx: &mut Context<Self>) {
        self.sessions_ui.filters = filters;
        self.save_settings(cx);
        cx.notify();
    }

    /// MonoCode `SessionFiltersMenu`: Archived, Status, Time, Provider,
    /// and Clear filters once anything is set.
    pub(crate) fn render_filter_menu(&self, position: Point<Pixels>, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let filters = self.sessions_ui.filters.clone();
        let harnesses = harnesses_in(&self.listed_sessions());
        let section = |label: &'static str| {
            div()
                .px_2()
                .pt_2()
                .pb(px(2.0))
                .text_size(px(10.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(fg.opacity(0.4))
                .child(label.to_uppercase())
        };
        let row = |id: SharedString,
                   label: SharedString,
                   checked: bool,
                   icon: Option<AnyElement>,
                   next: SessionFilters,
                   close: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_2()
                .h(px(28.0))
                .px_2()
                .rounded(px(8.0))
                .text_size(px(13.0))
                .line_height(relative(1.0))
                .text_color(fg)
                .cursor_pointer()
                .hover(move |s| s.bg(fg.opacity(0.05)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_session_filters(next.clone(), cx);
                    if close {
                        this.close_sidebar_menu(cx);
                    }
                }))
                .children(icon)
                .child(div().flex_1().min_w_0().truncate().child(label))
                .when(checked, |el| {
                    el.child(Icon::new(IconName::Check).size(IconSize::Sm).color(fg))
                })
        };
        let toggled = |f: &dyn Fn(&mut SessionFilters)| {
            let mut next = filters.clone();
            f(&mut next);
            next
        };
        let mut body = div().flex().flex_col();
        body = body.child(row(
            "filter-archived".into(),
            "Archived".into(),
            filters.show_archived,
            None,
            toggled(&|f| f.show_archived = !f.show_archived),
            true,
        ));
        body = body
            .child(section("Status"))
            .child(row(
                "filter-working".into(),
                "Working".into(),
                filters.status.working,
                None,
                toggled(&|f| f.status.working = !f.status.working),
                false,
            ))
            .child(row(
                "filter-approval".into(),
                "Needs approval".into(),
                filters.status.needs_approval,
                None,
                toggled(&|f| f.status.needs_approval = !f.status.needs_approval),
                false,
            ))
            .child(row(
                "filter-done".into(),
                "Done".into(),
                filters.status.done,
                None,
                toggled(&|f| f.status.done = !f.status.done),
                false,
            ))
            .child(section("Time"));
        for (time, label) in TimeFilter::ALL {
            body = body.child(row(
                SharedString::from(format!("filter-time-{label}")),
                label.into(),
                filters.time == time,
                None,
                toggled(&|f| f.time = time),
                false,
            ));
        }
        if !harnesses.is_empty() {
            body = body.child(section("Provider"));
            for kind in harnesses {
                let id = kind.id();
                let shown = !filters.hidden_harnesses.iter().any(|h| h == id);
                body = body.child(row(
                    SharedString::from(format!("filter-harness-{id}")),
                    kind.label().into(),
                    shown,
                    Some(HarnessIcon::new(id).size(px(14.0)).into_any_element()),
                    toggled(&|f| toggle_harness(f, kind)),
                    false,
                ));
            }
        }
        if filters.is_active() {
            body = body.child(div().my_1().h(px(1.0)).bg(fg.opacity(0.1))).child(
                div()
                    .id("filter-clear")
                    .flex()
                    .items_center()
                    .h(px(28.0))
                    .px_2()
                    .rounded(px(8.0))
                    .text_size(px(13.0))
                    .line_height(relative(1.0))
                    .text_color(fg.opacity(0.7))
                    .cursor_pointer()
                    .hover(move |s| s.bg(fg.opacity(0.05)).text_color(fg))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_session_filters(SessionFilters::default(), cx)
                    }))
                    .child("Clear filters"),
            );
        }
        let menu = popover_frame(cx)
            .id("filter-sessions-menu")
            .w(px(MENU_WIDTH))
            .max_h(px(480.0))
            .overflow_y_scroll()
            .p_1()
            // A press on the filter button toggles the menu itself; only
            // once every handler ran is it known whether it was outside.
            .on_mouse_down_out(cx.listener(|_, _, window, cx| {
                cx.defer_in(window, |this, _, cx| {
                    if !std::mem::take(&mut this.sessions_ui.filter_button_hit) {
                        this.close_sidebar_menu(cx);
                    }
                });
            }))
            .child(body);
        deferred(anchored().position(position).snap_to_window().child(menu))
            .with_priority(3)
            .into_any_element()
    }
}

/// MonoCode `prettyCwd`: the home folder as `~`.
pub(crate) fn pretty_path(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_string(),
    }
}

fn toggle_harness(filters: &mut SessionFilters, kind: HarnessKind) {
    let id = kind.id().to_string();
    if let Some(ix) = filters.hidden_harnesses.iter().position(|h| *h == id) {
        filters.hidden_harnesses.remove(ix);
    } else {
        filters.hidden_harnesses.push(id);
    }
}
