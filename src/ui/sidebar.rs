//! The navigator's threads (MonoCode's Sessions sidebar): the section's
//! header with its count, the working copy switcher and New thread, the
//! inline renames, and the thread-level dialogs. The list itself is
//! `sidebar_sessions.rs`, its cards `sidebar_cards.rs`.

use ely_gpui_component::primitives::{Icon, IconName, Tooltip};
use ely_gpui_component::theme::{ActiveTheme, IconSize};
use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    Window, div, prelude::*,
};

use crate::app::BenCodeApp;
use crate::ui::region::Region;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};

/// MonoCode's title bar and sidebar header are `h-10`.
pub const TITLEBAR_HEIGHT: f32 = 40.0;

/// A thread-level dialog opened from a card's context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionDialog {
    Delete(String),
    /// Every thread of a title-bar tab.
    DeleteMany(Vec<String>),
    /// The cards picked in the navigator.
    DeleteSelected(Vec<String>),
}

impl BenCodeApp {
    pub fn start_session_rename(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.id == id) else {
            return;
        };
        let title = crate::app::session_list::display_title(&session.title, &session.harness);
        self.sessions_ui.renaming_folder = None;
        self.sessions_ui.renaming_session = Some(id.to_string());
        self.begin_inline_rename(title, cx);
    }

    pub fn start_folder_rename(&mut self, folder_id: &str, cx: &mut Context<Self>) {
        let Some(name) = self
            .project_folders()
            .iter()
            .find(|f| f.id == folder_id)
            .map(|f| f.name.clone())
        else {
            return;
        };
        self.sessions_ui.renaming_session = None;
        self.sessions_ui.renaming_folder = Some(folder_id.to_string());
        self.begin_inline_rename(name, cx);
    }

    /// Fills the shared field, selects it and focuses it.
    fn begin_inline_rename(&mut self, text: String, cx: &mut Context<Self>) {
        let len = text.len();
        self.rename_input.update(cx, |input, cx| {
            input.set_text(text, cx);
            input.select(0..len, cx);
        });
        let handle = gpui::Focusable::focus_handle(self.rename_input.read(cx), cx);
        crate::ui::composer::focus_later(handle, cx);
        self.redraw_regions(&[Region::Rail], cx);
    }

    pub fn inline_rename_active(&self) -> bool {
        self.sessions_ui.renaming_session.is_some() || self.sessions_ui.renaming_folder.is_some()
    }

    pub fn cancel_inline_rename(&mut self, cx: &mut Context<Self>) {
        if self.sessions_ui.renaming_session.take().is_some()
            | self.sessions_ui.renaming_folder.take().is_some()
        {
            self.redraw_regions(&[Region::Rail], cx);
        }
    }

    /// Enter or blur: a non-blank name is kept, a blank one cancels
    /// (MonoCode `SessionRenameRow` / `FolderRenameRow`).
    pub fn commit_inline_rename(&mut self, cx: &mut Context<Self>) {
        let text = self.rename_input.read(cx).text().trim().to_string();
        let session = self.sessions_ui.renaming_session.take();
        let folder = self.sessions_ui.renaming_folder.take();
        if !text.is_empty() {
            if let Some(id) = session {
                self.rename_session(&id, &text, cx);
            } else if let Some(id) = folder {
                self.rename_folder(&id, &text, cx);
            }
        }
        self.redraw_regions(&[Region::Rail], cx);
    }

    pub fn rename_session(&mut self, id: &str, title: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
            return;
        };
        session.title = title.trim().to_string();
        self.persist_session(id);
        cx.notify();
    }

    /// The threads section's header, as the navigator's others ("Projects"
    /// sits right above, so the project is not named again): "Threads" and
    /// how many, the working copy switcher once the project has worktrees,
    /// and New thread.
    pub(crate) fn render_threads_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let has_worktrees = self
            .workspace(cx)
            .worktrees
            .iter()
            .any(|w| !w.missing && !w.is_main);
        let switcher = (has_worktrees || self.worktree_focus().is_some())
            .then(|| self.render_worktree_switcher(cx));
        let count = self.listed_sessions().len();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .px_3()
            .pt(px(8.0))
            .pb(px(4.0))
            .border_t_1()
            .border_color(fg.fill(Fill::Line))
            .child(
                div()
                    .flex_none()
                    .px_1()
                    .text_size(Text::Base.px())
                    .text_color(fg.ink(Ink::Secondary))
                    .child("Threads"),
            )
            .when(count > 0, |el| {
                // A pill beside the title, not loose text on its baseline.
                el.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .h(px(16.0))
                        .min_w(px(16.0))
                        .px(px(5.0))
                        .rounded_full()
                        .bg(fg.fill(Fill::Selected))
                        .text_size(Text::Xs.px())
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .line_height(px(16.0))
                        .text_color(fg.ink(Ink::Secondary))
                        .child(count.to_string()),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .justify_end()
                    .children(switcher),
            )
            .child(section_icon_button(
                "threads-new-thread",
                IconName::Plus,
                "New thread (⌘T)",
                cx,
                cx.listener(|this, _, _, cx| this.create_new_session(cx)),
            ))
    }

    /// MonoCode `SidebarWorktreeSwitcher` as a chip: the focused worktree's
    /// branch, else the project folder's, over the working copies.
    fn render_worktree_switcher(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        let focus = self.worktree_focus();
        let main_branch = || {
            self.workspace(cx)
                .worktrees
                .iter()
                .find(|w| w.is_main)
                .and_then(|w| w.branch.clone())
        };
        let title = match focus {
            Some(f) => f
                .branch
                .clone()
                .unwrap_or_else(|| "Detached worktree".to_string()),
            None => main_branch().unwrap_or_else(|| "Project folder".to_string()),
        };
        let tip = match focus {
            Some(f) => format!("{}\n{}", f.branch.as_deref().unwrap_or("detached"), f.path),
            None => format!(
                "{}: the project folder",
                main_branch().unwrap_or_else(|| "No branch".into())
            ),
        };
        let open = self.sidebar_menu_is_worktrees();
        div()
            .id("worktree-switcher")
            .flex()
            .min_w_0()
            .max_w(px(140.0))
            .items_center()
            .gap_1()
            .h(px(20.0))
            .px(px(6.0))
            .rounded(px(6.0))
            .text_size(Text::Sm.px())
            .text_color(if focus.is_some() {
                fg
            } else {
                fg.ink(Ink::Secondary)
            })
            .when(focus.is_some(), |el| el.bg(fg.fill(Fill::Hover)))
            .when(open, |el| el.bg(fg.fill(Fill::Selected)))
            .hover(move |s| s.bg(fg.fill(Fill::Selected)).text_color(fg))
            .tooltip(Tooltip::text(tip))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    this.sessions_ui.filter_button_hit = true;
                    this.toggle_worktree_menu(event.position, cx);
                }),
            )
            .child(
                Icon::new(IconName::GitBranch)
                    .size(IconSize::Xs)
                    .color(fg.ink(Ink::Muted)),
            )
            .child(div().min_w_0().truncate().child(title))
            .child(
                Icon::new(IconName::ChevronsUpDown)
                    .size(IconSize::Xs)
                    .color(fg.ink(Ink::Muted)),
            )
    }
}

/// A section header's button, as the navigator's "Projects" one: `size-5
/// rounded-md`, a `size-3.5` glyph at half strength that lights up on hover.
fn section_icon_button(
    id: &'static str,
    icon: IconName,
    tip: &'static str,
    cx: &Context<BenCodeApp>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let fg = cx.theme().colors.fg;
    let group = SharedString::from(id);
    div()
        .id(id)
        .group(group.clone())
        .flex()
        .flex_none()
        .size_5()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .hover(move |s| s.bg(fg.fill(Fill::Selected)))
        .tooltip(Tooltip::text(tip))
        .on_click(on_click)
        .child(
            Icon::new(icon)
                .size(IconSize::Sm)
                .color(fg.ink(Ink::Secondary))
                .group_hover_color(group, fg),
        )
}
