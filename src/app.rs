pub mod accounts;
mod agent;
pub mod chat_background;
pub mod commands;
pub mod file_pane;
mod integrations;
mod model_catalog;
mod panes;
mod preferences;
pub mod process_monitor;
pub mod project_files;
mod project_stats;
mod projects;
pub mod session_review;
pub mod reminders;
pub mod session_folders;
pub mod session_list;
mod surfaces;
mod tab_history;
mod tab_scope;
pub mod usage;
mod workspace_nav;
pub mod workspace_sync;
pub mod worktree_lifecycle;

use std::collections::{HashMap, HashSet};

use crate::ui::composer::mcp_tags::McpTag;
use crate::ui::composer::mentions::MentionIndex;
use ely_gpui_component::forms::{InputEvent, TextInput};
use ely_gpui_component::primitives::FocusScope;
use ely_gpui_component::theme::ActiveTheme;
use gpui::{
    AppContext, Context, Entity, Focusable, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, div, prelude::*,
};

pub use agent::{AgentRun, NEW_SESSION_TITLE, QUESTION_TOOL, TurnInput, can_compact, now_ms};
pub use preferences::{appearance_prefs, is_dark_appearance, theme_mode};
pub use projects::{is_path_in_project, normalize_project_path, same_project_path};
pub use surfaces::Surface;
pub use workspace_sync::WorkspaceCache;

use crate::db::{AppDb, SessionRow};
use crate::harness::{HarnessInfo, HarnessResolver, catalog};
use crate::ui::settings_modal::SettingsTab;

const RECENT_SESSION_LIMIT: usize = 50;
const INITIAL_OPEN_TABS: usize = 3;
const NOTE_TITLE_CHARS: usize = 80;

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
    /// Files, reviews and commits open to the right of the chat.
    pub file_pane: file_pane::FilePane,
    /// What each thread's agent changed, and the store that records it.
    pub checkpoints: session_review::Checkpoints,
    /// The Quit confirmation is open (agents are running).
    pub quit_confirm_open: bool,
    /// The loaded review of each diff tab in `file_pane`, by tab key.
    pub diff_docs: HashMap<String, crate::ui::diff_viewer::DiffDoc>,
    /// Whether ⌘W closes a tab of `file_pane` rather than the thread.
    pub file_pane_focused: bool,
    /// The chat's and the file pane's shares of the width.
    pub file_pane_shares: [f32; 2],
    /// The sidebar's Sessions tab: filters, picks, inline renames.
    pub sessions_ui: crate::ui::sidebar_sessions::SessionsUi,
    /// The sidebar's width (MonoCode keeps it for the run, not on disk).
    pub sidebar_width: f32,
    pub sidebar_resizing: bool,
    /// MonoCode `session_reminders`, every project's, soonest first.
    pub reminders: Vec<crate::db::Reminder>,
    /// Listing reminders failed ("Couldn’t load reminders." + Retry).
    pub reminder_error: Option<String>,
    /// Bumped by every reminder reload; a result of an older one is dropped.
    pub reminder_generation: u64,
    /// A reminder action failed (MonoCode's "Reminder" error dialog).
    pub reminder_failure: Option<String>,
    /// MonoCode `LinkSessionWorkItemDialog`, while open.
    pub link_dialog: Option<crate::ui::link_dialog::LinkDialog>,
    pub link_input: Entity<TextInput>,
    /// MonoCode `monocode.sidebarTabOrder`.
    pub sidebar_tab_order: Vec<SidebarMode>,
    /// Where the sash was pressed (window x).
    pub sidebar_drag_x: f32,
    /// The card, folder or filter menu open in the sidebar.
    pub sidebar_menu: Option<crate::ui::sidebar_menus::SidebarMenu>,
    /// Focused while cards are picked, so F2 / ⌫ / Esc reach them.
    pub session_list_focus: gpui::FocusHandle,
    pub permission_mode: PermissionMode,
    pub sidebar_mode: SidebarMode,
    pub search_query: String,
    /// Model key in MonoCode's `harness:model` form, e.g. `claude:opus`.
    pub selected_model: String,
    /// Installed harness CLIs, probed once at startup (never from render).
    pub harnesses: Vec<HarnessInfo>,
    pub is_model_picker_open: bool,
    pub is_permission_picker_open: bool,
    pub is_plus_menu_open: bool,
    pub is_branch_picker_open: bool,
    /// MonoCode `WorktreeBasePicker` (the "From main" chip) is open.
    pub is_base_picker_open: bool,
    /// The branch / base popover's state, its search, and the New branch
    /// dialog (MonoCode `BranchPicker`, `CreateBranchDialog`).
    pub branch_picker: crate::ui::composer::branch_picker::BranchPickerUi,
    pub branch_search_input: Entity<TextInput>,
    pub branch_create_open: bool,
    pub branch_create_input: Entity<TextInput>,
    /// MonoCode Inbox (GitHub source): the fetched list, filters,
    /// selection and loaded bodies, and its search field.
    pub inbox: crate::ui::inbox_view::InboxState,
    pub inbox_search_input: Entity<TextInput>,
    pub inbox_comment_input: Entity<TextInput>,
    /// The Inbox list's focus, for its ↑/↓ keys.
    pub inbox_focus: gpui::FocusHandle,
    /// A branch switch git refused because of local changes, awaiting "Stash & switch".
    pub blocked_branch_switch: Option<crate::app::workspace_sync::BranchTarget>,
    pub is_skill_picker_open: bool,
    pub skill_query: String,
    /// The `/` or `@` token at the caret, and the caret last seen.
    pub prompt_token: Option<crate::ui::composer::tokens::Token>,
    pub prompt_caret: usize,
    pub is_mention_picker_open: bool,
    pub mention_query: String,
    /// Highlighted row of the open `/` or `@` picker.
    pub picker_index: usize,
    /// The open picker's list, scrolled to keep the highlight in view.
    pub picker_scroll: gpui::ScrollHandle,
    /// The composer border brightens while the prompt has focus.
    pub prompt_focused: bool,
    /// The model picker's search field and highlighted row.
    pub model_search_input: Entity<TextInput>,
    /// MonoCode `/mcp`: the open server picker, its search, and the servers
    /// tagged in the prompt (shared with the prompt's highlighter).
    pub mcp_picker: Option<crate::ui::composer::McpPicker>,
    pub mcp_search_input: Entity<TextInput>,
    /// MonoCode `/add-to-folder`: the open folder picker's highlighted row,
    /// and its search.
    pub folder_picker: Option<usize>,
    pub folder_search_input: Entity<TextInput>,
    /// MonoCode `SkillPicker` "New skill": the open form and its name field.
    pub skill_draft: Option<crate::ui::composer::new_skill::SkillDraft>,
    pub skill_name_input: Entity<TextInput>,
    pub mcp_tags: std::rc::Rc<std::cell::RefCell<std::sync::Arc<Vec<McpTag>>>>,
    /// Other threads' MCP tags, kept with their drafts (MonoCode
    /// `getComposerMcpTags`).
    pub mcp_tag_drafts: HashMap<String, std::sync::Arc<Vec<McpTag>>>,
    pub model_picker_index: usize,
    pub favorite_models: Vec<String>,
    /// MonoCode's sidebar folders, per project.
    pub session_folders: std::collections::BTreeMap<String, Vec<session_folders::SessionFolder>>,
    pub recent_models: Vec<String>,
    pub last_model_settings: serde_json::Map<String, serde_json::Value>,
    /// Model catalog probes per harness: `None` while one runs, else when
    /// the last one ended.
    pub catalog_probes: HashMap<crate::harness::HarnessKind, Option<std::time::Instant>>,
    /// Find in conversation (⌘F): its field and the open bar.
    pub find_input: Entity<TextInput>,
    /// Every file of the project, for Go to File, `@` and Search.
    pub project_files: crate::app::project_files::ProjectFiles,
    /// The agent's pending questions: each thread's form state, the
    /// "Other" field, and the form's focus for its option keys.
    pub question_ui: HashMap<String, crate::ui::composer::question::QuestionUi>,
    pub question_custom_input: Entity<TextInput>,
    pub question_focus: gpui::FocusHandle,
    /// Where the prompt's `@`s are, for the file icons drawn over them.
    pub mention_marks: Vec<crate::ui::composer::MentionMark>,
    /// Threads stopped by their provider's usage limit.
    pub usage_limits: HashMap<String, crate::ui::composer::usage_limit::UsageLimit>,
    /// The image shown full-window (MonoCode `ImageLightbox`).
    pub lightbox: Option<std::path::PathBuf>,
    /// The composer's inline error (a failed paste or attach, an edit the
    /// provider refused), shown under the chips until the next edit.
    pub composer_error: Option<String>,
    /// File reads in flight per thread, and threads whose send waits for them.
    pub attaching: HashMap<String, usize>,
    pub send_after_attach: HashSet<String>,
    /// MonoCode "Edit and resend": the thread whose last message the
    /// composer holds, and threads whose provider is rewinding for a resend.
    pub editing_last_turn: Option<String>,
    pub edit_rewinding: HashSet<String>,
    /// MonoCode "New worktree": the base chosen per not-yet-started thread
    /// (`""` before the thread exists), and threads whose worktree is being
    /// made for their first send.
    pub new_worktrees: HashMap<String, String>,
    pub preparing_worktrees: HashSet<String>,
    /// MonoCode's composer cards (note, handoff): what a thread's composer
    /// carries until its next send.
    pub composer_cards: HashMap<String, crate::ui::composer::cards::ComposerCard>,
    /// MonoCode `ComposerRunner`: the composer geometry the mascot runs on
    /// (measured by layout, read by `RunnerLayer`), and the setting.
    pub runner_geometry: crate::ui::composer::runner_view::RunnerGeometry,
    pub composer_mascot_off: bool,
    /// MonoCode Appearance › Translucency: the glass panes' tint over the
    /// blurred desktop, and whether the main pane takes it (`ui::glass`).
    pub sidebar_opacity: f32,
    /// Keeps `settings.json` writes one at a time (`app/preferences.rs`).
    pub settings_write: preferences::SettingsWrite,
    /// Appearance › Theme, Color and Layout choices.
    pub appearance: crate::ui::appearance::AppearancePrefs,
    /// Appearance › Accent color: the custom colour picker is open.
    pub accent_picker_open: bool,
    /// Appearance › Chat background: the image as drawn behind the panes.
    pub chat_background: chat_background::ChatBackground,
    /// Each chat pane's place in the pane tree this frame, for the part of
    /// the background it shows.
    pub pane_rects: HashMap<String, crate::ui::layout::LayoutRect>,
    /// The sidebar opened from the icon rail while collapsed (MonoCode's
    /// drawer): the next press elsewhere closes it.
    pub sidebar_drawer_open: bool,
    pub body_glass: bool,
    /// The window background last set, so the blur toggles only on change.
    pub window_background: Option<gpui::WindowBackgroundAppearance>,
    /// The centred composer's last measurements, and a send from it whose
    /// docked composer is still dropping into place.
    pub dock_measure: std::rc::Rc<crate::ui::composer::DockMeasure>,
    pub dock_launch: Option<crate::ui::composer::DockLaunch>,
    /// A question just arrived for the focused thread; focus moves next frame.
    pub question_focus_wanted: bool,
    /// The queued message being edited in place, its field, and threads
    /// whose next message waits for that edit to end.
    pub queue_editing: Option<(String, usize)>,
    pub queue_edit_input: Entity<TextInput>,
    pub queue_held: std::collections::HashSet<String>,
    /// MonoCode `queueStatus: "paused"`: threads stopped with messages
    /// waiting; nothing is sent from their queue until Resume.
    pub queue_paused: std::collections::HashSet<String>,
    /// Go to File (⌘P).
    pub quick_open: crate::ui::quick_open::QuickOpen,
    pub quick_open_input: Entity<TextInput>,
    /// Title-bar tab strip: scroll, sweeps, unseen finishes.
    pub title_strip: crate::ui::titlebar::TitleStrip,
    pub transcript_find: Option<crate::ui::transcript::find::FindState>,
    /// Keyboard focus and highlight of the composer's menus.
    pub composer_menus: crate::ui::composer::MenuState,
    pub drafts: HashMap<String, String>,
    pub expanded_reasoning: std::collections::HashSet<String>,
    pub transcript_ui: crate::ui::transcript::TranscriptUiState,
    /// The text selected in a transcript, and the focus that takes ⌘C.
    pub transcript_selection: Entity<crate::ui::transcript::selection::TranscriptSelection>,
    pub transcript_focus: gpui::FocusHandle,
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
    /// The notes list's scroll, for its scroll bar.
    pub notes_scroll: gpui::UniformListScrollHandle,
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
    /// MonoCode `GitDiffIndex` sync fields and the Graph's history.
    pub git_sync: crate::git::sync::SyncInfo,
    pub git_history: Vec<crate::git::sync::HistoryCommit>,
    pub git_commit_input: Entity<TextInput>,
    /// MonoCode `GitChangesPanel`'s state (busy action, amend, sections,
    /// graph, the branch's pull request).
    pub changes_ui: crate::ui::git_changes_panel::ChangesUi,
    /// Git/filesystem snapshot for the active workspace; see `workspace_sync`.
    pub workspace: WorkspaceCache,
    /// Every rail project's +/- lines (MonoCode `useProjectDiffStats`).
    pub project_stats: project_stats::ProjectStats,
    /// Provider usage for the footer (MonoCode `rateLimitsCache`).
    pub usage: usage::UsageState,
    /// The footer's CPU and memory readout for BenCode itself.
    pub process_monitor: process_monitor::ProcessMonitor,
    /// Provider account profiles (MonoCode `providerAccounts`).
    pub accounts: accounts::AccountsState,
    /// The Add account form's name field.
    pub account_name_input: Entity<TextInput>,
    /// Settings › Accounts' name field, for adding and renaming.
    pub account_editor_input: Entity<TextInput>,
    pub file_tree: crate::ui::file_tree::FileTreeState,
    /// The Explorer's inline name field (MonoCode `NameRow`).
    pub file_dialog_input: Entity<TextInput>,
    /// Focused while the Explorer is clicked, so its keys reach it.
    pub file_tree_focus: gpui::FocusHandle,
    // Universal Search
    pub search_modal_input: Entity<TextInput>,
    pub search_scope: crate::ui::search_view::SearchScope,
    pub search_hits: Vec<crate::ui::search_view::SearchHit>,
    /// The hit list's scroll, for its scroll bar.
    pub search_scroll: gpui::UniformListScrollHandle,
    pub search_active_index: usize,
    // Inbox
    /// Rename/delete dialog opened from the thread list.
    pub session_dialog: Option<crate::ui::sidebar::SessionDialog>,
    /// A left press on a window drag region's background, until the
    /// pointer moves or lets go (`ui/window_drag.rs`).
    pub window_drag_pressed: bool,
    /// Settings › Worktrees: the chosen project and its working copies.
    pub worktrees_page: worktree_lifecycle::WorktreesPage,
    /// Settings › Worktrees: the open "Delete worktree?" dialog.
    pub worktree_deletion: Option<worktree_lifecycle::WorktreeDeletion>,
    /// Settings › Worktrees: the open "Create worktree" dialog.
    pub worktree_creation: Option<worktree_lifecycle::WorktreeCreation>,
    /// The "New branch name" field of `worktree_creation`.
    pub worktree_branch_input: Entity<TextInput>,
    pub rename_input: Entity<TextInput>,
    /// Root focus scope; Ely overlays hand focus back to it.
    pub focus_handle: gpui::FocusHandle,
    // Agent execution
    /// The running turn of each thread that has one, keyed by session id.
    pub runs: HashMap<String, AgentRun>,
    /// Prompts sent while a thread was busy, oldest first.
    pub prompt_queues: HashMap<String, Vec<agent::TurnInput>>,
    /// Files attached in each thread's composer, not sent yet.
    pub composer_attachments: HashMap<String, Vec<crate::harness::Attachment>>,
    /// Threads whose composer has Plan mode / Draft on.
    pub plan_mode: std::collections::HashSet<String>,
    pub draft_mode: std::collections::HashSet<String>,
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
    /// The project rail's menus, dialogs and drags.
    pub rail_ui: crate::ui::rail::RailUi,
    pub theme_preference: crate::settings::ThemePreference,
    /// Run Claude with `disableAllHooks` (MonoCode "Claude Code hooks" off).
    pub claude_hooks_disabled: bool,
    pub tab_history: tab_history::TabHistory,
    /// Set while Back/Forward switches tabs, so the move is not recorded.
    navigating_history: bool,
    pub is_terminal_open: bool,
    pub db: AppDb,
    /// Writes to `db`'s file in order, off the UI thread. `None` while `db` is
    /// the in-memory fallback, which a second connection cannot see.
    pub db_writer: Option<crate::db::DbWriter>,
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
        let db = AppDb::open_default().unwrap_or_else(|err| {
            log::error!("database unavailable ({err:#}); nothing will be saved this launch");
            AppDb::open_fallback()
        });
        let db_writer = db.file_path().and_then(|path| {
            crate::db::DbWriter::open(&path)
                .map_err(|err| {
                    log::error!("database writer unavailable, saving on the UI thread: {err:#}")
                })
                .ok()
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

        // MonoCode paints known `@file` labels in the prompt.
        let mention_index: std::rc::Rc<std::cell::RefCell<std::sync::Arc<MentionIndex>>> =
            Default::default();
        let painted_mentions = mention_index.clone();
        let skill_names: std::rc::Rc<std::cell::RefCell<std::sync::Arc<Vec<String>>>> =
            Default::default();
        let painted_skills = skill_names.clone();
        let mcp_tags: std::rc::Rc<std::cell::RefCell<std::sync::Arc<Vec<McpTag>>>> =
            Default::default();
        let painted_tags = mcp_tags.clone();
        let prompt_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line(1, 6)
                .placeholder(crate::ui::composer::PROMPT_PLACEHOLDER)
                .highlighter(move |text, cx| {
                    let mentions = painted_mentions.borrow().clone();
                    let skills = painted_skills.borrow().clone();
                    let tags = painted_tags.borrow().clone();
                    crate::ui::composer::prompt_highlights(text, &mentions, &skills, &tags, cx)
                })
        });
        let search_input = text_input(window, cx, "Search conversations...");
        let rename_input = text_input(window, cx, "");
        let worktree_branch_input = text_input(window, cx, "feature/my-task");
        let link_input = text_input(window, cx, "https://github.com/owner/repo/pull/123");
        let rail_ui = crate::ui::rail::RailUi::new(window, cx);
        let file_dialog_input = text_input(window, cx, "");
        let account_name_input = text_input(window, cx, "Work or Personal");
        let account_editor_input = text_input(window, cx, "Work or Personal");
        let name_keys_input = file_dialog_input.clone();
        let rename_keys_input = rename_input.clone();
        let model_search_input = text_input(window, cx, "Search models");
        let mcp_search_input = text_input(window, cx, "Search MCP servers…");
        let mcp_keys_input = mcp_search_input.clone();
        let folder_search_input = text_input(window, cx, "Choose or name a session folder…");
        let folder_keys_input = folder_search_input.clone();
        let branch_search_input = text_input(window, cx, "Search or create a branch...");
        let branch_keys_input = branch_search_input.clone();
        let branch_create_input = text_input(window, cx, "feature/my-branch");
        let inbox_search_input = text_input(window, cx, "Filter inbox");
        let inbox_comment_input = multiline_input(window, cx, "Leave a comment (⌘↩)", (2, 8));
        let skill_name_input = text_input(window, cx, "skill-name");
        let skill_keys_input = skill_name_input.clone();
        let find_input = text_input(window, cx, "Find in conversation");
        let find_keys_input = find_input.clone();
        let quick_open_input = text_input(window, cx, "Go to File (type > for commands)");
        let quick_keys_input = quick_open_input.clone();
        let question_custom_input = text_input(window, cx, "Type your answer");
        let question_focus = cx.focus_handle();
        let question_keys_focus = question_focus.clone();
        let queue_edit_input = text_input(window, cx, "Edit queued message");
        let queue_keys_input = queue_edit_input.clone();
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
        let git_commit_input = multiline_input(window, cx, "Message (⌘↩ to commit)", (1, 7));
        let search_modal_input =
            text_input(window, cx, "Search conversations, files, projects... (⌘K)");

        let composer_input = prompt_input.clone();
        let model_input = model_search_input.clone();
        let menu_focus = cx.focus_handle();
        let menu_keys_focus = menu_focus.clone();
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
            // A click or an arrow key moves the caret without an edit; the
            // `/` and `@` pickers still follow it (MonoCode `onSelect`).
            cx.observe(&prompt_input, |this: &mut Self, input, cx| {
                if input.read(cx).cursor() != this.prompt_caret {
                    this.sync_prompt_tokens(cx);
                }
            }),
            // Settings › Worktrees › Create: Create follows the name, Enter submits.
            cx.subscribe(
                &worktree_branch_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed if this.worktree_creation.is_some() => cx.notify(),
                    InputEvent::Submit => this.confirm_worktree_creation(cx),
                    _ => {}
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
                    // MonoCode re-lists the Explorer on focus.
                    this.refresh_file_tree(cx);
                    // MonoCode re-reads stale project stats on focus.
                    this.refresh_project_stats(cx);
                    this.auto_fetch_on_focus(cx);
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                this.on_system_appearance_changed(window.appearance(), cx)
            }),
            cx.subscribe(
                &model_search_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    // MonoCode re-highlights the current model as the list changes.
                    InputEvent::Changed => {
                        this.highlight_current_model(cx);
                        cx.notify();
                    }
                    InputEvent::Submit => this.pick_highlighted_model(cx),
                    _ => {}
                },
            ),
            cx.subscribe(
                &mcp_search_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if *event == InputEvent::Changed {
                        this.on_mcp_query_changed(cx);
                    }
                },
            ),
            cx.subscribe(
                &folder_search_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if *event == InputEvent::Changed {
                        this.on_folder_query_changed(cx);
                    }
                },
            ),
            cx.subscribe(
                &inbox_search_input,
                |_: &mut Self, _, event: &InputEvent, cx| {
                    if *event == InputEvent::Changed {
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &inbox_comment_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.post_inbox_comment(cx),
                    InputEvent::Changed => cx.notify(),
                    _ => {}
                },
            ),
            cx.subscribe(
                &branch_search_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if *event == InputEvent::Changed {
                        this.on_branch_query_changed(cx);
                    }
                },
            ),
            cx.subscribe_in(
                &skill_name_input,
                window,
                |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit => this.create_new_skill(window, cx),
                    InputEvent::Changed => {
                        if let Some(draft) = &mut this.skill_draft {
                            draft.error = None;
                        }
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe(
                &question_custom_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => this.on_question_custom_changed(cx),
                    InputEvent::Submit => {
                        if let Some(id) = this.selected_session_id.clone() {
                            this.continue_question(&id, cx);
                        }
                    }
                    _ => {}
                },
            ),
            cx.subscribe(
                &queue_edit_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if *event == InputEvent::Submit {
                        this.save_queue_edit(cx);
                    }
                },
            ),
            cx.subscribe(
                &quick_open_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if *event == InputEvent::Changed {
                        this.on_quick_open_query_changed(cx);
                    }
                },
            ),
            cx.subscribe(
                &find_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => this.on_find_query_changed(cx),
                    InputEvent::Submit => this.step_find(1, cx),
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
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.commit_staged_changes(cx),
                    // The Commit button follows the message.
                    InputEvent::Changed => cx.notify(),
                    _ => {}
                },
            ),
            // MonoCode `AddProviderAccount`: Enter submits the form.
            cx.subscribe(
                &account_name_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => {
                        if let Some(provider) = this.usage.popover {
                            this.add_provider_account(provider, cx);
                        }
                    }
                    InputEvent::Changed => cx.notify(),
                    _ => {}
                },
            ),
            // MonoCode `ProviderAccountEditor`: Enter submits the form.
            cx.subscribe(
                &account_editor_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.submit_account_editor(cx),
                    InputEvent::Changed => cx.notify(),
                    _ => {}
                },
            ),
            // MonoCode `NameRow`: Enter commits; blur commits unless wrong.
            cx.subscribe(
                &file_dialog_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.commit_tree_edit(false, cx),
                    InputEvent::Blur if this.tree_edit_active() => this.commit_tree_edit(true, cx),
                    InputEvent::Changed => {
                        this.file_tree.edit_state.submit_error = None;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe(
                &link_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.submit_link_dialog(cx),
                    InputEvent::Changed => {
                        if let Some(dialog) = this.link_dialog.as_mut()
                            && dialog.error.take().is_some()
                        {
                            cx.notify();
                        }
                    }
                    _ => {}
                },
            ),
            // MonoCode's inline renames commit on Enter and on blur.
            cx.subscribe(
                &rename_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Submit | InputEvent::Blur)
                        && this.inline_rename_active()
                    {
                        this.commit_inline_rename(cx);
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
        // Every quit path (⌘Q, Dock, logout) ends here. The work happens in the
        // callback itself: GPUI polls the returned future for only 200 ms.
        subscriptions.push(cx.on_app_quit(|this, _cx| {
            this.interrupt_runs_for_quit();
            async {}
        }));
        // Closing the window drops the app without quitting (macOS).
        subscriptions.push(cx.on_release(|this, _cx| this.interrupt_runs_for_quit()));

        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            // The text input binds these keys itself, deeper than any action
            // context, so the composer's Enter and the picker keys
            // (MonoCode `Composer.tsx:1742-1830`) are taken here first.
            let pasting = event.keystroke.key == "v"
                && event.keystroke.modifiers.platform
                && !event.keystroke.modifiers.shift;
            if pasting && composer_input.read(cx).focus_handle(cx).is_focused(window) {
                let attached = weak_app.update(cx, |this, cx| this.paste_into_composer(cx));
                if matches!(attached, Ok(true)) {
                    cx.stop_propagation();
                }
                return;
            }
            // MonoCode `NameRow`: Esc cancels.
            if event.keystroke.key == "escape"
                && name_keys_input.read(cx).focus_handle(cx).is_focused(window)
            {
                let cancelled = weak_app.update(cx, |this, cx| {
                    let active = this.tree_edit_active();
                    this.cancel_tree_edit(cx);
                    active
                });
                if matches!(cancelled, Ok(true)) {
                    cx.stop_propagation();
                }
                return;
            }
            // MonoCode's inline renames: Esc cancels.
            if event.keystroke.key == "escape"
                && rename_keys_input.read(cx).focus_handle(cx).is_focused(window)
            {
                let cancelled = weak_app.update(cx, |this, cx| {
                    let active = this.inline_rename_active();
                    this.cancel_inline_rename(cx);
                    active
                });
                if matches!(cancelled, Ok(true)) {
                    cx.stop_propagation();
                }
                return;
            }
            // MonoCode `QuestionForm`: the option keys while the form has focus.
            if question_keys_focus.is_focused(window) && !event.keystroke.modifiers.modified() {
                let key = event.keystroke.key.clone();
                if matches!(
                    weak_app.update(cx, |this, cx| this.question_key(&key, cx)),
                    Ok(true)
                ) {
                    cx.stop_propagation();
                }
                return;
            }
            // MonoCode `MessageQueue`: Esc cancels an edit in place.
            if event.keystroke.key == "escape"
                && queue_keys_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            {
                if matches!(
                    weak_app.update(cx, |this, cx| this.cancel_queue_edit(cx)),
                    Ok(true)
                ) {
                    cx.stop_propagation();
                }
                return;
            }
            // MonoCode `FilePicker`: ↑/↓, Enter, Esc and Tab belong to the picker.
            if quick_keys_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
            {
                let key = event.keystroke.key.as_str();
                if event.keystroke.modifiers.modified()
                    || !matches!(key, "up" | "down" | "enter" | "escape" | "tab")
                {
                    return;
                }
                let handled = weak_app.update(cx, |this, cx| {
                    let handled = this.quick_open_key(key, cx);
                    this.open_pending_quick_open(window, cx);
                    handled
                });
                if matches!(handled, Ok(true)) {
                    cx.stop_propagation();
                }
                return;
            }
            // MonoCode `McpServerPicker`: ↑/↓, Enter and Esc in its search.
            if mcp_keys_input.read(cx).focus_handle(cx).is_focused(window) {
                let key = event.keystroke.key.as_str();
                if !event.keystroke.modifiers.modified()
                    && matches!(
                        weak_app.update(cx, |this, cx| this.mcp_picker_key(key, cx)),
                        Ok(true)
                    )
                {
                    cx.stop_propagation();
                }
                return;
            }
            // MonoCode `BranchPicker`: ↑/↓, Enter and Esc in its search.
            if branch_keys_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
            {
                let key = event.keystroke.key.as_str();
                if !event.keystroke.modifiers.modified()
                    && matches!(
                        weak_app.update(cx, |this, cx| this.branch_picker_key(key, cx)),
                        Ok(true)
                    )
                {
                    cx.stop_propagation();
                }
                return;
            }
            // MonoCode `CreateSkillForm`: Esc goes back to the list.
            if event.keystroke.key == "escape"
                && skill_keys_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            {
                if matches!(
                    weak_app.update(cx, |this, cx| this.cancel_new_skill(cx)),
                    Ok(true)
                ) {
                    cx.stop_propagation();
                }
                return;
            }
            // MonoCode `SessionFolderPicker`: ↑/↓, Enter and Esc in its search.
            if folder_keys_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
            {
                let key = event.keystroke.key.as_str();
                if !event.keystroke.modifiers.modified()
                    && matches!(
                        weak_app.update(cx, |this, cx| this.folder_picker_key(key, cx)),
                        Ok(true)
                    )
                {
                    cx.stop_propagation();
                }
                return;
            }
            // MonoCode `TranscriptFind`: Enter steps, ⇧Enter steps back, Esc closes.
            if find_keys_input.read(cx).focus_handle(cx).is_focused(window) {
                let back = event.keystroke.modifiers.shift;
                let handled = weak_app.update(cx, |this, cx| match event.keystroke.key.as_str() {
                    "enter" => {
                        this.step_find(if back { -1 } else { 1 }, cx);
                        true
                    }
                    "escape" => this.close_find(cx),
                    _ => false,
                });
                if matches!(handled, Ok(true)) {
                    cx.stop_propagation();
                }
                return;
            }
            if event.keystroke.modifiers.modified() {
                return;
            }
            let key = event.keystroke.key.as_str();
            if !matches!(
                key,
                "enter" | "escape" | "up" | "down" | "left" | "right" | "tab" | "space"
            ) {
                return;
            }
            if menu_keys_focus.is_focused(window) {
                let handled = weak_app.update(cx, |this, cx| this.handle_menu_key(key, cx));
                if matches!(handled, Ok(true)) {
                    cx.stop_propagation();
                }
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
        let selected_model = catalog::default_model(&harnesses).key;

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
            file_pane: Default::default(),
            checkpoints: session_review::Checkpoints::new(crate::git::checkpoint::CheckpointStore::new(
                crate::git::checkpoint::CheckpointStore::default_dir(),
            )),
            diff_docs: HashMap::new(),
            file_pane_focused: false,
            file_pane_shares: [1.0, 1.0],
            sessions_ui: Default::default(),
            sidebar_menu: None,
            sidebar_width: crate::ui::sidebar::SIDEBAR_MIN_WIDTH,
            sidebar_resizing: false,
            reminders: Vec::new(),
            reminder_error: None,
            reminder_generation: 0,
            reminder_failure: None,
            link_dialog: None,
            link_input,
            sidebar_tab_order: crate::ui::sidebar::parse_tab_order(&[]),
            sidebar_drag_x: 0.0,
            session_list_focus: cx.focus_handle(),
            permission_mode: PermissionMode::default(),
            sidebar_mode: SidebarMode::Sessions,
            search_query: String::new(),
            selected_model,
            harnesses,
            is_model_picker_open: false,
            is_permission_picker_open: false,
            is_plus_menu_open: false,
            is_branch_picker_open: false,
            is_base_picker_open: false,
            branch_picker: Default::default(),
            branch_search_input,
            branch_create_open: false,
            branch_create_input,
            inbox: Default::default(),
            inbox_search_input,
            inbox_comment_input,
            inbox_focus: cx.focus_handle(),
            blocked_branch_switch: None,
            is_skill_picker_open: false,
            skill_query: String::new(),
            prompt_token: None,
            prompt_caret: 0,
            is_mention_picker_open: false,
            mention_query: String::new(),
            picker_index: 0,
            picker_scroll: gpui::ScrollHandle::new(),
            prompt_focused: false,
            model_search_input,
            mcp_picker: None,
            mcp_search_input,
            folder_picker: None,
            folder_search_input,
            skill_draft: None,
            skill_name_input,
            mcp_tags,
            mcp_tag_drafts: HashMap::new(),
            model_picker_index: 0,
            favorite_models: Vec::new(),
            session_folders: Default::default(),
            recent_models: Vec::new(),
            last_model_settings: Default::default(),
            catalog_probes: Default::default(),
            composer_menus: crate::ui::composer::MenuState::new(menu_focus),
            find_input,
            transcript_find: None,
            title_strip: Default::default(),
            quick_open: Default::default(),
            question_ui: HashMap::new(),
            question_custom_input,
            question_focus,
            question_focus_wanted: false,
            composer_error: None,
            attaching: HashMap::new(),
            send_after_attach: HashSet::new(),
            editing_last_turn: None,
            edit_rewinding: HashSet::new(),
            new_worktrees: HashMap::new(),
            preparing_worktrees: HashSet::new(),
            composer_cards: HashMap::new(),
            runner_geometry: Default::default(),
            composer_mascot_off: false,
            sidebar_opacity: crate::ui::glass::OPACITY_DEFAULT,
            settings_write: Default::default(),
            appearance: Default::default(),
            accent_picker_open: false,
            chat_background: Default::default(),
            pane_rects: HashMap::new(),
            sidebar_drawer_open: false,
            body_glass: true,
            window_background: None,
            lightbox: None,
            usage_limits: HashMap::new(),
            mention_marks: Vec::new(),
            dock_measure: Default::default(),
            dock_launch: None,
            queue_editing: None,
            queue_edit_input,
            queue_held: Default::default(),
            queue_paused: Default::default(),
            project_files: crate::app::project_files::ProjectFiles::new(mention_index),
            quick_open_input,
            drafts: HashMap::new(),
            expanded_reasoning: std::collections::HashSet::new(),
            transcript_ui: Default::default(),
            transcript_selection: cx.new(|_| Default::default()),
            transcript_focus: cx.focus_handle(),
            terminals: Default::default(),
            settings_tab: SettingsTab::Providers,
            surface: None,
            settings_return: None,
            notes,
            selected_note_id,
            note_filter_query: String::new(),
            notes_scroll: Default::default(),
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
            git_sync: Default::default(),
            git_history: Vec::new(),
            git_commit_input,
            changes_ui: Default::default(),
            workspace: WorkspaceCache::default(),
            project_stats: Default::default(),
            usage: Default::default(),
            process_monitor: Default::default(),
            accounts: Default::default(),
            account_name_input,
            account_editor_input,
            file_tree: Default::default(),
            file_dialog_input,
            file_tree_focus: cx.focus_handle(),
            search_modal_input,
            search_scope: crate::ui::search_view::SearchScope::All,
            search_hits: Vec::new(),
            search_scroll: Default::default(),
            search_active_index: 0,
            session_dialog: None,
            window_drag_pressed: false,
            worktrees_page: Default::default(),
            worktree_deletion: None,
            worktree_creation: None,
            worktree_branch_input,
            rename_input,
            focus_handle: cx.focus_handle(),
            runs: HashMap::new(),
            prompt_queues: HashMap::new(),
            composer_attachments: HashMap::new(),
            plan_mode: Default::default(),
            draft_mode: Default::default(),
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
            integrations: integrations::Integrations {
                skill_names,
                ..Default::default()
            },
            automation_time_error: None,
            search_focus_pending: false,
            search_submit: None,
            note_pending_delete: None,
            note_autosave_generation: 0,
            note_save_error: None,
            automation_pending_delete: None,
            quit_confirm_open: false,
            // [editor-pane init]
            editor: Default::default(),
            is_sidebar_open: true,
            is_rail_open: true,
            rail_ui,
            theme_preference: Default::default(),
            claude_hooks_disabled: false,
            tab_history: Default::default(),
            navigating_history: false,
            is_terminal_open: true,
            db,
            db_writer,
            _subscriptions: subscriptions,
        };
        app.apply_settings(saved);
        app.start_git_poll(cx);
        app.start_auto_fetch(cx);
        app.start_project_stats_poll(cx);
        app.start_session_age_tick(cx);
        app.start_reminder_poll(cx);
        app.load_folder_members(cx);
        app.start_clock(cx);
        app.start_usage_clock(cx);
        app.start_process_monitor(cx);
        app.load_account_profiles(cx);
        app.refresh_installed_catalogs(cx);
        app.start_inbox_poll(cx);
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
        self.selected_model = option.key.clone();
        self.record_recent_model(&option.key, cx);

        // MonoCode `onModelChange`: the old thread's settings fill the
        // remembered ones, and the new model takes what it accepts.
        let current = self
            .selected_session()
            .and_then(|s| s.model_settings.clone());
        if let Some(current) = &current {
            self.save_last_model_settings(current, true, cx);
        }
        let settings = self.preferred_model_settings(&option.key, current.as_ref());
        let harness_id = option.harness.id();
        let changed_session = self.selected_session_mut().map(|session| {
            if session.harness != harness_id {
                // A provider session id is only meaningful to the harness that
                // issued it, and so is the account it belongs to.
                session.provider_session_id = None;
                session.provider_account_id = None;
            }
            if session.model != option.key {
                // MonoCode `dropContextWindow`: the old model's window no
                // longer measures this thread.
                session.context_window = None;
                session.context_used = None;
            }
            session.model = option.key.clone();
            session.harness = harness_id.to_string();
            session.model_settings = Some(settings);
            session.id.clone()
        });
        if let Some(id) = changed_session {
            self.persist_session(&id);
        }
        cx.notify();
    }

    pub fn on_prompt_changed(&mut self, cx: &mut Context<Self>) {
        self.composer_error = None;
        self.skill_draft = None;
        let text = self.prompt_input.read(cx).text().to_string();
        // MonoCode `runsSessionFolderCommandOnSpace`.
        if text.trim_start() == "/add-to-folder " && self.selected_session_id.is_some() {
            self.is_skill_picker_open = false;
            self.is_mention_picker_open = false;
            self.start_folder_command(cx);
            return;
        }
        // MonoCode drops a tag once its token leaves the text.
        let tags = self.mcp_tags.borrow().clone();
        if !tags.is_empty() {
            let kept: Vec<McpTag> = tags
                .iter()
                .filter(|tag| {
                    !crate::ui::composer::mcp_tags::tagged_servers(&text, std::slice::from_ref(tag))
                        .is_empty()
                })
                .cloned()
                .collect();
            if kept.len() != tags.len() {
                *self.mcp_tags.borrow_mut() = kept.into();
            }
        }
        self.sync_prompt_tokens(cx);
    }

    /// MonoCode `syncTokensFromTextarea`: the `/` or `@` picker follows the
    /// token at the caret, after every edit and caret move.
    pub fn sync_prompt_tokens(&mut self, cx: &mut Context<Self>) {
        let input = self.prompt_input.read(cx);
        let (text, caret) = (input.text().to_string(), input.cursor());
        self.prompt_caret = caret;
        if self.skill_draft.is_some() {
            return;
        }
        let was = (self.is_skill_picker_open, self.is_mention_picker_open);
        let slash = crate::ui::composer::tokens::slash_token_at(&text, caret);
        let mention = slash
            .is_none()
            .then(|| crate::ui::composer::tokens::mention_token_at(&text, caret))
            .flatten();
        self.is_skill_picker_open = slash.is_some();
        self.is_mention_picker_open = mention.is_some();
        let query = slash
            .as_ref()
            .or(mention.as_ref())
            .map(|t| t.query.to_lowercase());
        let changed = match (&slash, &mention) {
            (Some(_), _) => self.skill_query != query.clone().unwrap_or_default(),
            (_, Some(_)) => self.mention_query != query.clone().unwrap_or_default(),
            _ => false,
        };
        if changed || was != (self.is_skill_picker_open, self.is_mention_picker_open) {
            self.picker_index = 0;
        }
        if let Some(query) = query {
            if slash.is_some() {
                self.skill_query = query;
            } else {
                self.mention_query = query;
            }
        }
        // MonoCode re-lists the project as the `@` picker opens.
        if self.is_mention_picker_open && !was.1 {
            self.index_project_files(cx);
        }
        self.prompt_token = slash.or(mention);
        cx.notify();
    }

    /// Puts `text` in the prompt with the caret at `caret`.
    pub fn set_prompt(&mut self, text: String, caret: usize, cx: &mut Context<Self>) {
        self.prompt_input.update(cx, |input, cx| {
            input.set_text(text, cx);
            let caret = caret.min(input.text().len());
            input.select(caret..caret, cx);
        });
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
            // MonoCode: ↑ in an empty composer (no text, files or card)
            // edits the last message where the provider can rewind.
            ("up", false) if self.is_editing_last_turn() || !self.composer_has_value(cx) => {
                match self.selected_session_id.clone() {
                    Some(id)
                        if self.prompt_input.read(cx).text().is_empty()
                            && (self.is_editing_last_turn() || self.can_edit_last_turn(&id)) =>
                    {
                        self.toggle_edit_last_turn(&id, cx);
                        true
                    }
                    _ => false,
                }
            }
            ("up", true) => self.move_picker(-1, cx),
            ("down", true) => self.move_picker(1, cx),
            // MonoCode swallows Tab even when nothing matches.
            ("tab", true) => {
                self.accept_picker(cx);
                true
            }
            // MonoCode runs a lone `/compact`, `/mcp` or `/add-to-folder`
            // before the picker takes Enter.
            ("enter", true)
                if crate::ui::composer::mode_commands::standalone_command(
                    self.prompt_input.read(cx).text(),
                )
                .is_some() =>
            {
                self.close_pickers(cx);
                self.submit_prompt(cx);
                true
            }
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
        self.skill_draft = None;
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

    /// Replaces the `/` or `@` token at the caret with `replacement`,
    /// keeping the text after it (MonoCode `replaceSlashToken`).
    fn replace_trigger(&mut self, _trigger: char, replacement: &str, cx: &mut Context<Self>) {
        let Some(token) = self.prompt_token.take() else {
            return;
        };
        let text = self.prompt_input.read(cx).text().to_string();
        let (next, caret) = crate::ui::composer::tokens::replace_token(&text, &token, replacement);
        self.set_prompt(next, caret, cx);
    }

    /// Takes the `/` token at the caret out of the prompt; returns where it
    /// stood (the end of the text when there was none).
    pub fn remove_prompt_token(&mut self, cx: &mut Context<Self>) -> usize {
        let text = self.prompt_input.read(cx).text().to_string();
        let Some(token) = self.prompt_token.take() else {
            return text.len();
        };
        let (next, caret) = crate::ui::composer::tokens::remove_token(&text, &token);
        self.set_prompt(next, caret, cx);
        caret
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

    /// A file dragged from the explorer attaches like one from the Finder
    /// (MonoCode `onExplorerFilePointerDrag`).
    pub fn attach_file_to_composer(
        &mut self,
        session_id: &str,
        rel_path: &str,
        cx: &mut Context<Self>,
    ) {
        let path = std::path::Path::new(&self.workspace_cwd()).join(rel_path);
        self.attach_external_paths_to_composer(session_id, &[path], cx);
    }

    /// Files dropped from the Finder attach to the composer (MonoCode).
    pub fn attach_external_paths_to_composer(
        &mut self,
        session_id: &str,
        paths: &[std::path::PathBuf],
        cx: &mut Context<Self>,
    ) {
        self.active_file_drop_target = None;
        self.focus_pane(session_id.to_string(), cx);
        self.attach_paths(paths.to_vec(), cx);
    }

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

    /// Runs `job` on the writer, or right here on the fallback database.
    pub(crate) fn db_write(
        &self,
        what: &'static str,
        job: impl FnOnce(&AppDb) -> anyhow::Result<()> + Send + 'static,
    ) {
        match &self.db_writer {
            Some(writer) => writer.run_logged(what, job),
            None => {
                if let Err(err) = job(&self.db) {
                    log::error!("database {what} failed: {err:#}");
                }
            }
        }
    }

    /// Runs `job` on the writer (after every queued write) and hands back its
    /// result; on the fallback database it runs now.
    pub(crate) fn db_read<T: Send + 'static>(
        &self,
        job: impl FnOnce(&AppDb) -> T + Send + 'static,
    ) -> tokio::sync::oneshot::Receiver<T> {
        match &self.db_writer {
            Some(writer) => writer.run(job),
            None => {
                let (done, result) = tokio::sync::oneshot::channel();
                // The receiver is right here, so this cannot fail.
                if done.send(job(&self.db)).is_err() {
                    log::trace!("database result dropped");
                }
                result
            }
        }
    }

    /// Waits for queued writes before a synchronous read or write of the
    /// same rows on `db`. Short in practice: the queue is usually empty.
    pub(crate) fn settle_db_writes(&self) {
        if let Some(writer) = &self.db_writer
            && !writer.flush(std::time::Duration::from_secs(2))
        {
            log::warn!("queued database writes are still running");
        }
    }
}

impl Render for BenCodeApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.apply_ui_scale(window);
        self.sync_chat_background(!cx.theme().is_dark(), cx);
        if std::mem::take(&mut self.question_focus_wanted) {
            window.focus(&self.question_focus, cx);
        }
        if let Some(path) = self.file_tree.pending_open.take() {
            self.open_file_in_editor(&path, window, cx);
        }
        self.sync_mention_marks(window, cx);
        // The runner layer reads what this layout measures.
        self.runner_geometry.clear();
        if !self.focus_handle.contains_focused(window, cx) && window.focused(cx).is_none() {
            window.focus(&self.focus_handle, cx);
        }
        self.sync_window_glass(window, cx);
        let glass = self.glass(cx);
        let colors = &cx.theme().colors;
        let (bg, fg) = (colors.bg, colors.fg);
        // Search, Inbox, Notes, Automations and Settings replace the
        // sidebar and the workspace column (MonoCode in-shell views).
        let surface = self.render_surface(cx);
        let workspace_visible = surface.is_none();
        let compact_rail = self.compact_rail_active();
        let title_bar_above = self.compact_title_bar();

        // Full size explicitly: the app is laid out inside `WindowRoot`'s
        // cached slot, not as the window's root.
        FocusScope::new(&self.focus_handle)
            .root()
            .size_full()
            .child(
                Self::bind_commands(div().id("bencode-root"), cx)
                    .on_drag_move::<crate::ui::sidebar::SidebarResize>(cx.listener(
                        |this, event: &gpui::DragMoveEvent<crate::ui::sidebar::SidebarResize>, window, cx| {
                            let drag = crate::ui::sidebar::SidebarResize {
                                start_x: this.sidebar_drag_x,
                                ..event.drag(cx).clone()
                            };
                            let width = crate::ui::sidebar::resized_width(
                                &drag,
                                crate::ui::scale::logical(event.event.position.x),
                                crate::ui::scale::logical(window.viewport_size().width),
                            );
                            if width != this.sidebar_width || !this.sidebar_resizing {
                                this.sidebar_width = width;
                                this.sidebar_resizing = true;
                                cx.notify();
                            }
                        },
                    ))
                    .on_mouse_up(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if std::mem::take(&mut this.sidebar_resizing) {
                                cx.notify();
                            }
                        }),
                    )
                    .relative()
                    .flex()
                    .flex_col()
                    .size_full()
                    .bg(glass.root(bg))
                    .text_color(fg)
                    // MonoCode `compactTitleBar`: above the icon rail.
                    .when(title_bar_above, |el| el.child(self.render_titlebar(window, cx)))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .size_full()
                            .min_h_0()
                            .overflow_hidden()
                            // Column 1: Leftmost Project Rail, or its icons
                            .when(self.is_rail_open, |el| {
                                el.child(self.render_project_rail(cx))
                            })
                            .when(compact_rail, |el| el.child(self.render_compact_rail(cx)))
                            // Column 2: Workspace Sidebar (when open, or
                            // drawn out from the icon rail)
                            .when(self.sidebar_shown() && workspace_visible, |el| {
                                let drawer = self.sidebar_is_drawer();
                                el.child(
                                    div()
                                        .flex()
                                        .flex_none()
                                        .h_full()
                                        .when(drawer, |el| {
                                            el.on_mouse_down_out(cx.listener(
                                                |this, event: &gpui::MouseDownEvent, _, cx| {
                                                    this.dismiss_sidebar_drawer(event, cx)
                                                },
                                            ))
                                        })
                                        .child(self.render_sidebar(cx)),
                                )
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
                                        .when(!title_bar_above, |el| {
                                            el.child(self.render_titlebar(window, cx))
                                        })
                                        // MonoCode `body-glass`: the pane under
                                        // the title bar.
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .flex_1()
                                                .min_h_0()
                                                .bg(glass.body(bg))
                                                .child(self.render_workspace_with_dock(window, cx))
                                                .child(self.render_usage_footer(cx)),
                                        ),
                                )
                            })
                            .children(surface),
                    )
                    .children(self.render_sidebar_menu(cx))
                    .children(self.render_rail_overlays(cx))
                    .children(self.render_tree_menu(cx))
                    .children(self.render_terminal_menu(cx))
                    .children(self.render_git_menu(cx))
                    .children(self.render_link_dialog(cx))
                    .children(self.render_reminder_notices(cx))
                    .children(self.render_session_dialog(cx))
                    .children(self.render_worktree_deletion(cx))
                    .children(self.render_account_removal(cx))
                    .children(self.render_worktree_creation(cx))
                    .children(self.render_quick_open(cx))
                    .children(self.render_lightbox(cx))
                    .children(self.render_git_confirm(cx))
                    .children(self.render_session_undo_confirm(cx))
                    .children(self.render_branch_switch_confirm(cx))
                    .children(self.render_branch_create_dialog(cx))
                    .children(self.render_pr_action_confirm(cx))
                    .children(self.render_file_tree_dialog(cx))
                    .children(self.render_quit_confirm(cx)),
            )
    }
}
