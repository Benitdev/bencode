//! The navigator's thread cards (MonoCode `SessionCard`, in one line and
//! in the look of the navigator's other rows, `rail::widgets::nav_row`):
//! harness, title, badges and status, the model and branch in the tooltip,
//! in the states MonoCode draws (picked, needs approval, open, draft),
//! dragged to group or split, right-clicked for its menu.

use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnimationExt, AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, SharedString, Styled, canvas, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::app::session_list::{self, LiveStates, NO_BRANCH_LABEL, format_age, git_label};
use crate::db::SessionRow;
use crate::harness::catalog;
use crate::ui::drag_drop::DraggedSession;
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::rail::widgets::{ROW_HEIGHT, UNSELECTED_OPACITY, nav_label, selection_strong};
use crate::ui::region::Region;
use crate::ui::scale::{self, px};
use crate::ui::sidebar_sessions::ListDrop;
use crate::ui::spinner::terminal_spinner;
use crate::ui::tokens::{Fill, Ink, Status, Text, Tone as _};
use crate::ui::virtual_rows;

/// The slide-in opens to this height (a usual card's); the cap comes off
/// when it finishes.
const CARD_MAX_HEIGHT: f32 = 160.0;
/// How far the selected thread's accent bar stands off its row's top and
/// bottom.
const SELECTED_BAR_INSET: f32 = 8.0;

impl BenCodeApp {
    /// MonoCode's card status, shortened to fit beside the title:
    /// approval, working, done, draft, else age.
    fn session_status(
        &self,
        session: &SessionRow,
        draft: bool,
        states: &LiveStates,
        now: i64,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let row = |color: gpui::Hsla| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .text_size(Text::Sm.px())
                .text_color(color)
        };
        let icon =
            |name: IconName, color: gpui::Hsla| Icon::new(name).size(IconSize::Xs).color(color);
        if states.approval.contains(&session.id) {
            let amber = Status::Attention.color(colors.fg);
            return row(amber)
                .child(icon(IconName::CircleAlert, amber))
                .child(if session.orchestration.is_some() {
                    "Input"
                } else {
                    "Approve"
                })
                .into_any_element();
        }
        // The spinner says it all where the title needs the room.
        if states.busy.contains(&session.id) {
            return row(colors.accent)
                .child(terminal_spinner(colors.accent, cx))
                .into_any_element();
        }
        if states.done.contains(&session.id) {
            let green = Status::Done.color(colors.fg);
            return row(green)
                .child(icon(IconName::Check, green))
                .child("Done")
                .into_any_element();
        }
        if draft {
            let muted = colors.fg.ink(Ink::Secondary);
            return row(muted)
                .child(icon(IconName::CircleDashed, muted))
                .child("Draft")
                .into_any_element();
        }
        row(colors.fg.ink(Ink::Muted))
            .child(format_age(session.updated_at, now))
            .into_any_element()
    }

    /// The card title; a title that just changed sweeps in as the old one
    /// drifts up and fades (MonoCode `ParticleText`, without particles).
    /// A working thread's title shimmers, as a busy project's name.
    fn render_card_title(&self, id: &str, title: &str, busy: bool, cx: &gpui::App) -> AnyElement {
        // The navigator rows' name (`nav_label`), as a project's.
        let text = |text: String| {
            nav_label().child(if busy {
                crate::ui::spinner::shimmer(text, cx)
            } else {
                text.into_any_element()
            })
        };
        let Some(change) = self.sessions_ui.title_changes.get(id) else {
            return text(title.to_string()).flex_1().into_any_element();
        };
        let (old, key) = (change.old.clone(), change.serial);
        let sweep = crate::ui::sidebar_sessions::TITLE_SWEEP;
        let ease = crate::ui::motion::cubic_bezier(0.65, 0.0, 0.35, 1.0);
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .child(text(title.to_string()).with_animation(
                SharedString::from(format!("title-in-{id}-{key}")),
                gpui::Animation::new(sweep),
                {
                    let ease = crate::ui::motion::cubic_bezier(0.65, 0.0, 0.35, 1.0);
                    move |el, delta| el.opacity(ease(delta))
                },
            ))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .child(text(old).with_animation(
                        SharedString::from(format!("title-out-{id}-{key}")),
                        gpui::Animation::new(sweep),
                        move |el, delta| {
                            let t = ease(delta);
                            el.opacity(1.0 - t).mt(px(-6.0 * t))
                        },
                    )),
            )
            .into_any_element()
    }

    /// The card's branch: the thread's, else the project's live one, or
    /// "No branch selected" once its worktree is gone; and the tooltip that
    /// names its repository and worktree (MonoCode's git line).
    fn session_branch(&self, session: &SessionRow, cx: &gpui::App) -> (String, String) {
        if session.worktree_removed {
            return (NO_BRANCH_LABEL.to_string(), NO_BRANCH_LABEL.to_string());
        }
        let repo = self.workspace(cx).repo.clone().or_else(|| {
            std::path::Path::new(&session.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        });
        // MonoCode falls back to the project's live branch.
        let live = (!self.git_status.branch.is_empty() && session.worktree_cwd.is_none())
            .then_some(self.git_status.branch.as_str());
        let branch = session.branch.as_deref().or(live);
        let full = git_label(repo.as_deref(), branch);
        let tip = match session.worktree_cwd.as_deref().filter(|w| !w.is_empty()) {
            Some(tree) if !full.is_empty() => format!("{full}\n{tree}"),
            _ => full,
        };
        (git_label(None, branch), tip)
    }

    /// A thread in one line (BenCode's own, where MonoCode's card takes
    /// three): its harness, title, badges and status, Archive over the
    /// status on hover. The model and branch are in its tooltip, and a
    /// worktree's thread marks its branch.
    pub(crate) fn render_session_card(
        &self,
        session: &SessionRow,
        states: &LiveStates,
        now: i64,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let id = session.id.clone();
        let active = self.selected_session_id.as_deref() == Some(id.as_str());
        let picked = self.sessions_ui.selection.ids.contains(&id);
        let needs_approval = states.approval.contains(&id);
        let draft = session.is_draft();
        let drop_target = self.sessions_ui.drop == Some(ListDrop::Session(id.clone()));
        let title = session_list::display_title(&session.title, &session.harness);
        let group = SharedString::from(format!("session-card-{id}"));
        // A working thread cannot be archived: it keeps its spinner on hover.
        let busy = states.busy.contains(&id);
        // MonoCode: the linked-update dot leads the status.
        let status = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .when(!busy, |el| el.group_hover(group.clone(), |s| s.invisible()))
            .children(self.linked_update_dot(session, cx))
            .child(self.session_status(session, draft, states, now, cx));
        // MonoCode `orchestrationExpanded`: an orchestrator lists its agents
        // while open, picked or working.
        let expanded =
            session.orchestration.is_some() && (active || picked || states.busy.contains(&id));

        if self.sessions_ui.renaming_session.as_deref() == Some(id.as_str()) {
            return self.render_session_rename_row(active, needs_approval, cx);
        }

        let model = catalog::display_label(&session.harness, &session.model);
        let (branch, branch_tip) = self.session_branch(session, cx);
        let tooltip = [title.clone(), model, branch_tip.clone()]
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let in_worktree = !session.worktree_removed
            && session
                .worktree_cwd
                .as_deref()
                .is_some_and(|tree| !tree.is_empty());
        let archive_label = if session.archived {
            "Unarchive"
        } else {
            "Archive"
        };
        let archive_id = id.clone();
        let archive = div()
            .id(SharedString::from(format!("{group}-archive")))
            .group(SharedString::from(format!("{group}-archive")))
            .absolute()
            .right_0()
            .size(px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .invisible()
            .group_hover(group.clone(), |s| s.visible())
            .cursor_pointer()
            .hover(move |s| s.bg(fg.fill(Fill::Strong)))
            .tooltip(Tooltip::text(archive_label))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_archive_session(&archive_id, cx);
            }))
            .child(
                Icon::new(IconName::Archive)
                    .size(IconSize::Xs)
                    .color(fg.ink(Ink::Secondary))
                    .group_hover_color(format!("{group}-archive"), fg),
            );
        // As a project's row: `h-8`, the icon in a `size-4` box, `gap-2`
        // (the card's 1px border is inside the height).
        let row = div()
            .flex()
            .min_w_0()
            .h(px(ROW_HEIGHT - 2.0))
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .size_4()
                    .items_center()
                    .justify_center()
                    .child(HarnessIcon::new(&session.harness).size(px(14.0))),
            )
            .when(session.pinned, |el| {
                el.child(
                    Icon::new(IconName::Pin)
                        .size(IconSize::Xs)
                        .color(fg.ink(Ink::Muted)),
                )
            })
            .child(self.render_card_title(&id, &title, busy, cx))
            .when(in_worktree, |el| {
                el.child(
                    div()
                        .id(SharedString::from(format!("{group}-worktree")))
                        .flex_none()
                        .tooltip(Tooltip::text(format!("In a worktree: {branch}")))
                        .child(
                            Icon::new(IconName::GitBranch)
                                .size(IconSize::Xs)
                                .color(fg.ink(Ink::Muted)),
                        ),
                )
            })
            .children(self.work_item_badge(session, cx))
            .when(session.automation_id.is_some(), |el| {
                el.child(
                    div()
                        .id(SharedString::from(format!("{group}-automation")))
                        .flex_none()
                        .tooltip(Tooltip::text("Started by an automation"))
                        .child(
                            Icon::new(IconName::Zap)
                                .size(IconSize::Xs)
                                .color(Status::Attention.color(fg)),
                        ),
                )
            })
            .children(self.orchestration_button(session, states, cx))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_none()
                    .min_w(px(18.0))
                    .h(px(18.0))
                    .items_center()
                    .justify_end()
                    .child(status)
                    .when(!busy, |el| el.child(archive)),
            );

        let agents = if expanded {
            self.orchestration_agents(session, states, cx)
        } else {
            None
        };
        let accent = colors.accent;
        let selection_bg = selection_strong(cx);
        let dashed = fg.opacity(if needs_approval || picked || active {
            0.30
        } else {
            0.25
        });
        let open_id = id.clone();
        let menu_id = id.clone();
        let (drop_hover_id, drop_id) = (id.clone(), id.clone());
        let drag = DraggedSession {
            session_id: id.clone(),
            title: title.clone(),
        };
        let card = div()
            .id(group.clone())
            .group(group.clone())
            .relative()
            // A block: its rows stack, and each is laid out once (as a
            // flex column every row is measured several times over).
            .w_full()
            .min_w_0()
            .px_2()
            // Opening the agents keeps the row; only the bottom grows.
            .when(expanded, |el| el.pb(px(10.0)))
            .rounded(px(6.0))
            .border_1()
            .border_color(gpui::transparent_black())
            // The navigator rows' look (`nav_row`): the thread in view on
            // the selection's strong fill, the others at 65% until hovered;
            // one waiting on the user stays in full.
            .when(!(active || picked || needs_approval || drop_target), |el| {
                el.opacity(UNSELECTED_OPACITY)
            })
            .map(|el| {
                if drop_target {
                    el
                } else if picked {
                    el.bg(accent.opacity(0.15))
                        .when(draft, |el| el.border_dashed().border_color(dashed))
                } else if needs_approval {
                    el.bg(fg.opacity(0.20)).border_dashed().border_color(dashed)
                } else if active {
                    el.bg(selection_bg)
                        .when(draft, |el| el.border_dashed().border_color(dashed))
                } else if draft {
                    el.border_dashed()
                        .border_color(dashed)
                        .hover(move |s| s.bg(fg.fill(Fill::Hover)))
                } else if expanded {
                    el.bg(fg.fill(Fill::Hover))
                        .hover(move |s| s.bg(fg.fill(Fill::Strong)))
                } else {
                    el.hover(move |s| s.bg(fg.fill(Fill::Hover)))
                }
            })
            .tooltip(Tooltip::text(tooltip))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.on_session_card_click(&open_id, event, window, cx)
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_session_menu(&menu_id, event.position, cx);
                }),
            )
            .on_drag(drag, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
            .on_drag_move::<DraggedSession>(cx.listener(
                move |this, event: &gpui::DragMoveEvent<DraggedSession>, _, cx| {
                    let target = ListDrop::Session(drop_hover_id.clone());
                    let inside = event.bounds.contains(&event.event.position);
                    let own = event.drag(cx).session_id == drop_hover_id;
                    this.set_list_drop(target, inside && !own, cx);
                },
            ))
            .on_drop(cx.listener(move |this, dragged: &DraggedSession, _, cx| {
                this.drop_on_list(&dragged.session_id, ListDrop::Session(drop_id.clone()), cx);
            }))
            .when(drop_target, |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(6.0))
                        .bg(accent.opacity(0.20)),
                )
            })
            // The thread in view also carries the accent's bar at its left
            // edge, so it reads apart from a hovered row at a glance.
            .when(active && !(picked || needs_approval || drop_target), |el| {
                // Settings' accent colour; Default is the text colour.
                let bar = crate::ui::appearance::user_accent(cx).map_or(fg, |a| a.color);
                el.child(
                    div()
                        .absolute()
                        .left(px(1.0))
                        .top(px(SELECTED_BAR_INSET))
                        .w(px(3.0))
                        .h(px(ROW_HEIGHT - 2.0 - 2.0 * SELECTED_BAR_INSET))
                        .rounded_full()
                        .bg(bar),
                )
            })
            .when(
                self.sessions_ui.reveal.as_deref() == Some(id.as_str()),
                |el| el.child(self.reveal_card(cx)),
            )
            .child(row)
            .children(agents);
        if !self.card_is_fresh(session) {
            return card.into_any_element();
        }
        // MonoCode `SessionListItem`: the card opens in place over 380ms,
        // pushing the rows below down, and fades in over 220ms.
        let ease = crate::ui::motion::cubic_bezier(0.32, 0.72, 0.0, 1.0);
        div()
            .overflow_hidden()
            .child(card)
            .with_animation(
                SharedString::from(format!("slide-in-{id}")),
                gpui::Animation::new(std::time::Duration::from_millis(380)),
                move |el, delta| {
                    let open = ease(delta);
                    let fade = (delta * 380.0 / 220.0).min(1.0);
                    // Uncapped once open, so a tall card is never clipped.
                    let el = if delta < 1.0 {
                        el.max_h(px(CARD_MAX_HEIGHT * open))
                    } else {
                        el
                    };
                    el.opacity(fade)
                },
            )
            .into_any_element()
    }

    /// The inline title field that replaces a card while renaming
    /// (MonoCode `SessionRenameRow`), in the row's place and type: amber
    /// while it needs approval, the selection's fill while open.
    fn render_session_rename_row(
        &self,
        active: bool,
        needs_approval: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let amber = Status::Attention.color(colors.fg);
        div()
            .flex()
            .w_full()
            .h(px(ROW_HEIGHT))
            .items_center()
            .px_1()
            .rounded(px(6.0))
            .when(needs_approval, |el| el.bg(amber.opacity(0.10)))
            .when(!needs_approval && active, |el| el.bg(selection_strong(cx)))
            // MonoCode's input: `rounded bg-content/10 px-2 py-1` with a
            // `ring-1 ring-accent/40`; the 1px border takes the ring's
            // place, so the padding gives up that pixel.
            .child(
                nav_label()
                    .flex_1()
                    .rounded(px(4.0))
                    .px(px(7.0))
                    .py(px(2.0))
                    .bg(colors.fg.fill(Fill::Strong))
                    .border_1()
                    .border_color(colors.accent.opacity(0.4))
                    .text_color(colors.fg)
                    .child(self.rename_input.clone()),
            )
            .into_any_element()
    }

    /// MonoCode `onSessionCardSelect`: ⇧ ranges, ⌘ toggles, a plain click
    /// clears the picks and opens the thread.
    fn on_session_card_click(
        &mut self,
        id: &str,
        event: &ClickEvent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let mods = event.modifiers();
        let add = mods.platform || mods.control;
        self.close_sidebar_menu(cx);
        if mods.shift {
            let order = self.sessions_ui.order.clone();
            let active = self.selected_session_id.clone();
            self.sessions_ui
                .selection
                .select_range(id, &order, active.as_deref(), add);
            window.focus(&self.session_list_focus, cx);
            self.redraw_regions(&[Region::Rail], cx);
            return;
        }
        if add {
            self.sessions_ui.selection.toggle(id);
            window.focus(&self.session_list_focus, cx);
            self.redraw_regions(&[Region::Rail], cx);
            return;
        }
        self.sessions_ui.selection.clear();
        self.open_session(id.to_string(), cx);
    }
}

impl BenCodeApp {
    /// Laid over the card the keyboard moved to: once placed, scrolls the
    /// thread list the least that shows the whole card, then lets go.
    fn reveal_card(&self, cx: &Context<Self>) -> impl IntoElement {
        let scroll = self.sessions_ui.scroll.clone();
        let app = cx.entity().downgrade();
        canvas(
            move |bounds, _, cx| {
                let view = scroll.bounds();
                let scroll_top = scale::logical(-scroll.offset().y);
                let top = scale::logical(bounds.top() - view.top()) + scroll_top;
                let item = (top, top + scale::logical(bounds.size.height));
                let to =
                    virtual_rows::reveal_top(item, scroll_top, scale::logical(view.size.height));
                if let Some(to) = to {
                    let offset = scroll.offset();
                    scroll.set_offset(gpui::point(offset.x, -px(to)));
                }
                let app = app.clone();
                cx.defer(move |cx| {
                    if let Err(err) = app.update(cx, |this, cx| {
                        this.sessions_ui.reveal = None;
                        // The list was laid out at the old offset.
                        if to.is_some() {
                            this.redraw_regions(&[Region::Rail], cx);
                        }
                    }) {
                        log::debug!("card revealed after app drop: {err:#}");
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }
}
