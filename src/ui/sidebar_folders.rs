//! MonoCode's sidebar folders (`Sidebar.tsx` `FolderRow`): a tinted shell
//! with the folder's row (icon, name, count) over its threads, and the
//! same shell for the Pinned group. A click folds it, F2 or its menu
//! renames it inline, and cards dropped on it join it.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, SharedString, Styled, div, prelude::*, px, rgb,
};

use crate::app::BenCodeApp;
use crate::app::session_folders::{FolderTarget, SessionFolder};
use crate::app::session_list::LiveStates;
use crate::db::SessionRow;
use crate::ui::drag_drop::DraggedSession;
use crate::ui::sidebar_sessions::ListDrop;
use crate::ui::spinner::terminal_spinner;

/// MonoCode's `text-[13px] font-semibold leading-snug` row label.
const NAME_LINE_HEIGHT: f32 = 13.0 * 1.375;

/// MonoCode `REMINDERS_COLOR`.
const REMINDERS_COLOR: u32 = 0xf59e0b;

/// The two built-in groups that share the folder shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionGroup {
    Pinned,
    Reminders,
}

/// What the folder row shows of its threads while folded (MonoCode's
/// approval, busy and done aggregate).
#[derive(Clone, Copy, Default)]
struct FolderState {
    approval: bool,
    busy: bool,
    done: bool,
}

fn folder_state(sessions: &[&SessionRow], states: &LiveStates) -> FolderState {
    FolderState {
        approval: sessions.iter().any(|s| states.approval.contains(&s.id)),
        busy: sessions.iter().any(|s| states.busy.contains(&s.id)),
        done: sessions.iter().any(|s| states.done.contains(&s.id)),
    }
}

/// The row's leading glyph: the group icon, swapped for a chevron on hover
/// (and while a plain folder is open). MonoCode: `size-4` slot, `size-3.5`
/// glyphs.
fn folder_glyph(group: &SharedString, icon: AnyElement, open: bool, keep_icon: bool, fg: Hsla) -> AnyElement {
    let chevron = Icon::new(if open {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    })
    .size(IconSize::Sm)
    .color(fg);
    let show_icon = !open || keep_icon;
    div()
        .relative()
        .size(px(16.0))
        .flex_none()
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .when(!show_icon, |el| el.invisible())
                .group_hover(group.clone(), |s| s.invisible())
                .child(icon),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .when(show_icon, |el| el.invisible())
                .group_hover(group.clone(), |s| s.visible())
                .child(chevron),
        )
        .into_any_element()
}

impl BenCodeApp {
    /// The count, with the folded aggregate status before it.
    fn folder_count(
        &self,
        key: &str,
        count: usize,
        open: bool,
        state: FolderState,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = &cx.theme().colors;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .text_size(px(11.0))
            .text_color(colors.fg.opacity(0.45))
            .map(|el| {
                if open {
                    el
                } else if state.approval {
                    el.child(
                        Icon::new(IconName::CircleAlert)
                            .size(IconSize::Xs)
                            .color(rgb(0xfbbf24)),
                    )
                } else if state.busy {
                    el.child(terminal_spinner(
                        SharedString::from(format!("folder-spin-{key}")),
                        colors.accent,
                    ))
                } else if state.done {
                    el.child(
                        Icon::new(IconName::Check)
                            .size(IconSize::Xs)
                            .color(rgb(0x34d399)),
                    )
                } else {
                    el
                }
            })
            .child(count.to_string())
    }

    /// A folder and, unless folded, its threads (`cards`) and its
    /// "New session" button.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_session_folder(
        &self,
        folder: &SessionFolder,
        sessions: &[&SessionRow],
        cards: Option<Vec<AnyElement>>,
        before_loose: bool,
        searching: bool,
        states: &LiveStates,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let accent = colors.accent;
        let open = cards.is_some();
        let state = folder_state(sessions, states);
        let drop_target = self.sessions_ui.drop == Some(ListDrop::Folder(folder.id.clone()));
        let header = if self.sessions_ui.renaming_folder.as_deref() == Some(folder.id.as_str()) {
            self.render_folder_rename_row(sessions.len(), cx)
        } else {
            let group = SharedString::from(format!("folder-{}", folder.id));
            let (toggle_id, menu_id) = (folder.id.clone(), folder.id.clone());
            // MonoCode's folder glyph is `text-content` whatever the tint.
            let icon = Icon::new(IconName::Folder)
                .size(IconSize::Sm)
                .color(fg)
                .into_any_element();
            div()
                .id(SharedString::from(format!("folder-row-{}", folder.id)))
                .group(group.clone())
                .relative()
                .flex()
                .items_center()
                .gap_1p5()
                .h(px(32.0))
                .px_2()
                .when(open, |el| el.rounded(px(6.0)))
                .hover(move |s| s.bg(fg.opacity(0.10)))
                .tooltip(Tooltip::text(folder.name.clone()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !searching {
                        this.toggle_folder(&toggle_id, cx);
                    }
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_folder_menu(&menu_id, event.position, cx);
                    }),
                )
                .when(drop_target, |el| {
                    el.child(div().absolute().inset_0().rounded(px(6.0)).bg(accent.opacity(0.20)))
                })
                .child(folder_glyph(&group, icon, open, false, fg))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.0))
                        .line_height(px(NAME_LINE_HEIGHT))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .child(SharedString::from(folder.name.clone())),
                )
                .child(self.folder_count(&folder.id, sessions.len(), open, state, cx))
                .into_any_element()
        };
        let (hover_id, drop_id, new_in) = (folder.id.clone(), folder.id.clone(), folder.id.clone());
        let new_group = SharedString::from(format!("folder-new-{new_in}"));
        div()
            .id(SharedString::from(format!("folder-shell-{}", folder.id)))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(6.0))
            // MonoCode `folderShellFill`: the tint at 18%.
            .bg(folder.accent().map_or(fg.opacity(0.05), |accent| accent.opacity(0.18)))
            .when(open || before_loose, |el| el.mb(px(6.0)))
            .on_drag_move::<DraggedSession>(cx.listener(
                move |this, event: &gpui::DragMoveEvent<DraggedSession>, _, cx| {
                    let inside = event.bounds.contains(&event.event.position);
                    this.set_list_drop(ListDrop::Folder(hover_id.clone()), inside, cx);
                },
            ))
            .on_drop(cx.listener(move |this, dragged: &DraggedSession, _, cx| {
                this.drop_on_list(&dragged.session_id, ListDrop::Folder(drop_id.clone()), cx);
            }))
            .child(header)
            .when_some(cards, |el, cards| {
                el.child(div().flex().flex_col().gap(px(1.0)).p_1().children(cards))
                    .child(
                        div()
                            .border_t_1()
                            .border_color(fg.opacity(crate::ui::sidebar::STROKE_OPACITY))
                            .p_1()
                            // MonoCode: `border border-transparent px-2.5
                            // py-1.5 rounded-md`, `content/45` until hovered.
                            .child(
                                div()
                                    .id(new_group.clone())
                                    .group(new_group.clone())
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px(px(10.0))
                                    .py(px(6.0))
                                    .rounded(px(6.0))
                                    .border_1()
                                    .border_color(gpui::transparent_black())
                                    .text_color(fg.opacity(0.45))
                                    .hover(move |s| s.bg(fg.opacity(0.10)).text_color(fg))
                                    .tooltip(Tooltip::text("New session"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.new_session_in_folder(&new_in, cx)
                                    }))
                                    .child(
                                        Icon::new(IconName::Plus)
                                            .size(IconSize::Xs)
                                            .color(fg.opacity(0.45))
                                            .group_hover_color(new_group, fg),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .line_height(px(NAME_LINE_HEIGHT))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("New session"),
                                    ),
                            ),
                    )
            })
            .into_any_element()
    }

    /// MonoCode's "Pinned" and "Reminders" shells: their threads as
    /// compact cards; Reminders wears MonoCode's amber.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_session_group(
        &self,
        kind: SessionGroup,
        sessions: &[&SessionRow],
        cards: Option<Vec<AnyElement>>,
        before_loose: bool,
        searching: bool,
        states: &LiveStates,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let open = cards.is_some();
        let (key, name, icon, tint) = match kind {
            SessionGroup::Pinned => ("pinned-sessions", "Pinned", IconName::Pin, None),
            SessionGroup::Reminders => (
                "reminder-sessions",
                "Reminders",
                IconName::Clock,
                Some(gpui::Hsla::from(rgb(REMINDERS_COLOR))),
            ),
        };
        let group = SharedString::from(key);
        // MonoCode: Pin is `text-content`; Clock takes the group's tint.
        let icon = Icon::new(icon)
            .size(IconSize::Sm)
            .color(tint.unwrap_or(fg))
            .into_any_element();
        let header = div()
            .id(SharedString::from(format!("{key}-row")))
            .group(group.clone())
            .flex()
            .items_center()
            .gap_1p5()
            .h(px(32.0))
            .px_2()
            .when(open, |el| el.rounded(px(6.0)))
            .hover(move |s| s.bg(fg.opacity(0.10)))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !searching {
                    match kind {
                        SessionGroup::Pinned => this.toggle_pinned_group(cx),
                        SessionGroup::Reminders => this.toggle_reminders_group(cx),
                    }
                }
            }))
            .child(folder_glyph(&group, icon, open, true, fg))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.0))
                    .line_height(px(NAME_LINE_HEIGHT))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg)
                    .child(name),
            )
            .child(self.folder_count(key, sessions.len(), open, folder_state(sessions, states), cx));
        div()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(6.0))
            .bg(tint.map_or(fg.opacity(0.05), |tint| tint.opacity(0.18)))
            .when(open || before_loose, |el| el.mb(px(6.0)))
            .child(header)
            .when_some(cards, |el, cards| {
                el.child(div().flex().flex_col().gap(px(1.0)).p_1().children(cards))
            })
            .into_any_element()
    }

    /// MonoCode `FolderRenameRow`: the name field, the count kept.
    fn render_folder_rename_row(&self, count: usize, cx: &Context<Self>) -> AnyElement {
        let colors = &cx.theme().colors;
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .py(px(6.0))
            .child(
                div().size(px(16.0)).flex().flex_none().items_center().justify_center().child(
                    Icon::new(IconName::ChevronDown)
                        .size(IconSize::Sm)
                        .color(colors.fg.opacity(0.5)),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    // MonoCode `px-2 py-0.5` with a `ring-1`; the 1px
                    // border stands in for the ring, so the padding gives
                    // up that pixel.
                    .rounded(px(4.0))
                    .px(px(7.0))
                    .py(px(1.0))
                    .bg(colors.fg.opacity(0.10))
                    .border_1()
                    .border_color(colors.accent.opacity(0.4))
                    .text_size(px(13.0))
                    .line_height(px(NAME_LINE_HEIGHT))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors.fg)
                    .child(self.rename_input.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.0))
                    .text_color(colors.fg.opacity(0.45))
                    .child(count.to_string()),
            )
            .into_any_element()
    }

    /// MonoCode `onNewInFolder`: a new thread, filed in the folder.
    fn new_session_in_folder(&mut self, folder_id: &str, cx: &mut Context<Self>) {
        let id = self.create_new_session_id(cx);
        self.search_input.update(cx, |input, cx| input.set_text("", cx));
        self.search_query.clear();
        self.place_session_in_folder(&id, &FolderTarget::Existing(folder_id.to_string()), cx);
    }
}
