//! Left column: mode switcher, thread search and filters, and the thread list.

use ely_gpui_component::buttons::{ButtonVariant, IconButton, SegmentedControl};
use ely_gpui_component::chat::{Conversation, ConversationList};
use ely_gpui_component::forms::SearchInput;
use ely_gpui_component::layout::Sidebar;
use ely_gpui_component::overlays::{ConfirmDialog, PromptDialog};
use ely_gpui_component::primitives::IconName;
use ely_gpui_component::theme::{ActiveTheme, ControlSize, TextSize};
use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement, SharedString, Styled, Window,
    div,
};
use jiff::Timestamp;

use crate::app::{BenCodeApp, FilterMode, SidebarMode};
use crate::db::SessionRow;

/// A thread-level dialog opened from the list's row menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionDialog {
    Rename(String),
    Delete(String),
}

const FILTERS: [(FilterMode, &str); 4] = [
    (FilterMode::All, "All"),
    (FilterMode::Active, "Active"),
    (FilterMode::Pinned, "Pinned"),
    (FilterMode::Archived, "Archived"),
];

fn filter_key(mode: FilterMode) -> &'static str {
    FILTERS.iter().find(|(m, _)| *m == mode).map_or("All", |(_, key)| key)
}

fn keeps(mode: FilterMode, session: &SessionRow) -> bool {
    match mode {
        FilterMode::All => true,
        FilterMode::Active => !session.archived,
        FilterMode::Pinned => session.pinned,
        FilterMode::Archived => session.archived,
    }
}

fn conversation(session: &SessionRow) -> Conversation {
    let title = if session.title.trim().is_empty() { "Untitled thread" } else { session.title.as_str() };
    Conversation {
        key: session.id.clone().into(),
        title: title.to_string().into(),
        at: Timestamp::from_millisecond(session.updated_at).unwrap_or(Timestamp::UNIX_EPOCH),
        pinned: session.pinned,
    }
}

fn sidebar_mode_key(mode: SidebarMode) -> &'static str {
    match mode {
        SidebarMode::Sessions => "sessions",
        SidebarMode::Files => "files",
        SidebarMode::Changes => "changes",
    }
}

impl BenCodeApp {
    /// Sessions | Files | Changes switcher shared by every sidebar panel.
    pub fn render_sidebar_mode_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        let changes = self.git_status.staged.len() + self.git_status.unstaged.len();
        let changes_label = if changes > 0 { format!("Changes {changes}") } else { "Changes".into() };
        div().p_2().child(
            SegmentedControl::new("sidebar-mode", sidebar_mode_key(self.sidebar_mode))
                .size(ControlSize::Sm)
                .segment("sessions", "Threads", Some(IconName::MessageSquare))
                .segment("files", "Files", Some(IconName::Folder))
                .segment("changes", changes_label, Some(IconName::GitBranch))
                .on_change(cx.listener(|this, key: &SharedString, _, cx| {
                    this.sidebar_mode = match key.as_ref() {
                        "files" => SidebarMode::Files,
                        "changes" => SidebarMode::Changes,
                        _ => SidebarMode::Sessions,
                    };
                    if this.sidebar_mode == SidebarMode::Changes {
                        this.refresh_workspace(cx);
                    }
                    cx.notify();
                })),
        )
    }

    pub fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let filter = self.filter_mode;
        let conversations: Vec<Conversation> =
            self.sessions.iter().filter(|s| keeps(filter, s)).map(conversation).collect();
        let today = jiff::Zoned::now().date();

        Sidebar::new("threads-sidebar", false)
            .child(self.render_sidebar_mode_tabs(cx))
            .child(div().px_2().child(SearchInput::new("thread-search", &self.search_input).size(ControlSize::Sm)))
            .child(self.render_filter_row(cx))
            .child(
                div().flex_1().min_h_0().child(
                    ConversationList::new("thread-list", conversations, today)
                        .query(self.search_query.clone())
                        .active(
                            self.selected_session_id.clone().map(SharedString::from),
                            cx.listener(|this, key: &SharedString, _, cx| this.select_session(key.to_string(), cx)),
                        )
                        .actions(
                            cx.listener(|this, key: &SharedString, window, cx| this.open_rename(key, window, cx)),
                            cx.listener(|this, key: &SharedString, _, cx| this.toggle_pin_session(key, cx)),
                            cx.listener(|this, key: &SharedString, _, cx| {
                                this.session_dialog = Some(SessionDialog::Delete(key.to_string()));
                                cx.notify();
                            }),
                        ),
                ),
            )
    }

    fn render_filter_row(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let filters = FILTERS.iter().fold(
            SegmentedControl::new("thread-filter", filter_key(self.filter_mode)).size(ControlSize::Sm),
            |control, (_, key)| control.segment(*key, *key, None),
        );
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .child(filters.on_change(cx.listener(|this, key: &SharedString, _, cx| {
                this.filter_mode = FILTERS.iter().find(|(_, k)| *k == key.as_ref()).map_or(FilterMode::All, |(m, _)| *m);
                cx.notify();
            })))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(theme.text_size(TextSize::Xs))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.colors.fg_subtle)
                    .child("THREADS")
                    .child(
                        IconButton::new("new-thread", IconName::Plus)
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .tooltip("New thread")
                            .on_click(cx.listener(|this, _, _, cx| this.create_new_session(cx))),
                    ),
            )
    }

    fn open_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.sessions.iter().find(|s| s.id == id).map(|s| s.title.clone()).unwrap_or_default();
        self.rename_input.update(cx, |input, cx| input.set_text(title, cx));
        self.session_dialog = Some(SessionDialog::Rename(id.to_string()));
        window.refresh();
        cx.notify();
    }

    fn rename_session(&mut self, id: &str, title: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else { return };
        session.title = title.trim().to_string();
        self.persist_session(id);
        cx.notify();
    }

    /// The rename or delete dialog, when one is open.
    pub fn render_session_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let dialog = self.session_dialog.clone()?;
        let close = cx.listener(|this, _: &(), _, cx| {
            this.session_dialog = None;
            cx.notify();
        });
        let close = move |window: &mut Window, cx: &mut gpui::App| close(&(), window, cx);
        Some(match dialog {
            SessionDialog::Rename(id) => PromptDialog::new("rename-thread", "Rename thread", &self.rename_input, close)
                .label("Title")
                .submit("Rename")
                .check(|text| if text.trim().is_empty() { Err("A title is required".into()) } else { Ok(()) })
                .on_submit({
                    let submit = cx.listener(move |this, title: &str, _, cx| this.rename_session(&id, title, cx));
                    move |title, window, cx| submit(title, window, cx)
                })
                .into_any_element(),
            SessionDialog::Delete(id) => {
                let title = self.sessions.iter().find(|s| s.id == id).map_or("this thread".into(), |s| s.title.clone());
                let delete = cx.listener(move |this, _: &(), _, cx| this.delete_session(&id, cx));
                ConfirmDialog::new("delete-thread", "Delete thread?", format!("“{title}” and its transcript will be removed."), close)
                    .confirm("Delete")
                    .destructive()
                    .on_confirm(move |window, cx| delete(&(), window, cx))
                    .into_any_element()
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_select_the_right_sessions() {
        let pinned = SessionRow { pinned: true, ..Default::default() };
        let archived = SessionRow { archived: true, ..Default::default() };
        assert!(keeps(FilterMode::Pinned, &pinned) && !keeps(FilterMode::Pinned, &archived));
        assert!(keeps(FilterMode::Archived, &archived) && !keeps(FilterMode::Active, &archived));
        assert!(FILTERS.iter().all(|(mode, key)| filter_key(*mode) == *key));
    }

    #[test]
    fn conversation_uses_placeholder_title_and_millis() {
        let row = SessionRow { id: "s".into(), updated_at: 1_000, ..Default::default() };
        let c = conversation(&row);
        assert_eq!(c.title.as_ref(), "Untitled thread");
        assert_eq!(c.at.as_millisecond(), 1_000);
    }
}
