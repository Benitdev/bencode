//! Inbox: MonoCode's cross-source review queue. BenCode has no source
//! connections yet, so it shows MonoCode's empty state instead of made-up
//! items (`features/inbox/ui/InboxView.tsx:1001-1004,1378-1384`).

use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::primitives::IconName;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div};

use crate::app::{BenCodeApp, Surface};

impl BenCodeApp {
    pub fn open_inbox_modal(&mut self, cx: &mut Context<Self>) {
        self.show_surface(Surface::Inbox, cx);
    }

    pub(crate) fn render_inbox_body(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                EmptyState::new("inbox-empty", IconName::Inbox, "Select an inbox item")
                    .body("Add a connection to start using the Inbox."),
            )
            .into_any_element()
    }
}
