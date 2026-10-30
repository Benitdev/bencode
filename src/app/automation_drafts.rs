//! Automation drafts that outlive the editor: leaving an automation with
//! unsaved edits keeps them (one draft per automation, the new one under
//! the empty id) until Save, or a confirmed Reset, Cancel or Discard. Also
//! starting new automations (blank, a template, Duplicate) without losing
//! a new one already begun. BenCode's own: MonoCode drops edits on leaving.

use gpui::Context;
use serde_json::Value;

use super::automations::{AutomationPage, editable, settings_differ};
use super::{BenCodeApp, unique_id};
use crate::db::AutomationRow;
use crate::schedule;

/// The stash key of the automation not yet saved (its `id` is empty).
pub const NEW_DRAFT: &str = "";

/// Edits waiting on "Discard changes?".
#[derive(Clone, Debug)]
pub enum Discard {
    /// Reset (a stored automation) or Cancel (a new one) in the editor.
    Editor,
    /// Discard on the New automation page's "Continue editing" row.
    StashedNew,
    /// Starting this new automation would drop the unsaved new one.
    ReplaceNew(Box<AutomationRow>),
}

/// `auto` copied as a new, unsaved automation: "<name> copy", fresh
/// trigger ids, no run history, and paused, so the copy never runs beside
/// the original until it is turned on.
pub fn duplicate(auto: &AutomationRow) -> AutomationRow {
    let triggers = schedule::triggers_of(auto)
        .into_iter()
        .map(|mut trigger| {
            trigger.insert("id".into(), Value::from(unique_id("trigger")));
            trigger
        })
        .collect();
    let mut copy = AutomationRow {
        id: String::new(),
        name: format!("{} copy", auto.name.trim()),
        enabled: false,
        next_run_at: 0,
        last_run_at: None,
        last_run_status: None,
        last_session_id: None,
        skipped_run_at: None,
        created_at: 0,
        updated_at: 0,
        ..auto.clone()
    };
    copy.extra.remove("lastRunError");
    schedule::apply_triggers(&mut copy, triggers);
    copy
}

impl BenCodeApp {
    /// The editor's automation with the name exactly as typed, to stash.
    fn typed_draft(&self, cx: &gpui::App) -> Option<AutomationRow> {
        let mut draft = self.automations.draft.clone()?;
        draft.name = self.automations.name_input.read(cx).text().to_string();
        draft.prompt = self.automations.prompt_input.read(cx).text().to_string();
        Some(draft)
    }

    /// Whether `draft` differs from what it was opened from: its stored
    /// row, or for a new automation the blank, template or copy it began as.
    pub(crate) fn draft_changed(&self, draft: &AutomationRow) -> bool {
        if draft.id != NEW_DRAFT {
            return self
                .automations
                .items
                .iter()
                .find(|a| a.id == draft.id)
                .is_some_and(|stored| settings_differ(draft, stored));
        }
        self.automations
            .new_origin
            .as_ref()
            .is_none_or(|origin| settings_differ(draft, origin))
    }

    /// Whether automation `id` (the empty id: the new one) has edits
    /// kept for later.
    pub(crate) fn automation_stashed(&self, id: &str) -> bool {
        self.automations.stashed.contains_key(id)
    }

    /// Any automation edit not saved: the editor's, or one left behind.
    pub(crate) fn automation_edits_unsaved(&self, cx: &gpui::App) -> bool {
        self.automation_dirty(cx)
            || self
                .automations
                .stashed
                .values()
                .any(|draft| self.draft_changed(draft))
    }

    /// Keeps the editor's edits for when its automation is opened again,
    /// or forgets an earlier stash when there are none.
    fn stash_editor(&mut self, cx: &gpui::App) {
        let Some(draft) = self.typed_draft(cx) else {
            return;
        };
        let changed = self
            .editor_draft(cx)
            .is_some_and(|d| self.draft_changed(&d));
        if changed {
            self.automations.stashed.insert(draft.id.clone(), draft);
        } else {
            self.automations.stashed.remove(&draft.id);
            if draft.id == NEW_DRAFT {
                self.automations.new_origin = None;
            }
        }
    }

    /// MonoCode `beginCreate`: back to the New automation page; the
    /// editor's edits are kept.
    pub(crate) fn show_automation_picker(&mut self, cx: &mut Context<Self>) {
        self.leave_automation_editor(AutomationPage::New, cx);
    }

    /// Shows `page` instead of the editor, keeping its edits.
    pub(super) fn leave_automation_editor(&mut self, page: AutomationPage, cx: &mut Context<Self>) {
        self.stash_editor(cx);
        self.close_automation_editor(page, cx);
    }

    /// Shows `page` instead of the editor, dropping the editor's draft.
    pub(super) fn close_automation_editor(&mut self, page: AutomationPage, cx: &mut Context<Self>) {
        self.automations.page = page;
        self.automations.draft = None;
        cx.notify();
    }

    /// The editor shows automation `id` (the empty id: the new one).
    pub(crate) fn editing_automation(&self, id: &str) -> bool {
        self.automations.page == AutomationPage::Editor
            && self.automations.draft.as_ref().is_some_and(|d| d.id == id)
    }

    /// Opens stored automation `id`, with the edits left on it if any.
    pub(crate) fn select_automation(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.editing_automation(id) {
            return;
        }
        let Some(stored) = self.automations.items.iter().find(|a| a.id == id) else {
            return;
        };
        let fresh = editable(stored);
        self.stash_editor(cx);
        let draft = self
            .automations
            .stashed
            .remove(id)
            .filter(|draft| self.draft_changed(draft))
            .unwrap_or(fresh);
        self.show_stored_draft(draft, cx);
    }

    /// Opens stored automation `id` as stored, dropping its edits.
    pub(super) fn reopen_automation(&mut self, id: &str, cx: &mut Context<Self>) {
        self.automations.stashed.remove(id);
        if let Some(stored) = self.automations.items.iter().find(|a| a.id == id) {
            let draft = editable(stored);
            self.show_stored_draft(draft, cx);
        }
    }

    fn show_stored_draft(&mut self, draft: AutomationRow, cx: &mut Context<Self>) {
        let id = draft.id.clone();
        self.automations.selected_id = Some(id.clone());
        self.automations.error = None;
        self.automations.runs.clear();
        self.load_automation_runs(&id, cx);
        self.open_automation_editor(draft, cx);
    }

    /// Starts a new automation from `draft`, asking first when that would
    /// drop a new one with unsaved edits.
    pub(crate) fn begin_new_automation(&mut self, draft: AutomationRow, cx: &mut Context<Self>) {
        let editing_new = self.editing_automation(NEW_DRAFT);
        let unsaved =
            self.automation_stashed(NEW_DRAFT) || (editing_new && self.automation_dirty(cx));
        if unsaved {
            self.automations.pending_discard = Some(Discard::ReplaceNew(Box::new(draft)));
            cx.notify();
            return;
        }
        self.open_new_automation(draft, cx);
    }

    fn open_new_automation(&mut self, draft: AutomationRow, cx: &mut Context<Self>) {
        self.stash_editor(cx);
        // The new automation it replaces, already confirmed or unchanged.
        self.automations.stashed.remove(NEW_DRAFT);
        self.automations.new_origin = Some(draft.clone());
        self.open_automation_editor(draft, cx);
    }

    /// The New automation page's "Continue editing".
    pub(crate) fn resume_new_automation(&mut self, cx: &mut Context<Self>) {
        if let Some(draft) = self.automations.stashed.remove(NEW_DRAFT) {
            self.open_automation_editor(draft, cx);
        }
    }

    /// Duplicate: the automation on screen, as a new paused one.
    pub(crate) fn duplicate_automation(&mut self, cx: &mut Context<Self>) {
        if let Some(draft) = self.editor_draft(cx) {
            self.begin_new_automation(duplicate(&draft), cx);
        }
    }

    /// Reset (a stored automation) or Cancel (a new one): asks first when
    /// there are edits to lose.
    pub(crate) fn discard_automation_edits(&mut self, cx: &mut Context<Self>) {
        if self.automation_dirty(cx) {
            self.automations.pending_discard = Some(Discard::Editor);
            cx.notify();
        } else {
            self.discard_now(Discard::Editor, cx);
        }
    }

    /// The New automation page's "Discard": asks first.
    pub(crate) fn discard_stashed_new(&mut self, cx: &mut Context<Self>) {
        self.automations.pending_discard = Some(Discard::StashedNew);
        cx.notify();
    }

    /// "Discard changes?" confirmed.
    pub(crate) fn confirm_discard(&mut self, cx: &mut Context<Self>) {
        if let Some(discard) = self.automations.pending_discard.take() {
            self.discard_now(discard, cx);
        }
    }

    fn discard_now(&mut self, discard: Discard, cx: &mut Context<Self>) {
        match discard {
            Discard::Editor => match self.edited_automation().map(|a| a.id.clone()) {
                Some(id) => self.reopen_automation(&id, cx),
                None => {
                    self.forget_new_automation();
                    self.close_automation_editor(AutomationPage::New, cx);
                }
            },
            Discard::StashedNew => {
                self.forget_new_automation();
                cx.notify();
            }
            Discard::ReplaceNew(draft) => {
                if self
                    .automations
                    .draft
                    .as_ref()
                    .is_some_and(|d| d.id == NEW_DRAFT)
                {
                    self.automations.draft = None;
                }
                self.open_new_automation(*draft, cx);
            }
        }
    }

    fn forget_new_automation(&mut self) {
        self.automations.stashed.remove(NEW_DRAFT);
        self.automations.new_origin = None;
    }

    /// After a list load: edits of automations gone from it are dropped,
    /// and the editor closes when its automation was deleted.
    pub(super) fn drop_vanished_drafts(&mut self, cx: &mut Context<Self>) {
        let items = &self.automations.items;
        self.automations
            .stashed
            .retain(|id, _| id == NEW_DRAFT || items.iter().any(|a| &a.id == id));
        let editing = self.automations.draft.as_ref().map(|d| d.id.clone());
        if let Some(id) = editing.filter(|id| id != NEW_DRAFT)
            && !self.automations.items.iter().any(|a| a.id == id)
        {
            self.close_automation_editor(AutomationPage::Overview, cx);
        }
    }

    /// A save of automation `id` (`was_new`: created from the new draft)
    /// landed: its edits are stored, and the editor shows it if it still
    /// shows that automation. Edits made while the save was on its way
    /// stay, in the editor or in the stash.
    pub(super) fn automation_saved(&mut self, id: &str, was_new: bool, cx: &mut Context<Self>) {
        let shown = self.editing_automation(if was_new { NEW_DRAFT } else { id });
        if was_new {
            self.forget_new_automation();
        }
        let stored = self.automations.items.iter().find(|a| a.id == id);
        let matches = |draft: &AutomationRow| stored.is_some_and(|s| !settings_differ(draft, s));
        if shown {
            let edited_since = self.editor_draft(cx).is_some_and(|draft| !matches(&draft));
            if was_new || !edited_since {
                self.reopen_automation(id, cx);
            }
        } else if self.automations.stashed.get(id).is_some_and(matches) {
            self.automations.stashed.remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::ScheduleKind;

    #[test]
    fn a_duplicate_is_a_paused_copy_with_its_own_triggers() {
        let mut auto = AutomationRow {
            id: "a".into(),
            name: "Nightly test sweep".into(),
            prompt: "Run the tests.".into(),
            enabled: true,
            next_run_at: 9,
            last_run_at: Some(5),
            last_session_id: Some("s1".into()),
            skipped_run_at: Some(7),
            ..Default::default()
        };
        auto.extra
            .insert("lastRunError".into(), Value::from("boom"));
        auto.extra
            .insert("modelSettings".into(), serde_json::json!({}));
        let trigger = schedule::new_time_trigger("t1".into(), ScheduleKind::Daily);
        schedule::apply_triggers(&mut auto, vec![trigger]);
        let copy = duplicate(&auto);
        assert_eq!(copy.name, "Nightly test sweep copy");
        assert!(copy.id.is_empty() && !copy.enabled);
        assert_eq!((copy.next_run_at, copy.last_run_at), (0, None));
        assert_eq!((&copy.last_session_id, copy.skipped_run_at), (&None, None));
        let ids = |a: &AutomationRow| a.triggers.as_ref().unwrap()[0]["id"].clone();
        assert_ne!(ids(&copy), ids(&auto));
        assert_eq!(schedule::schedule_label(&copy), "Daily at 09:00");
        assert!(!copy.extra.contains_key("lastRunError"));
        assert!(copy.extra.contains_key("modelSettings"));
    }
}
