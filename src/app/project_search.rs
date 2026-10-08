//! MonoCode `ProjectSearch` (Explorer › Search in files, ⌘⇧F): the query,
//! its toggles and globs, and the results of the last search. A search
//! starts 200ms after the last edit; a newer one cancels it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ely_gpui_component::forms::{InputEvent, TextInput};
use gpui::{Context, Entity, Focusable, ScrollHandle, Window};

use crate::app::{BenCodeApp, SidebarMode, text_input};
use crate::project_search::{SearchMatch, SearchOptions, SearchResult};

/// MonoCode waits this long after typing before it searches.
const DEBOUNCE: Duration = Duration::from_millis(200);

/// One of the query's toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchToggle {
    MatchCase,
    WholeWord,
    Regex,
}

pub struct ProjectSearchState {
    /// Shown in place of the Explorer's tree.
    pub open: bool,
    pub query_input: Entity<TextInput>,
    pub include_input: Entity<TextInput>,
    pub exclude_input: Entity<TextInput>,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub result: SearchResult,
    /// The options `result` answers, for highlighting its previews.
    pub searched: SearchOptions,
    /// The folder searched, which `result`'s paths are relative to.
    pub root: String,
    pub scroll: ScrollHandle,
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
}

impl ProjectSearchState {
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> Self {
        Self {
            open: false,
            query_input: text_input(window, cx, "Search"),
            include_input: text_input(window, cx, "files to include"),
            exclude_input: text_input(window, cx, "files to exclude"),
            case_sensitive: false,
            whole_word: false,
            regex: false,
            loading: false,
            error: None,
            result: SearchResult::default(),
            searched: SearchOptions::default(),
            root: String::new(),
            scroll: ScrollHandle::new(),
            generation: 0,
            cancel: None,
        }
    }

    pub fn is_on(&self, toggle: SearchToggle) -> bool {
        match toggle {
            SearchToggle::MatchCase => self.case_sensitive,
            SearchToggle::WholeWord => self.whole_word,
            SearchToggle::Regex => self.regex,
        }
    }

    fn options(&self, cx: &gpui::App) -> SearchOptions {
        let text = |input: &Entity<TextInput>| input.read(cx).text().trim().to_string();
        SearchOptions {
            query: text(&self.query_input),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
            include: text(&self.include_input),
            exclude: text(&self.exclude_input),
        }
    }

    /// Stops the search under way, if any; its result is dropped.
    fn cancel(&mut self) {
        self.generation += 1;
        if let Some(token) = self.cancel.take() {
            token.store(true, Ordering::Release);
        }
    }
}

impl BenCodeApp {
    /// MonoCode `onFindInProject`: the Explorer tab with the search shown
    /// and its query selected.
    pub fn open_project_search(&mut self, cx: &mut Context<Self>) {
        if self.surface.is_some() {
            self.close_surface(cx);
        }
        self.show_sidebar(SidebarMode::Files, cx);
        self.project_search.open = true;
        let input = self.project_search.query_input.clone();
        input.update(cx, |input, cx| {
            let len = input.text().len();
            input.select(0..len, cx);
        });
        crate::ui::composer::focus_later(input.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    /// Back to the tree; false when the search was not shown.
    pub fn close_project_search(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.project_search.open {
            return false;
        }
        self.project_search.open = false;
        self.project_search.cancel();
        self.project_search.loading = false;
        cx.notify();
        true
    }

    pub fn toggle_project_search_option(&mut self, toggle: SearchToggle, cx: &mut Context<Self>) {
        let state = &mut self.project_search;
        let flag = match toggle {
            SearchToggle::MatchCase => &mut state.case_sensitive,
            SearchToggle::WholeWord => &mut state.whole_word,
            SearchToggle::Regex => &mut state.regex,
        };
        *flag = !*flag;
        self.schedule_project_search(cx);
    }

    pub(crate) fn on_project_search_input(
        &mut self,
        input: &Entity<TextInput>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => self.schedule_project_search(cx),
            // MonoCode: Enter in the query opens the first result.
            InputEvent::Submit if *input == self.project_search.query_input => {
                if let Some(first) = self.project_search.result.matches.first().cloned() {
                    self.open_search_match(&first, window, cx);
                }
            }
            _ => {}
        }
    }

    /// Searches `DEBOUNCE` after the last change; an empty query clears.
    fn schedule_project_search(&mut self, cx: &mut Context<Self>) {
        let root = self.workspace_cwd();
        let options = self.project_search.options(cx);
        let state = &mut self.project_search;
        state.cancel();
        if options.query.is_empty() || matches!(root.trim(), "" | "~") {
            state.loading = false;
            state.error = None;
            state.result = SearchResult::default();
            state.searched = options;
            cx.notify();
            return;
        }
        let generation = state.generation;
        let token = Arc::new(AtomicBool::new(false));
        state.cancel = Some(token.clone());
        let timer = cx.background_executor().timer(DEBOUNCE);
        cx.spawn(async move |this, cx| {
            timer.await;
            let started = this.update(cx, |this, cx| {
                let current = this.project_search.generation == generation;
                if current {
                    this.project_search.loading = true;
                    this.project_search.error = None;
                    cx.notify();
                }
                current
            });
            if !matches!(started, Ok(true)) {
                return;
            }
            let (path, query) = (std::path::PathBuf::from(&root), options.clone());
            let result = cx
                .background_executor()
                .spawn(async move { crate::project_search::search(&path, &query, &token) })
                .await;
            let landed = this.update(cx, |this, cx| {
                let state = &mut this.project_search;
                if state.generation != generation {
                    return;
                }
                state.cancel = None;
                state.loading = false;
                state.root = root;
                state.searched = options;
                match result {
                    Ok(result) => {
                        state.error = None;
                        state.result = result;
                    }
                    Err(err) => {
                        log::warn!("search in files failed: {err}");
                        state.error = Some(err);
                        state.result = SearchResult::default();
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("search in files after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// Opens a result in the editor with its match selected.
    pub fn open_search_match(&mut self, hit: &SearchMatch, window: &mut Window, cx: &mut Context<Self>) {
        let path = std::path::Path::new(&self.project_search.root)
            .join(&hit.relative)
            .to_string_lossy()
            .into_owned();
        let len = self.project_search.searched.query.len();
        // A regex match's length is unknown; select nothing then.
        let len = if self.project_search.searched.regex { 0 } else { len };
        self.open_file_at(&path, hit.line, hit.column, len, window, cx);
    }
}
