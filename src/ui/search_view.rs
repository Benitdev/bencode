use ely_gpui_component::{
    forms::TextInput,
    layout::on_axis,
    primitives::{Icon, IconName},
    theme::{ActiveTheme, IconSize, Radius, TextSize},
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::app::{BenCodeApp, ViewMode};
use crate::ui::theme::MonoTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchScope {
    All,
    Conversations,
    Files,
    Projects,
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub scope: SearchScope,
    pub icon: IconName,
    pub target_id: String,
}

impl BenCodeApp {
    pub fn open_search_modal(&mut self, cx: &mut Context<Self>) {
        self.is_search_open = true;
        self.search_scope = SearchScope::All;
        self.search_active_index = 0;
        self.search_modal_input.update(cx, |input, cx| {
            input.set_text("", cx);
        });
        self.update_search_hits(cx);
    }

    pub fn close_search_modal(&mut self, cx: &mut Context<Self>) {
        self.is_search_open = false;
        cx.notify();
    }

    pub fn update_search_hits(&mut self, cx: &mut Context<Self>) {
        let query = self.search_modal_input.read(cx).text().trim().to_lowercase();
        let scope = self.search_scope;

        if query.is_empty() {
            self.search_hits.clear();
            self.search_active_index = 0;
            cx.notify();
            return;
        }

        let mut hits = Vec::new();

        // 1. Search Conversations
        if scope == SearchScope::All || scope == SearchScope::Conversations {
            for s in &self.sessions {
                let matches_title = s.title.to_lowercase().contains(&query);
                let matches_cwd = s.cwd.to_lowercase().contains(&query);
                let matches_model = s.model.to_lowercase().contains(&query);
                let matches_blocks = s.blocks.iter().any(|b| {
                    b.text.as_deref().unwrap_or("").to_lowercase().contains(&query)
                });

                if matches_title || matches_cwd || matches_model || matches_blocks {
                    hits.push(SearchHit {
                        id: format!("conv-{}", s.id),
                        title: s.title.clone(),
                        subtitle: format!("{} • {}", s.model, s.cwd),
                        scope: SearchScope::Conversations,
                        icon: IconName::MessageSquare,
                        target_id: s.id.clone(),
                    });
                }
            }
        }

        // 2. Search Workspace Files
        if scope == SearchScope::All || scope == SearchScope::Files {
            let cwd = self.current_cwd.clone();
            // Fast scan workspace directory
            if let Ok(entries) = std::fs::read_dir(&cwd) {
                for entry in entries.flatten().take(60) {
                    let path = entry.path();
                    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if file_name.starts_with('.') || file_name == "target" || file_name == "node_modules" {
                        continue;
                    }
                    if file_name.to_lowercase().contains(&query) {
                        let is_dir = path.is_dir();
                        hits.push(SearchHit {
                            id: format!("file-{}", file_name),
                            title: file_name.to_string(),
                            subtitle: path.to_string_lossy().to_string(),
                            scope: SearchScope::Files,
                            icon: if is_dir { IconName::Folder } else { IconName::FileText },
                            target_id: file_name.to_string(),
                        });
                    }
                }
            }
        }

        // 3. Search Projects
        if scope == SearchScope::All || scope == SearchScope::Projects {
            for proj in &self.recent_projects {
                if proj.to_lowercase().contains(&query) {
                    let name = std::path::Path::new(proj)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(proj)
                        .to_string();
                    hits.push(SearchHit {
                        id: format!("proj-{}", proj),
                        title: name,
                        subtitle: proj.clone(),
                        scope: SearchScope::Projects,
                        icon: IconName::Folder,
                        target_id: proj.clone(),
                    });
                }
            }
        }

        self.search_hits = hits;
        self.search_active_index = 0;
        cx.notify();
    }

    pub fn execute_search_hit(&mut self, hit: SearchHit, cx: &mut Context<Self>) {
        match hit.scope {
            SearchScope::Conversations => {
                self.selected_session_id = Some(hit.target_id);
                self.active_view_mode = ViewMode::Chat;
                self.close_search_modal(cx);
            }
            SearchScope::Files => {
                self.selected_diff_path = Some(hit.target_id);
                self.active_view_mode = ViewMode::Changes;
                self.close_search_modal(cx);
            }
            SearchScope::Projects => {
                self.current_cwd = hit.target_id;
                self.refresh_git_status(cx);
                self.close_search_modal(cx);
            }
            SearchScope::All => {
                self.close_search_modal(cx);
            }
        }
    }

    pub fn render_search_modal(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let query = self.search_modal_input.read(cx).text().trim().to_string();
        let current_scope = self.search_scope;
        let hits_count = self.search_hits.len();
        let active_idx = self.search_active_index;

        div()
            .absolute()
            .inset_0()
            .bg(gpui::rgba(0x000000aa))
            .flex()
            .items_start()
            .justify_center()
            .pt(px(80.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(640.0))
                    .max_h(px(520.0))
                    .rounded(theme.radius(Radius::Lg))
                    .bg(MonoTheme::bg_surface())
                    .border_1()
                    .border_color(MonoTheme::border_stroke())
                    .shadow_lg()
                    .overflow_hidden()
                    // 1. Search Header Row
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(MonoTheme::border_stroke())
                            .bg(MonoTheme::bg_base())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .flex_1()
                                    .child(
                                        Icon::new(IconName::Search)
                                            .size(IconSize::Sm)
                                            .color(MonoTheme::fg_muted()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .child(self.search_modal_input.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .id("close-search-modal-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .px_2()
                                    .py_1()
                                    .rounded(theme.radius(Radius::Sm))
                                    .bg(MonoTheme::bg_hover())
                                    .text_color(MonoTheme::fg_muted())
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .cursor_pointer()
                                    .child("ESC")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_search_modal(cx);
                                    })),
                            ),
                    )
                    // 2. Scope Filter Pills
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_4()
                            .py_2()
                            .border_b_1()
                            .border_color(MonoTheme::border_stroke())
                            .bg(MonoTheme::bg_surface())
                            .child(self.render_scope_pill(SearchScope::All, "All", current_scope, cx))
                            .child(self.render_scope_pill(SearchScope::Conversations, "Conversations", current_scope, cx))
                            .child(self.render_scope_pill(SearchScope::Files, "Files", current_scope, cx))
                            .child(self.render_scope_pill(SearchScope::Projects, "Projects", current_scope, cx)),
                    )
                    // 3. Results Container
                    .child(
                        on_axis(div().id("search-hits-scroll"))
                            .flex_1()
                            .overflow_y_scroll()
                            .min_h(px(260.0))
                            .max_h(px(400.0))
                            .p_2()
                            .when(query.is_empty(), |el| {
                                // Empty state matching MonoCode
                                el.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .justify_center()
                                        .py_12()
                                        .gap_3()
                                        .child(
                                            Icon::new(IconName::Search)
                                                .size(IconSize::Lg)
                                                .color(MonoTheme::fg_subtle()),
                                        )
                                        .child(
                                            div()
                                                .text_size(theme.text_size(TextSize::Sm))
                                                .text_color(MonoTheme::fg_muted())
                                                .child("Find files, conversations, messages, and projects"),
                                        ),
                                )
                            })
                            .when(!query.is_empty() && hits_count == 0, |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .justify_center()
                                        .py_12()
                                        .child(
                                            div()
                                                .text_size(theme.text_size(TextSize::Sm))
                                                .text_color(MonoTheme::fg_muted())
                                                .child(format!("No results found for \"{}\"", query)),
                                        ),
                                )
                            })
                            .when(!query.is_empty() && hits_count > 0, |el| {
                                let mut list = div().flex().flex_col().gap_0p5();
                                for (idx, hit) in self.search_hits.iter().enumerate() {
                                    let hit_clone = hit.clone();
                                    let is_active = idx == active_idx;
                                    list = list.child(
                                        div()
                                            .id(SharedString::from(format!("search-hit-{}", hit.id)))
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .px_3()
                                            .py_2()
                                            .rounded(theme.radius(Radius::Md))
                                            .bg(if is_active {
                                                MonoTheme::bg_active()
                                            } else {
                                                gpui::rgba(0x00000000)
                                            })
                                            .hover(|s| s.bg(MonoTheme::bg_hover()))
                                            .cursor_pointer()
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .child(
                                                        Icon::new(hit.icon)
                                                            .size(IconSize::Sm)
                                                            .color(if is_active {
                                                                MonoTheme::accent()
                                                            } else {
                                                                MonoTheme::fg_muted()
                                                            }),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .min_w_0()
                                                            .child(
                                                                div()
                                                                    .font_weight(FontWeight::MEDIUM)
                                                                    .text_size(theme.text_size(TextSize::Sm))
                                                                    .text_color(MonoTheme::fg_primary())
                                                                    .child(hit.title.clone()),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_size(theme.text_size(TextSize::Xs))
                                                                    .text_color(MonoTheme::fg_muted())
                                                                    .overflow_hidden()
                                                                    .child(hit.subtitle.clone()),
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py_0p5()
                                                    .rounded(theme.radius(Radius::Sm))
                                                    .bg(MonoTheme::bg_hover())
                                                    .text_size(theme.text_size(TextSize::Xs))
                                                    .text_color(MonoTheme::fg_subtle())
                                                    .child(match hit.scope {
                                                        SearchScope::Conversations => "Thread",
                                                        SearchScope::Files => "File",
                                                        SearchScope::Projects => "Project",
                                                        SearchScope::All => "",
                                                    }),
                                            )
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.execute_search_hit(hit_clone.clone(), cx);
                                            })),
                                    );
                                }
                                el.child(list)
                            }),
                    )
                    // 4. Footer Hints
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .py_2()
                            .border_t_1()
                            .border_color(MonoTheme::border_stroke())
                            .bg(MonoTheme::bg_base())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_subtle())
                                    .child("↑↓ to navigate")
                                    .child("•")
                                    .child("↵ to select")
                                    .child("•")
                                    .child("esc to dismiss"),
                            )
                            .child(
                                div()
                                    .text_size(theme.text_size(TextSize::Xs))
                                    .text_color(MonoTheme::fg_subtle())
                                    .child(format!("{} results", hits_count)),
                            ),
                    ),
            )
    }

    fn render_scope_pill(
        &self,
        scope: SearchScope,
        label: &'static str,
        current: SearchScope,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let is_active = scope == current;

        div()
            .id(SharedString::from(format!("search-scope-pill-{:?}", scope)))
            .px_2p5()
            .py_1()
            .rounded(theme.radius(Radius::Sm))
            .bg(if is_active {
                MonoTheme::accent()
            } else {
                MonoTheme::bg_hover()
            })
            .text_color(if is_active {
                MonoTheme::on_accent()
            } else {
                MonoTheme::fg_muted()
            })
            .text_size(theme.text_size(TextSize::Xs))
            .font_weight(if is_active {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            })
            .cursor_pointer()
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.search_scope = scope;
                this.update_search_hits(cx);
            }))
    }
}
