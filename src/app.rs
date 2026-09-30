use std::sync::Arc;

use ely_gpui_component::theme::ActiveTheme;
use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div};

use crate::db::{Block, MonoCodeDb, SessionRow};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Chat,
    Changes,
    Terminal,
}

pub struct BenCodeApp {
    pub sessions: Vec<SessionRow>,
    pub selected_session_id: Option<String>,
    pub open_tabs: Vec<String>,
    pub active_tab_id: Option<String>,
    pub active_view_mode: ViewMode,
    pub is_agent_running: bool,
    pub active_prompt: String,
    pub db: Arc<MonoCodeDb>,
}

impl BenCodeApp {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        let db = MonoCodeDb::open_default().unwrap_or_else(|e| {
            log::warn!("Could not open MonoCode DB: {e}");
            panic!("Could not open MonoCode DB at ~/Library/Application Support/com.monocode.desktop/monocode.db: {e}");
        });

        let sessions = db.list_recent_sessions(50).unwrap_or_default();
        let selected_session_id = sessions.first().map(|s| s.id.clone());

        // Open first 3 sessions as initial tabs
        let open_tabs: Vec<String> = sessions.iter().take(3).map(|s| s.id.clone()).collect();
        let active_tab_id = selected_session_id.clone();

        Self {
            sessions,
            selected_session_id,
            open_tabs,
            active_tab_id,
            active_view_mode: ViewMode::Chat,
            is_agent_running: false,
            active_prompt: String::new(),
            db: Arc::new(db),
        }
    }

    pub fn select_session(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.open_tabs.contains(&id) {
            self.open_tabs.push(id.clone());
        }
        self.selected_session_id = Some(id.clone());
        self.active_tab_id = Some(id);
        cx.notify();
    }

    pub fn switch_tab(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_session_id = Some(id.clone());
        self.active_tab_id = Some(id);
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

    pub fn create_new_session(&mut self, cx: &mut Context<Self>) {
        let new_id = format!("bencode-{}", jiff::Timestamp::now().as_millisecond());
        let new_session = SessionRow {
            id: new_id.clone(),
            title: "New AI Thread".to_string(),
            cwd: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "~".to_string()),
            harness: "claude".to_string(),
            model: "claude-3-7-sonnet".to_string(),
            created_at: jiff::Timestamp::now().as_millisecond(),
            updated_at: jiff::Timestamp::now().as_millisecond(),
            branch: Some("main".to_string()),
            blocks: vec![Block {
                id: "b1".to_string(),
                role: "assistant".to_string(),
                text: Some("Ready for your instructions. I can edit files, run bash commands, and inspect git diffs.".to_string()),
                turn_model: None,
                tool: None,
                started_at: Some(jiff::Timestamp::now().as_millisecond()),
            }],
        };

        self.sessions.insert(0, new_session);
        self.open_tabs.push(new_id.clone());
        self.selected_session_id = Some(new_id.clone());
        self.active_tab_id = Some(new_id);
        cx.notify();
    }

    pub fn handle_send_or_stop(&mut self, cx: &mut Context<Self>) {
        if self.is_agent_running {
            self.is_agent_running = false;
        } else {
            self.is_agent_running = true;
            if let Some(session_id) = &self.selected_session_id {
                if let Some(s) = self.sessions.iter_mut().find(|s| &s.id == session_id) {
                    s.blocks.push(Block {
                        id: format!("usr-{}", jiff::Timestamp::now().as_millisecond()),
                        role: "user".to_string(),
                        text: Some("Inspect codebase architecture and optimize performance".to_string()),
                        turn_model: None,
                        tool: None,
                        started_at: Some(jiff::Timestamp::now().as_millisecond()),
                    });
                    s.blocks.push(Block {
                        id: format!("ast-{}", jiff::Timestamp::now().as_millisecond()),
                        role: "assistant".to_string(),
                        text: Some("Running full system audit via BenCode GPUI core. All tabs, git diffs, and terminal sessions are running natively.".to_string()),
                        turn_model: None,
                        tool: None,
                        started_at: Some(jiff::Timestamp::now().as_millisecond()),
                    });
                }
            }
            self.is_agent_running = false;
        }
        cx.notify();
    }
}

impl Render for BenCodeApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;

        let selected_session = self.sessions
            .iter()
            .find(|s| self.selected_session_id.as_deref() == Some(&s.id))
            .cloned();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.bg)
            .text_color(colors.fg)
            // 1. Top Titlebar with tabs and mode toggles
            .child(self.render_titlebar(cx))
            // 2. Middle Body: Project Rail + Sidebar + Main View
            .child(
                div()
                    .flex()
                    .flex_1()
                    .overflow_hidden()
                    .child(self.render_project_rail(cx))
                    .child(self.render_sidebar(cx))
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
    }
}
