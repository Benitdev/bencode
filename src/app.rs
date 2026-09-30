use std::sync::Arc;

use ely_gpui_component::theme::ActiveTheme;
use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div};

use crate::db::{Block, MonoCodeDb, SessionRow};

pub struct BenCodeApp {
    pub sessions: Vec<SessionRow>,
    pub selected_session_id: Option<String>,
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

        Self {
            sessions,
            selected_session_id,
            is_agent_running: false,
            active_prompt: String::new(),
            db: Arc::new(db),
        }
    }

    pub fn select_session(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_session_id = Some(id);
        cx.notify();
    }

    pub fn create_new_session(&mut self, cx: &mut Context<Self>) {
        let new_id = format!("bencode-{}", jiff::Timestamp::now().as_millisecond());
        let new_session = SessionRow {
            id: new_id.clone(),
            title: "New AI Session".to_string(),
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
                text: Some("Hello! I am your native BenCode agent. How can I assist you with your codebase today?".to_string()),
                turn_model: None,
                tool: None,
                started_at: Some(jiff::Timestamp::now().as_millisecond()),
            }],
        };

        self.sessions.insert(0, new_session);
        self.selected_session_id = Some(new_id);
        cx.notify();
    }

    pub fn handle_send_or_stop(&mut self, cx: &mut Context<Self>) {
        if self.is_agent_running {
            self.is_agent_running = false;
        } else {
            self.is_agent_running = true;
            // Add a mock response or trigger background harness
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
                        text: Some("Analyzing repository structure using native Rust GPUI engine. Zero WebKit overhead detected. Everything running at 120 FPS.".to_string()),
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
            .size_full()
            .bg(colors.bg)
            .text_color(colors.fg)
            .child(self.render_sidebar(cx))
            .child(self.render_transcript_panel(selected_session.as_ref(), cx))
    }
}
