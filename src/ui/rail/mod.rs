//! MonoCode `ProjectRail`: the visit arrows, Search / Inbox / Notes /
//! Automations, the Pinned, Groups and Projects lists with their diff
//! stats, and Settings at the foot; while Settings is open its sections
//! take the rail's body. Rows drag to reorder and the right edge drags to
//! resize. The saved state lives in [`model`].

pub mod model;
mod cards;
mod menu;
mod menu_view;
mod notify;
mod remove;
mod reorder;
mod settings_nav;
mod state;
mod widgets;

use std::cell::Cell;
use std::rc::Rc;

use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Bounds, Context, DragMoveEvent, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Pixels, Point, SharedString, Styled,
    anchored, canvas, deferred, div, prelude::*, px, relative,
};

pub use state::RailUi;

use crate::app::session_folders::{palette_color, parse_hex};
use crate::app::{BenCodeApp, Surface, same_project_path};
use crate::ui::window_drag::claim_press;
use crate::ui::mascot::{Mascot, mascot_for};
use crate::ui::sidebar::TITLEBAR_HEIGHT;
use crate::ui::sidebar_menus::SidebarMenuKind;
use model::{ProjectGroup, RailSections, path_key, rail_sections, sync_rail_order};
use reorder::RailResize;
use widgets::{RailTrailing, TitleButtonState, rail_action, rail_search, section_button, title_icon_button};

/// MonoCode `w-[78px]`: room for the traffic lights.
const TRAFFIC_LIGHT_SPACE: f32 = 78.0;
/// MonoCode `border-stroke`.
const STROKE_OPACITY: f32 = 0.07;
/// MonoCode `Popover` (`DEFAULT_GAP`) and the "Open project" one's width.
const POPOVER_GAP: f32 = 6.0;
const ADD_PROJECT_WIDTH: f32 = 230.0;

/// MonoCode `projectName`: the folder's last segment.
pub(super) fn project_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').find(|s| !s.is_empty()).unwrap_or(trimmed)
}

/// MonoCode `tabGroupColor`: a palette colour other than the untinted
/// first, by a hash of `seed`.
fn tab_group_color(seed: &str) -> Hsla {
    let hash = seed
        .encode_utf16()
        .fold(0u32, |hash, unit| hash.wrapping_mul(31).wrapping_add(u32::from(unit)));
    let count = crate::app::session_folders::FOLDER_COLORS.len() as u32;
    let index = (hash % (count - 1) + 1) as usize;
    palette_color(index).unwrap_or_default()
}

/// MonoCode `projectGroupColor`.
pub(super) fn group_color(group: &ProjectGroup) -> Hsla {
    group
        .custom_color
        .as_deref()
        .and_then(parse_hex)
        .or_else(|| group.color_index.and_then(palette_color))
        .unwrap_or_else(|| tab_group_color(&group.id))
}

/// `open -R` (MonoCode `revealPath`), off the UI thread's way: spawning
/// returns at once.
pub(crate) fn reveal_project(path: &str) {
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-R").arg(path).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path).spawn()
    };
    if let Err(err) = result {
        log::error!("could not reveal {path}: {err}");
    }
}

type Anchor = Rc<Cell<Option<Bounds<Pixels>>>>;

impl BenCodeApp {
    /// A project's colour: MonoCode `resolveTabGroupColor`, a saved custom
    /// colour or palette pick, else a hash of its name.
    pub fn project_color(&self, cwd: &str) -> Hsla {
        let prefs = &self.settings.rail;
        let key = path_key(cwd);
        prefs
            .tab_group_custom_colors
            .get(&key)
            .and_then(|hex| parse_hex(hex))
            .or_else(|| prefs.tab_group_colors.get(&key).and_then(|ix| palette_color(*ix)))
            .unwrap_or_else(|| tab_group_color(project_name(cwd)))
    }

    /// A project's mascot: its saved pick, else hashed from its name.
    pub(super) fn project_mascot(&self, path: &str) -> &'static Mascot {
        let pick = self.settings.rail.tab_group_mascots.get(&path_key(path));
        mascot_for(project_name(path), pick.map(String::as_str))
    }

    pub fn render_project_rail(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        // `.sidebar-glass`: the background mixed 10% toward black, plain in
        // light, tinted at the sidebar opacity over the window's blur.
        let glass = self.glass(cx).sidebar(colors.bg, theme.is_dark());
        let fg = colors.fg;
        let settings_open = self.surface == Some(Surface::Settings);
        div()
            .id("project-rail")
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(self.rail_width()))
            .h_full()
            .bg(glass)
            .border_r_1()
            .border_color(fg.opacity(STROKE_OPACITY))
            .text_color(fg)
            .line_height(relative(1.5))
            .on_drag_move::<RailResize>(cx.listener(|this, event: &DragMoveEvent<RailResize>, window, cx| {
                this.track_rail_resize(event, window, cx);
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.finish_rail_drags(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.finish_rail_drags(cx)))
            .child(self.render_rail_top_strip(settings_open, cx))
            .map(|el| {
                if settings_open {
                    el.child(self.render_settings_nav(cx))
                } else {
                    el.child(self.render_rail_actions(cx))
                        .child(self.render_rail_projects(cx))
                        .child(
                            // `flex shrink-0 flex-col gap-px p-2`
                            div().flex().flex_none().flex_col().gap(px(1.0)).p_2().child(rail_action(
                                "rail-settings",
                                IconName::Settings,
                                "Settings",
                                false,
                                RailTrailing::Shortcut("⌘,"),
                                cx,
                                |this, _, _, cx| this.open_settings(cx),
                            )),
                        )
                }
            })
            .child(self.render_rail_sash(cx))
    }

    /// `flex h-10 shrink-0 select-none items-center pr-1.5`, a drag region
    /// holding the traffic-light gap and `TabVisitNav` (without the panel
    /// toggle while Settings is open).
    fn render_rail_top_strip(&self, settings_open: bool, cx: &Context<Self>) -> impl IntoElement {
        self.window_drag_region(div(), cx)
            .flex()
            .flex_none()
            .items_center()
            .h(TITLEBAR_HEIGHT)
            .pr(px(6.0))
            .when(cfg!(target_os = "macos"), |el| {
                el.child(div().flex_none().w(px(TRAFFIC_LIGHT_SPACE)))
            })
            // `DevModeSlot`: the flexible gap before the arrows.
            .child(div().flex_1().min_w_0())
            .child(
                claim_press(div())
                    .flex()
                    .flex_none()
                    .items_center()
                    .children(self.history_buttons("rail", cx))
                    .when(!settings_open, |el| {
                        el.child(title_icon_button(
                            "rail-toggle-projects",
                            IconName::PanelLeft,
                            "Toggle Projects",
                            TitleButtonState::Active,
                            cx,
                            |this, _, _, cx| {
                                this.is_rail_open = false;
                                this.close_rail_menu(cx);
                                cx.notify();
                            },
                        ))
                    }),
            )
    }

    /// `flex shrink-0 flex-col gap-px px-2 pb-2 pt-0.5`.
    fn render_rail_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(1.0))
            .px_2()
            .pb_2()
            .pt(px(2.0))
            .child(rail_search(self.surface_open(Surface::Search), cx))
            // `mt-0.5`
            .child(div().mt(px(2.0)))
            .child(
                rail_action(
                    "rail-inbox",
                    IconName::Inbox,
                    "Inbox",
                    self.surface_open(Surface::Inbox),
                    if self.inbox_has_unseen() {
                        RailTrailing::Dot
                    } else {
                        RailTrailing::None
                    },
                    cx,
                    |this, _, _, cx| this.open_inbox_modal(cx),
                )
                // MonoCode `onOpenContextMenu`: the notification menu.
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_inbox_menu(event.position, cx);
                    }),
                ),
            )
            .child(rail_action(
                "rail-notes",
                IconName::FileText,
                "Notes",
                self.surface_open(Surface::Notes),
                RailTrailing::None,
                cx,
                |this, _, _, cx| this.open_notes(cx),
            ))
            .child(rail_action(
                "rail-automations",
                IconName::Zap,
                "Automations",
                self.surface_open(Surface::Automations),
                RailTrailing::None,
                cx,
                |this, _, _, cx| this.open_automations(cx),
            ))
    }

    /// The rail's projects in saved order (MonoCode `collectRailProjects`
    /// + `syncProjectRailOrder`): archived ones left out unless open.
    pub(super) fn rail_order(&self) -> Vec<String> {
        let prefs = &self.settings.rail;
        let projects: Vec<String> = self
            .recent_projects
            .iter()
            .filter(|p| !prefs.is_archived(p) || same_project_path(p, &self.current_cwd))
            .cloned()
            .collect();
        sync_rail_order(&prefs.project_rail_order, &projects)
    }

    pub(super) fn rail_sections_for(&self, order: &[String]) -> RailSections {
        rail_sections(order, &self.settings.pinned_projects, &self.settings.rail)
    }

    /// The scrolling lists: Pinned (when any), Groups (when any), Projects.
    fn render_rail_projects(&self, cx: &Context<Self>) -> impl IntoElement {
        let order = self.rail_order();
        let sections = self.rail_sections_for(&order);
        let empty = sections.ungrouped.is_empty() && sections.groups.is_empty() && sections.pinned.is_empty();
        let pinned = sections.pinned;
        // `flex min-h-0 flex-1 flex-col overflow-y-auto pb-2`
        div()
            .id("rail-projects")
            .flex()
            .flex_1()
            .min_h_0()
            .flex_col()
            .overflow_y_scroll()
            .pb_2()
            .when(!pinned.is_empty(), |el| {
                el.child(self.render_project_section("Pinned", "pinned", &pinned, false, false, cx))
            })
            .when(!sections.groups.is_empty(), |el| {
                el.child(self.render_groups_section(&sections.groups, cx))
            })
            .child(self.render_project_section("Projects", "projects", &sections.ungrouped, true, empty, cx))
    }

    /// MonoCode `AddProjectButton`, lit while its popover is open.
    fn render_add_project_button(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let open = matches!(
            self.sidebar_menu.as_ref().map(|m| &m.kind),
            Some(SidebarMenuKind::AddProject)
        );
        let anchor: Anchor = Rc::default();
        let button_anchor = anchor.clone();
        section_button("rail-add-project", IconName::Plus, "Open project", open, fg)
            .child(canvas(move |bounds, _, _| button_anchor.set(Some(bounds)), |_, _, _, _| {}).absolute().size_full())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let at = anchor.get().map_or(event.position, |b| {
                        Point::new(b.left(), b.bottom() + px(POPOVER_GAP))
                    });
                    this.toggle_add_project_menu(at, cx);
                }),
            )
    }

    fn toggle_add_project_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let was_open = matches!(
            self.sidebar_menu.as_ref().map(|m| &m.kind),
            Some(SidebarMenuKind::AddProject)
        );
        self.close_sidebar_menu(cx);
        self.close_rail_menu(cx);
        if was_open {
            // The popover's outside-press check sees this and stays closed.
            self.sessions_ui.filter_button_hit = true;
            return;
        }
        self.open_sidebar_menu(SidebarMenuKind::AddProject, at, cx);
    }

    /// The "Open project" popover: `Popover width={230} className="p-1"`.
    /// MonoCode's "Open folder on a machine…" needs its remote machines,
    /// which BenCode does not have.
    pub(crate) fn render_add_project_menu(&self, position: Point<Pixels>, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        // `flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5
        // text-[13px] text-content/80 hover:bg-content/8 hover:text-content`
        let row = div()
            .id("rail-open-folder")
            .group("rail-open-folder")
            .flex()
            .w_full()
            .items_center()
            .gap(px(10.0))
            .rounded(px(8.0))
            .px(px(10.0))
            .py(px(6.0))
            .text_size(px(13.0))
            .text_color(fg.opacity(0.8))
            .cursor_pointer()
            .hover(move |s| s.bg(fg.opacity(0.08)).text_color(fg))
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_sidebar_menu(cx);
                this.open_project_dialog(cx);
            }))
            .child(
                Icon::new(IconName::FolderPlus)
                    .size(IconSize::Sm)
                    .color(fg.opacity(0.8))
                    .group_hover_color("rail-open-folder", fg),
            )
            .child("Open folder…");
        let menu = crate::ui::sidebar_popovers::popover_frame(cx)
            .id("rail-add-project-menu")
            .w(px(ADD_PROJECT_WIDTH))
            .p_1()
            .on_mouse_down_out(cx.listener(|_, _, window, cx| {
                cx.defer_in(window, |this, _, cx| {
                    if !std::mem::take(&mut this.sessions_ui.filter_button_hit) {
                        this.close_sidebar_menu(cx);
                    }
                });
            }))
            .child(row);
        deferred(anchored().position(position).snap_to_window().child(menu))
            .with_priority(3)
            .into_any_element()
    }

    /// MonoCode `toggleProjectPin`: pinning appends to the Pinned list.
    pub(super) fn toggle_project_pin(&mut self, path: &str, cx: &mut Context<Self>) {
        let pins = &self.settings.pinned_projects;
        let next: Vec<String> = if pins.iter().any(|p| same_project_path(p, path)) {
            pins.iter().filter(|p| !same_project_path(p, path)).cloned().collect()
        } else {
            pins.iter().cloned().chain(std::iter::once(path.to_string())).collect()
        };
        self.set_pinned_projects(next, cx);
    }

    /// The rail's menus and dialogs, drawn over the window.
    /// The Delete confirmation also serves Settings › Archive, so it shows
    /// with the rail hidden; the menus belong to the rail.
    pub fn render_rail_overlays(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let menus = if self.is_rail_open {
            vec![
                self.render_rail_menu(cx),
                self.render_inbox_menu(cx),
                self.render_rail_submenu(cx),
                self.render_mute_picker(cx),
            ]
        } else {
            Vec::new()
        };
        menus
            .into_iter()
            .chain(std::iter::once(self.render_remove_project_dialog(cx)))
            .flatten()
            .collect()
    }

    /// Back / Forward over visited tabs (MonoCode `TabVisitNav`), dimmed
    /// when there is nowhere to go.
    pub fn history_buttons(&self, prefix: &str, cx: &Context<Self>) -> [AnyElement; 2] {
        let state = |enabled: bool| {
            if enabled {
                TitleButtonState::Normal
            } else {
                TitleButtonState::Disabled
            }
        };
        [
            title_icon_button(
                SharedString::from(format!("{prefix}-nav-back")),
                IconName::ChevronLeft,
                "Back (⌘[)",
                state(self.tab_history.can_go_back()),
                cx,
                |this, _, _, cx| this.go_back(cx),
            )
            .into_any_element(),
            title_icon_button(
                SharedString::from(format!("{prefix}-nav-forward")),
                IconName::ChevronRight,
                "Forward (⌘])",
                state(self.tab_history.can_go_forward()),
                cx,
                |this, _, _, cx| this.go_forward(cx),
            )
            .into_any_element(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_names_are_the_last_segment() {
        assert_eq!(project_name("/Users/me/bencode/"), "bencode");
        assert_eq!(project_name("/Users/me/bencode"), "bencode");
    }

    #[test]
    fn project_colours_skip_the_untinted_first() {
        let first = palette_color(0).unwrap();
        for name in ["bencode", "monocode", "a", "doclasse-pf-frontend"] {
            assert_ne!(tab_group_color(name), first);
        }
    }

    #[test]
    fn group_colours_prefer_custom_then_palette() {
        let group = ProjectGroup {
            id: "g".into(),
            name: "G".into(),
            color_index: Some(2),
            ..Default::default()
        };
        assert_eq!(group_color(&group), palette_color(2).unwrap());
        let custom = ProjectGroup {
            custom_color: Some("#ff0000".into()),
            ..group
        };
        assert_eq!(group_color(&custom), parse_hex("#ff0000").unwrap());
    }
}
