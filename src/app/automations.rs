//! Automations: the surface's state, the editor's draft, and loading and
//! saving rows without blocking the UI thread. MonoCode
//! `AutomationsContent` (`features/automations/ui/AutomationsView.tsx`) and
//! `features/automations/model/automations.ts`. Runs are started in
//! `automation_runs.rs`; the views are in `ui/automations/`.

use ely_gpui_component::forms::{InputEvent, TextInput};
use gpui::{Context, Entity, Window};
use jiff::tz::TimeZone;
use serde_json::Value;

use super::{BenCodeApp, PermissionMode, Surface, multiline_input, now_ms, text_input, unique_id};
use crate::db::{AutomationRow, AutomationRunRow, DEFAULT_GRACE_MINUTES};
use crate::schedule::{self, ScheduleKind, Trigger};
use crate::ui::automations::templates::{AutomationTemplate, TemplateCategory};

/// MonoCode allows this many triggers per automation.
pub const MAX_TRIGGERS: usize = 20;

/// The two pages of an automation's editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EditorTab {
    #[default]
    Settings,
    History,
}

/// Where a run happens: MonoCode `workspaceMode`, as the editor offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkingCopy {
    /// The project folder itself.
    Current,
    /// A worktree made for the run.
    Worktree,
}

impl WorkingCopy {
    pub const ALL: [Self; 2] = [Self::Current, Self::Worktree];

    pub fn id(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Worktree => "worktree",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "Current",
            Self::Worktree => "Fresh worktree",
        }
    }

    /// The stored mode; anything but `worktree` (MonoCode's `existing`
    /// too) runs in the project folder.
    pub fn of(auto: &AutomationRow) -> Self {
        match auto.workspace_mode.as_deref() {
            Some(mode) if mode == Self::Worktree.id() => Self::Worktree,
            _ => Self::Current,
        }
    }
}

/// The Automations surface's data, selection, fields and pending dialogs.
pub struct AutomationsState {
    pub items: Vec<AutomationRow>,
    pub selected_id: Option<String>,
    /// The run history of the automation in the editor.
    pub runs: Vec<AutomationRunRow>,
    pub filter_input: Entity<TextInput>,
    pub name_input: Entity<TextInput>,
    pub prompt_input: Entity<TextInput>,
    /// What the editor shows: a stored automation being edited, or a new
    /// one (empty `id`). Its name and prompt are in the fields above.
    pub draft: Option<AutomationRow>,
    /// The New automation page is shown instead of the editor.
    pub picker_open: bool,
    pub category: TemplateCategory,
    pub tab: EditorTab,
    pub advanced_open: bool,
    /// A save is on its way to the database.
    pub saving: bool,
    /// The last failed load, save, run or delete.
    pub error: Option<String>,
    /// Automation id awaiting delete confirmation.
    pub pending_delete: Option<String>,
    /// Counts list loads; `listed` is the newest one shown, so a load that
    /// lands late never replaces a newer one.
    list_requests: u64,
    listed: u64,
    /// Counts run-history loads; only the newest one is shown.
    run_requests: u64,
}

impl AutomationsState {
    /// The fields, with no automations loaded yet.
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> Self {
        Self {
            items: Vec::new(),
            selected_id: None,
            runs: Vec::new(),
            filter_input: text_input(window, cx, "Filter automations"),
            name_input: text_input(window, cx, "Untitled"),
            prompt_input: multiline_input(window, cx, "What should the agent do?", (6, 18)),
            draft: None,
            picker_open: true,
            category: TemplateCategory::default(),
            tab: EditorTab::default(),
            advanced_open: false,
            saving: false,
            error: None,
            pending_delete: None,
            list_requests: 0,
            listed: 0,
            run_requests: 0,
        }
    }
}

/// MonoCode `newAutomationDraft`: weekday mornings in a fresh worktree,
/// with no trigger yet.
fn new_draft(cwd: String, model: &str) -> AutomationRow {
    let harness = model
        .split_once(':')
        .map_or("claude", |(harness, _)| harness);
    AutomationRow {
        harness: harness.to_string(),
        model: model.to_string(),
        cwd,
        schedule_kind: ScheduleKind::Weekdays.id().to_string(),
        time: "09:00".to_string(),
        day_of_week: 1,
        enabled: true,
        triggers: Some(Vec::new()),
        trigger_kind: Some("time".to_string()),
        trigger_event: Some(String::new()),
        workspace_mode: Some(WorkingCopy::Worktree.id().to_string()),
        reuse_session: Some(false),
        session_folder_id: Some(String::new()),
        runtime_mode: Some(PermissionMode::Auto.id().to_string()),
        missed_run_grace_minutes: Some(DEFAULT_GRACE_MINUTES),
        ..Default::default()
    }
}

/// MonoCode `draftFromTemplate`.
fn draft_from_template(cwd: String, model: &str, template: &AutomationTemplate) -> AutomationRow {
    let mut draft = new_draft(cwd, model);
    let mut trigger = schedule::new_time_trigger(unique_id("trigger"), template.schedule);
    trigger.insert("time".into(), Value::from(template.time));
    trigger.insert("dayOfWeek".into(), Value::from(template.day_of_week));
    schedule::apply_triggers(&mut draft, vec![trigger]);
    draft.name = template.name.to_string();
    draft.prompt = template.prompt.to_string();
    draft
}

/// MonoCode `draftFromAutomation`: `auto` with its triggers spelled out
/// and the settings older rows lack filled in.
fn editable(auto: &AutomationRow) -> AutomationRow {
    let mut draft = auto.clone();
    schedule::apply_triggers(&mut draft, schedule::triggers_of(auto));
    draft.workspace_mode = Some(WorkingCopy::of(auto).id().to_string());
    draft.reuse_session = Some(auto.reuse_session.unwrap_or(false));
    draft.session_folder_id = Some(auto.session_folder_id.clone().unwrap_or_default());
    draft.runtime_mode = Some(access_mode(auto).id().to_string());
    draft.missed_run_grace_minutes = Some(grace_minutes(auto));
    draft
}

/// The access runs start with; MonoCode's default is Auto.
pub fn access_mode(auto: &AutomationRow) -> PermissionMode {
    auto.runtime_mode
        .as_deref()
        .and_then(PermissionMode::from_id)
        .unwrap_or(PermissionMode::Auto)
}

pub fn grace_minutes(auto: &AutomationRow) -> i64 {
    auto.missed_run_grace_minutes
        .unwrap_or(DEFAULT_GRACE_MINUTES)
}

/// Whether the editor's `draft` differs from the stored `auto` in anything
/// the editor can change.
fn settings_differ(draft: &AutomationRow, auto: &AutomationRow) -> bool {
    let auto = editable(auto);
    draft.name != auto.name
        || draft.prompt != auto.prompt
        || draft.model != auto.model
        || draft.cwd != auto.cwd
        || draft.enabled != auto.enabled
        || draft.triggers != auto.triggers
        || draft.workspace_mode != auto.workspace_mode
        || draft.reuse_session != auto.reuse_session
        || draft.session_folder_id != auto.session_folder_id
        || draft.runtime_mode != auto.runtime_mode
        || draft.missed_run_grace_minutes != auto.missed_run_grace_minutes
}

/// MonoCode's `valid`: a name, instructions, a project and a model.
pub fn draft_is_valid(draft: &AutomationRow) -> bool {
    !draft.name.trim().is_empty()
        && !draft.prompt.trim().is_empty()
        && !draft.cwd.is_empty()
        && !draft.model.is_empty()
}

/// `query` must already be lower-case.
pub fn automation_matches(auto: &AutomationRow, project: &str, query: &str) -> bool {
    query.is_empty()
        || [&auto.name, &auto.prompt, &auto.cwd]
            .into_iter()
            .any(|text| text.to_lowercase().contains(query))
        || project.to_lowercase().contains(query)
}

impl BenCodeApp {
    pub fn open_automations(&mut self, cx: &mut Context<Self>) {
        self.show_surface(Surface::Automations, cx);
        // MonoCode opens on the New automation page; unsaved edits stay.
        if !self.automation_dirty(cx) {
            self.show_automation_picker(cx);
        }
        self.refresh_automations(cx);
        cx.notify();
    }

    /// Reloads the list, and the open run history with it.
    pub(crate) fn refresh_automations(&mut self, cx: &mut Context<Self>) {
        let request = self.next_automation_list();
        self.db_then(
            cx,
            |db| db.list_automations(),
            move |this, listed, cx| match listed {
                Ok(items) => this.show_automations(request, items, cx),
                Err(err) => this.automation_failed("load automations", &err, cx),
            },
        );
        if self.automations.tab == EditorTab::History
            && let Some(id) = self.edited_automation().map(|a| a.id.clone())
        {
            self.load_automation_runs(&id, cx);
        }
    }

    fn next_automation_list(&mut self) -> u64 {
        self.automations.list_requests += 1;
        self.automations.list_requests
    }

    /// Shows the list `request` loaded, unless a later load is already shown.
    fn show_automations(
        &mut self,
        request: u64,
        items: Vec<AutomationRow>,
        cx: &mut Context<Self>,
    ) {
        if request < self.automations.listed {
            return;
        }
        self.automations.listed = request;
        self.automations.items = items;
        if self.selected_automation().is_none() {
            self.automations.selected_id = self.automations.items.first().map(|a| a.id.clone());
        }
        // The automation in the editor was deleted elsewhere.
        let editing = self.automations.draft.as_ref().map(|d| d.id.clone());
        if let Some(id) = editing.filter(|id| !id.is_empty())
            && !self.automations.items.iter().any(|a| a.id == id)
        {
            self.show_automation_picker(cx);
        }
        cx.notify();
    }

    fn automation_failed(&mut self, what: &str, err: &anyhow::Error, cx: &mut Context<Self>) {
        log::error!("could not {what}: {err:#}");
        self.automations.error = Some(format!("Could not {what}: {err}"));
        cx.notify();
    }

    pub(crate) fn selected_automation(&self) -> Option<&AutomationRow> {
        let id = self.automations.selected_id.as_deref()?;
        self.automations.items.iter().find(|a| a.id == id)
    }

    /// The stored row behind the editor; `None` for a new automation.
    pub(crate) fn edited_automation(&self) -> Option<&AutomationRow> {
        let id = &self.automations.draft.as_ref()?.id;
        self.automations.items.iter().find(|a| &a.id == id)
    }

    /// The editor's automation with the name and instructions typed so far.
    pub(crate) fn editor_draft(&self, cx: &gpui::App) -> Option<AutomationRow> {
        let mut draft = self.automations.draft.clone()?;
        draft.name = self
            .automations
            .name_input
            .read(cx)
            .text()
            .trim()
            .to_string();
        draft.prompt = self.automations.prompt_input.read(cx).text().to_string();
        Some(draft)
    }

    /// A new automation, or a stored one with unsaved edits.
    pub(crate) fn automation_dirty(&self, cx: &gpui::App) -> bool {
        match (self.editor_draft(cx), self.edited_automation()) {
            (Some(draft), Some(stored)) => settings_differ(&draft, stored),
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// MonoCode `beginCreate`: back to the New automation page.
    pub(crate) fn show_automation_picker(&mut self, cx: &mut Context<Self>) {
        self.automations.picker_open = true;
        self.automations.draft = None;
        cx.notify();
    }

    fn open_automation_editor(&mut self, draft: AutomationRow, cx: &mut Context<Self>) {
        let (name, prompt) = (draft.name.clone(), draft.prompt.clone());
        self.automations
            .name_input
            .update(cx, |input, cx| input.set_text(name, cx));
        self.automations
            .prompt_input
            .update(cx, |input, cx| input.set_text(prompt, cx));
        self.automations.draft = Some(draft);
        self.automations.picker_open = false;
        self.automations.tab = EditorTab::Settings;
        self.automations.advanced_open = false;
        cx.notify();
    }

    pub(crate) fn select_automation(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(auto) = self.automations.items.iter().find(|a| a.id == id) else {
            return;
        };
        let draft = editable(auto);
        self.automations.selected_id = Some(id.to_string());
        self.automations.error = None;
        self.automations.runs.clear();
        self.load_automation_runs(id, cx);
        self.open_automation_editor(draft, cx);
    }

    /// Loads the run history of automation `id`; a later call wins.
    pub(crate) fn load_automation_runs(&mut self, id: &str, cx: &mut Context<Self>) {
        self.automations.run_requests += 1;
        let request = self.automations.run_requests;
        let id = id.to_string();
        self.db_then(
            cx,
            move |db| db.list_automation_runs(&id),
            move |this, runs, cx| {
                if request != this.automations.run_requests {
                    return;
                }
                this.automations.runs = runs.unwrap_or_else(|err| {
                    log::error!("list_automation_runs failed: {err:#}");
                    Vec::new()
                });
                cx.notify();
            },
        );
    }

    /// MonoCode `defaultDraftTarget`: the open project, and the selected
    /// automation's model, else the composer's.
    fn new_automation_target(&self) -> (String, String) {
        let cwd = if self.current_cwd.is_empty() {
            self.recent_projects.first().cloned().unwrap_or_default()
        } else {
            self.current_cwd.clone()
        };
        let model = self
            .selected_automation()
            .map_or_else(|| self.selected_model.clone(), |auto| auto.model.clone());
        (cwd, model)
    }

    pub(crate) fn begin_blank_automation(&mut self, cx: &mut Context<Self>) {
        let (cwd, model) = self.new_automation_target();
        self.open_automation_editor(new_draft(cwd, &model), cx);
    }

    pub(crate) fn begin_automation_from(
        &mut self,
        template: &AutomationTemplate,
        cx: &mut Context<Self>,
    ) {
        let (cwd, model) = self.new_automation_target();
        self.open_automation_editor(draft_from_template(cwd, &model, template), cx);
    }

    /// Reset (a stored automation) or Cancel (a new one).
    pub(crate) fn discard_automation_edits(&mut self, cx: &mut Context<Self>) {
        match self.edited_automation().map(|a| a.id.clone()) {
            Some(id) => self.select_automation(&id, cx),
            None => self.show_automation_picker(cx),
        }
    }

    /// Changes the editor's automation; nothing is stored until Save.
    pub(crate) fn edit_automation(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut AutomationRow),
    ) {
        if let Some(draft) = self.automations.draft.as_mut() {
            edit(draft);
            cx.notify();
        }
    }

    fn edit_automation_triggers(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Vec<Trigger>),
    ) {
        self.edit_automation(cx, |draft| {
            let mut triggers = schedule::triggers_of(draft);
            edit(&mut triggers);
            schedule::apply_triggers(draft, triggers);
        });
    }

    pub(crate) fn add_automation_trigger(&mut self, kind: ScheduleKind, cx: &mut Context<Self>) {
        let trigger = schedule::new_time_trigger(unique_id("trigger"), kind);
        self.edit_automation_triggers(cx, |triggers| {
            if triggers.len() < MAX_TRIGGERS {
                triggers.push(trigger);
            }
        });
    }

    pub(crate) fn remove_automation_trigger(&mut self, index: usize, cx: &mut Context<Self>) {
        self.edit_automation_triggers(cx, |triggers| {
            if index < triggers.len() {
                triggers.remove(index);
            }
        });
    }

    pub(crate) fn set_automation_trigger(
        &mut self,
        index: usize,
        key: &str,
        value: Value,
        cx: &mut Context<Self>,
    ) {
        self.edit_automation_triggers(cx, |triggers| {
            if let Some(trigger) = triggers.get_mut(index) {
                trigger.insert(key.to_string(), value);
            }
        });
    }

    /// Save or Create: stores the editor's automation and reopens it.
    pub(crate) fn save_automation_draft(&mut self, cx: &mut Context<Self>) {
        if self.automations.saving {
            return;
        }
        let Some(draft) = self.editor_draft(cx).filter(draft_is_valid) else {
            return;
        };
        let now = now_ms();
        let Some(next_run_at) = schedule::next_automation_run_at(&draft, now, &TimeZone::system())
        else {
            self.automations.error = Some("One of the triggers has no valid time.".to_string());
            cx.notify();
            return;
        };
        // Run bookkeeping comes from the stored row: a run may have ended
        // while the editor was open.
        let saved = match self.edited_automation() {
            Some(stored) => AutomationRow {
                next_run_at,
                updated_at: now,
                created_at: stored.created_at,
                last_run_at: stored.last_run_at,
                last_run_status: stored.last_run_status.clone(),
                last_session_id: stored.last_session_id.clone(),
                ..draft
            },
            None => AutomationRow {
                id: unique_id("auto"),
                next_run_at,
                created_at: now,
                updated_at: now,
                ..draft
            },
        };
        self.automations.saving = true;
        let (request, id) = (self.next_automation_list(), saved.id.clone());
        self.db_then(
            cx,
            move |db| {
                db.save_automation(&saved)?;
                db.list_automations()
            },
            move |this, listed, cx| {
                this.automations.saving = false;
                match listed {
                    Ok(items) => {
                        this.show_automations(request, items, cx);
                        this.select_automation(&id, cx);
                    }
                    Err(err) => this.automation_failed("save the automation", &err, cx),
                }
            },
        );
        cx.notify();
    }

    pub(crate) fn delete_automation(&mut self, id: &str, cx: &mut Context<Self>) {
        self.automations.pending_delete = None;
        let (request, id) = (self.next_automation_list(), id.to_string());
        self.db_then(
            cx,
            {
                let id = id.clone();
                move |db| {
                    db.delete_automation(&id)?;
                    db.list_automations()
                }
            },
            move |this, listed, cx| match listed {
                Ok(items) => {
                    if this.automations.selected_id.as_deref() == Some(id.as_str()) {
                        this.automations.selected_id = None;
                    }
                    this.show_automations(request, items, cx);
                }
                Err(err) => this.automation_failed("delete the automation", &err, cx),
            },
        );
        cx.notify();
    }

    /// The switch on a list card: stored at once, unlike the editor's.
    pub(crate) fn set_automation_enabled(
        &mut self,
        id: &str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        // Shown at once; the reload below confirms or undoes it.
        if let Some(auto) = self.automations.items.iter_mut().find(|a| a.id == id) {
            auto.enabled = enabled;
        }
        if let Some(draft) = self.automations.draft.as_mut().filter(|d| d.id == id) {
            draft.enabled = enabled;
        }
        let (request, id) = (self.next_automation_list(), id.to_string());
        self.db_then(
            cx,
            move |db| {
                let toggled = db.toggle_automation(&id, enabled);
                // The list either way: after a failure it shows what is stored.
                db.list_automations().map(|items| (items, toggled))
            },
            move |this, listed, cx| match listed {
                Ok((items, toggled)) => {
                    this.show_automations(request, items, cx);
                    if let Err(err) = toggled {
                        this.automation_failed("update the automation", &err, cx);
                    }
                }
                Err(err) => this.automation_failed("update the automation", &err, cx),
            },
        );
        cx.notify();
    }

    pub(crate) fn show_automation_tab(&mut self, tab: EditorTab, cx: &mut Context<Self>) {
        self.automations.tab = tab;
        if tab == EditorTab::History
            && let Some(id) = self.edited_automation().map(|a| a.id.clone())
        {
            self.load_automation_runs(&id, cx);
        }
        cx.notify();
    }

    /// A run history row: shows the thread the run happened in.
    pub(crate) fn open_automation_run(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if !self.sessions.iter().any(|s| s.id == session_id) {
            self.automations.error = Some("The session of this run no longer exists.".to_string());
            cx.notify();
            return;
        }
        self.close_surface(cx);
        self.open_session(session_id.to_string(), cx);
    }

    /// Typing a name or instructions changes what Save and Reset offer.
    pub(crate) fn on_automation_input_event(
        &mut self,
        _: Entity<TextInput>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Changed) {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::automations::templates::TEMPLATES;

    #[test]
    fn a_template_becomes_a_draft_with_its_trigger() {
        let weekly = TEMPLATES
            .iter()
            .find(|t| t.name == "Weekly changelog")
            .unwrap();
        let draft = draft_from_template("/repo".into(), "codex:gpt-5", weekly);
        assert_eq!(
            (draft.name.as_str(), draft.harness.as_str()),
            ("Weekly changelog", "codex")
        );
        assert_eq!(schedule::schedule_label(&draft), "Friday at 16:00");
        assert_eq!(WorkingCopy::of(&draft), WorkingCopy::Worktree);
        assert!(draft.enabled && draft.id.is_empty() && draft_is_valid(&draft));
    }

    #[test]
    fn a_blank_draft_needs_a_name_and_instructions() {
        let mut draft = new_draft("/repo".into(), "claude:sonnet");
        assert_eq!(schedule::schedule_label(&draft), "No trigger");
        assert!(!draft_is_valid(&draft));
        draft.name = "Nightly".into();
        draft.prompt = " ".into();
        assert!(!draft_is_valid(&draft));
        draft.prompt = "Run the tests.".into();
        assert!(draft_is_valid(&draft));
    }

    #[test]
    fn only_editable_settings_make_the_editor_dirty() {
        // A row from before triggers and session settings existed.
        let stored = AutomationRow {
            id: "a".into(),
            name: "Standup".into(),
            prompt: "Summarize.".into(),
            model: "claude:sonnet".into(),
            cwd: "/repo".into(),
            schedule_kind: "daily".into(),
            time: "09:15".into(),
            enabled: true,
            ..Default::default()
        };
        let mut draft = editable(&stored);
        assert_eq!(WorkingCopy::of(&draft), WorkingCopy::Current);
        assert!(!settings_differ(&draft, &stored));
        // A run finishing behind the editor changes nothing the user edits.
        let ran = AutomationRow {
            next_run_at: 99,
            last_run_at: Some(9),
            ..stored.clone()
        };
        assert!(!settings_differ(&draft, &ran));
        draft.missed_run_grace_minutes = Some(30);
        assert!(settings_differ(&draft, &stored));
    }

    #[test]
    fn working_copies_read_the_stored_mode() {
        let mode = |mode: Option<&str>| {
            WorkingCopy::of(&AutomationRow {
                workspace_mode: mode.map(str::to_string),
                ..Default::default()
            })
        };
        assert_eq!(mode(Some("worktree")), WorkingCopy::Worktree);
        // MonoCode's `existing`, and rows that predate the setting.
        assert_eq!(mode(Some("existing")), WorkingCopy::Current);
        assert_eq!(mode(None), WorkingCopy::Current);
        for copy in WorkingCopy::ALL {
            assert_eq!(mode(Some(copy.id())), copy);
        }
    }

    #[test]
    fn the_filter_reads_name_prompt_and_project() {
        let auto = AutomationRow {
            name: "Find Bugs".into(),
            prompt: "Review history".into(),
            cwd: "/src/app".into(),
            ..Default::default()
        };
        assert!(automation_matches(&auto, "Storefront", ""));
        assert!(automation_matches(&auto, "Storefront", "bugs"));
        assert!(automation_matches(&auto, "Storefront", "storefront"));
        assert!(automation_matches(&auto, "Storefront", "/src"));
        assert!(!automation_matches(&auto, "Storefront", "deploy"));
    }
}
