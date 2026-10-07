//! Pinning and archiving threads (MonoCode `sessionStore`
//! `setSessionPinned`, `setSessionArchived`).

use gpui::Context;

use crate::app::BenCodeApp;

impl BenCodeApp {
    pub fn toggle_pin_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
            return;
        };
        let was_pinned = session.pinned;
        // Memory first: a queued save of this thread then carries the new value.
        session.pinned = !was_pinned;
        let id = id.to_string();
        self.db_write("toggle pin", move |db| db.toggle_pinned(&id, was_pinned));
        cx.notify();
    }

    /// Pins or unpins `id` (a no-op when it already is).
    pub fn set_session_pinned(&mut self, id: &str, pinned: bool, cx: &mut Context<Self>) {
        if self.sessions.iter().any(|s| s.id == id && s.pinned != pinned) {
            self.toggle_pin_session(id, cx);
        }
    }

    /// Archives or unarchives `id` (a no-op when it already is).
    pub fn set_session_archived(&mut self, id: &str, archived: bool, cx: &mut Context<Self>) {
        if self.sessions.iter().any(|s| s.id == id && s.archived != archived) {
            self.toggle_archive_session(id, cx);
        }
    }

    pub fn toggle_archive_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
            return;
        };
        let was_archived = session.archived;
        session.archived = !was_archived;
        let id = id.to_string();
        self.db_write("toggle archive", move |db| db.toggle_archived(&id, was_archived));
        cx.notify();
    }
}
