pub mod accounts;
mod agent;
pub mod agy_accounts;
pub mod alerts;
pub mod app_calls;
pub mod artifacts;
mod automation_runs;
pub mod automations;
pub mod backlog;
pub mod blame;
pub mod browser;
pub mod chat_background;
pub mod cli_import;
pub mod commands;
mod composer_input;
pub mod file_pane;
pub mod frame_bench;
pub mod git_actions;
pub mod github_accounts;
pub mod go_to_definition;
pub mod habits;
pub mod harness_updates;
pub mod ids;
pub mod in_flight;
mod inbox_attention;
mod integrations;
pub mod item_threads;
pub mod live_agents;
mod model_catalog;
pub mod mono_background;
pub mod mono_board;
pub mod mono_changes;
pub mod mono_files;
pub mod mono_menu_bar;
pub mod mono_overview;
pub mod mono_sessions;
pub mod monos;
pub mod note_images;
pub mod notes;
mod panes;
mod preferences;
pub mod project_files;
pub mod project_search;
mod project_stats;
mod projects;
pub mod release_notes;
pub mod reminders;
pub mod remote;
pub mod remote_runs;
mod session_flags;
pub mod session_folders;
pub mod session_list;
pub mod session_review;
#[cfg(feature = "snapshot")]
pub mod snapshot;
mod source_control;
pub mod subagents;
mod surfaces;
mod tab_history;
mod tab_scope;
pub mod task_list;
pub mod thread_state;
pub mod updater;
pub mod usage;
pub mod work_item_link;
mod workspace_nav;
pub mod workspace_sync;
pub mod worktree_lifecycle;

use std::collections::HashMap;

use crate::ui::composer::mcp_tags::McpTag;
use crate::ui::composer::mentions::MentionIndex;
use crate::ui::composer::{ComposerPopover, TokenPicker};
use bencode_ui::forms::{InputEvent, TextInput};
use bencode_ui::primitives::FocusScope;
use bencode_ui::theme::ActiveTheme;
use gpui::{
    AnyElement, AppContext, Context, Entity, Focusable, IntoElement, ParentElement, Render,
    StyleRefinement, Styled, Subscription, Window, div, prelude::*,
};

pub use agent::{AgentRun, NEW_SESSION_TITLE, QUESTION_TOOL, TurnInput, can_compact, now_ms};
pub use ids::unique_id;
pub use preferences::{appearance_prefs, is_dark_appearance, theme_mode};
pub use projects::{is_home_path, is_path_in_project, normalize_project_path, same_project_path};
pub use surfaces::Surface;
pub use workspace_sync::WorkspaceCache;

use crate::db::{AppDb, SessionRow};
use crate::harness::{HarnessInfo, HarnessResolver, catalog};
use crate::startup_trace::mark as trace;
use crate::ui::region::Region;
use crate::ui::settings_modal::SettingsTab;

const INITIAL_OPEN_TABS: usize = 3;

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

/// What the inspector, at the workspace's right, shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InspectorMode {
    #[default]
    Changes,
    Files,
}

/// The working copy a project's workspace is narrowed to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeFocus {
    pub path: String,
    pub branch: Option<String>,
    /// Picked in the thread list's switcher, so the list and the title
    /// bar's tabs narrow to it. A focus that followed a thread opened there
    /// (or the composer's workspace menu) leaves them whole.
    pub narrows: bool,
}

pub struct BenCodeApp {
    pub sessions: Vec<SessionRow>,
    pub selected_session_id: Option<String>,
    /// Open workspace tabs, each with its split layout and focused pane.
    pub tabs: crate::ui::layout::TabSet,
    /// Files, reviews and commits open to the right of the chat.
    pub file_pane: file_pane::FilePane,
    /// A file pane tab's right-click menu.
    pub file_pane_menu: Option<crate::ui::file_pane_menu::PaneTabMenu>,
    /// What each thread's agent changed, and the store that records it.
    pub checkpoints: session_review::Checkpoints,
    /// The Quit confirmation is open (agents are running).
    pub quit_confirm_open: bool,
    /// The release feed, the update in progress, and the "Updated to" note.
    pub updater: updater::UpdaterState,
    /// The loaded review of each diff tab in `file_pane`, by tab key.
    pub diff_docs: HashMap<String, crate::ui::diff_viewer::DiffDoc>,
    /// Reviews show the old file beside the new one, saved in settings.
    pub diff_split: bool,
    /// Whether ⌘W closes a tab of `file_pane` rather than the thread.
    pub file_pane_focused: bool,
    /// The chat's and the file pane's shares of the width.
    pub file_pane_shares: [f32; 2],
    /// The file pane sits left of the chat, saved in settings.
    pub file_pane_left: bool,
    /// The in-app browser's tabs and the agents' way to them.
    pub browser: browser::BrowserState,
    /// `bencode app`: its socket and the credentials turns carry.
    pub app_access: app_calls::AppAccessState,
    /// Monos' habits: the runs in flight and the scheduler's queue.
    pub habits: habits::HabitsState,
    /// Sessions Monos watch, until their results go back to them.
    pub mono_sessions: mono_sessions::MonoSessionsState,
    /// Monos' own chat backgrounds, as drawn.
    pub mono_backgrounds: mono_background::MonoBackgrounds,
    /// The Monos icon in the macOS menu bar.
    pub mono_menu_bar: mono_menu_bar::MonoMenuBar,
    /// A Mono's artifacts: the reader and its page.
    pub artifacts: artifacts::ArtifactsState,
    /// The navigator's threads: filters, picks, inline renames.
    pub sessions_ui: crate::ui::sidebar_sessions::SessionsUi,
    /// The inspector's width (`inspectorWidth` in `settings.json`).
    pub inspector_width: f32,
    pub inspector_resizing: bool,
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
    /// The threads linked to each Inbox item (`app/item_threads.rs`).
    pub item_threads: item_threads::ItemThreads,
    /// Where the inspector's sash was pressed (window x).
    pub inspector_drag_x: f32,
    /// The card, folder or filter menu open over the navigator's threads.
    pub sidebar_menu: Option<crate::ui::sidebar_menus::SidebarMenu>,
    /// Focused while cards are picked, so F2 / ⌫ / Esc reach them.
    pub session_list_focus: gpui::FocusHandle,
    pub permission_mode: PermissionMode,
    pub inspector_mode: InspectorMode,
    pub search_query: String,
    /// Model key in MonoCode's `harness:model` form, e.g. `claude:opus`.
    pub selected_model: String,
    /// Installed harness CLIs, probed once at startup (never from render).
    pub harnesses: Vec<HarnessInfo>,
    /// The composer's open toolbar popover (Plus, model, access, branch or
    /// the "From main" base chip).
    pub composer_popover: Option<ComposerPopover>,
    /// The branch / base popover's state, its search, and the New branch
    /// dialog (MonoCode `BranchPicker`, `CreateBranchDialog`).
    pub branch_picker: crate::ui::composer::branch_picker::BranchPickerUi,
    pub branch_search_input: Entity<TextInput>,
    pub branch_create_open: bool,
    pub branch_create_input: Entity<TextInput>,
    /// MonoCode Inbox (GitHub source): the fetched list, filters,
    /// selection and loaded bodies, and its search field.
    /// The Inbox's data and selection, a model the regions showing it
    /// follow; read through `inbox(cx)`, changed through `inbox_mut`.
    pub inbox_state: Entity<crate::ui::inbox_view::InboxState>,
    pub inbox_search_input: Entity<TextInput>,
    pub inbox_comment_input: Entity<TextInput>,
    /// A Backlog issue's estimated and actual hours, as typed.
    pub backlog_estimate_input: Entity<TextInput>,
    pub backlog_actual_input: Entity<TextInput>,
    /// Settings › Integrations: the Backlog connection and its form.
    pub backlog: backlog::BacklogState,
    /// Settings › Integrations: the `gh` accounts and each project's.
    pub github: github_accounts::GithubAccountsState,
    pub backlog_space_input: Entity<TextInput>,
    pub backlog_key_input: Entity<TextInput>,
    pub backlog_disconnect_open: bool,
    /// The Inbox list's focus, for its ↑/↓ keys.
    pub inbox_focus: gpui::FocusHandle,
    /// A branch switch git refused because of local changes, awaiting "Stash & switch".
    pub blocked_branch_switch: Option<crate::app::workspace_sync::BranchTarget>,
    /// The open `/` skill or `@` mention picker.
    pub token_picker: Option<TokenPicker>,
    pub skill_query: String,
    /// The `/` or `@` token at the caret, and the caret last seen.
    pub prompt_token: Option<crate::ui::composer::tokens::Token>,
    pub prompt_caret: usize,
    pub mention_query: String,
    /// The `@` picker's last ranking of the project's files.
    pub(crate) mention_ranked: crate::ui::quick_open::RankCache<Vec<(String, Vec<usize>)>>,
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
    pub model_picker_index: usize,
    pub favorite_models: Vec<String>,
    /// MonoCode's sidebar folders, per project.
    pub session_folders: std::collections::BTreeMap<String, Vec<session_folders::SessionFolder>>,
    pub recent_models: Vec<String>,
    pub last_model_settings: serde_json::Map<String, serde_json::Value>,
    /// Model catalog probes per harness: `None` while one runs, else when
    /// the last one ended.
    pub catalog_probes: HashMap<crate::harness::HarnessKind, Option<std::time::Instant>>,
    /// The Providers page's rescan of PATH for agent CLIs is running.
    pub harness_rescan: bool,
    /// Each provider's last "Import sessions" (`app/cli_import.rs`).
    pub cli_imports: crate::app::cli_import::CliImports,
    /// The launch check's harness updates and each row's progress.
    pub harness_updates: crate::app::harness_updates::HarnessUpdates,
    /// Sounds, notifications and what decides them.
    pub alerts: crate::app::alerts::Alerts,
    /// Find in conversation (⌘F): its field and the open bar.
    pub find_input: Entity<TextInput>,
    /// Every file of the project, for Go to File, `@` and Search.
    pub project_files: crate::app::project_files::ProjectFiles,
    /// The agent's pending question: its "Other" field (the form state is
    /// in `threads`).
    pub question_custom_input: Entity<TextInput>,
    pub question_focus: gpui::FocusHandle,
    /// Where the prompt's `@`s are, for the file icons drawn over them.
    pub mention_marks: Vec<crate::ui::composer::MentionMark>,
    /// Pictures open as file pane tabs, by full path.
    pub image_tabs: HashMap<String, crate::ui::image_view::ImageTab>,
    /// The image shown full-window (MonoCode `ImageLightbox`).
    pub lightbox: Option<std::path::PathBuf>,
    /// The preview's right-click menu, while open.
    pub lightbox_menu: Option<crate::ui::lightbox::LightboxMenu>,
    /// The composer's inline error (a failed paste or attach, an edit the
    /// provider refused), shown under the chips until the next edit.
    pub composer_error: Option<String>,
    /// MonoCode "Edit and resend": the thread whose last message the
    /// composer holds.
    pub editing_last_turn: Option<String>,
    /// MonoCode "New worktree": the base chosen per not-yet-started thread
    /// (`""` before the thread exists).
    pub new_worktrees: HashMap<String, String>,
    /// MonoCode `ComposerRunner`: the composer geometry the mascot runs on
    /// (measured by layout, read by `RunnerLayer`), and the setting.
    pub runner_geometry: crate::ui::composer::runner_view::RunnerGeometry,
    /// The window's parts as cached views (`ui/region.rs`).
    pub regions: crate::ui::region::Regions,
    /// The busy marks over the regions (`ui/marks.rs`), made on first draw.
    pub mark_layer: Option<Entity<crate::ui::marks::MarkLayer>>,
    pub composer_mascot_off: bool,
    /// The navigator's Agents section (MonoCode `LiveAgentsPreview`) and its
    /// setting.
    pub live_agents_ui: crate::ui::rail::LiveAgentsUi,
    pub live_agents_off: bool,
    /// Settings › General: resume interrupted turns at launch without asking.
    pub resume_interrupted_auto: bool,
    /// The turns a quit, restart or crash cut off, offered at launch.
    pub resume: in_flight::ResumeState,
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
    pub body_glass: bool,
    /// The window background last set, so the blur toggles only on change.
    pub window_background: Option<gpui::WindowBackgroundAppearance>,
    /// The window's Liquid Glass while the glass is on (macOS 26 and later).
    pub native_glass: Option<crate::ui::native_glass::NativeGlass>,
    /// Liquid Glass could not be put in; GPUI's blur stands in for it.
    pub native_glass_failed: bool,
    /// The centred composer's last measurements, and a send from it whose
    /// docked composer is still dropping into place.
    pub dock_measure: std::rc::Rc<crate::ui::composer::DockMeasure>,
    pub dock_launch: Option<crate::ui::composer::DockLaunch>,
    /// A question just arrived for the focused thread; focus moves next frame.
    pub question_focus_wanted: bool,
    /// The queued message being edited in place, and its field.
    pub queue_editing: Option<(String, usize)>,
    /// The git pill's action, the last one picked (MonoCode `LastGitAction`).
    pub last_git_action: git_actions::GitAction,
    pub queue_edit_input: Entity<TextInput>,
    /// Go to File (⌘P).
    pub quick_open: crate::ui::quick_open::QuickOpen,
    pub quick_open_input: Entity<TextInput>,
    /// Title-bar tab strip: scroll, sweeps, unseen finishes.
    pub title_strip: crate::ui::titlebar::TitleStrip,
    pub transcript_find: Option<crate::ui::transcript::find::FindState>,
    /// Keyboard focus and highlight of the composer's menus.
    pub composer_menus: crate::ui::composer::MenuState,
    pub expanded_reasoning: std::collections::HashSet<String>,
    pub transcript_ui: crate::ui::transcript::TranscriptUiState,
    /// The text the pointer selected in the window (a transcript, a
    /// review's lines, an Inbox item); `WindowRoot` mounts its layer.
    pub text_selection: Entity<bencode_ui::selection::TextSelection>,
    /// Why Add to notes failed for the text it was given, shown in the
    /// selection's menu while that text is selected (MonoCode).
    pub selection_note_error: Option<(String, String)>,
    /// The focus that takes a transcript selection's ⌘C.
    pub transcript_focus: gpui::FocusHandle,
    /// The focus that takes a review selection's ⌘C.
    pub diff_focus: gpui::FocusHandle,
    /// The focus an Inbox item's selection keys work in.
    pub inbox_text_focus: gpui::FocusHandle,
    /// Each project's terminals (MonoCode project terminal docks).
    /// The terminal docks, a model the regions showing them follow; read
    /// through `terminals(cx)`, changed through `terminals_mut`.
    pub terminals_state: Entity<crate::ui::terminal_pane::TerminalDocks>,
    pub settings_tab: SettingsTab,
    /// The full-height view replacing the workspace, if any.
    pub surface: Option<Surface>,
    /// What Settings returns to when it closes.
    pub settings_return: Option<Surface>,
    /// The Notes surface: its notes, selection, fields and dialogs.
    pub notes: notes::NotesState,
    /// The Automations surface: definitions, run history, fields and dialogs.
    pub automations: automations::AutomationsState,
    /// Explorer › Search in files.
    pub project_search: project_search::ProjectSearchState,
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
    /// The open folder's git and file snapshot, a model the regions that
    /// show it follow; read through `workspace(cx)`, changed through
    /// `workspace_mut`.
    pub workspace_state: Entity<WorkspaceCache>,
    /// Every rail project's +/- lines (MonoCode `useProjectDiffStats`).
    pub project_stats: project_stats::ProjectStats,
    /// Provider usage for the usage chip (MonoCode `rateLimitsCache`).
    pub usage: usage::UsageState,
    /// Each account's usage, a model the usage regions follow.
    pub usage_data: Entity<usage::UsageSnapshots>,
    /// Provider account profiles (MonoCode `providerAccounts`).
    pub accounts: accounts::AccountsState,
    pub agy_accounts: agy_accounts::AgyAccountsState,
    /// The Add account form's name field.
    pub account_name_input: Entity<TextInput>,
    /// Settings › Accounts' name field, for adding and renaming.
    pub account_editor_input: Entity<TextInput>,
    /// The code a provider's sign-in page shows, pasted back to its CLI.
    pub login_code_input: Entity<TextInput>,
    pub file_tree: crate::ui::file_tree::FileTreeState,
    /// The Explorer's inline name field (MonoCode `NameRow`).
    pub file_dialog_input: Entity<TextInput>,
    /// Focused while the Explorer is clicked, so its keys reach it.
    pub file_tree_focus: gpui::FocusHandle,
    // Universal Search
    pub search_modal_input: Entity<TextInput>,
    pub search: crate::ui::search_view::SearchState,
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
    /// Root focus scope; bencode-ui overlays hand focus back to it.
    pub focus_handle: gpui::FocusHandle,
    // Agent execution
    /// The running turn of each thread that has one, keyed by session id.
    pub runs: HashMap<String, AgentRun>,
    /// Each thread's composer and queue state; dropped when the thread is.
    pub threads: HashMap<String, thread_state::ThreadState>,
    next_run_id: u64,
    pub prompt_input: Entity<TextInput>,
    /// The prompt's scroll box (`render_prompt_field`).
    pub prompt_scroll: crate::ui::caret_follow::CaretFollow,
    pub search_input: Entity<TextInput>,
    /// Multi-pane transcript list states keyed by session id.
    pub transcripts: std::collections::HashMap<String, crate::ui::transcript::TranscriptView>,
    /// Visual drop hint for an active pane drag over an edge of another pane.
    pub active_pane_drop: Option<crate::ui::drag_drop::PaneDropTarget>,
    /// Active file drop target session id.
    pub active_file_drop_target: Option<String>,
    /// Destructive git action awaiting confirmation.
    pub git_confirm: Option<crate::ui::git_changes_panel::GitConfirm>,
    /// Preferences as last loaded or saved (`settings.json`).
    pub settings: crate::settings::AppSettings,
    /// External editors and MCP servers, scanned once in the background.
    pub integrations: integrations::Integrations,
    pub editor: crate::ui::editor_pane::EditorState,
    /// Line blame and File History tabs.
    pub blame: blame::BlameState,
    /// Go to Definition: the jump under way, its picker, its message.
    pub definition: go_to_definition::DefinitionState,
    /// The inspector, Files or Changes at the workspace's right (⇧⌘B).
    pub inspector_open: bool,
    /// The navigator: projects, agents and the project's threads (⌘B).
    pub is_rail_open: bool,
    /// The project rail's menus, dialogs and drags.
    pub rail_ui: crate::ui::rail::RailUi,
    /// The Mono in view, its details panel and its soul.
    pub monos: monos::MonosState,
    /// Settings › Connections, the folder dialog and the machines' catalogs.
    pub remote: remote::RemoteUi,
    /// The turns machines run for this BenCode (`remote-runs.json`).
    pub remote_runs: Vec<remote_runs::RemoteRunRecord>,
    pub theme_preference: crate::settings::ThemePreference,
    /// Run Claude with `disableAllHooks` (MonoCode "Claude Code hooks" off).
    pub claude_hooks_disabled: bool,
    pub tab_history: tab_history::TabHistory,
    /// Set while Back/Forward switches tabs, so the move is not recorded.
    navigating_history: bool,
    pub db: AppDb,
    /// Writes to `db`'s file in order, off the UI thread. `None` while `db` is
    /// the in-memory fallback, which a second connection cannot see.
    pub db_writer: Option<crate::db::DbWriter>,
    /// For the startup trace: the first render is marked.
    rendered_once: bool,
    _subscriptions: Vec<Subscription>,
}

pub(crate) fn text_input(
    window: &mut Window,
    cx: &mut Context<BenCodeApp>,
    placeholder: &str,
) -> Entity<TextInput> {
    let placeholder = placeholder.to_string();
    cx.new(|cx| TextInput::new(window, cx).placeholder(placeholder))
}

pub(crate) fn multiline_input(
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
        launch: crate::launch::LaunchData,
        cx: &mut Context<Self>,
    ) -> Self {
        trace("app construction");
        let crate::launch::LaunchData {
            settings: saved,
            db,
            sessions,
            notes: saved_notes,
            automations: saved_automations,
            ..
        } = launch;
        let db_writer = db.file_path().and_then(|path| {
            crate::db::DbWriter::open(&path)
                .map_err(|err| {
                    log::error!("database writer unavailable, saving on the UI thread: {err:#}")
                })
                .ok()
        });

        // A Mono's conversation is no project's: it opens from the rail,
        // never as a tab.
        let is_mono_session = |id: &str| {
            saved
                .monos
                .iter()
                .any(|mono| mono.session_id.as_deref() == Some(id))
        };
        let tabs = crate::ui::layout::TabSet::with_sessions(
            sessions
                .iter()
                // Nor is one left in the home folder (a Mono's that was).
                .filter(|s| !is_mono_session(&s.id) && !is_home_path(&s.cwd))
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
            // It grows to fit; its box scrolls (`render_prompt_field`).
            TextInput::new(window, cx)
                .multi_line(1, usize::MAX)
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
        let login_code_input = text_input(window, cx, "Paste code");
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
        let hours_input = |window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                TextInput::new(window, cx)
                    .placeholder("—")
                    .filter(|ch| ch.is_ascii_digit() || matches!(ch, '.' | ','))
            })
        };
        let backlog_estimate_input = hours_input(window, cx);
        let backlog_actual_input = hours_input(window, cx);
        let backlog_space_input = text_input(window, cx, "yourspace.backlog.com");
        let backlog_key_input =
            cx.new(|cx| TextInput::new(window, cx).placeholder("API key").masked());
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
        let mut notes = notes::NotesState::new(window, cx);
        let note_tag_keys_input = notes.tag_input.clone();
        let note_body_keys_input = notes.body_input.clone();
        let mut automations = automations::AutomationsState::new(window, cx);
        let project_search = project_search::ProjectSearchState::new(window, cx);
        // No row cap: the Changes panel's box scrolls it (`max-h-40`).
        let git_commit_input =
            multiline_input(window, cx, "Message (⌘↩ to commit)", (1, usize::MAX));
        let search_modal_input = text_input(window, cx, "Search threads, files and projects");
        let search_keys_input = search_modal_input.clone();

        let composer_input = prompt_input.clone();
        let model_input = model_search_input.clone();
        let menu_focus = cx.focus_handle();
        let menu_keys_focus = menu_focus.clone();
        let weak_app = cx.weak_entity();
        // Models the regions follow (`docs/regions.md`, stage 5): a change
        // redraws the regions that show it, not the app.
        let usage_data = cx.new(|_| usage::UsageSnapshots::default());
        let inbox_state = cx.new(|_| crate::ui::inbox_view::InboxState::default());
        let terminals_state = cx.new(|_| crate::ui::terminal_pane::TerminalDocks::default());
        let workspace_state = cx.new(|_| WorkspaceCache::default());
        // Not followed: each document redraws what shows it
        // (`Document::on_change`).
        let text_selection = cx.new(|_| bencode_ui::selection::TextSelection::default());
        let mut regions = crate::ui::region::Regions::default();
        regions.follow(
            &usage_data,
            &[
                Region::Rail,
                Region::CompactRail,
                Region::Mono,
                Region::Surface,
            ],
        );
        regions.follow(
            &inbox_state,
            &[
                Region::Surface,
                Region::Rail,
                Region::CompactRail,
                Region::Titlebar,
            ],
        );
        // An Inbox item's tab beside the chat, only while one is open.
        regions.follow_while(&inbox_state, &[Region::FilePane], |app| {
            app.file_pane.shows_work_item()
        });
        regions.follow(&terminals_state, &[Region::Terminal, Region::Titlebar]);
        regions.follow(
            &workspace_state,
            &[
                Region::Titlebar,
                Region::Rail,
                Region::CompactRail,
                Region::Inspector,
                Region::Chat,
                Region::FilePane,
                Region::Surface,
                Region::Mono,
            ],
        );
        let mut subscriptions = vec![
            cx.subscribe(
                &prompt_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.submit_prompt(cx),
                    InputEvent::Changed => this.on_prompt_changed(cx),
                    InputEvent::Focus | InputEvent::Blur => {
                        this.prompt_focused = *event == InputEvent::Focus;
                        this.redraw_composer(cx);
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
                        this.redraw_regions(&[Region::Rail], cx);
                    }
                },
            ),
            // Files may have changed while BenCode was in the background.
            cx.observe_window_activation(window, |this, window, cx| {
                this.on_window_activation(window.is_window_active(), cx);
                if window.is_window_active() {
                    this.recheck_open_files_on_disk(cx);
                    // MonoCode re-lists the Explorer on focus.
                    this.refresh_file_tree(cx);
                    // MonoCode re-reads stale project stats on focus.
                    this.refresh_project_stats(cx);
                    this.auto_fetch_on_focus(cx);
                    this.probe_update_on_focus(cx);
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                this.on_system_appearance_changed(window.appearance(), cx)
            }),
            cx.observe_window_visibility(window, |this, visibility, _, cx| {
                this.workspace_mut(cx, |cache| cache.hidden = !visibility.is_visible());
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
            cx.subscribe(&backlog_space_input, Self::on_backlog_form_input),
            cx.subscribe(&backlog_key_input, Self::on_backlog_form_input),
            cx.subscribe(
                &inbox_search_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if *event == InputEvent::Changed {
                        this.redraw_regions(&[Region::Surface], cx);
                    }
                },
            ),
            cx.subscribe(
                &backlog_estimate_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Submit | InputEvent::Blur) {
                        this.commit_backlog_hours(cx);
                    }
                },
            ),
            cx.subscribe(
                &backlog_actual_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Submit | InputEvent::Blur) {
                        this.commit_backlog_hours(cx);
                    }
                },
            ),
            cx.subscribe(
                &inbox_comment_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.post_inbox_comment(cx),
                    InputEvent::Changed => this.redraw_regions(&[Region::Surface], cx),
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
                        this.redraw_composer(cx);
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
            cx.subscribe_in(
                &project_search.query_input,
                window,
                Self::on_project_search_input,
            ),
            cx.subscribe_in(
                &project_search.include_input,
                window,
                Self::on_project_search_input,
            ),
            cx.subscribe_in(
                &project_search.exclude_input,
                window,
                Self::on_project_search_input,
            ),
            cx.subscribe(&automations.filter_input, Self::on_automation_input_event),
            cx.subscribe(&automations.name_input, Self::on_automation_input_event),
            cx.subscribe(&automations.prompt_input, Self::on_automation_input_event),
            cx.subscribe(&notes.title_input, Self::on_note_input_event),
            cx.subscribe(&notes.body_input, Self::on_note_input_event),
            cx.subscribe(&notes.tag_input, Self::on_note_tag_input_event),
            cx.subscribe(
                &notes.filter_input,
                |this: &mut Self, _, event: &InputEvent, cx| {
                    if *event == InputEvent::Changed {
                        this.redraw_regions(&[Region::Surface], cx);
                    }
                },
            ),
            cx.subscribe(
                &git_commit_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.commit_staged_changes(cx),
                    // The Commit button follows the message.
                    InputEvent::Changed => this.redraw_regions(&[Region::Inspector], cx),
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
                    InputEvent::Changed => this.redraw_regions(&[Region::Surface], cx),
                    _ => {}
                },
            ),
            // Enter sends the pasted sign-in code; its Submit button follows
            // the text in the popover or in Settings.
            cx.subscribe(
                &login_code_input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.submit_login_code(cx),
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
                        this.redraw_regions(&[Region::Inspector], cx);
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
        subscriptions.push(cx.on_app_quit(|this, cx| {
            this.interrupt_runs_for_quit();
            // ⌘Q ends the dock's shells; an update's restart keeps them.
            if !this.updater.keep_terminals {
                this.end_all_terminal_sessions(cx);
            }
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
            // Images on the clipboard go into the note as files of its own.
            if pasting
                && note_body_keys_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            {
                if matches!(
                    weak_app.update(cx, |this, cx| this.paste_into_note(cx)),
                    Ok(true)
                ) {
                    cx.stop_propagation();
                }
                return;
            }
            // MonoCode `NoteTagsEditor`: Backspace in the empty field takes
            // the last tag.
            if event.keystroke.key == "backspace"
                && note_tag_keys_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            {
                if matches!(
                    weak_app.update(cx, |this, cx| this.pop_note_tag(cx)),
                    Ok(true)
                ) {
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
                && rename_keys_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
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
            // Search: ↑/↓, Enter (⌘Enter mentions a file), Tab / ⇧Tab and Esc.
            if search_keys_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
            {
                let key = event.keystroke.key.as_str();
                let mods = &event.keystroke.modifiers;
                let mention = key == "enter" && mods.platform;
                let shift_tab = key == "tab" && mods.shift;
                if (mods.modified() && !mention && !shift_tab)
                    || !matches!(key, "up" | "down" | "enter" | "escape" | "tab")
                {
                    return;
                }
                let handled = weak_app.update(cx, |this, cx| {
                    this.search_key(key, mods.shift, mention, window, cx)
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
            .filter(|cwd| !cwd.is_empty() && !is_home_path(cwd))
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| normalize_project_path(&p.to_string_lossy()))
            })
            .unwrap_or_else(|| ".".to_string());
        let recent_projects = projects::recent_projects(
            &current_cwd,
            sessions.iter().filter(|s| !is_mono_session(&s.id)),
            projects::home_dir().as_deref(),
        );

        let harnesses = HarnessResolver::discover();
        let selected_model = catalog::default_model(&harnesses).key;

        notes.items = saved_notes;
        notes.selected_id = notes.items.first().map(|n| n.id.clone());
        automations.items = saved_automations;
        automations.selected_id = automations.items.first().map(|a| a.id.clone());

        // Once the app is built and on screen.
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| {
                if frame_bench::enabled() || snapshots_enabled() {
                    return;
                }
                this.start_automation_scheduler(cx);
                this.start_habit_scheduler(cx);
                // The open dock's shell starts once the saved tabs are back.
                this.restore_terminals(cx);
            });
        })
        .detach();

        let browser = browser::BrowserState::new(window, cx);
        let app_access = app_calls::AppAccessState::new(cx);
        let artifacts = artifacts::ArtifactsState::new(window, cx);
        let mut app = Self {
            sessions,
            selected_session_id,
            tabs,
            file_pane: Default::default(),
            file_pane_menu: None,
            checkpoints: session_review::Checkpoints::new(
                crate::git::checkpoint::CheckpointStore::new(
                    crate::git::checkpoint::CheckpointStore::default_dir(),
                ),
            ),
            diff_docs: HashMap::new(),
            diff_split: false,
            file_pane_focused: false,
            file_pane_shares: [1.0, 1.0],
            file_pane_left: false,
            browser,
            app_access,
            habits: Default::default(),
            mono_sessions: Default::default(),
            mono_backgrounds: Default::default(),
            mono_menu_bar: Default::default(),
            artifacts,
            sessions_ui: Default::default(),
            sidebar_menu: None,
            inspector_width: crate::ui::inspector::INSPECTOR_DEFAULT_WIDTH,
            inspector_resizing: false,
            reminders: Vec::new(),
            reminder_error: None,
            reminder_generation: 0,
            reminder_failure: None,
            link_dialog: None,
            link_input,
            item_threads: Default::default(),
            inspector_drag_x: 0.0,
            session_list_focus: cx.focus_handle(),
            permission_mode: PermissionMode::default(),
            inspector_mode: InspectorMode::default(),
            search_query: String::new(),
            selected_model,
            harnesses,
            composer_popover: None,
            branch_picker: Default::default(),
            branch_search_input,
            branch_create_open: false,
            branch_create_input,
            inbox_state,
            inbox_search_input,
            inbox_comment_input,
            backlog_estimate_input,
            backlog_actual_input,
            backlog: Default::default(),
            github: Default::default(),
            backlog_space_input,
            backlog_key_input,
            backlog_disconnect_open: false,
            inbox_focus: cx.focus_handle(),
            blocked_branch_switch: None,
            token_picker: None,
            skill_query: String::new(),
            prompt_token: None,
            prompt_caret: 0,
            mention_query: String::new(),
            mention_ranked: Default::default(),
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
            model_picker_index: 0,
            favorite_models: Vec::new(),
            session_folders: Default::default(),
            recent_models: Vec::new(),
            last_model_settings: Default::default(),
            catalog_probes: Default::default(),
            harness_rescan: false,
            cli_imports: Default::default(),
            harness_updates: Default::default(),
            alerts: crate::app::alerts::Alerts {
                window_active: true,
                ..Default::default()
            },
            composer_menus: crate::ui::composer::MenuState::new(menu_focus),
            find_input,
            transcript_find: None,
            title_strip: Default::default(),
            quick_open: Default::default(),
            question_custom_input,
            question_focus,
            question_focus_wanted: false,
            composer_error: None,
            editing_last_turn: None,
            new_worktrees: HashMap::new(),
            runner_geometry: Default::default(),
            regions,
            mark_layer: None,
            composer_mascot_off: false,
            live_agents_ui: Default::default(),
            live_agents_off: false,
            resume_interrupted_auto: false,
            resume: Default::default(),
            sidebar_opacity: crate::ui::glass::OPACITY_DEFAULT,
            settings_write: Default::default(),
            appearance: Default::default(),
            accent_picker_open: false,
            chat_background: Default::default(),
            pane_rects: HashMap::new(),
            body_glass: true,
            window_background: None,
            native_glass: None,
            native_glass_failed: false,
            image_tabs: HashMap::new(),
            lightbox: None,
            lightbox_menu: None,
            mention_marks: Vec::new(),
            dock_measure: Default::default(),
            dock_launch: None,
            queue_editing: None,
            last_git_action: Default::default(),
            queue_edit_input,
            project_files: crate::app::project_files::ProjectFiles::new(mention_index),
            quick_open_input,
            expanded_reasoning: std::collections::HashSet::new(),
            transcript_ui: Default::default(),
            text_selection,
            selection_note_error: None,
            transcript_focus: cx.focus_handle(),
            diff_focus: cx.focus_handle(),
            inbox_text_focus: cx.focus_handle(),
            terminals_state,
            settings_tab: SettingsTab::Providers,
            surface: None,
            settings_return: None,
            notes,
            automations,
            project_search,
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
            workspace_state,
            project_stats: Default::default(),
            usage: Default::default(),
            usage_data,
            accounts: Default::default(),
            agy_accounts: Default::default(),
            account_name_input,
            account_editor_input,
            login_code_input,
            file_tree: Default::default(),
            file_dialog_input,
            file_tree_focus: cx.focus_handle(),
            search_modal_input,
            search: Default::default(),
            session_dialog: None,
            window_drag_pressed: false,
            worktrees_page: Default::default(),
            worktree_deletion: None,
            worktree_creation: None,
            worktree_branch_input,
            rename_input,
            focus_handle: cx.focus_handle(),
            runs: HashMap::new(),
            threads: HashMap::new(),
            next_run_id: 0,
            prompt_input,
            prompt_scroll: Default::default(),
            search_input,
            transcripts: std::collections::HashMap::new(),
            active_pane_drop: None,
            active_file_drop_target: None,
            git_confirm: None,
            settings: Default::default(),
            integrations: integrations::Integrations {
                skill_names,
                ..Default::default()
            },
            quit_confirm_open: false,
            updater: Default::default(),
            editor: Default::default(),
            blame: Default::default(),
            definition: Default::default(),
            inspector_open: false,
            is_rail_open: true,
            rail_ui,
            monos: monos::MonosState::new(window, cx),
            remote: remote::RemoteUi::new(window, cx),
            remote_runs: Vec::new(),
            theme_preference: Default::default(),
            claude_hooks_disabled: false,
            tab_history: Default::default(),
            navigating_history: false,
            db,
            db_writer,
            rendered_once: false,
            _subscriptions: subscriptions,
        };
        trace("app state built");
        app.apply_settings(saved, cx);
        // Git and the folder's listings load on the background executor
        // while the first frame draws; the folder is the one the restored
        // worktree focus (`apply_settings`) picks.
        app.refresh_workspace(cx);
        app.refresh_integrations(cx);
        if frame_bench::enabled() || snapshots_enabled() {
            // Only what fills the window from the data folder: a bench runs
            // on a copy and starts no agent, fetch or sign-in.
            app.start_git_poll(cx);
            app.start_project_stats_poll(cx);
            app.load_folder_members(cx);
            #[cfg(feature = "snapshot")]
            if snapshot::enabled() {
                app.start_snapshots(window, cx);
                return app;
            }
            app.start_frame_bench(window, cx);
            return app;
        }
        app.init_remote(cx);
        // First on the writer, before any turn can write a new list.
        app.load_interrupted_turns(cx);
        // After it: the lock it claims says whether this BenCode follows them.
        app.follow_remote_runs(cx);
        app.start_git_poll(cx);
        app.start_auto_fetch(cx);
        app.start_project_stats_poll(cx);
        app.start_session_age_tick(cx);
        app.start_reminder_poll(cx);
        app.load_folder_members(cx);
        app.start_clock(cx);
        app.start_usage_clock(cx);
        app.load_account_profiles(cx);
        app.refresh_installed_catalogs(cx);
        app.start_harness_update_check(cx);
        app.listen_for_notification_clicks(cx);
        app.listen_for_mono_menu_bar(window.window_handle(), cx);
        app.load_backlog_account(cx);
        app.start_inbox_poll(cx);
        app.start_update_probe(cx);
        app.reopen_saved_mono(cx);
        trace("launch jobs started");
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
        let account = self
            .selected_session()
            .and_then(|session| self.account_for_harness(session, harness_id));
        let changed_session = self.selected_session_mut().map(|session| {
            if session.harness != harness_id {
                // A provider session id is only meaningful to the harness that
                // issued it, and so is the account it belongs to.
                session.provider_session_id = None;
                session.provider_account_id = account;
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

    /// Runs `job` on the database thread (after every queued write) and
    /// hands its result to `land` back on the app, so nothing waits on the
    /// UI thread.
    pub(crate) fn db_then<T: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        job: impl FnOnce(&AppDb) -> anyhow::Result<T> + Send + 'static,
        land: impl FnOnce(&mut Self, anyhow::Result<T>, &mut Context<Self>) + 'static,
    ) {
        let result = self.db_read(job);
        cx.spawn(async move |this, cx| {
            let result = result
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the database writer stopped")));
            if let Err(err) = this.update(cx, |this, cx| land(this, result, cx)) {
                log::debug!("database result after app drop: {err:#}");
            }
        })
        .detach();
    }
}

/// Whether this launch takes the window's pictures (`app/snapshot.rs`):
/// its window is drawn but never shown, so it takes no one's keyboard.
pub fn snapshots_enabled() -> bool {
    #[cfg(feature = "snapshot")]
    return snapshot::enabled();
    #[cfg(not(feature = "snapshot"))]
    false
}

fn any_elements(items: impl IntoIterator<Item = impl IntoElement>) -> Vec<AnyElement> {
    items
        .into_iter()
        .map(IntoElement::into_any_element)
        .collect()
}

impl Render for BenCodeApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // How often the whole app re-renders, for perf work: run with
        // RUST_LOG=bencode::app=trace.
        if log::log_enabled!(log::Level::Trace) {
            use std::sync::atomic::{AtomicU64, Ordering};
            static RENDERS: AtomicU64 = AtomicU64::new(0);
            let n = RENDERS.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_multiple_of(120) {
                log::trace!("app renders: {n}");
            }
        }
        self.apply_ui_scale(window);
        self.sync_chat_background(!cx.theme().is_dark(), cx);
        self.sync_mono_background(!cx.theme().is_dark(), cx);
        if std::mem::take(&mut self.question_focus_wanted) {
            window.focus(&self.question_focus, cx);
        }
        if std::mem::take(&mut self.notes.focus_source) {
            let source = self.notes.body_input.read(cx).focus_handle(cx);
            window.focus(&source, cx);
        }
        if let Some(path) = self.file_tree.pending_open.take() {
            self.open_file_in_editor(&path, window, cx);
        }
        self.sync_artifact_page(window, cx);
        self.sync_mention_marks(window, cx);
        if !self.focus_handle.contains_focused(window, cx) && window.focused(cx).is_none() {
            window.focus(&self.focus_handle, cx);
        }
        self.sync_window_glass(window, cx);
        let glass = self.glass(cx);
        let colors = &cx.theme().colors;
        let (bg, fg) = (colors.bg, colors.fg);
        // Inbox, Notes, Automations and Settings replace everything right
        // of the navigator (MonoCode in-shell views).
        let surface_open = self.surface.is_some();
        // A Mono's conversation does too (MonoCode `monoCovers`).
        let mono_open = self.mono_covers() && self.mono_in_view().is_some();
        let workspace_visible = !surface_open && !mono_open;
        let compact_rail = self.compact_rail_active();
        let title_bar_above = self.compact_title_bar();
        // A browser page is a native view above everything GPUI draws: it
        // hides while one of these dialogs or a menu is open
        // (`app/browser.rs`).
        let dialogs: Vec<AnyElement> = [
            any_elements(self.render_link_dialog(cx)),
            any_elements(self.render_session_dialog(cx)),
            any_elements(self.render_worktree_deletion(cx)),
            any_elements(self.render_account_removal(cx)),
            any_elements(self.render_agy_account_removal(cx)),
            any_elements(self.render_backlog_disconnect(cx)),
            any_elements(self.render_machine_removal(cx)),
            any_elements(self.render_remote_folder_dialog(cx)),
            any_elements(self.render_worktree_creation(cx)),
            any_elements(self.render_search_dialog(cx)),
            any_elements(self.render_quick_open(cx)),
            any_elements(self.render_definition_picker(cx)),
            any_elements(self.render_lightbox(cx)),
            any_elements(self.render_git_confirm(cx)),
            any_elements(self.render_git_error(cx)),
            any_elements(self.render_session_undo_confirm(cx)),
            any_elements(self.render_branch_switch_confirm(cx)),
            any_elements(self.render_branch_create_dialog(cx)),
            any_elements(self.render_pr_action_confirm(cx)),
            any_elements(self.render_inbox_batch(cx)),
            any_elements(self.render_file_tree_dialog(cx)),
            any_elements(self.render_terminal_close_confirm(cx)),
            any_elements(self.render_whats_new(cx)),
            any_elements(self.render_resume_interrupted(cx)),
            any_elements(self.render_quit_confirm(cx)),
            any_elements(self.render_mono_settings(cx)),
            any_elements(self.render_mono_dialogs(cx)),
            any_elements(self.render_mono_background_dialog(cx)),
            any_elements(self.render_artifact_delete(cx)),
        ]
        .into_iter()
        .flatten()
        .collect();
        self.browser
            .obscured
            .set(!dialogs.is_empty() || self.context_menu_open(cx));

        // The regions this frame lays out (`ui/region.rs`): each is a
        // cached view, at a size given here.
        self.regions.begin_frame();
        let mut drawn: Vec<Region> = Vec::new();
        let mut region = |this: &mut Self, region: Region, style, cx: &mut Context<Self>| {
            drawn.push(region);
            this.regions.element(region, style, cx)
        };
        let titlebar_style = || {
            StyleRefinement::default()
                .w_full()
                .flex_none()
                .h(crate::ui::scale::px(crate::ui::sidebar::TITLEBAR_HEIGHT))
        };
        let titlebar_above =
            title_bar_above.then(|| region(self, Region::Titlebar, titlebar_style(), cx));
        let rail = self.is_rail_open.then(|| {
            let width = crate::ui::scale::px(self.rail_width());
            let style = StyleRefinement::default().flex_none().h_full().w(width);
            region(self, Region::Rail, style, cx)
        });
        let compact = compact_rail.then(|| {
            let width = crate::ui::scale::px(crate::ui::rail::COMPACT_RAIL_WIDTH);
            let style = StyleRefinement::default().flex_none().h_full().w(width);
            region(self, Region::CompactRail, style, cx)
        });
        let main = workspace_visible.then(|| {
            let titlebar =
                (!title_bar_above).then(|| region(self, Region::Titlebar, titlebar_style(), cx));
            // Files and Changes, at the right of the workspace under the
            // title bar: the navigator keeps the threads in view.
            let inspector = self.inspector_open.then(|| {
                let width = crate::ui::scale::px(self.inspector_width);
                let style = StyleRefinement::default().flex_none().h_full().w(width);
                region(self, Region::Inspector, style, cx)
            });
            div()
                .flex()
                .flex_col()
                .flex_1()
                .h_full()
                .min_w_0()
                .overflow_hidden()
                .children(titlebar)
                // MonoCode `body-glass`: the pane under the title bar.
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .bg(glass.body(bg))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(self.render_workspace_with_dock(window, cx)),
                        )
                        .children(inspector),
                )
        });
        let column = || StyleRefinement::default().flex_1().min_w_0().h_full();
        let surface = surface_open.then(|| region(self, Region::Surface, column(), cx));
        let mono_view = mono_open.then(|| region(self, Region::Mono, column(), cx));
        if workspace_visible {
            drawn.push(Region::Chat);
            if !self.file_pane.is_empty() {
                drawn.push(Region::FilePane);
            }
            if self.terminals(cx).layout(&self.current_cwd).open {
                drawn.push(Region::Terminal);
            }
        }
        self.settle_undrawn_regions(&drawn);
        let mark_layer = self
            .mark_layer
            .get_or_insert_with(|| cx.new(|_| crate::ui::marks::MarkLayer))
            .clone();

        // Full size explicitly: the app is laid out inside `WindowRoot`'s
        // slot, not as the window's root.
        let app = FocusScope::new(&self.focus_handle)
            .root()
            .size_full()
            .child(
                div()
                    .id("bencode-root")
                    .on_drag_move::<crate::ui::inspector::InspectorResize>(cx.listener(
                        |this,
                         event: &gpui::DragMoveEvent<crate::ui::inspector::InspectorResize>,
                         window,
                         cx| {
                            let drag = crate::ui::inspector::InspectorResize {
                                start_x: this.inspector_drag_x,
                                ..event.drag(cx).clone()
                            };
                            let width = crate::ui::inspector::resized_width(
                                &drag,
                                crate::ui::scale::logical(event.event.position.x),
                                crate::ui::scale::logical(window.viewport_size().width),
                            );
                            if width != this.inspector_width || !this.inspector_resizing {
                                this.inspector_width = width;
                                this.inspector_resizing = true;
                                cx.notify();
                            }
                        },
                    ))
                    .on_mouse_up(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if std::mem::take(&mut this.inspector_resizing) {
                                this.save_settings(cx);
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
                    .children(titlebar_above)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .size_full()
                            .min_h_0()
                            .overflow_hidden()
                            // The navigator: projects, agents and threads,
                            // or its icons
                            .children(rail)
                            .children(compact)
                            // The title bar over the chat and file pane,
                            // the terminal dock and the inspector
                            .children(main)
                            .children(surface)
                            .children(mono_view),
                    )
                    // Over the regions, under the menus and dialogs.
                    .child(mark_layer)
                    .children(self.render_sidebar_menu(cx))
                    .children(self.render_rail_overlays(cx))
                    .children(self.render_tree_menu(cx))
                    .children(self.render_terminal_menu(cx))
                    .children(self.render_git_menu(cx))
                    .children(self.render_corner_notices(window, cx))
                    .children(dialogs),
            );
        if crate::startup_trace::enabled() && !self.rendered_once {
            self.rendered_once = true;
            trace("app rendered (elements built)");
        }
        // The commands sit above the focus scope, not inside it: while the
        // scope's own handle holds focus (nothing else has it), actions
        // dispatch from the scope upward and never reach a child's handlers,
        // so no shortcut or menu item would work.
        Self::bind_commands(div().id("bencode-commands"), cx)
            .size_full()
            .child(app)
    }
}
