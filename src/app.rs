mod agent;
mod workspace_sync;

use std::collections::HashSet;

use ely_gpui_component::forms::{InputEvent, TextInput};
use ely_gpui_component::primitives::FocusScope;
use ely_gpui_component::theme::ActiveTheme;
use ely_gpui_component::terminal::{Launch, Terminal};
use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, div, prelude::*,
};

pub use agent::{AgentRun, NEW_SESSION_TITLE, now_ms};
pub use workspace_sync::WorkspaceCache;

use crate::db::{Block, MonoCodeDb, SessionRow};
use crate::harness::{HarnessInfo, HarnessKind, HarnessResolver, catalog};
use crate::ui::settings_modal::SettingsTab;

const RECENT_SESSION_LIMIT: usize = 50;
const INITIAL_OPEN_TABS: usize = 3;
const DEFAULT_CONTEXT_WINDOW: i64 = 200_000;
const RECENT_PROJECT_LIMIT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Chat,
    Changes,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FilterMode {
    #[default]
    All,
    Active,
    Pinned,
    Archived,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PermissionMode {
    #[default]
    Auto,
    Confirm,
    ReadOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SidebarMode {
    #[default]
    Sessions,
    Files,
    Changes,
}

pub struct BenCodeApp {
    pub sessions: Vec<SessionRow>,
    pub selected_session_id: Option<String>,
    pub open_tabs: Vec<String>,
    pub active_tab_id: Option<String>,
    pub active_view_mode: ViewMode,
    pub filter_mode: FilterMode,
    pub permission_mode: PermissionMode,
    pub sidebar_mode: SidebarMode,
    pub expanded_folders: HashSet<String>,
    pub selected_diff_path: Option<String>,
    pub search_query: String,
    /// Model key in MonoCode's `harness:model` form, e.g. `claude:opus`.
    pub selected_model: String,
    /// Installed harness CLIs, probed once at startup (never from render).
    pub harnesses: Vec<HarnessInfo>,
    pub is_model_picker_open: bool,
    pub is_branch_picker_open: bool,
    pub is_skill_picker_open: bool,
    pub skill_query: String,
    pub is_mention_picker_open: bool,
    pub mention_query: String,
    pub terminal: Option<Entity<Terminal>>,
    pub is_settings_open: bool,
    pub settings_tab: SettingsTab,
    pub is_notes_open: bool,
    pub notes: Vec<crate::db::Note>,
    pub selected_note_id: Option<String>,
    pub note_filter_query: String,
    pub note_filter_input: Entity<TextInput>,
    pub note_title_input: Entity<TextInput>,
    pub note_body_input: Entity<TextInput>,
    pub is_automations_open: bool,
    pub automations: Vec<crate::db::AutomationRow>,
    pub selected_automation_id: Option<String>,
    pub automation_runs: Vec<crate::db::AutomationRunRow>,
    pub automation_name_input: Entity<TextInput>,
    pub automation_prompt_input: Entity<TextInput>,
    pub automation_time_input: Entity<TextInput>,
    // Workspace & Projects
    pub current_cwd: String,
    pub recent_projects: Vec<String>,
    // Git & Source Control
    pub git_status: crate::git::GitDetailedStatus,
    pub git_commits: Vec<crate::git::GitCommitInfo>,
    pub git_commit_input: Entity<TextInput>,
    pub git_staged_collapsed: bool,
    pub git_unstaged_collapsed: bool,
    pub git_history_collapsed: bool,
    /// Git/filesystem snapshot for the active workspace; see `workspace_sync`.
    pub workspace: WorkspaceCache,
    // Universal Search
    pub is_search_open: bool,
    pub search_modal_input: Entity<TextInput>,
    pub search_scope: crate::ui::search_view::SearchScope,
    pub search_hits: Vec<crate::ui::search_view::SearchHit>,
    pub search_active_index: usize,
    // Inbox
    pub is_inbox_open: bool,
    pub theme_name: String,
    /// Rename/delete dialog opened from the thread list.
    pub session_dialog: Option<crate::ui::sidebar::SessionDialog>,
    pub rename_input: Entity<TextInput>,
    /// Root focus scope; Ely overlays hand focus back to it.
    pub focus_handle: gpui::FocusHandle,
    // Agent execution
    pub active_run: Option<AgentRun>,
    next_run_id: u64,
    pub prompt_input: Entity<TextInput>,
    pub search_input: Entity<TextInput>,
    // [ui-agent-flow fields]
    // [ui-git-files fields]
    // [ui-panels fields]
    pub db: MonoCodeDb,
    _subscriptions: Vec<Subscription>,
}

fn text_input(window: &mut Window, cx: &mut Context<BenCodeApp>, placeholder: &str) -> Entity<TextInput> {
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
    cx.new(|cx| TextInput::new(window, cx).multi_line(rows.0, rows.1).placeholder(placeholder))
}

/// Distinct session working directories, most recent first, current one on top.
fn recent_projects(current_cwd: &str, sessions: &[SessionRow]) -> Vec<String> {
    let mut seen = HashSet::new();
    std::iter::once(current_cwd.to_string())
        .chain(sessions.iter().map(|s| s.cwd.clone()))
        .filter(|cwd| !cwd.is_empty() && seen.insert(cwd.clone()))
        .take(RECENT_PROJECT_LIMIT)
        .collect()
}

impl BenCodeApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let db = MonoCodeDb::open_default().unwrap_or_else(|err| {
            log::warn!("MonoCode DB unavailable ({err:#}); using BenCode's local database");
            MonoCodeDb::open_fallback()
        });

        let sessions = db.list_recent_sessions(RECENT_SESSION_LIMIT).unwrap_or_else(|err| {
            log::error!("failed to load sessions: {err:#}");
            Vec::new()
        });
        let selected_session_id = sessions.first().map(|s| s.id.clone());
        let open_tabs: Vec<String> = sessions.iter().take(INITIAL_OPEN_TABS).map(|s| s.id.clone()).collect();

        let prompt_input = multiline_input(window, cx, "Ask the agent, / for skills, @ for files…", (1, 6));
        let search_input = text_input(window, cx, "Search threads... (⌘K)");
        let note_filter_input = text_input(window, cx, "Filter notes...");
        let note_title_input = text_input(window, cx, "Note title...");
        let note_body_input = multiline_input(window, cx, "Write note or scratchpad in markdown...", (5, 25));
        let automation_name_input = text_input(window, cx, "Automation name...");
        let automation_prompt_input = multiline_input(window, cx, "Automation prompt...", (3, 10));
        let automation_time_input = text_input(window, cx, "09:00");
        let git_commit_input = text_input(window, cx, "Message (⌘↩ to commit)...");
        let search_modal_input = text_input(window, cx, "Search conversations, files, projects... (⌘K)");

        let subscriptions = vec![
            cx.subscribe(&prompt_input, |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Submit => this.submit_prompt(cx),
                InputEvent::Changed => this.on_prompt_changed(cx),
                _ => {}
            }),
            cx.subscribe(&search_input, |this: &mut Self, input, event: &InputEvent, cx| {
                if *event == InputEvent::Changed {
                    this.search_query = input.read(cx).text().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe(&note_filter_input, |this: &mut Self, input, event: &InputEvent, cx| {
                if *event == InputEvent::Changed {
                    this.note_filter_query = input.read(cx).text().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe(&git_commit_input, |this: &mut Self, _, event: &InputEvent, cx| {
                if *event == InputEvent::Submit {
                    this.commit_staged_changes(cx);
                }
            }),
            cx.subscribe(&search_modal_input, |this: &mut Self, _, event: &InputEvent, cx| {
                if *event == InputEvent::Changed {
                    this.update_search_hits(cx);
                }
            }),
        ];

        let current_cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let recent_projects = recent_projects(&current_cwd, &sessions);

        let harnesses = HarnessResolver::discover();
        let selected_model = catalog::default_model(&harnesses).key.to_string();

        let terminal_cwd = Some(std::path::PathBuf::from(&current_cwd));
        let terminal = cx.new(|cx| {
            Terminal::spawn(
                Launch {
                    program: None,
                    cwd: terminal_cwd,
                    env: vec![
                        ("TERM".into(), "xterm-256color".into()),
                        ("COLORTERM".into(), "truecolor".into()),
                    ],
                },
                cx,
            )
            .unwrap_or_else(|err| {
                log::error!("failed to start terminal: {err:#}");
                Terminal::replay(b"Terminal unavailable\r\n", 80, 24, cx)
            })
        });

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
            let _ = this.update(cx, |this, cx| this.refresh_workspace(cx));
        })
        .detach();

        Self {
            sessions,
            active_tab_id: selected_session_id.clone(),
            selected_session_id,
            open_tabs,
            active_view_mode: ViewMode::Chat,
            filter_mode: FilterMode::All,
            permission_mode: PermissionMode::Auto,
            sidebar_mode: SidebarMode::Sessions,
            expanded_folders: HashSet::new(),
            selected_diff_path: None,
            search_query: String::new(),
            selected_model,
            harnesses,
            is_model_picker_open: false,
            is_branch_picker_open: false,
            is_skill_picker_open: false,
            skill_query: String::new(),
            is_mention_picker_open: false,
            mention_query: String::new(),
            terminal: Some(terminal),
            is_settings_open: false,
            settings_tab: SettingsTab::Providers,
            is_notes_open: false,
            notes,
            selected_note_id,
            note_filter_query: String::new(),
            note_filter_input,
            note_title_input,
            note_body_input,
            is_automations_open: false,
            automations,
            selected_automation_id,
            automation_runs: Vec::new(),
            automation_name_input,
            automation_prompt_input,
            automation_time_input,
            current_cwd,
            recent_projects,
            git_status: Default::default(),
            git_commits: Vec::new(),
            git_commit_input,
            git_staged_collapsed: false,
            git_unstaged_collapsed: false,
            git_history_collapsed: false,
            workspace: WorkspaceCache::default(),
            is_search_open: false,
            search_modal_input,
            search_scope: crate::ui::search_view::SearchScope::All,
            search_hits: Vec::new(),
            search_active_index: 0,
            is_inbox_open: false,
            theme_name: "MonoCode Dark".to_string(),
            session_dialog: None,
            rename_input: text_input(window, cx, "Thread title"),
            focus_handle: cx.focus_handle(),
            active_run: None,
            next_run_id: 0,
            prompt_input,
            search_input,
            // [ui-agent-flow init]
            // [ui-git-files init]
            // [ui-panels init]
            db,
            _subscriptions: subscriptions,
        }
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
        self.is_model_picker_open = false;

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

    pub fn set_session_branch(&mut self, branch: String, cx: &mut Context<Self>) {
        if let Some(session) = self.selected_session_mut() {
            session.branch = Some(branch);
        }
        self.is_branch_picker_open = false;
        cx.notify();
    }

    pub fn on_prompt_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.prompt_input.read(cx).text().to_string();
        self.is_skill_picker_open = false;
        self.is_mention_picker_open = false;

        if let Some(query) = trigger_query(&text, '/') {
            self.is_skill_picker_open = true;
            self.skill_query = query;
        } else if let Some(query) = trigger_query(&text, '@') {
            self.is_mention_picker_open = true;
            self.mention_query = query;
        }
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

    /// Replaces the trailing `trigger…` token of the prompt with `replacement`.
    fn replace_trigger(&mut self, trigger: char, replacement: &str, cx: &mut Context<Self>) {
        self.prompt_input.update(cx, |input, cx| {
            let text = input.text().to_string();
            let prefix = text.rfind(trigger).map_or("", |idx| &text[..idx]);
            input.set_text(format!("{prefix}{replacement} "), cx);
        });
    }

    pub fn select_session(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.open_tabs.contains(&id) {
            self.open_tabs.push(id.clone());
        }
        self.switch_tab(id, cx);
    }

    pub fn switch_tab(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(model) = self.sessions.iter().find(|s| s.id == id).map(|s| s.model.clone())
            && catalog::find(&model).is_some() {
                self.selected_model = model;
            }
        self.selected_session_id = Some(id.clone());
        self.active_tab_id = Some(id);
        self.selected_diff_path = None;
        self.refresh_workspace_if_moved(cx);
        cx.notify();
    }

    pub fn close_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(pos) = self.open_tabs.iter().position(|t| t == id) else { return };
        self.open_tabs.remove(pos);
        if self.active_tab_id.as_deref() == Some(id) {
            self.active_tab_id = self.open_tabs.first().cloned();
            self.selected_session_id = self.active_tab_id.clone();
        }
        cx.notify();
    }

    pub fn toggle_pin_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else { return };
        let was_pinned = session.pinned;
        if let Err(err) = self.db.toggle_pinned(id, was_pinned) {
            log::error!("failed to toggle pin for {id}: {err:#}");
            return;
        }
        session.pinned = !was_pinned;
        cx.notify();
    }

    pub fn delete_session(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.active_run.as_ref().is_some_and(|run| run.session_id == id) {
            log::warn!("refusing to delete session {id} while its agent is running");
            return;
        }
        if let Err(err) = self.db.delete_session(id) {
            log::error!("failed to delete session {id}: {err:#}");
            return;
        }
        self.sessions.retain(|s| s.id != id);
        self.close_tab(id, cx);
        cx.notify();
    }

    pub fn create_new_session(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        let id = format!("bencode-{now}");
        let harness = catalog::find(&self.selected_model)
            .map(|m| m.harness)
            .unwrap_or(HarnessKind::Claude);
        let branch = Some(self.git_status.branch.clone()).filter(|b| !b.is_empty());

        let mut welcome = Block::new(
            "b1",
            "assistant",
            "Ready for your instructions. I can edit files, run commands, and inspect git diffs.",
        );
        welcome.started_at = Some(now);

        let session = SessionRow {
            id: id.clone(),
            title: NEW_SESSION_TITLE.to_string(),
            cwd: self.current_cwd.clone(),
            harness: harness.id().to_string(),
            model: self.selected_model.clone(),
            created_at: now,
            updated_at: now,
            branch,
            context_used: Some(0),
            context_window: Some(DEFAULT_CONTEXT_WINDOW),
            blocks: vec![welcome],
            ..Default::default()
        };

        self.sessions.insert(0, session);
        self.persist_session(&id);
        self.open_tabs.push(id.clone());
        self.switch_tab(id, cx);
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

        FocusScope::new(&self.focus_handle).root().child(
            div()
                .flex()
                .flex_col()
                .size_full()
                .bg(bg)
                .text_color(fg)
                .child(self.render_titlebar(cx))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .child(self.render_project_rail(cx))
                        .child(match self.sidebar_mode {
                            SidebarMode::Sessions => self.render_sidebar(cx).into_any_element(),
                            SidebarMode::Files => self.render_file_tree(cx).into_any_element(),
                            SidebarMode::Changes => self.render_git_changes_panel(cx).into_any_element(),
                        })
                        .child(match self.active_view_mode {
                            ViewMode::Chat => self.render_transcript_panel(cx).into_any_element(),
                            ViewMode::Changes => self.render_diff_viewer(cx).into_any_element(),
                            ViewMode::Terminal => self.render_terminal_pane(cx).into_any_element(),
                        }),
                )
                .child(self.render_usage_footer(cx))
                .when(self.is_settings_open, |el| el.child(self.render_settings_modal(cx)))
                .when(self.is_notes_open, |el| el.child(self.render_notes_modal(cx)))
                .when(self.is_automations_open, |el| el.child(self.render_automations_modal(cx)))
                .when(self.is_search_open, |el| el.child(self.render_search_modal(cx)))
                .when(self.is_inbox_open, |el| el.child(self.render_inbox_modal(cx)))
                .children(self.render_session_dialog(cx)),
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
        assert_eq!(trigger_query("a/b", '/'), None, "mid-word slash is a path, not a skill");
        assert_eq!(trigger_query("@file done", '@'), None, "token already finished");
        assert_eq!(trigger_query("xin chào /ski", '/').as_deref(), Some("ski"));
    }

    #[test]
    fn recent_projects_are_unique_and_current_first() {
        let session = |cwd: &str| SessionRow { cwd: cwd.into(), ..Default::default() };
        let projects = recent_projects("/here", &[session("/a"), session("/here"), session("/a"), session("")]);
        assert_eq!(projects, ["/here", "/a"]);
    }
}
