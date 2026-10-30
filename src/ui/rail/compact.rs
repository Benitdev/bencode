//! MonoCode `CompactProjectRail` and `CompactRailAction` (`Sidebar.tsx`):
//! the column of icons the navigator leaves behind when collapsed with
//! Appearance › "Collapsed navigator" on "Icon rail". It expands the
//! navigator, opens the views, lists the agents at work (one icon each,
//! so a waiting one is a click away) and, from the project's icon, the
//! project's threads and the other projects.

use std::rc::Rc;

use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, ClickEvent, Context, Div, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    Pixels, Point, SharedString, Stateful, Styled, anchored, canvas, deferred, div, prelude::*,
};

use super::live_agents::live_agent_elapsed;
use super::widgets::{figure_icon, project_figure_icon};
use super::{TITLEBAR_HEIGHT, path_key};
use crate::app::live_agents::LiveAgent;
use crate::app::session_list::{compare_sessions, format_age, session_title};
use crate::app::{BenCodeApp, Surface, same_project_path};
use crate::ui::appearance::CollapsedRailMode;
use crate::ui::scale::px;
use crate::ui::sidebar_menus::SidebarMenuKind;
use crate::ui::thumbnail::{LOGO, thumbnail};
use crate::ui::tokens::{Fill, Ink, Status, Text, Tone as _};

/// MonoCode `w-12`.
pub const COMPACT_RAIL_WIDTH: f32 = 48.0;
/// The project popover: wider than MonoCode's 230, for threads' titles.
const PROJECTS_WIDTH: f32 = 280.0;
/// The project's threads the popover lists before "Show all".
const POPOVER_THREADS: usize = 10;
/// The agents the rail shows before "+N".
const RAIL_AGENTS: usize = 6;
const POPOVER_GAP: f32 = 6.0;

impl BenCodeApp {
    /// MonoCode `compactRailVisible`: the rail is collapsed to icons.
    /// Settings brings its own navigation instead.
    pub fn compact_rail_active(&self) -> bool {
        self.appearance.collapsed_rail == CollapsedRailMode::Compact
            && !self.is_rail_open
            && self.surface != Some(Surface::Settings)
    }

    /// MonoCode `compactTitleBar`: on macOS the title bar runs above the
    /// icon rail, which is narrower than the traffic lights.
    pub fn compact_title_bar(&self) -> bool {
        cfg!(target_os = "macos")
            && self.compact_rail_active()
            && self.surface.is_none()
            && self.bots.view.is_none()
    }

    /// MonoCode `action(active, open)`: a view's icon opens it, or leaves
    /// it when it is the one open.
    fn toggle_surface(&mut self, surface: Surface, cx: &mut Context<Self>) {
        if self.surface_open(surface) {
            self.close_surface(cx);
            return;
        }
        match surface {
            Surface::Inbox => self.open_inbox_modal(cx),
            Surface::Notes => self.open_notes(cx),
            Surface::Automations => self.open_automations(cx),
            Surface::Settings => self.open_settings(cx),
        }
    }

    /// MonoCode `CompactProjectRail`: `sidebar-glass flex h-full w-12
    /// shrink-0 flex-col items-center`, a hairline down its right edge;
    /// `usage` is the focused thread's usage icon, above Settings.
    pub fn render_compact_rail(
        &self,
        usage: Option<AnyElement>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let fg = colors.fg;
        let glass = self.glass(cx).sidebar(colors.bg, theme.is_dark());
        let title_bar_above = self.compact_title_bar();
        let surface_action =
            |id: &'static str, icon: IconName, label: &'static str, surface: Surface, dot: bool| {
                compact_action(id, icon, label, self.surface_open(surface), dot, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_surface(surface, cx)))
            };
        let needs_you = self.inbox_needs_you(cx);
        let automation_badge = self.automation_badge();
        div()
            .id("compact-project-rail")
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .w(px(COMPACT_RAIL_WIDTH))
            .h_full()
            .bg(glass)
            .border_r_1()
            .border_color(fg.fill(Fill::Line))
            // Without the title bar above, its height is held (and drags
            // the window): `h-10 w-full shrink-0 border-b border-stroke`.
            .when(!title_bar_above, |el| {
                el.child(
                    self.window_drag_region(div(), cx)
                        .flex_none()
                        .w_full()
                        .h(px(TITLEBAR_HEIGHT))
                        .border_b_1()
                        .border_color(fg.fill(Fill::Line)),
                )
            })
            .child(
                // `flex w-full shrink-0 flex-col items-center gap-1.5 py-1.5`
                div()
                    .flex()
                    .flex_none()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .gap_1p5()
                    .py_1p5()
                    .child(
                        compact_action(
                            "compact-rail-expand",
                            IconName::PanelLeft,
                            "Show Navigator (⌘B)",
                            false,
                            false,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.is_rail_open = true;
                            cx.notify();
                        })),
                    )
                    .children(self.render_compact_bots(cx))
                    .child(self.render_compact_project_button(cx))
                    .child(
                        compact_action(
                            "compact-rail-search",
                            IconName::Search,
                            "Search (⌘K)",
                            self.search.open,
                            false,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_search(cx))),
                    )
                    .child(
                        surface_action(
                            "compact-rail-inbox",
                            IconName::Inbox,
                            if needs_you {
                                "Inbox, items need you"
                            } else {
                                "Inbox"
                            },
                            Surface::Inbox,
                            needs_you,
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
                    .child(surface_action(
                        "compact-rail-notes",
                        IconName::StickyNote,
                        "Notes",
                        Surface::Notes,
                        false,
                    ))
                    .child(surface_action(
                        "compact-rail-automations",
                        IconName::Zap,
                        automation_badge.map_or("Automations", |why| why.rail_tip()),
                        Surface::Automations,
                        automation_badge.is_some(),
                    )),
            )
            // Under the views, so their icons hold still as agents come
            // and go.
            .children(self.render_compact_agents(cx))
            // `min-h-2 flex-1`
            .child(div().flex_1().min_h_2())
            .child(
                // `flex w-full flex-col items-center gap-1 py-1.5`
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .gap_1()
                    .py_1p5()
                    .children(usage)
                    .child(
                        compact_action(
                            "compact-rail-settings",
                            IconName::Settings,
                            "Settings (⌘,)",
                            false,
                            false,
                            cx,
                        )
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx)),
                        ),
                    ),
            )
    }

    /// The Agents section as icons: each agent's project figure, moving
    /// while it works, an amber dot while it waits on the user and a green
    /// one when it finished unseen; a click opens its thread. Past
    /// `RAIL_AGENTS`, "+N" brings the navigator back with the rest.
    fn render_compact_agents(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let agents = self.live_agents();
        if agents.is_empty() {
            return None;
        }
        let fg = cx.theme().colors.fg;
        let now = crate::app::now_ms();
        let extra = agents.len().saturating_sub(RAIL_AGENTS);
        let buttons = agents[..agents.len() - extra]
            .iter()
            .map(|agent| self.render_compact_agent(agent, now, cx));
        let more = (extra > 0).then(|| {
            div()
                .id("compact-agents-more")
                .flex()
                .flex_none()
                .size(px(32.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(Text::Sm.px())
                .text_color(fg.ink(Ink::Secondary))
                .hover(move |s| s.bg(fg.fill(Fill::Strong)).text_color(fg))
                .tooltip(Tooltip::text(format!("{extra} more agents")))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.is_rail_open = true;
                    cx.notify();
                }))
                .child(format!("+{extra}"))
        });
        Some(
            div()
                .id("compact-agents")
                .flex()
                .flex_col()
                .flex_initial()
                .min_h_0()
                .items_center()
                .w_full()
                .gap_1p5()
                .pb_1p5()
                .overflow_hidden()
                .child(
                    div()
                        .w(px(20.0))
                        .h(px(1.0))
                        .mb_1()
                        .bg(fg.fill(Fill::Strong)),
                )
                .children(buttons)
                .children(more)
                .into_any_element(),
        )
    }

    fn render_compact_agent(&self, agent: &LiveAgent, now: i64, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let selection = fg.opacity(if theme.is_dark() { 0.10 } else { 0.06 });
        let amber = Status::Attention.color(fg);
        let selected = self.selected_session_id.as_deref() == Some(agent.id.as_str());
        let live = !agent.needs_approval && !agent.done;
        let activity = self.live_agent_activity(agent);
        let project = self.rail_project_label(&agent.cwd);
        let tip = [
            Some(agent.title.as_str()),
            Some(project.as_str()),
            Some(activity.as_str()),
            live_agent_elapsed(agent, now).as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
        let dot = if agent.needs_approval {
            Some(amber)
        } else if agent.done {
            Some(Status::Done.color(fg))
        } else {
            None
        };
        let id = agent.id.clone();
        div()
            .id(SharedString::from(format!("compact-agent-{}", agent.id)))
            .relative()
            .flex()
            .flex_none()
            .size(px(32.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .map(|el| {
                if selected {
                    el.bg(selection)
                } else if agent.needs_approval {
                    el.bg(amber.opacity(0.10))
                        .hover(move |s| s.bg(amber.opacity(0.18)))
                } else {
                    el.hover(move |s| s.bg(fg.fill(Fill::Strong)))
                }
            })
            .tooltip(Tooltip::text(tip))
            .on_click(cx.listener(move |this, _, _, cx| this.select_live_agent(&id, cx)))
            .child(figure_icon(
                self.project_figure(&agent.cwd),
                self.project_color(&agent.cwd),
                live,
                14.0,
                cx,
            ))
            .children(dot.map(|color| {
                div()
                    .absolute()
                    .top(px(5.0))
                    .right(px(5.0))
                    .size(px(6.0))
                    .rounded_full()
                    .bg(color)
            }))
            .into_any_element()
    }

    /// MonoCode `railMonosPinned`: each Bot at the top of the icon rail,
    /// above a divider (`CompactMonoButton`), its figure moving with its
    /// mood. Unpinned, the icon rail shows none (MonoCode lists them in
    /// its project picker, which BenCode's compact picker does not).
    fn render_compact_bots(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.bots_enabled() || !self.settings.rail_bots_pinned || self.settings.bots.is_empty()
        {
            return None;
        }
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let selection = fg.opacity(if theme.is_dark() { 0.10 } else { 0.06 });
        let buttons = self.settings.bots.iter().map(|bot| {
            let selected = self.surface.is_none() && self.bots.view.as_deref() == Some(&bot.id);
            let id = bot.id.clone();
            div()
                .id(SharedString::from(format!("compact-bot-{}", bot.id)))
                .flex()
                .flex_none()
                .size(px(32.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .map(|el| {
                    if selected {
                        el.bg(selection)
                    } else {
                        el.hover(move |s| s.bg(fg.fill(Fill::Strong)))
                    }
                })
                .tooltip(Tooltip::text(bot.display_name()))
                .on_click(cx.listener(move |this, _, _, cx| this.open_bot(&id, cx)))
                .child(crate::ui::figure::figure_view(
                    bot.figure(),
                    16.0,
                    self.bot_color(bot),
                    self.bot_look(bot),
                    cx,
                ))
        });
        Some(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1p5()
                .children(buttons)
                .child(div().w(px(20.0)).h(px(1.0)).bg(fg.fill(Fill::Strong)))
                .into_any_element(),
        )
    }

    /// The current project's logo or figure (MonoCode's `compact` project
    /// picker); a press lists the rail's projects to switch to.
    fn render_compact_project_button(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let cwd = self.current_cwd.clone();
        let open = matches!(
            self.sidebar_menu.as_ref().map(|menu| &menu.kind),
            Some(SidebarMenuKind::CompactProjects)
        );
        let label = if cwd.trim().is_empty() {
            "Open project".to_string()
        } else {
            self.rail_project_label(&cwd)
        };
        let anchor: Rc<std::cell::Cell<Option<gpui::Bounds<Pixels>>>> = Rc::default();
        let button_anchor = anchor.clone();
        div()
            .id("compact-rail-project")
            .relative()
            .flex()
            .flex_none()
            .size(px(32.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .when(open, |el| el.bg(fg.fill(Fill::Strong)))
            .hover(move |el| el.bg(fg.fill(Fill::Strong)))
            .tooltip(Tooltip::text(label))
            .child(
                canvas(
                    move |bounds, _, _| button_anchor.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(self.project_glyph(&cwd, cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let at = anchor.get().map_or(event.position, |bounds| {
                        Point::new(bounds.right() + px(POPOVER_GAP), bounds.top())
                    });
                    this.toggle_compact_projects_menu(at, cx);
                }),
            )
    }

    /// A project's logo, else its figure; a folder with no project.
    pub(crate) fn project_glyph(&self, path: &str, cx: &Context<Self>) -> AnyElement {
        if path.trim().is_empty() {
            let fg = cx.theme().colors.fg;
            return Icon::new(IconName::FolderPlus)
                .size(IconSize::Md)
                .color(fg.ink(Ink::Secondary))
                .into_any_element();
        }
        let busy = self
            .runs
            .keys()
            .filter_map(|id| self.sessions.iter().find(|s| &s.id == id))
            .any(|s| crate::app::is_path_in_project(&s.cwd, path));
        match self
            .settings
            .rail
            .tab_group_logos
            .get(&path_key(path))
            .filter(|_| !busy)
        {
            // `ProjectLogoIcon className="size-4 rounded-sm"`
            Some(file) => thumbnail(file, LOGO)
                .size_4()
                .rounded(px(4.0))
                .into_any_element(),
            None => project_figure_icon(
                self.project_figure(path),
                self.project_color(path),
                busy,
                cx,
            ),
        }
    }

    fn toggle_compact_projects_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let was_open = matches!(
            self.sidebar_menu.as_ref().map(|menu| &menu.kind),
            Some(SidebarMenuKind::CompactProjects)
        );
        self.close_sidebar_menu(cx);
        self.close_rail_menu(cx);
        if was_open {
            // The popover's outside-press check sees this and stays closed.
            self.sessions_ui.filter_button_hit = true;
            return;
        }
        self.open_sidebar_menu(SidebarMenuKind::CompactProjects, at, cx);
    }

    /// The icon rail's popover: the current project's latest threads (a
    /// new one, "Show all" past `POPOVER_THREADS`), the rail's projects in
    /// its order, the current one marked, then "Open folder…" and "Open
    /// folder on a machine…". MonoCode's picker also searches and carries
    /// each project's menu; those stay on the full rail here.
    pub(crate) fn render_compact_projects_menu(
        &self,
        position: Point<Pixels>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let fg = cx.theme().colors.fg;
        // `flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5
        // text-[13px] text-content/80 hover:bg-content/8 hover:text-content`
        let row = |id: SharedString| {
            div()
                .id(id)
                .flex()
                .w_full()
                .items_center()
                .gap(px(10.0))
                .rounded(px(8.0))
                .px(px(10.0))
                .py(px(6.0))
                .text_size(Text::Md.px())
                .text_color(fg.ink(Ink::Strong))
                .cursor_pointer()
                .hover(move |s| s.bg(fg.fill(Fill::Selected)).text_color(fg))
        };
        let projects = self.rail_order().into_iter().enumerate().map(|(ix, path)| {
            let current = same_project_path(&path, &self.current_cwd);
            let target = path.clone();
            row(SharedString::from(format!("compact-project-{ix}")))
                .when(current, |el| el.bg(fg.fill(Fill::Selected)).text_color(fg))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .size_4()
                        .items_center()
                        .justify_center()
                        .child(self.project_glyph(&path, cx)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(self.rail_project_label(&path)),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.close_sidebar_menu(cx);
                    if this.surface.is_some() {
                        this.close_surface(cx);
                    }
                    this.switch_project(target.clone(), cx);
                }))
        });
        let open_folder = row("compact-project-open-folder".into())
            .child(
                Icon::new(IconName::FolderPlus)
                    .size(IconSize::Sm)
                    .color(fg.ink(Ink::Strong)),
            )
            .child("Open folder…")
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_sidebar_menu(cx);
                this.open_project_dialog(cx);
            }));
        let open_remote = row("compact-project-open-remote-folder".into())
            .child(
                Icon::new(IconName::Globe)
                    .size(IconSize::Sm)
                    .color(fg.ink(Ink::Strong)),
            )
            .child("Open folder on a machine…")
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_sidebar_menu(cx);
                this.open_remote_folder_dialog(cx);
            }));
        let threads = self.render_compact_threads(cx);
        let list = div()
            .id("compact-projects-list")
            .flex()
            .flex_col()
            .max_h(px(320.0))
            .overflow_y_scroll()
            .children(projects);
        let menu = crate::ui::sidebar_popovers::popover_frame(cx)
            .id("compact-projects-menu")
            .occlude()
            .w(px(PROJECTS_WIDTH))
            .p_1()
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|_, _, window, cx| {
                cx.defer_in(window, |this, _, cx| {
                    if !std::mem::take(&mut this.sessions_ui.filter_button_hit) {
                        this.close_sidebar_menu(cx);
                    }
                });
            }))
            .children(threads)
            .child(list)
            .child(div().my_1().h(px(1.0)).bg(fg.fill(Fill::Strong)))
            .child(open_folder)
            .child(open_remote);
        deferred(anchored().position(position).snap_to_window().child(menu))
            .with_priority(3)
            .into_any_element()
    }

    /// The popover's threads: the current project's as the navigator
    /// lists them, latest first, each with its run's state (still: the
    /// popover is the shell's, which the busy clock does not redraw), and a
    /// divider under them. None without a project.
    fn render_compact_threads(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if matches!(self.current_cwd.trim(), "" | "~") {
            return None;
        }
        let theme = cx.theme();
        let fg = theme.colors.fg;
        let accent = theme.colors.accent;
        let states = self.session_live_states();
        let mut threads = self.listed_sessions();
        threads.retain(|s| !s.archived);
        threads.sort_by(|a, b| compare_sessions(a, b));
        let extra = threads.len().saturating_sub(POPOVER_THREADS);
        threads.truncate(POPOVER_THREADS);
        let now = crate::app::now_ms();
        let empty = threads.is_empty();

        let new_thread = div()
            .id("compact-threads-new")
            .flex()
            .flex_none()
            .size(px(22.0))
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .cursor_pointer()
            .hover(move |s| s.bg(fg.fill(Fill::Selected)))
            .tooltip(Tooltip::text("New thread (⌘T)"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_sidebar_menu(cx);
                if this.surface.is_some() {
                    this.close_surface(cx);
                }
                this.create_new_session(cx);
            }))
            .child(
                Icon::new(IconName::Plus)
                    .size(IconSize::Sm)
                    .color(fg.ink(Ink::Secondary)),
            );
        let header = div()
            .flex()
            .items_center()
            .gap_1()
            .pl(px(10.0))
            .pr(px(4.0))
            .pt(px(4.0))
            .pb(px(2.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Muted))
                    .child(self.rail_project_label(&self.current_cwd)),
            )
            .child(new_thread);

        let rows = threads.into_iter().map(|session| {
            let selected = self.selected_session_id.as_deref() == Some(session.id.as_str());
            let status = if states.approval.contains(&session.id) {
                Some(
                    Icon::new(IconName::CircleAlert)
                        .size(IconSize::Xs)
                        .color(Status::Attention.color(fg))
                        .into_any_element(),
                )
            } else if states.busy.contains(&session.id) {
                Some(
                    div()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(accent)
                        .into_any_element(),
                )
            } else if states.done.contains(&session.id) {
                Some(
                    Icon::new(IconName::Check)
                        .size(IconSize::Xs)
                        .color(Status::Done.color(fg))
                        .into_any_element(),
                )
            } else {
                None
            };
            let id = session.id.clone();
            div()
                .id(SharedString::from(format!("compact-thread-{}", session.id)))
                .flex()
                .w_full()
                .items_center()
                .gap(px(8.0))
                .rounded(px(8.0))
                .px(px(10.0))
                .py(px(5.0))
                .text_size(Text::Md.px())
                .text_color(fg.ink(Ink::Strong))
                .cursor_pointer()
                .when(selected, |el| el.bg(fg.fill(Fill::Selected)).text_color(fg))
                .hover(move |s| s.bg(fg.fill(Fill::Selected)).text_color(fg))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.close_sidebar_menu(cx);
                    this.select_live_agent(&id, cx);
                }))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .size_4()
                        .items_center()
                        .justify_center()
                        .children(status),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(session_title(session)),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(Text::Sm.px())
                        .text_color(fg.ink(Ink::Muted))
                        .child(format_age(session.updated_at, now)),
                )
        });
        let show_all = (extra > 0).then(|| {
            div()
                .id("compact-threads-all")
                .flex()
                .w_full()
                .items_center()
                .rounded(px(8.0))
                .px(px(10.0))
                .py(px(5.0))
                .pl(px(34.0))
                .text_size(Text::Sm.px())
                .text_color(fg.ink(Ink::Secondary))
                .cursor_pointer()
                .hover(move |s| s.bg(fg.fill(Fill::Selected)).text_color(fg))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_sidebar_menu(cx);
                    this.is_rail_open = true;
                    cx.notify();
                }))
                .child(format!("Show all ({extra} more)"))
        });
        Some(
            div()
                .flex()
                .flex_col()
                .child(header)
                .when(empty, |el| {
                    el.child(
                        div()
                            .px(px(10.0))
                            .py(px(5.0))
                            .text_size(Text::Sm.px())
                            .text_color(fg.ink(Ink::Muted))
                            .child("No threads yet"),
                    )
                })
                .child(
                    div()
                        .id("compact-threads-list")
                        .flex()
                        .flex_col()
                        .max_h(px(300.0))
                        .overflow_y_scroll()
                        .children(rows),
                )
                .children(show_all)
                .child(div().my_1().h(px(1.0)).bg(fg.fill(Fill::Strong)))
                .into_any_element(),
        )
    }
}

/// MonoCode `CompactRailAction`: `grid size-8 place-items-center
/// rounded-md`, `bg-selection text-content` when active, else
/// `text-content/50 hover:bg-content/10 hover:text-content`; a `size-4`
/// icon and, with `dot`, a `size-1.5` accent dot at its top right.
fn compact_action(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    active: bool,
    dot: bool,
    cx: &Context<BenCodeApp>,
) -> Stateful<Div> {
    let theme = cx.theme();
    let (fg, accent) = (theme.colors.fg, theme.colors.accent);
    let selection = fg.opacity(if theme.is_dark() { 0.10 } else { 0.06 });
    div()
        .id(id)
        .group(id)
        .relative()
        .flex()
        .flex_none()
        .size(px(32.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .tooltip(Tooltip::text(label))
        .map(|el| {
            if active {
                el.bg(selection)
            } else {
                el.hover(move |s| s.bg(fg.fill(Fill::Strong)))
            }
        })
        .child(
            Icon::new(icon)
                .size(IconSize::Md)
                .color(if active { fg } else { fg.ink(Ink::Secondary) })
                .when(!active, |icon| icon.group_hover_color(id, fg)),
        )
        .when(dot, |el| {
            el.child(
                div()
                    .absolute()
                    .top(px(6.0))
                    .right(px(6.0))
                    .size(px(6.0))
                    .rounded_full()
                    .bg(accent),
            )
        })
}
