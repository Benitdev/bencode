//! MonoCode `SessionCard`: model row, title, git line and status, in the
//! states MonoCode draws (picked, needs approval, open, draft), dragged to
//! group or split, right-clicked for its menu.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnimationExt, AnyElement, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, SharedString, Styled, div, prelude::*, rgb,
};

use crate::app::BenCodeApp;
use crate::app::session_list::{self, LiveStates, NO_BRANCH_LABEL, format_relative, git_label};
use crate::db::SessionRow;
use crate::harness::catalog;
use crate::ui::drag_drop::DraggedSession;
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::sidebar_sessions::ListDrop;
use crate::ui::spinner::terminal_spinner;

/// The slide-in opens to this height (a usual card's); the cap comes off
/// when it finishes.
const CARD_MAX_HEIGHT: f32 = 160.0;

/// MonoCode's `text-[13px] leading-snug` card title.
const TITLE_LINE_HEIGHT: f32 = 13.0 * 1.375;

/// MonoCode's amber-400 and emerald-400 status colours.
const AMBER: u32 = 0xfbbf24;
const EMERALD: u32 = 0x34d399;

impl BenCodeApp {
    /// MonoCode's card status: approval, working, done, draft, else age.
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
                .text_size(px(11.0))
                .text_color(color)
        };
        let icon =
            |name: IconName, color: gpui::Hsla| Icon::new(name).size(IconSize::Xs).color(color);
        if states.approval.contains(&session.id) {
            let amber: gpui::Hsla = rgb(AMBER).into();
            return row(amber)
                .child(icon(IconName::CircleAlert, amber))
                .child(if session.orchestration.is_some() {
                    "Needs input"
                } else {
                    "Need approval"
                })
                .into_any_element();
        }
        if states.busy.contains(&session.id) {
            return row(colors.accent)
                .child(terminal_spinner(colors.accent, cx))
                .child("Working...")
                .into_any_element();
        }
        if states.done.contains(&session.id) {
            let green: gpui::Hsla = rgb(EMERALD).into();
            return row(green)
                .child(icon(IconName::Check, green))
                .child("Done")
                .into_any_element();
        }
        if draft {
            let muted = colors.fg.opacity(0.55);
            return row(muted)
                .child(icon(IconName::CircleDashed, muted))
                .child("Draft")
                .into_any_element();
        }
        row(colors.fg.opacity(0.45))
            .child(format_relative(session.updated_at, now))
            .into_any_element()
    }

    /// The card title; a title that just changed sweeps in as the old one
    /// drifts up and fades (MonoCode `ParticleText`, without particles).
    fn render_card_title(&self, id: &str, title: &str, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let text = |text: String| {
            div()
                .min_w_0()
                .truncate()
                .text_size(px(13.0))
                .line_height(px(TITLE_LINE_HEIGHT))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg)
                .child(text)
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

    /// MonoCode's card git line: `repo/branch`, or "No branch selected"
    /// once the thread's worktree is gone.
    fn session_git_label(&self, session: &SessionRow) -> String {
        if session.worktree_removed {
            return NO_BRANCH_LABEL.to_string();
        }
        let repo = self.workspace.repo.clone().or_else(|| {
            std::path::Path::new(&session.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        });
        // MonoCode falls back to the project's live branch.
        let live = (!self.git_status.branch.is_empty() && session.worktree_cwd.is_none())
            .then_some(self.git_status.branch.as_str());
        git_label(repo.as_deref(), session.branch.as_deref().or(live))
    }

    /// MonoCode `SessionCard`; `compact` (in a folder or group) drops the
    /// model row and puts the status after the title.
    pub(crate) fn render_session_card(
        &self,
        session: &SessionRow,
        compact: bool,
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
        // MonoCode: the linked-update dot leads the status.
        let status = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .children(self.linked_update_dot(session, cx))
            .child(self.session_status(session, draft, states, now, cx));
        // MonoCode `orchestrationExpanded`: an orchestrator lists its agents
        // while open, picked or working, and keeps its model row then.
        let expanded =
            session.orchestration.is_some() && (active || picked || states.busy.contains(&id));
        // The card's insets follow where it is listed; its layout follows
        // whether the model row shows.
        let in_group = compact;
        let compact = compact && !expanded;

        if self.sessions_ui.renaming_session.as_deref() == Some(id.as_str()) {
            return self.render_session_rename_row(active, needs_approval, cx);
        }

        let title_row = div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .when(!compact, |el| el.mt_1())
            .when(session.pinned, |el| {
                el.child(
                    Icon::new(IconName::Pin)
                        .size(IconSize::Xs)
                        .color(fg.opacity(0.45)),
                )
            })
            .child(self.render_card_title(&id, &title, cx));
        let (model_row, title_row) = if compact {
            (None, title_row.child(status))
        } else {
            let model = catalog::display_label(&session.harness, &session.model);
            let row = div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap_1p5()
                        .child(HarnessIcon::new(&session.harness).size(px(14.0)))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.0))
                                .text_color(fg.opacity(0.5))
                                .child(model),
                        ),
                )
                .child(status);
            (Some(row), title_row)
        };

        let git = self.session_git_label(session);
        let git_tip = match session.worktree_cwd.as_deref().filter(|w| !w.is_empty()) {
            Some(tree) if !git.is_empty() => format!("{git}\n{tree}"),
            _ => git.clone(),
        };
        let archive_label = if session.archived {
            "Unarchive"
        } else {
            "Archive"
        };
        let archive_id = id.clone();
        let bottom = div()
            .mt_1()
            .flex()
            .items_center()
            .gap_2()
            .map(|el| {
                if git.is_empty() {
                    el.child(div().flex_1().min_w_0())
                } else {
                    el.child(
                        div()
                            .id(SharedString::from(format!("{group}-git")))
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_1()
                            .text_size(px(11.0))
                            .text_color(fg.opacity(0.45))
                            .tooltip(Tooltip::text(git_tip))
                            .child(
                                div().flex_none().child(
                                    Icon::new(IconName::GitBranch)
                                        .size(IconSize::Xs)
                                        .color(fg.opacity(0.45)),
                                ),
                            )
                            .child(div().min_w_0().truncate().child(git)),
                    )
                }
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(1.0))
                    .child(
                        div()
                            .id(SharedString::from(format!("{group}-archive")))
                            .group(SharedString::from(format!("{group}-archive")))
                            .size(px(20.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .invisible()
                            .group_hover(group.clone(), |s| s.visible())
                            .cursor_pointer()
                            .hover(move |s| s.bg(fg.opacity(0.10)))
                            .tooltip(Tooltip::text(archive_label))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_archive_session(&archive_id, cx);
                            }))
                            .child(
                                Icon::new(IconName::Archive)
                                    .size(IconSize::Xs)
                                    .color(fg.opacity(0.5))
                                    .group_hover_color(format!("{group}-archive"), fg),
                            ),
                    )
                    .children(self.work_item_badge(session, cx))
                    .when(session.automation_id.is_some(), |el| {
                        el.child(
                            div()
                                .id(SharedString::from(format!("{group}-automation")))
                                .size(px(20.0))
                                .mr(px(-4.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .tooltip(Tooltip::text("Started by an automation"))
                                .child(
                                    Icon::new(IconName::Zap)
                                        .size(IconSize::Xs)
                                        .color(rgb(AMBER)),
                                ),
                        )
                    })
                    .children(self.orchestration_button(session, states, cx)),
            );

        let agents = if expanded {
            self.orchestration_agents(session, states, cx)
        } else {
            None
        };
        let accent = colors.accent;
        let selection_bg = colors.active;
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
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .px(px(10.0))
            // Opening the agents keeps the top inset; only the bottom grows.
            .pt(px(if in_group { 6.0 } else { 8.0 }))
            .pb(px(if expanded {
                10.0
            } else if in_group {
                6.0
            } else {
                8.0
            }))
            .rounded(px(6.0))
            .border_1()
            .border_color(gpui::transparent_black())
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
                        .hover(move |s| s.bg(fg.opacity(0.05)))
                } else if expanded {
                    el.bg(fg.opacity(0.05))
                        .hover(move |s| s.bg(fg.opacity(0.10)))
                } else {
                    el.hover(move |s| s.bg(fg.opacity(0.05)))
                }
            })
            .tooltip(Tooltip::text(title))
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
            .children(model_row)
            .child(title_row)
            .children(agents)
            .child(bottom);
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
    /// (MonoCode `SessionRenameRow`: `px-2.5 py-2`, amber while it needs
    /// approval, the selection fill while open).
    fn render_session_rename_row(
        &self,
        active: bool,
        needs_approval: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let amber: gpui::Hsla = rgb(AMBER).into();
        div()
            .w_full()
            .px(px(10.0))
            .py(px(8.0))
            .rounded(px(6.0))
            .when(needs_approval, |el| el.bg(amber.opacity(0.10)))
            .when(!needs_approval && active, |el| el.bg(colors.active))
            // MonoCode's input: `rounded bg-content/10 px-2 py-1` with a
            // `ring-1 ring-accent/40`; the 1px border takes the ring's
            // place, so the padding gives up that pixel.
            .child(
                div()
                    .rounded(px(4.0))
                    .px(px(7.0))
                    .py(px(3.0))
                    .bg(colors.fg.opacity(0.10))
                    .border_1()
                    .border_color(colors.accent.opacity(0.4))
                    .text_size(px(13.0))
                    .line_height(px(TITLE_LINE_HEIGHT))
                    .font_weight(FontWeight::SEMIBOLD)
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
            cx.notify();
            return;
        }
        if add {
            self.sessions_ui.selection.toggle(id);
            window.focus(&self.session_list_focus, cx);
            cx.notify();
            return;
        }
        self.sessions_ui.selection.clear();
        self.open_session(id.to_string(), cx);
    }
}
