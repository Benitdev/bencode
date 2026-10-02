//! Inbox: MonoCode's cross-source review queue. BenCode has no source
//! connections yet, so it shows MonoCode's empty state instead of made-up
//! items (`features/inbox/ui/InboxView.tsx:1001-1004,1378-1384`).

use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::overlays::Dialog;
use ely_gpui_component::primitives::IconName;
use gpui::{Context, IntoElement, ParentElement};

use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;

impl BenCodeApp {
    pub fn open_inbox_modal(&mut self, cx: &mut Context<Self>) {
        self.is_inbox_open = true;
        cx.notify();
    }

    pub fn close_inbox_modal(&mut self, cx: &mut Context<Self>) {
        self.is_inbox_open = false;
        cx.notify();
    }

    pub fn render_inbox_modal(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let close = app_callback(cx, |this, cx| this.close_inbox_modal(cx));
        Dialog::new("inbox", "Inbox", close).child(
            EmptyState::new("inbox-empty", IconName::Inbox, "Nothing in your inbox")
                .body("Add a connection to start using the Inbox."),
        )
    }
}
