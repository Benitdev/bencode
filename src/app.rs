mod agent;
pub mod commands;
mod integrations;
mod panes;
mod preferences;
mod projects;
mod surfaces;
mod tab_history;
mod tab_scope;
mod workspace_nav;
pub mod workspace_sync;

use std::collections::HashMap;

use ely_gpui_component::forms::{InputEvent, TextInput};
use ely_gpui_component::primitives::FocusScope;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AppContext, Context, Entity, Focusable, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, div, prelude::*,
};

pub use agent::{AgentRun, NEW_SESSION_TITLE, now_ms};
pub use preferences::{is_dark_appearance, theme_mode};
pub use projects::{is_path_in_project, normalize_project_path, same_project_path};
pub use surfaces::Surface;
pub use workspace_sync::WorkspaceCache;

use crate::db::{MonoCodeDb, SessionRow};
use crate::harness::{HarnessInfo, HarnessResolver, catalog};
use crate::ui::settings_modal::SettingsTab;

const RECENT_SESSION_LIMIT: usize = 50;
const INITIAL_OPEN_TABS: usize = 3;
const DEFAULT_CONTEXT_WINDOW: i64 = 200_000;
const NOTE_TITLE_CHARS: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Chat,
    Editor,
    Changes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FilterMode {
    #[default]
    All,
    Active,
    Pinned,
    Archived,
}

/// MonoCode's per-session access modes (`RuntimeMode`, `session.ts:368-390`),
/// stored in `sessions.runtime_mode` by their MonoCode ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PermissionMode {
    #[default]
    Supervised,
    AutoAcceptEdits,
    Auto,
    FullAccess,
}

impl PermissionMode {
    pub const ALL: [Self; 4] = [
        Self::Supervised,
        Self::AutoAcceptEdits,
        Self::Auto,
        Self::FullAccess,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Supervised => "supervised",
            Self::AutoAcceptEdits => "auto-accept-edits",
            Self::Auto => "auto",
            Self::FullAccess => "full-access",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.id() == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SidebarMode {
    #[default]
    Sessions,
    Files,
    Changes,
}

/// The working copy a project's workspace is narrowed to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeFocus {
    pub path: String,
    pub branch: Option<String>,
}

pub struct BenCodeApp {
    pub sessions: Vec<SessionRow>,
    pub selected_session_id: Option<String>,
    /// Open workspace tabs, each with its split layout and focused pane.
    pub tabs: crate::ui::layout::TabSet,
    pub active_view_mode: ViewMode,
    pub filter_mode: FilterMode,
    pub permission_mode: PermissionMode,
    pub sidebar_mode: SidebarMode,
    pub selected_diff_path: Option<String>,
    pub search_query: String,
    /// Model key in MonoCode's `harness:model` form, e.g. `claude:opus`.
    pub selected_model: String,
    /// Installed harness CLIs, probed once at startup (never from render).
    pub harnesses: Vec<HarnessInfo>,
    pub is_model_picker_open: bool,
    pub is_permission_picker_open: bool,
    pub is_plus_menu_open: bool,
    pub is_branch_picker_open: bool,
    /// A branch switch git refused because of local changes, awaiting "Stash & switch".
    pub blocked_branch_switch: Option<crate::app::workspace_sync::BranchTarget>,
    pub is_skill_picker_open: bool,
    pub skill_query: String,
    pub is_mention_picker_open: bool,
    pub mention_query: String,
    /// Highlighted row of the open `/` or `@` picker.
    pub picker_index: usize,
    /// The composer border brightens while the prompt has focus.
    pub prompt_focused: bool,
    /// The model picker's search field and highlighted row.
    pub model_search_input: Entity<TextInput>,
    pub model_picker_index: usize,
    pub drafts: HashMap<String, String>,
    pub expanded_reasoning: std::collections::HashSet<String>,
    pub transcript_ui: crate::ui::transcript::TranscriptUiState,
    /// Each project's terminals (MonoCode project terminal docks).
    pub terminals: crate::ui::terminal_pane::TerminalDocks,
    pub settings_tab: SettingsTab,
    /// The full-height view replacing the workspace, if any.
    pub surface: Option<Surface>,
    /// What Settings returns to when it closes.
    pub settings_return: Option<Surface>,
    pub notes: Vec<crate::db::Note>,
    pub selected_note_id: Option<String>,
    pub note_filter_query: String,
    pub note_filter_input: Entity<TextInput>,
    pub note_title_input: Entity<TextInput>,
    pub note_body_input: Entity<TextInput>,
    pub automations: Vec<crate::db::AutomationRow>,
    pub selected_automation_id: Option<String>,
    pub automation_runs: Vec<crate::db::AutomationRunRow>,
    pub automation_name_input: Entity<TextInput>,
    pub automation_prompt_input: Entity<TextInput>,
    pub automation_time_input: Entity<TextInput>,
    // Workspace & Projects
    pub current_cwd: String,
    /// Worktree each project's workspace is narrowed to, keyed by project.
    /// Kept for this run only, like MonoCode.
    pub worktree_focuses: HashMap<String, WorktreeFocus>,
    /// Pane last focused in each project, for returning to it from the rail.
    pub project_return: HashMap<String, String>,
    /// Tab last active in each (project, workspace) pair.
    pub workspace_return: HashMap<String, String>,
    pub recent_projects: Vec<String>,
    // Git & Source Control
    pub git_status: crate::git::GitDetailedStatus,
    pub git_commits: Vec<crate::git::GitCommitInfo>,
    pub git_commit_input: Entity<TextInput>,
    pub git_history_collapsed: bool,
    /// Git/filesystem snapshot for the active workspace; see `workspace_sync`.
    pub workspace: WorkspaceCache,
    pub file_tree: crate::ui::file_tree::FileTreeState,
    pub file_dialog_input: Entity<TextInput>,
    // Universal Search
    pub search_modal_input: Entity<TextInput>,
    pub search_scope: crate::ui::search_view::SearchScope,
    pub search_hits: Vec<crate::ui::search_view::SearchHit>,
    pub search_active_index: usize,
    // Inbox
    /// Rename/delete dialog opened from the thread list.
    pub session_dialog: Option<crate::ui::sidebar::SessionDialog>,
    pub rename_input: Entity<TextInput>,
    /// Root focus scope; Ely overlays hand focus back to it.
    pub focus_handle: gpui::FocusHandle,
    // Agent execution
    /// The running turn of each thread that has one, keyed by session id.
    pub runs: HashMap<String, AgentRun>,
    /// Prompts sent while a thread was busy, oldest first.
    pub prompt_queues: HashMap<String, Vec<String>>,
    next_run_id: u64,
    pub prompt_input: Entity<TextInput>,
    pub search_input: Entity<TextInput>,
    // [ui-agent-flow fields]
    /// Multi-pane transcript list states keyed by session id.
    pub transcripts: std::collections::HashMap<String, crate::ui::transcript::TranscriptView>,
    /// Visual drop hint for an active pane drag over an edge of another pane.
    pub active_pane_drop: Option<crate::ui::drag_drop::PaneDropTarget>,
    /// Active file drop target session id.
    pub active_file_drop_target: Option<String>,
    // [ui-git-files fields]
    /// Destructive git action awaiting confirmation.
    pub git_confirm: Option<crate::ui::git_changes_panel::GitConfirm>,
    // [ui-panels fields]
    /// Preferences as last loaded or saved (`settings.json`).
    pub settings: crate::settings::AppSettings,
    /// External editors and MCP servers, scanned once in the background.
    pub integrations: integrations::Integrations,
    /// Validation message for the automation time field.
    pub automation_time_error: Option<String>,
    /// Set on open; the search dialog focuses its query field once drawn.
    pub search_focus_pending: bool,
    /// Enter in the search field opens the top hit; subscribed on first open.
    pub search_submit: Option<Subscription>,
    /// Note id awaiting delete confirmation.
    pub note_pending_delete: Option<String>,
    /// Bumped on every note edit; a pending autosave only runs if it still matches.
    pub note_autosave_generation: u64,
    /// Last failed note save, shown with a Retry action.
    pub note_save_error: Option<String>,
    /// Automation id awaiting delete confirmation.
    pub automation_pending_delete: Option<String>,
    // [editor-pane fields]
    pub editor: crate::ui::editor_pane::EditorState,
    pub is_sidebar_open: bool,
    /// The project rail (⌘B); the session sidebar is `is_sidebar_open` (⇧⌘B).
    pub is_rail_open: bool,
    pub theme_preference: crate::settings::ThemePreference,
    /// Run Claude with `disableAllHooks` (MonoCode "Claude Code hooks" off).
    pub claude_hooks_disabled: bool,
    pub tab_history: tab_history::TabHistory,
    /// Set while Back/Forward switches tabs, so the move is not recorded.
    navigating_history: bool,
    pub is_terminal_open: bool,
    pub db: MonoCodeDb,
    _subscriptions: Vec<Subscription>,
}

fn text_input(
    window: &mut Window,
    cx: &mut Context<BenCodeApp>,
    placeholder: &str,
) -> Entity<TextInput> {
    let placeholder = placeholder.to_string();
    cx.new(|cx| TextInput::new(window, cx).placeholder(placeholder))
}

fn multiline_input(
    window: &mut Window,
    cx: &mut Context<BenCodeApp>,
    placeholder: &str,
    rows: (usize, usize),
) -> Entity<TextInput> {
    let placeholder = placeholder.to_string();
    cx.new(|cx| {
        TextInput::new(window, cx)
            .multi_line(rows.0, rows.1)
            .placeholder(placeholder)
    })
}

impl BenCodeApp {
    pub fn new(
        window: &mut Window,
        saved: crate::settings::AppSettings,
        cx: &mut Context<Self>,
    ) -> Self {
        let db = MonoCodeDb::open_default().unwrap_or_else(|err| {
            log::warn!("MonoCode DB unavailable ({err:#}); using BenCode's local database");
            MonoCodeDb::open_fallback()
        });

        let sessions = db
            .list_recent_sessions(RECENT_SESSION_LIMIT)
            .unwrap_or_else(|err| {
                log::error!("failed to load sessions: {err:#}");
                Vec::new()
            });
        let tabs = crate::ui::layout::TabSet::with_sessions(
            sessions
                .iter()
                .take(INITIAL_OPEN_TABS)
                .map(|s| s.id.as_str()),
        );
        let selected_session_id = tabs.focused_session().map(str::to_string);

        let prompt_input = multiline_input(
            window,
            cx,
            "Ask, build, / for commands, @ for references...",
            (1, 6),
        );
        let search_input = text_input(window, cx, "Search conversations...");
        let model_search_input = text_input(window, cx, "Search models");
        let note_filter_input = text_input(window, cx, "Filter notes...");
        let note_title_input = text_input(window, cx, "Note title...");
        let note_body_input = multiline_input(
            window,
            cx,
            "Write note or scratchpad in markdown...",
            (5, 25),
        );
        let automation_name_input = text_input(window, cx, "Automation name...");
        let automation_prompt_input = multiline_input(window, cx, "Automation prompt...", (3, 10));
        let automation_time_input = text_input(window, cx, "09:00");
        let git_commit_input = text_input(window, cx, "Message (⌘↩ to commit)...");
        let search_modal_input =
            text_input(window, cx, "Search conversations, files, projects... (⌘K)");

        let composer_input = prompt_input.clone();
        let model_input = model_search_input.clone();
        let weak_app = cx.weak_entity();
        let mut subscriptions = vec![
            cx.subscribe(
                &prompt_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.submit_prompt(cx),
                    InputEvent::Changed => this.on_prompt_changed(cx),
                    InputEvent::Focus | InputEvent::Blur => {
                        this.prompt_focused = *event == InputEvent::Focus;
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &search_input,
                |this: &mut Self, input, event: &InputEvent, cx| {
                    if *event == InputEvent::Changed {
                        this.search_query = input.read(cx).text().to_string();
                        cx.notify();
                    }
                },
            ),
            // Files may have changed while BenCode was in the background.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.recheck_open_files_on_disk(cx);
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                this.on_system_appearance_changed(window.appearance(), cx)
            }),
            cx.subscribe(
                &model_search_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => {
                        this.model_picker_index = 0;
                        cx.notify();
                    }
                    InputEvent::Submit => this.pick_highlighted_model(cx),
                    _ => {}
                },
            ),
            cx.subscribe(&note_title_input, Self::on_note_input_event),
            cx.subscribe(&note_body_input, Self::on_note_input_event),
            cx.subscribe(
                &note_filter_input,
                |this: &mut Self, input, event: &InputEvent, cx| {
                    if *event == InputEvent::Changed {
                        this.note_filter_query = input.read(cx).text().to_string();
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &git_commit_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if *event == InputEvent::Submit {
                        this.commit_staged_changes(cx);
                    }
                },
            ),
            cx.subscribe(
                &search_modal_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if *event == InputEvent::Changed {
                        this.update_search_hits(cx);
                    }
                },
            ),
        ];

        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            // The text input binds these keys itself, deeper than any action
            // context, so the composer's Enter and the picker keys
            // (MonoCode `Composer.tsx:1742-1830`) are taken here first.
            if event.keystroke.modifiers.modified() {
                return;
            }
            let key = event.keystroke.key.as_str();
            if !matches!(key, "enter" | "escape" | "up" | "down" | "tab") {
                return;
            }
            if model_input.read(cx).focus_handle(cx).is_focused(window) {
                let handled = weak_app.update(cx, |this, cx| match key {
                    "up" => {
                        this.move_model_picker(-1, cx);
                        true
                    }
                    "down" => {
                        this.move_model_picker(1, cx);
                        true
                    }
                    "escape" => {
                        this.close_model_picker(cx);
                        true
                    }
                    _ => false,
                });
                if matches!(handled, Ok(true)) {
                    cx.stop_propagation();
                }
                return;
            }
            if !composer_input.read(cx).focus_handle(cx).is_focused(window) {
                return;
            }
            let handled = weak_app.update(cx, |this, cx| this.handle_composer_key(key, cx));
            match handled {
                Ok(true) => cx.stop_propagation(),
                Ok(false) => {}
                Err(err) => log::debug!("composer key after app drop: {err:#}"),
            }
        }));

        // The project of the thread that opens focused, else the launch dir.
        let current_cwd = selected_session_id
            .as_deref()
            .and_then(|id| sessions.iter().find(|s| s.id == id))
            .map(|s| normalize_project_path(&s.cwd))
            .filter(|cwd| !cwd.is_empty() && cwd != "~")
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| normalize_project_path(&p.to_string_lossy()))
            })
            .unwrap_or_else(|| ".".to_string());
        let recent_projects = projects::recent_projects(&current_cwd, &sessions);

        let harnesses = HarnessResolver::discover();
        let selected_model = catalog::default_model(&harnesses).key.to_string();

        let notes = db.list_notes().unwrap_or_else(|err| {
            log::error!("failed to load notes: {err:#}");
            Vec::new()
        });
        let selected_note_id = notes.first().map(|n| n.id.clone());
        let automations = db.list_automations().unwrap_or_else(|err| {
            log::error!("failed to load automations: {err:#}");
            Vec::new()
        });
        let selected_automation_id = automations.first().map(|a| a.id.clone());

        // Git shells out several times; load it in the background after construction.
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| {
                this.refresh_workspace(cx);
                this.refresh_integrations(cx);
                this.start_automation_scheduler(cx);
                if this.is_terminal_open {
                    this.ensure_project_terminal(cx);
                }
            });
        })
        .detach();

        let mut app = Self {
            sessions,
            selected_session_id,
            tabs,
            active_view_mode: ViewMode::Chat,
            filter_mode: FilterMode::All,
            permission_mode: PermissionMode::default(),
            sidebar_mode: SidebarMode::Sessions,
            selected_diff_path: None,
            search_query: String::new(),
            selected_model,
            harnesses,
            is_model_picker_open: false,
            is_permission_picker_open: false,
            is_plus_menu_open: false,
            is_branch_picker_open: false,
            blocked_branch_switch: None,
            is_skill_picker_open: false,
            skill_query: String::new(),
            is_mention_picker_open: false,
            mention_query: String::new(),
            picker_index: 0,
            prompt_focused: false,
            model_search_input,
            model_picker_index: 0,
            drafts: HashMap::new(),
            expanded_reasoning: std::collections::HashSet::new(),
            transcript_ui: Default::default(),
            terminals: Default::default(),
            settings_tab: SettingsTab::Providers,
            surface: None,
            settings_return: None,
            notes,
            selected_note_id,
            note_filter_query: String::new(),
            note_filter_input,
            note_title_input,
            note_body_input,
            automations,
            selected_automation_id,
            automation_runs: Vec::new(),
            automation_name_input,
            automation_prompt_input,
            automation_time_input,
            current_cwd,
            worktree_focuses: HashMap::new(),
            project_return: HashMap::new(),
            workspace_return: HashMap::new(),
            recent_projects,
            git_status: Default::default(),
            git_commits: Vec::new(),
            git_commit_input,
            git_history_collapsed: false,
            workspace: WorkspaceCache::default(),
            file_tree: Default::default(),
            file_dialog_input: text_input(window, cx, "Name"),
            search_modal_input,
            search_scope: crate::ui::search_view::SearchScope::All,
            search_hits: Vec::new(),
            search_active_index: 0,
            session_dialog: None,
            rename_input: text_input(window, cx, "Thread title"),
            focus_handle: cx.focus_handle(),
            runs: HashMap::new(),
            prompt_queues: HashMap::new(),
            next_run_id: 0,
            prompt_input,
            search_input,
            // [ui-agent-flow init]
            transcripts: std::collections::HashMap::new(),
            active_pane_drop: None,
            active_file_drop_target: None,
            // [ui-git-files init]
            git_confirm: None,
            // [ui-panels init]
            settings: Default::default(),
            integrations: Default::default(),
            automation_time_error: None,
            search_focus_pending: false,
            search_submit: None,
            note_pending_delete: None,
            note_autosave_generation: 0,
            note_save_error: None,
            automation_pending_delete: None,
            // [editor-pane init]
            editor: Default::default(),
            is_sidebar_open: true,
            is_rail_open: true,
            theme_preference: Default::default(),
            claude_hooks_disabled: false,
            tab_history: Default::default(),
            navigating_history: false,
            is_terminal_open: true,
            db,
            _subscriptions: subscriptions,
        };
        app.apply_settings(saved);
        app.start_git_poll(cx);
        app.start_clock(cx);
        app
    }

    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.show_surface(Surface::Settings, cx);
    }

    pub fn selected_session(&self) -> Option<&SessionRow> {
        let id = self.selected_session_id.as_deref()?;
        self.sessions.iter().find(|s| s.id == id)
    }

    pub fn selected_session_mut(&mut self) -> Option<&mut SessionRow> {
        let id = self.selected_session_id.as_deref()?;
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    /// Switches the model (and, if needed, the harness) of the current thread.
    pub fn set_session_model(&mut self, model_key: &str, cx: &mut Context<Self>) {
        let Some(option) = catalog::find(model_key) else {
            log::warn!("unknown model key {model_key}");
            return;
        };
        self.selected_model = option.key.to_string();
        self.save_settings(cx);

        let harness_id = option.harness.id();
        let changed_session = self.selected_session_mut().map(|session| {
            if session.harness != harness_id {
                // A provider session id is only meaningful to the harness that issued it.
                session.provider_session_id = None;
            }
            session.model = option.key.to_string();
            session.harness = harness_id.to_string();
            session.id.clone()
        });
        if let Some(id) = changed_session {
            self.persist_session(&id);
        }
        cx.notify();
    }

    pub fn on_prompt_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.prompt_input.read(cx).text().to_string();
        self.is_skill_picker_open = false;
        self.is_mention_picker_open = false;
        self.picker_index = 0;

        if let Some(query) = trigger_query(&text, '/') {
            self.is_skill_picker_open = true;
            self.skill_query = query;
        } else if let Some(query) = trigger_query(&text, '@') {
            self.is_mention_picker_open = true;
            self.mention_query = query;
        }
        cx.notify();
    }

    /// Enter sends; with a picker open, ↑/↓ move, Tab/Enter pick and Esc
    /// closes it. Returns whether the key was used.
    fn handle_composer_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let picker_open = self.is_skill_picker_open || self.is_mention_picker_open;
        match (key, picker_open) {
            ("enter", false) => {
                self.submit_prompt(cx);
                true
            }
            // MonoCode: ↑ in an empty composer brings the last prompt back.
            ("up", false) if self.prompt_input.read(cx).text().is_empty() => {
                self.recall_last_turn(cx);
                true
            }
            ("up", true) => self.move_picker(-1, cx),
            ("down", true) => self.move_picker(1, cx),
            ("tab", true) => self.accept_picker(cx),
            ("enter", true) => {
                if !self.accept_picker(cx) {
                    // Nothing matches: close the picker and send.
                    self.close_pickers(cx);
                    self.submit_prompt(cx);
                }
                true
            }
            ("escape", true) => {
                self.close_pickers(cx);
                true
            }
            _ => false,
        }
    }

    pub fn close_pickers(&mut self, cx: &mut Context<Self>) {
        self.is_skill_picker_open = false;
        self.is_mention_picker_open = false;
        cx.notify();
    }

    pub fn insert_skill(&mut self, skill_name: &str, cx: &mut Context<Self>) {
        self.replace_trigger('/', skill_name, cx);
        self.is_skill_picker_open = false;
        cx.notify();
    }

    pub fn insert_mention(&mut self, mention: &str, cx: &mut Context<Self>) {
        self.replace_trigger('@', &format!("@{mention}"), cx);
        self.is_mention_picker_open = false;
        cx.notify();
    }

    /// Appends `text` to the prompt as its own word.
    pub fn append_to_prompt(&mut self, text: &str, cx: &mut Context<Self>) {
        self.prompt_input.update(cx, |input, cx| {
            let current = input.text().trim_end().to_string();
            let joined = if current.is_empty() {
                format!("{text} ")
            } else {
                format!("{current} {text} ")
            };
            input.set_text(joined, cx);
        });
    }

    /// Replaces the trailing `trigger…` token of the prompt with `replacement`.
    fn replace_trigger(&mut self, trigger: char, replacement: &str, cx: &mut Context<Self>) {
        self.prompt_input.update(cx, |input, cx| {
            let text = input.text().to_string();
            let prefix = text.rfind(trigger).map_or("", |idx| &text[..idx]);
            input.set_text(format!("{prefix}{replacement} "), cx);
        });
    }

    /// Recalls the last user prompt into the composer prompt input.
    pub fn recall_last_turn(&mut self, cx: &mut Context<Self>) {
        let last_prompt = self
            .selected_session()
            .and_then(|session| session.blocks.iter().rev().find(|b| b.role == "user"))
            .and_then(|block| block.text.clone());
        if let Some(prompt) = last_prompt {
            self.prompt_input.update(cx, |input, cx| {
                input.set_text(prompt, cx);
            });
            cx.notify();
        }
    }

    /// Sets the prompt text directly into the composer (e.g. for editing last turn).
    pub fn edit_turn(&mut self, text: &str, cx: &mut Context<Self>) {
        let text_owned = text.to_string();
        self.prompt_input.update(cx, |input, cx| {
            input.set_text(text_owned, cx);
        });
        cx.notify();
    }

    /// Saves a turn as a note titled after its thread (or the text's first
    /// line), linked to that thread and project, then opens it (MonoCode
    /// `SessionPane` "Save as note").
    pub fn save_turn_to_note(&mut self, text: &str, cx: &mut Context<Self>) {
        let session = self.selected_session();
        let title = session
            .map(|s| s.title.clone())
            .filter(|t| !t.is_empty() && t != NEW_SESSION_TITLE)
            .or_else(|| {
                text.lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "Untitled".to_string());
        let upsert = crate::db::NoteUpsert {
            id: format!("note-{}", now_ms()),
            title: title.chars().take(NOTE_TITLE_CHARS).collect(),
            body: text.to_string(),
            tags: Vec::new(),
            source_session_id: session.map(|s| s.id.clone()),
            source_cwd: session.map(|s| s.cwd.clone()).filter(|cwd| !cwd.is_empty()),
        };
        match self.db.upsert_note(&upsert) {
            Ok(note) => {
                let id = note.id.clone();
                self.notes.insert(0, note);
                self.open_notes(cx);
                self.select_note(id, cx);
            }
            Err(err) => log::error!("failed to save turn as note: {err:#}"),
        }
        cx.notify();
    }

    /// Attaches a file to the composer prompt input.
    pub fn attach_file_to_composer(
        &mut self,
        session_id: &str,
        path: &str,
        cx: &mut Context<Self>,
    ) {
        self.active_file_drop_target = None;
        self.focus_pane(session_id.to_string(), cx);

        self.prompt_input.update(cx, |input, cx| {
            let current = input.text();
            let addition = if current.trim().is_empty() || current.ends_with(' ') {
                format!("@{path} ")
            } else {
                format!(" @{path} ")
            };
            input.set_text(format!("{current}{addition}"), cx);
        });
        cx.notify();
    }

    /// Attaches external files dropped from the OS (Finder) to the composer.
    pub fn attach_external_paths_to_composer(
        &mut self,
        session_id: &str,
        paths: &[std::path::PathBuf],
        cx: &mut Context<Self>,
    ) {
        self.active_file_drop_target = None;
        self.focus_pane(session_id.to_string(), cx);

        let cwd_path = std::path::Path::new(&self.workspace.cwd);
        let mut additions = Vec::new();
        for path in paths {
            let rel = path.strip_prefix(cwd_path).unwrap_or(path);
            additions.push(format!("@{} ", rel.to_string_lossy()));
        }

        if !additions.is_empty() {
            let added_text = additions.join("");
            self.prompt_input.update(cx, |input, cx| {
                let current = input.text();
                let spacer = if current.is_empty() || current.ends_with(' ') {
                    ""
                } else {
                    " "
                };
                input.set_text(format!("{current}{spacer}{added_text}"), cx);
            });
        }
        cx.notify();
    }

    pub fn toggle_pin_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
            return;
        };
        let was_pinned = session.pinned;
        if let Err(err) = self.db.toggle_pinned(id, was_pinned) {
            log::error!("failed to toggle pin for {id}: {err:#}");
            return;
        }
        session.pinned = !was_pinned;
        cx.notify();
    }

    pub fn toggle_archive_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
            return;
        };
        let was_archived = session.archived;
        if let Err(err) = self.db.toggle_archived(id, was_archived) {
            log::error!("failed to toggle archive for {id}: {err:#}");
            return;
        }
        session.archived = !was_archived;
        cx.notify();
    }
}

/// If the prompt ends in a `trigger` token (`/skill`, `@file`) that starts a
/// word, returns the lower-cased text typed after the trigger.
fn trigger_query(text: &str, trigger: char) -> Option<String> {
    let idx = text.rfind(trigger)?;
    let after = &text[idx + trigger.len_utf8()..];
    let starts_word = text[..idx].chars().last().is_none_or(char::is_whitespace);
    (starts_word && !after.contains(char::is_whitespace)).then(|| after.to_lowercase())
}

impl Render for BenCodeApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focus_handle.contains_focused(window, cx) && window.focused(cx).is_none() {
            window.focus(&self.focus_handle, cx);
        }
        let colors = &cx.theme().colors;
        let (bg, fg) = (colors.bg, colors.fg);
        // Search, Inbox, Notes, Automations and Settings replace the
        // sidebar and the workspace column (MonoCode in-shell views).
        let surface = self.render_surface(cx);
        let workspace_visible = surface.is_none();

        FocusScope::new(&self.focus_handle).root().child(
            Self::bind_commands(div().id("bencode-root"), cx)
                .flex()
                .flex_col()
                .size_full()
                .bg(bg)
                .text_color(fg)
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .size_full()
                        .min_h_0()
                        .overflow_hidden()
                        // Column 1: Leftmost Project Rail
                        .when(self.is_rail_open, |el| {
                            el.child(self.render_project_rail(cx))
                        })
                        // Column 2: Workspace Sidebar (when open)
                        .when(self.is_sidebar_open && workspace_visible, |el| {
                            el.child(self.render_sidebar(cx))
                        })
                        // Column 3: Main Area (TitleBar + Views + Terminal Drawer + UsageFooter)
                        .when(workspace_visible, |el| {
                            el.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .h_full()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .child(self.render_titlebar(cx))
                                    .child(
                                        div().flex().flex_1().min_h_0().overflow_hidden().child(
                                            match self.active_view_mode {
                                                ViewMode::Chat => self
                                                    .render_transcript_panel(cx)
                                                    .into_any_element(),
                                                ViewMode::Editor => self
                                                    .render_editor_pane(window, cx)
                                                    .into_any_element(),
                                                ViewMode::Changes => {
                                                    self.render_diff_viewer(cx).into_any_element()
                                                }
                                            },
                                        ),
                                    )
                                    .when(self.is_terminal_open, |el| {
                                        el.child(self.render_terminal_drawer(cx))
                                    })
                                    .child(self.render_usage_footer(cx)),
                            )
                        })
                        .children(surface),
                )
                .children(self.render_session_dialog(cx))
                .children(self.render_git_confirm(cx))
                .children(self.render_branch_switch_confirm(cx))
                .children(self.render_file_tree_dialog(cx)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_query_detects_word_start_tokens() {
        assert_eq!(trigger_query("/rev", '/').as_deref(), Some("rev"));
        assert_eq!(trigger_query("fix @Src/Ma", '@').as_deref(), Some("src/ma"));
        assert_eq!(
            trigger_query("a/b", '/'),
            None,
            "mid-word slash is a path, not a skill"
        );
        assert_eq!(
            trigger_query("@file done", '@'),
            None,
            "token already finished"
        );
        assert_eq!(trigger_query("xin chào /ski", '/').as_deref(), Some("ski"));
    }
}
