use std::sync::Arc;

use ely_gpui_component::forms::{InputEvent, TextInput};
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, IconSize, Radius, TextSize};
use gpui::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Subscription, Window, div, prelude::*, px,
};

use crate::db::{Block, MonoCodeDb, SessionRow, TurnModel};
use crate::harness::HarnessResolver;
use crate::ui::settings_modal::SettingsTab;
use crate::ui::theme::MonoTheme;

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

use ely_gpui_component::terminal::{Launch, Terminal};

pub struct BenCodeApp {
    pub sessions: Vec<SessionRow>,
    pub selected_session_id: Option<String>,
    pub open_tabs: Vec<String>,
    pub active_tab_id: Option<String>,
    pub active_view_mode: ViewMode,
    pub filter_mode: FilterMode,
    pub permission_mode: PermissionMode,
    pub sidebar_mode: SidebarMode,
    pub expanded_folders: std::collections::HashSet<String>,
    pub is_agent_running: bool,
    pub selected_diff_path: Option<String>,
    pub search_query: String,
    pub selected_model: String,
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
    // Universal Search
    pub is_search_open: bool,
    pub search_modal_input: Entity<TextInput>,
    pub search_scope: crate::ui::search_view::SearchScope,
    pub search_hits: Vec<crate::ui::search_view::SearchHit>,
    pub search_active_index: usize,
    // Inbox
    pub is_inbox_open: bool,
    pub prompt_input: Entity<TextInput>,
    pub search_input: Entity<TextInput>,
    pub db: Arc<MonoCodeDb>,
    _subscriptions: Vec<Subscription>,
}

impl BenCodeApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let db = MonoCodeDb::open_default().unwrap_or_else(|e| {
            log::warn!("Could not open MonoCode DB: {e}");
            panic!("Could not open DB: {e}");
        });

        let sessions = db.list_recent_sessions(50).unwrap_or_default();
        let selected_session_id = sessions.first().map(|s| s.id.clone());

        // Open first 3 sessions as initial tabs
        let open_tabs: Vec<String> = sessions.iter().take(3).map(|s| s.id.clone()).collect();
        let active_tab_id = selected_session_id.clone();

        let prompt_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line(1, 6)
                .placeholder("Ask Claude Code or type / for skills, @ for files...")
        });

        let search_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .placeholder("Search threads... (⌘K)")
        });

        let note_filter_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .placeholder("Filter notes...")
        });

        let note_title_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .placeholder("Note title...")
        });

        let note_body_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line(5, 25)
                .placeholder("Write note or scratchpad in markdown...")
        });

        let automation_name_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .placeholder("Automation name...")
        });

        let automation_prompt_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line(3, 10)
                .placeholder("Automation prompt...")
        });

        let automation_time_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .placeholder("09:00")
        });

        let mut subscriptions = Vec::new();

        let prompt_sub = cx.subscribe(&prompt_input, |this: &mut BenCodeApp, _, event: &InputEvent, cx| {
            if *event == InputEvent::Submit {
                this.submit_prompt(cx);
            } else if *event == InputEvent::Changed {
                this.on_prompt_changed(cx);
            }
        });
        subscriptions.push(prompt_sub);

        let search_sub = cx.subscribe(&search_input, |this: &mut BenCodeApp, input, event: &InputEvent, cx| {
            if *event == InputEvent::Changed {
                this.search_query = input.read(cx).text().to_string();
                cx.notify();
            }
        });
        subscriptions.push(search_sub);

        let note_filter_sub = cx.subscribe(&note_filter_input, |this: &mut BenCodeApp, input, event: &InputEvent, cx| {
            if *event == InputEvent::Changed {
                this.note_filter_query = input.read(cx).text().to_string();
                cx.notify();
            }
        });
        subscriptions.push(note_filter_sub);

        let current_cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());

        let recent_projects = vec![
            current_cwd.clone(),
            "/Users/benit/Documents/sources/monocode".to_string(),
            "/Users/benit/Documents/sources/bencode".to_string(),
        ];

        let git_status = crate::git::get_detailed_status(&current_cwd);
        let git_commits = crate::git::get_recent_commits(&current_cwd, 8);

        let git_commit_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .placeholder("Message (⌘↩ to commit)...")
        });

        let git_commit_sub = cx.subscribe(&git_commit_input, |this: &mut BenCodeApp, _, event: &InputEvent, cx| {
            if *event == InputEvent::Submit {
                this.commit_staged_changes(cx);
            }
        });
        subscriptions.push(git_commit_sub);

        let search_modal_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .placeholder("Search conversations, files, projects... (⌘K)")
        });

        let search_modal_sub = cx.subscribe(&search_modal_input, |this: &mut BenCodeApp, _, event: &InputEvent, cx| {
            if *event == InputEvent::Changed {
                this.update_search_hits(cx);
            }
        });
        subscriptions.push(search_modal_sub);

        // Detect available harnesses
        let harnesses = HarnessResolver::discover();
        let default_model = if harnesses.iter().any(|h| h.id == "claude" && h.available) {
            "Claude 3.7 Sonnet".to_string()
        } else if harnesses.iter().any(|h| h.id == "antigravity" && h.available) {
            "Gemini 3.8 Flash".to_string()
        } else {
            "Claude 3.7 Sonnet".to_string()
        };

        let default_cwd = std::env::current_dir().ok();
        let terminal = cx.new(|cx| {
            Terminal::spawn(
                Launch {
                    program: None,
                    cwd: default_cwd,
                    env: vec![
                        ("TERM".into(), "xterm-256color".into()),
                        ("COLORTERM".into(), "truecolor".into()),
                    ],
                },
                cx,
            ).unwrap_or_else(|_| Terminal::replay(b"Terminal ready\r\n", 80, 24, cx))
        });

        let notes = db.list_notes().unwrap_or_default();
        let selected_note_id = notes.first().map(|n| n.id.clone());

        let automations = db.list_automations().unwrap_or_default();
        let selected_automation_id = automations.first().map(|a| a.id.clone());

        Self {
            sessions,
            selected_session_id,
            open_tabs,
            active_tab_id,
            active_view_mode: ViewMode::Chat,
            filter_mode: FilterMode::All,
            permission_mode: PermissionMode::Auto,
            sidebar_mode: SidebarMode::Sessions,
            expanded_folders: std::collections::HashSet::new(),
            is_agent_running: false,
            selected_diff_path: None,
            search_query: String::new(),
            selected_model: default_model,
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
            git_status,
            git_commits,
            git_commit_input,
            git_staged_collapsed: false,
            git_unstaged_collapsed: false,
            git_history_collapsed: false,
            is_search_open: false,
            search_modal_input,
            search_scope: crate::ui::search_view::SearchScope::All,
            search_hits: Vec::new(),
            search_active_index: 0,
            is_inbox_open: false,
            prompt_input,
            search_input,
            db: Arc::new(db),
            _subscriptions: subscriptions,
        }
    }

    pub fn set_session_model(&mut self, model: String, harness: String, cx: &mut Context<Self>) {
        self.selected_model = model.clone();
        if let Some(session_id) = &self.selected_session_id {
            if let Some(s) = self.sessions.iter_mut().find(|s| &s.id == session_id) {
                s.model = model;
                s.harness = harness;
            }
        }
        self.is_model_picker_open = false;
        cx.notify();
    }

    pub fn set_session_branch(&mut self, branch: String, cx: &mut Context<Self>) {
        if let Some(session_id) = &self.selected_session_id {
            if let Some(s) = self.sessions.iter_mut().find(|s| &s.id == session_id) {
                s.branch = Some(branch);
            }
        }
        self.is_branch_picker_open = false;
        cx.notify();
    }

    pub fn on_prompt_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.prompt_input.read(cx).text().to_string();

        // Slash command check
        if let Some(idx) = text.rfind('/') {
            let after = &text[idx + 1..];
            if !after.contains(' ') && (idx == 0 || text[..idx].ends_with(' ') || text[..idx].ends_with('\n')) {
                self.is_skill_picker_open = true;
                self.skill_query = after.to_lowercase();
                self.is_mention_picker_open = false;
                cx.notify();
                return;
            }
        }
        self.is_skill_picker_open = false;

        // Mention check
        if let Some(idx) = text.rfind('@') {
            let after = &text[idx + 1..];
            if !after.contains(' ') && (idx == 0 || text[..idx].ends_with(' ') || text[..idx].ends_with('\n')) {
                self.is_mention_picker_open = true;
                self.mention_query = after.to_lowercase();
                cx.notify();
                return;
            }
        }
        self.is_mention_picker_open = false;
        cx.notify();
    }

    pub fn insert_skill(&mut self, skill_name: &str, cx: &mut Context<Self>) {
        self.prompt_input.update(cx, |this, cx| {
            let text = this.text().to_string();
            if let Some(idx) = text.rfind('/') {
                let prefix = &text[..idx];
                this.set_text(format!("{}{}{} ", prefix, skill_name, if prefix.is_empty() { "" } else { "" }), cx);
            } else {
                this.set_text(format!("{} ", skill_name), cx);
            }
        });
        self.is_skill_picker_open = false;
        cx.notify();
    }

    pub fn insert_mention(&mut self, mention: &str, cx: &mut Context<Self>) {
        self.prompt_input.update(cx, |this, cx| {
            let text = this.text().to_string();
            if let Some(idx) = text.rfind('@') {
                let prefix = &text[..idx];
                this.set_text(format!("{}@{} ", prefix, mention), cx);
            } else {
                this.set_text(format!("@{} ", mention), cx);
            }
        });
        self.is_mention_picker_open = false;
        cx.notify();
    }

    pub fn select_session(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.open_tabs.contains(&id) {
            self.open_tabs.push(id.clone());
        }
        self.selected_session_id = Some(id.clone());
        self.active_tab_id = Some(id);
        self.selected_diff_path = None;
        cx.notify();
    }

    pub fn switch_tab(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_session_id = Some(id.clone());
        self.active_tab_id = Some(id);
        self.selected_diff_path = None;
        cx.notify();
    }

    pub fn close_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(pos) = self.open_tabs.iter().position(|t| t == id) {
            self.open_tabs.remove(pos);
            if self.active_tab_id.as_deref() == Some(id) {
                self.active_tab_id = self.open_tabs.first().cloned();
                self.selected_session_id = self.active_tab_id.clone();
            }
            cx.notify();
        }
    }

    pub fn toggle_pin_session(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(s) = self.sessions.iter_mut().find(|s| s.id == id) {
            s.pinned = !s.pinned;
            let _ = self.db.toggle_pinned(id, !s.pinned);
            cx.notify();
        }
    }

    pub fn delete_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let _ = self.db.delete_session(id);
        self.sessions.retain(|s| s.id != id);
        self.close_tab(id, cx);
        cx.notify();
    }

    pub fn create_new_session(&mut self, cx: &mut Context<Self>) {
        let new_id = format!("bencode-{}", jiff::Timestamp::now().as_millisecond());
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "~".to_string());

        let new_session = SessionRow {
            id: new_id.clone(),
            title: "New AI Thread".to_string(),
            cwd,
            harness: "claude".to_string(),
            model: self.selected_model.clone(),
            created_at: jiff::Timestamp::now().as_millisecond(),
            updated_at: jiff::Timestamp::now().as_millisecond(),
            branch: Some("main".to_string()),
            context_used: Some(0),
            context_window: Some(200_000),
            pinned: false,
            archived: false,
            blocks: vec![Block {
                id: "b1".to_string(),
                role: "assistant".to_string(),
                text: Some("Ready for your instructions. I can edit files, run bash commands, and inspect git diffs.".to_string()),
                turn_model: None,
                tool: None,
                second_opinion: None,
                started_at: Some(jiff::Timestamp::now().as_millisecond()),
                duration_ms: None,
            }],
        };

        let _ = self.db.upsert_session(&new_session);

        self.sessions.insert(0, new_session);
        self.open_tabs.push(new_id.clone());
        self.selected_session_id = Some(new_id.clone());
        self.active_tab_id = Some(new_id);
        self.selected_diff_path = None;
        cx.notify();
    }

    pub fn submit_prompt(&mut self, cx: &mut Context<Self>) {
        let prompt_text = self.prompt_input.read(cx).text().trim().to_string();
        if prompt_text.is_empty() {
            return;
        }

        // Clear prompt input field
        self.prompt_input.update(cx, |input, cx| {
            input.set_text("", cx);
        });

        let now = jiff::Timestamp::now().as_millisecond();
        let user_block = Block {
            id: format!("usr-{}", now),
            role: "user".to_string(),
            text: Some(prompt_text.clone()),
            turn_model: None,
            tool: None,
            second_opinion: None,
            started_at: Some(now),
            duration_ms: None,
        };

        if let Some(session_id) = &self.selected_session_id {
            if let Some(s) = self.sessions.iter_mut().find(|s| &s.id == session_id) {
                s.blocks.push(user_block);
                s.updated_at = now;

                if s.title == "New AI Thread" || s.title.is_empty() {
                    let preview: String = prompt_text.chars().take(36).collect();
                    s.title = format!("{}...", preview);
                }

                // Append assistant response block
                let assistant_block = Block {
                    id: format!("ast-{}", now + 1),
                    role: "assistant".to_string(),
                    text: Some(format!(
                        "⚡ BenCode executing task: \"{}\"\nInspecting workspace context and running tools natively via Apple Metal.",
                        prompt_text
                    )),
                    turn_model: Some(TurnModel {
                        harness: Some(s.harness.clone()),
                        id: Some(s.model.clone()),
                        name: Some(s.model.clone()),
                    }),
                    tool: None,
                    second_opinion: None,
                    started_at: Some(now + 1),
                    duration_ms: None,
                };
                s.blocks.push(assistant_block);

                // Persist updated session to SQLite
                let _ = self.db.upsert_session(s);
            }
        }
        cx.notify();
    }

    pub fn handle_send_or_stop(&mut self, cx: &mut Context<Self>) {
        if self.is_agent_running {
            self.is_agent_running = false;
            cx.notify();
        } else {
            self.submit_prompt(cx);
        }
    }

    pub fn render_sidebar_mode_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let current_mode = self.sidebar_mode;

        let total_changes = self.git_status.staged.len() + self.git_status.unstaged.len();
        let adds: usize = self.git_status.staged.iter().map(|f| f.additions).sum::<usize>()
            + self.git_status.unstaged.iter().map(|f| f.additions).sum::<usize>();
        let dels: usize = self.git_status.staged.iter().map(|f| f.deletions).sum::<usize>()
            + self.git_status.unstaged.iter().map(|f| f.deletions).sum::<usize>();

        div()
            .flex()
            .items_center()
            .justify_between()
            .p_1p5()
            .border_b_1()
            .border_color(MonoTheme::border_stroke())
            .bg(MonoTheme::bg_surface())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .flex_1()
                    .p_0p5()
                    .rounded(theme.radius(Radius::Sm))
                    .bg(MonoTheme::bg_base())
                    // Tab 1: Sessions
                    .child(
                        div()
                            .id("sidebar-tab-sessions")
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_1()
                            .h(px(24.0))
                            .rounded(theme.radius(Radius::Sm))
                            .bg(if current_mode == SidebarMode::Sessions {
                                MonoTheme::bg_active()
                            } else {
                                gpui::rgba(0x00000000)
                            })
                            .text_color(if current_mode == SidebarMode::Sessions {
                                MonoTheme::fg_primary()
                            } else {
                                MonoTheme::fg_muted()
                            })
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(if current_mode == SidebarMode::Sessions {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .cursor_pointer()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .child(Icon::new(IconName::MessageSquare).size(IconSize::Xs))
                            .child("Sessions")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_mode = SidebarMode::Sessions;
                                cx.notify();
                            })),
                    )
                    // Tab 2: Files
                    .child(
                        div()
                            .id("sidebar-tab-files")
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_1()
                            .h(px(24.0))
                            .rounded(theme.radius(Radius::Sm))
                            .bg(if current_mode == SidebarMode::Files {
                                MonoTheme::bg_active()
                            } else {
                                gpui::rgba(0x00000000)
                            })
                            .text_color(if current_mode == SidebarMode::Files {
                                MonoTheme::fg_primary()
                            } else {
                                MonoTheme::fg_muted()
                            })
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(if current_mode == SidebarMode::Files {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .cursor_pointer()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .child(Icon::new(IconName::Folder).size(IconSize::Xs))
                            .child("Files")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_mode = SidebarMode::Files;
                                cx.notify();
                            })),
                    )
                    // Tab 3: Changes
                    .child(
                        div()
                            .id("sidebar-tab-changes")
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_1()
                            .h(px(24.0))
                            .rounded(theme.radius(Radius::Sm))
                            .bg(if current_mode == SidebarMode::Changes {
                                MonoTheme::bg_active()
                            } else {
                                gpui::rgba(0x00000000)
                            })
                            .text_color(if current_mode == SidebarMode::Changes {
                                MonoTheme::fg_primary()
                            } else {
                                MonoTheme::fg_muted()
                            })
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(if current_mode == SidebarMode::Changes {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .cursor_pointer()
                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                            .child(Icon::new(IconName::GitBranch).size(IconSize::Xs))
                            .child("Changes")
                            .when(adds > 0 || dels > 0, |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_0p5()
                                        .text_size(theme.text_size(TextSize::Xs))
                                        .font_weight(FontWeight::BOLD)
                                        .when(adds > 0, |el| {
                                            el.child(
                                                div()
                                                    .text_color(MonoTheme::success())
                                                    .child(format!("+{}", adds)),
                                            )
                                        })
                                        .when(dels > 0, |el| {
                                            el.child(
                                                div()
                                                    .text_color(MonoTheme::status_error())
                                                    .child(format!("-{}", dels)),
                                            )
                                        }),
                                )
                            })
                            .when(total_changes > 0 && adds == 0 && dels == 0, |el| {
                                el.child(
                                    div()
                                        .px_1()
                                        .py_0p5()
                                        .rounded(theme.radius(Radius::Sm))
                                        .bg(MonoTheme::accent())
                                        .text_color(MonoTheme::on_accent())
                                        .text_size(theme.text_size(TextSize::Xs))
                                        .font_weight(FontWeight::BOLD)
                                        .child(total_changes.to_string()),
                                )
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_mode = SidebarMode::Changes;
                                this.refresh_git_status(cx);
                            })),
                    ),
            )
    }
}

impl Render for BenCodeApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_session = self.sessions
            .iter()
            .find(|s| self.selected_session_id.as_deref() == Some(&s.id))
            .cloned();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(MonoTheme::bg_base())
            .text_color(MonoTheme::fg_primary())
            // 1. Top Titlebar with tabs and mode toggles
            .child(self.render_titlebar(cx))
            // 2. Middle Body: Project Rail + Sidebar + Main View
            .child(
                div()
                    .flex()
                    .flex_1()
                    .overflow_hidden()
                    .child(self.render_project_rail(cx))
                    .child(
                        match self.sidebar_mode {
                            SidebarMode::Sessions => self.render_sidebar(cx).into_any_element(),
                            SidebarMode::Files => self.render_file_tree(cx).into_any_element(),
                            SidebarMode::Changes => self.render_git_changes_panel(cx).into_any_element(),
                        }
                    )
                    .child(
                        match self.active_view_mode {
                            ViewMode::Chat => self.render_transcript_panel(selected_session.as_ref(), cx).into_any_element(),
                            ViewMode::Changes => self.render_diff_viewer(selected_session.as_ref(), cx).into_any_element(),
                            ViewMode::Terminal => self.render_terminal_pane(selected_session.as_ref(), cx).into_any_element(),
                        }
                    ),
            )
            // 3. Bottom Usage Footer
            .child(self.render_usage_footer(cx))
            // 4. Modal Overlays (Settings, Notes, Automations, Search, Inbox)
            .when(self.is_settings_open, |el| el.child(self.render_settings_modal(cx)))
            .when(self.is_notes_open, |el| el.child(self.render_notes_modal(cx)))
            .when(self.is_automations_open, |el| el.child(self.render_automations_modal(cx)))
            .when(self.is_search_open, |el| el.child(self.render_search_modal(cx)))
            .when(self.is_inbox_open, |el| el.child(self.render_inbox_modal(cx)))
    }
}
