mod db;

use db::{MonoCodeDb, SessionRow};
use ely_gpui_component::{
    Assets,
    layout::on_axis,
    theme::{ActiveTheme, Mode, Radius, TextSize, Theme},
};
use gpui::{
    App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    ScrollHandle, SharedString, Styled, Window, WindowBounds, WindowOptions, TitlebarOptions,
    Bounds, div, point, prelude::*, px, size,
};
use std::sync::Arc;

struct MonoCodeApp {
    sessions: Vec<SessionRow>,
    selected_session_id: Option<String>,
    _scroll: ScrollHandle,
    transcript_scroll: ScrollHandle,
    _db: Arc<MonoCodeDb>,
}

impl MonoCodeApp {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        let db = MonoCodeDb::open_default().unwrap_or_else(|e| {
            log::warn!("Could not open MonoCode DB: {e}");
            panic!("Could not open MonoCode DB at ~/Library/Application Support/com.monocode.desktop/monocode.db: {e}");
        });

        let sessions = db.list_recent_sessions(30).unwrap_or_default();
        let selected_session_id = sessions.first().map(|s| s.id.clone());

        Self {
            sessions,
            selected_session_id,
            _scroll: ScrollHandle::new(),
            transcript_scroll: ScrollHandle::new(),
            _db: Arc::new(db),
        }
    }

    pub fn select_session(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_session_id = Some(id);
        self.transcript_scroll.set_offset(point(px(0.0), px(0.0)));
        cx.notify();
    }
}

impl Render for MonoCodeApp {
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
            .child(self.render_main_panel(selected_session, cx))
    }
}

impl MonoCodeApp {
    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(280.0))
            .h_full()
            .pt(px(48.0))
            .px_3()
            .pb_4()
            .border_r_1()
            .border_color(colors.border)
            .bg(colors.surface)
            // App Header
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .pb_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Lg))
                                    .font_weight(FontWeight::BOLD)
                                    .child("MonoCode"),
                            )
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(colors.fg_subtle)
                                    .child("Native GPUI • 120 FPS"),
                            ),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(theme.radius(Radius::Sm))
                            .bg(colors.hover)
                            .text_size(theme.text_size(TextSize::Xs))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(colors.accent)
                            .child("PoC"),
                    ),
            )
            // Section Title
            .child(
                div()
                    .px_2()
                    .py_2()
                    .text_size(theme.text_size(TextSize::Xs))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors.fg_muted)
                    .child("RECENT SESSIONS"),
            )
            // Session List (Loaded from real SQLite database)
            .child(
                on_axis(div().id("sessions-scroll"))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_1()
                    .overflow_y_scroll()
                    .children(self.sessions.iter().map(|session| {
                        let is_selected = self.selected_session_id.as_deref() == Some(&session.id);
                        let id = session.id.clone();
                        let title = if session.title.trim().is_empty() {
                            "Untitled Session"
                        } else {
                            &session.title
                        };
                        let harness = session.harness.to_uppercase();
                        let branch = session.branch.clone().unwrap_or_else(|| "main".into());

                        div()
                            .id(SharedString::from(format!("session-{}", session.id)))
                            .flex()
                            .flex_col()
                            .px_3()
                            .py_2()
                            .rounded(theme.radius(Radius::Md))
                            .cursor_pointer()
                            .when(is_selected, |el| el.bg(colors.hover))
                            .hover(|style| style.bg(colors.hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_session(id.clone(), cx);
                            }))
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Sm))
                                    .font_weight(if is_selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                    .text_color(if is_selected { colors.fg } else { colors.fg_muted })
                                    .child(title.to_string()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .pt_1()
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(colors.accent)
                                            .child(harness),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Xs))
                                            .text_color(colors.fg_subtle)
                                            .child(format!("⎇ {}", branch)),
                                    ),
                            )
                    })),
            )
    }

    fn render_main_panel(&mut self, session: Option<SessionRow>, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .pt(px(48.0))
            .bg(colors.bg)
            // Header bar
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(54.0))
                    .px_6()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Base))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(session.as_ref().map(|s| s.title.as_str()).unwrap_or("No session selected").to_string()),
                            )
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(colors.fg_subtle)
                                    .child(session.as_ref().map(|s| s.cwd.as_str()).unwrap_or("").to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(colors.surface)
                                    .border_1()
                                    .border_color(colors.border)
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .child(session.as_ref().map(|s| s.model.as_str()).unwrap_or("Model").to_string()),
                            ),
                    ),
            )
            // Transcript / Content area
            .child(
                on_axis(div().id("transcript-scroll"))
                    .flex_1()
                    .p_6()
                    .overflow_y_scroll()
                    .child(
                        if let Some(s) = session {
                            div()
                                .flex()
                                .flex_col()
                                .gap_4()
                                .max_w(px(760.0))
                                .mx_auto()
                                .child(
                                    div()
                                        .p_4()
                                        .rounded(theme.radius(Radius::Lg))
                                        .bg(colors.surface)
                                        .border_1()
                                        .border_color(colors.border)
                                        .child(
                                            div()
                                                .text_size(theme.text_size(TextSize::Sm))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(colors.fg)
                                                .child(format!("Session ID: {}", s.id)),
                                        )
                                        .child(
                                            div()
                                                .pt_2()
                                                .text_size(theme.text_size(TextSize::Sm))
                                                .text_color(colors.fg_muted)
                                                .child(format!("Agent Harness: {} (Model: {})", s.harness, s.model)),
                                        )
                                        .child(
                                            div()
                                                .pt_1()
                                                .text_size(theme.text_size(TextSize::Sm))
                                                .text_color(colors.fg_muted)
                                                .child(format!("Working Directory: {}", s.cwd)),
                                        ),
                                )
                                .child(
                                    div()
                                        .p_4()
                                        .rounded(theme.radius(Radius::Lg))
                                        .bg(colors.surface)
                                        .border_1()
                                        .border_color(colors.border)
                                        .child(
                                            div()
                                                .text_size(theme.text_size(TextSize::Sm))
                                                .text_color(colors.fg)
                                                .child("⚡ This transcript view is rendered natively with Metal GPU shaders via GPUI! Zero WebKit, Zero Chromium, Zero JavaScript runtime."),
                                        ),
                                )
                        } else {
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .h_full()
                                .child("Select a session from the sidebar to view")
                        },
                    ),
            )
            // Composer Dock at the bottom
            .child(
                div()
                    .p_4()
                    .border_t_1()
                    .border_color(colors.border)
                    .bg(colors.surface)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .max_w(px(760.0))
                            .mx_auto()
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(40.0))
                                    .px_4()
                                    .rounded(theme.radius(Radius::Md))
                                    .bg(colors.bg)
                                    .border_1()
                                    .border_color(colors.border)
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(theme.text_size(TextSize::Sm))
                                            .text_color(colors.fg_subtle)
                                            .child("Ask agent or enter instruction... (GPUI Native Composer)"),
                                    ),
                            )
                            .child(
                                div()
                                    .px_4()
                                    .h(px(40.0))
                                    .rounded(theme.radius(Radius::Md))
                                    .bg(colors.accent)
                                    .text_color(colors.on_accent)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_size(theme.text_size(TextSize::Sm))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Send"),
                            ),
                    ),
            )
    }
}

fn main() {
    env_logger::init();

    gpui_platform::application()
        .with_assets(Assets)
        .run(|cx: &mut App| {
            ely_gpui_component::init(cx);
            Theme::set_mode(Mode::Dark, cx);

            let bounds = Bounds::centered(None, size(px(1120.0), px(720.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("MonoCode (Native GPUI)".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.0), px(18.0))),
                }),
                window_min_size: Some(size(px(800.0), px(500.0))),
                ..Default::default()
            };

            cx.open_window(options, |window, cx| {
                cx.new(|cx| MonoCodeApp::new(window, cx))
            })
            .expect("Failed to open GPUI window");

            cx.activate(true);
        });
}
