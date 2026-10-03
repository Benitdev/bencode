//! Universal search over threads, workspace files and recent projects.

use ely_gpui_component::buttons::SegmentedControl;
use ely_gpui_component::data_display::Tag;
use ely_gpui_component::feedback::EmptyState;
use ely_gpui_component::forms::{InputEvent, SearchInput};
use ely_gpui_component::lists::ListItem;
use ely_gpui_component::primitives::{Icon, IconName};
use ely_gpui_component::theme::{ActiveTheme, ControlSize, IconSize};
use gpui::{
    AnyElement, Context, Focusable, IntoElement, ParentElement, SharedString, Styled, div, px,
    uniform_list,
};

use crate::app::{BenCodeApp, Surface, ViewMode};
use crate::db::SessionRow;

const SEARCH_FILE_HIT_LIMIT: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchScope {
    All,
    Conversations,
    Files,
    Projects,
}

const SCOPES: [(SearchScope, &str, &str); 4] = [
    (SearchScope::All, "all", "All"),
    (SearchScope::Conversations, "threads", "Threads"),
    (SearchScope::Files, "files", "Files"),
    (SearchScope::Projects, "projects", "Projects"),
];

impl SearchScope {
    fn key(self) -> &'static str {
        SCOPES
            .iter()
            .find(|(scope, ..)| *scope == self)
            .map_or("all", |(_, key, _)| key)
    }

    fn from_key(key: &str) -> Option<Self> {
        SCOPES
            .iter()
            .find(|(_, k, _)| *k == key)
            .map(|(scope, ..)| *scope)
    }

    fn includes(self, other: SearchScope) -> bool {
        self == SearchScope::All || self == other
    }

    fn tag(self) -> &'static str {
        match self {
            SearchScope::Conversations => "Thread",
            SearchScope::Files => "File",
            SearchScope::Projects => "Project",
            SearchScope::All => "",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub title: SharedString,
    pub subtitle: SharedString,
    pub scope: SearchScope,
    pub icon: IconName,
    pub target_id: String,
}

fn session_hits(query: &str, sessions: &[SessionRow]) -> impl Iterator<Item = SearchHit> {
    sessions
        .iter()
        .filter(move |s| {
            [&s.title, &s.cwd, &s.model]
                .iter()
                .any(|field| field.to_lowercase().contains(query))
                || s.blocks.iter().any(|b| {
                    b.text
                        .as_deref()
                        .is_some_and(|t| t.to_lowercase().contains(query))
                })
        })
        .map(|s| SearchHit {
            title: s.title.clone().into(),
            subtitle: format!("{} • {}", s.model, s.cwd).into(),
            scope: SearchScope::Conversations,
            icon: IconName::MessageSquare,
            target_id: s.id.clone(),
        })
}

fn file_hits(query: &str, root: &str, files: &[SharedString]) -> impl Iterator<Item = SearchHit> {
    files
        .iter()
        .filter(move |path| path.to_lowercase().contains(query))
        .take(SEARCH_FILE_HIT_LIMIT)
        .map(move |path| SearchHit {
            title: path.rsplit('/').next().unwrap_or(path).to_string().into(),
            subtitle: format!("{root}/{path}").into(),
            scope: SearchScope::Files,
            icon: IconName::FileText,
            target_id: path.to_string(),
        })
}

fn project_hits(query: &str, projects: &[String]) -> impl Iterator<Item = SearchHit> {
    projects
        .iter()
        .filter(move |p| p.to_lowercase().contains(query))
        .map(|p| SearchHit {
            title: std::path::Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(p)
                .to_string()
                .into(),
            subtitle: p.clone().into(),
            scope: SearchScope::Projects,
            icon: IconName::Folder,
            target_id: p.clone(),
        })
}

/// Everything in `scope` matching `query`, threads first, then files, then projects.
fn collect_hits(query: &str, scope: SearchScope, app: &BenCodeApp) -> Vec<SearchHit> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    if scope.includes(SearchScope::Conversations) {
        hits.extend(session_hits(&query, &app.sessions));
    }
    if scope.includes(SearchScope::Files) {
        hits.extend(file_hits(&query, &app.workspace.cwd, &app.workspace.files));
    }
    if scope.includes(SearchScope::Projects) {
        hits.extend(project_hits(&query, &app.recent_projects));
    }
    hits
}

impl BenCodeApp {
    pub fn open_search_modal(&mut self, cx: &mut Context<Self>) {
        self.show_surface(Surface::Search, cx);
        self.search_scope = SearchScope::All;
        self.search_focus_pending = true;
        if self.search_submit.is_none() {
            self.search_submit = Some(cx.subscribe(
                &self.search_modal_input,
                |this, _, event: &InputEvent, cx| {
                    if *event == InputEvent::Submit {
                        this.open_search_hit(this.search_active_index, cx);
                    }
                },
            ));
        }
        self.search_modal_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.update_search_hits(cx);
    }

    pub fn close_search_modal(&mut self, cx: &mut Context<Self>) {
        if self.surface_open(Surface::Search) {
            self.close_surface(cx);
        }
    }

    pub fn update_search_hits(&mut self, cx: &mut Context<Self>) {
        let query = self.search_modal_input.read(cx).text().to_string();
        self.search_hits = collect_hits(&query, self.search_scope, self);
        self.search_active_index = 0;
        cx.notify();
    }

    /// Opens the latest thread in `cwd`, or starts one there.
    fn open_project(&mut self, cwd: String, cx: &mut Context<Self>) {
        self.active_view_mode = ViewMode::Chat;
        self.switch_project(cwd, cx);
    }

    fn open_search_hit(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(hit) = self.search_hits.get(ix).cloned() else {
            return;
        };
        match hit.scope {
            SearchScope::Conversations => {
                self.active_view_mode = ViewMode::Chat;
                self.open_session(hit.target_id, cx);
            }
            SearchScope::Files
                if self
                    .workspace
                    .changes
                    .iter()
                    .any(|c| c.path == hit.target_id) =>
            {
                self.active_view_mode = ViewMode::Changes;
                self.select_diff_path(hit.target_id, cx);
            }
            SearchScope::Files => {
                self.active_view_mode = ViewMode::Chat;
                self.append_to_prompt(&format!("@{}", hit.target_id), cx);
            }
            SearchScope::Projects => self.open_project(hit.target_id, cx),
            SearchScope::All => {}
        }
        self.close_search_modal(cx);
    }

    pub(crate) fn render_search_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if std::mem::take(&mut self.search_focus_pending) {
            self.focus_search_input(cx);
        }
        let scopes = SCOPES.iter().fold(
            SegmentedControl::new("search-scope", self.search_scope.key()).size(ControlSize::Sm),
            |control, (_, key, label)| control.segment(*key, *label, None),
        );
        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .max_w(px(720.0))
            .mx_auto()
            .p_4()
            .child(SearchInput::new(
                "search-modal-query",
                &self.search_modal_input,
            ))
            .child(
                scopes.on_change(cx.listener(|this, key: &SharedString, _, cx| {
                    this.search_scope = SearchScope::from_key(key).unwrap_or(SearchScope::All);
                    this.update_search_hits(cx);
                })),
            )
            .child(self.render_search_results(cx))
            .into_any_element()
    }

    /// The dialog takes focus as it opens, so the query field takes it back after that frame.
    fn focus_search_input(&self, cx: &mut Context<Self>) {
        let focus = self.search_modal_input.read(cx).focus_handle(cx);
        cx.defer(move |cx| {
            let Some(window) = cx.active_window() else {
                return;
            };
            if let Err(err) = window.update(cx, |_, window, cx| window.focus(&focus, cx)) {
                log::warn!("search: could not focus the query field: {err:#}");
            }
        });
    }

    fn render_search_results(&self, cx: &Context<Self>) -> AnyElement {
        let height = cx.theme().palette_size().height;
        if self.search_modal_input.read(cx).text().trim().is_empty() {
            return EmptyState::new(
                "search-idle",
                IconName::Search,
                "Find threads, files and projects",
            )
            .into_any_element();
        }
        if self.search_hits.is_empty() {
            return EmptyState::new("search-none", IconName::SearchX, "No results")
                .body("Try another word or a wider scope.")
                .into_any_element();
        }
        let rows = cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
            range
                .map(|ix| this.render_search_row(ix, cx))
                .collect::<Vec<_>>()
        });
        div()
            .h(height)
            .child(uniform_list("search-hits", self.search_hits.len(), rows).size_full())
            .into_any_element()
    }

    fn render_search_row(&self, ix: usize, cx: &Context<Self>) -> ListItem {
        let hit = &self.search_hits[ix];
        let muted = cx.theme().colors.fg_muted;
        ListItem::new(("search-hit", ix), hit.title.clone())
            .description(hit.subtitle.clone())
            .leading(Icon::new(hit.icon).size(IconSize::Sm).color(muted))
            .trailing(Tag::new(("search-hit-tag", ix), hit.scope.tag()))
            .current(ix == self.search_active_index)
            .on_click(cx.listener(move |this, _, _, cx| this.open_search_hit(ix, cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, title: &str) -> SessionRow {
        SessionRow {
            id: id.into(),
            title: title.into(),
            ..Default::default()
        }
    }

    #[test]
    fn scope_keys_round_trip() {
        for (scope, key, _) in SCOPES {
            assert_eq!(SearchScope::from_key(scope.key()), Some(scope));
            assert_eq!(scope.key(), key);
        }
        assert!(SearchScope::All.includes(SearchScope::Files));
        assert!(!SearchScope::Projects.includes(SearchScope::Files));
    }

    #[test]
    fn hits_match_case_insensitively_and_cap_files() {
        let sessions = [session("a", "Fix Parser"), session("b", "Docs")];
        let threads: Vec<_> = session_hits("parser", &sessions).collect();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].target_id, "a");

        let files: Vec<SharedString> = (0..50).map(|i| format!("src/mod{i}.rs").into()).collect();
        let hits: Vec<_> = file_hits("mod", "/repo", &files).collect();
        assert_eq!(hits.len(), SEARCH_FILE_HIT_LIMIT);
        assert_eq!(hits[0].title.as_ref(), "mod0.rs");
        assert_eq!(hits[0].subtitle.as_ref(), "/repo/src/mod0.rs");
    }

    #[test]
    fn project_hits_use_folder_name() {
        let projects = vec!["/home/me/bencode".to_string(), "/tmp/other".to_string()];
        let hits: Vec<_> = project_hits("ben", &projects).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title.as_ref(), "bencode");
    }
}
