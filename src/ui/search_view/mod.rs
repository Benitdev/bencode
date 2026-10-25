//! Search (⌘K): a dialog over the window that finds threads, the
//! project's files and recent projects in one list. With no query it lists what was used last; typing ranks the
//! matches (`hits.rs`) and marks what matched, a transcript match showing
//! the line it is in. All lists a few of each kind under a heading; Tab
//! moves to one kind to see every match. The keyboard drives it: ↑↓, ↵,
//! ⌘↵ to mention a file in the prompt, Tab / ⇧Tab, Esc.
//!
//! A query looks through every transcript, so it is matched on the
//! background executor, over threads and projects taken when the dialog
//! opened. A keystroke drops the search before it (one already matching
//! runs out, its answer unused), and a query waits `SEARCH_PAUSE` first,
//! so typing does not start a search per letter.

mod hits;

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use bencode_ui::buttons::SegmentedControl;
use bencode_ui::data_display::Tag;
use bencode_ui::feedback::EmptyState;
use bencode_ui::primitives::{Backdrop, Icon, IconName};
use bencode_ui::theme::{ActiveTheme, ControlSize, IconSize};
use bencode_ui::typography::Kbd;
use gpui::{
    AnyElement, Context, Focusable, FontWeight, HighlightStyle, InteractiveElement, IntoElement,
    ListAlignment, ListState, MouseButton, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, StyledText, Task, Window, div, list, prelude::*,
};

pub use hits::SearchScope;
use hits::{Found, HitKind, Marked, SearchHit, Sources};

use crate::app::BenCodeApp;
use crate::app::file_pane::PaneTab;
use crate::db::SessionRow;
use crate::ui::app_callback::app_callback;
use crate::ui::composer::focus_later;
use crate::ui::file_tree::resolve_entry_icon;
use crate::ui::provider_icon::HarnessIcon;
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Status, Text, Tone as _};

const DIALOG_WIDTH: f32 = 680.0;
const DIALOG_HEIGHT: f32 = 540.0;
const ROW_HEIGHT: f32 = 48.0;

const SCOPES: [(SearchScope, &str, &str); 4] = [
    (SearchScope::All, "all", "All"),
    (SearchScope::Conversations, "threads", "Threads"),
    (SearchScope::Files, "files", "Files"),
    (SearchScope::Projects, "projects", "Projects"),
];

fn scope_key(scope: SearchScope) -> &'static str {
    SCOPES
        .iter()
        .find(|(s, ..)| *s == scope)
        .map_or("all", |(_, key, _)| key)
}

fn scope_from_key(key: &str) -> Option<SearchScope> {
    SCOPES.iter().find(|(_, k, _)| *k == key).map(|(s, ..)| *s)
}

/// What one line of the list is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    /// The heading over a kind, in All.
    Heading(SearchScope),
    /// A hit, by its index in `hits`.
    Hit(usize),
}

/// How long a query waits for the next keystroke before it is matched.
const SEARCH_PAUSE: Duration = Duration::from_millis(40);

/// The threads and projects a search looks through, taken when the dialog
/// opens so the matching can run off the UI thread.
struct Corpus {
    sessions: Vec<SessionRow>,
    projects: Vec<String>,
    home: String,
}

pub struct SearchState {
    pub open: bool,
    pub scope: SearchScope,
    corpus: Option<Arc<Corpus>>,
    /// The search under way; replacing it cancels the one before.
    searching: Option<Task<()>>,
    found: Found,
    rows: Vec<Row>,
    list: ListState,
    /// The hit Enter opens.
    active: usize,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            open: false,
            scope: SearchScope::All,
            corpus: None,
            searching: None,
            found: Found::default(),
            rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(400.0)),
            active: 0,
        }
    }
}

impl SearchState {
    fn set(&mut self, found: Found) {
        let headed = self.scope == SearchScope::All;
        self.rows.clear();
        for (ix, hit) in found.hits.iter().enumerate() {
            let kind = hit.scope();
            if headed && (ix == 0 || found.hits[ix - 1].scope() != kind) {
                self.rows.push(Row::Heading(kind));
            }
            self.rows.push(Row::Hit(ix));
        }
        self.found = found;
        self.active = 0;
        self.list.reset(self.rows.len());
    }

    fn active_hit(&self) -> Option<&SearchHit> {
        self.found.hits.get(self.active)
    }

    /// Moves the active hit by `by`, wrapping, and keeps it in view.
    fn step(&mut self, by: isize) {
        let count = self.found.hits.len();
        if count == 0 {
            return;
        }
        self.active = (self.active as isize + by).rem_euclid(count as isize) as usize;
        let row = self.rows.iter().position(|r| *r == Row::Hit(self.active));
        // The heading over the first hit of a kind comes into view with it.
        let row = row.map(|r| match r.checked_sub(1).map(|h| self.rows[h]) {
            Some(Row::Heading(_)) => r - 1,
            _ => r,
        });
        if let Some(row) = row {
            self.list.scroll_to_reveal_item(row);
        }
    }
}

/// `text` with its marks in the accent.
fn marked(text: &Marked, cx: &gpui::App) -> StyledText {
    let style = HighlightStyle {
        color: Some(cx.theme().colors.accent),
        font_weight: Some(FontWeight::SEMIBOLD),
        ..Default::default()
    };
    StyledText::new(text.text.clone())
        .with_highlights(text.marks.iter().map(|m: &Range<usize>| (m.clone(), style)))
}

impl BenCodeApp {
    /// ⌘K and the rail's Search: opens the dialog, or closes it.
    pub fn toggle_search(&mut self, cx: &mut Context<Self>) {
        if !self.close_search_modal(cx) {
            self.open_search_modal(cx);
        }
    }

    pub fn open_search_modal(&mut self, cx: &mut Context<Self>) {
        self.close_quick_open(cx);
        self.index_project_files(cx);
        self.search.open = true;
        self.search.scope = SearchScope::All;
        self.search.corpus = Some(Arc::new(Corpus {
            sessions: self.sessions.clone(),
            projects: self.recent_projects.clone(),
            home: std::env::var("HOME").unwrap_or_default(),
        }));
        self.search_modal_input
            .update(cx, |input, cx| input.set_text("", cx));
        focus_later(self.search_modal_input.read(cx).focus_handle(cx), cx);
        self.update_search_hits(cx);
        cx.notify();
    }

    /// Closes the dialog; `false` when it was not open.
    pub fn close_search_modal(&mut self, cx: &mut Context<Self>) -> bool {
        if !std::mem::take(&mut self.search.open) {
            return false;
        }
        self.search.corpus = None;
        self.search.searching = None;
        self.refocus_prompt(cx);
        cx.notify();
        true
    }

    /// The dialog is the shell's, not a region's: notifying its query
    /// field redraws the shell while every region is reused as painted.
    fn redraw_search(&self, cx: &mut Context<Self>) {
        self.search_modal_input.update(cx, |_, cx| cx.notify());
    }

    /// Searches for the query field's text. With no query the recent
    /// items show at once; a query is matched off the UI thread.
    pub fn update_search_hits(&mut self, cx: &mut Context<Self>) {
        let Some(corpus) = self.search.corpus.clone() else {
            return;
        };
        let query = self.search_modal_input.read(cx).text().to_string();
        let scope = self.search.scope;
        let files = self.project_files.files.clone();
        let recent_files = self.quick_open_recents();
        let changed: HashSet<String> = self
            .workspace(cx)
            .changes
            .iter()
            .map(|c| c.path.clone())
            .collect();
        let find = move || {
            hits::find(
                &query,
                scope,
                &Sources {
                    sessions: &corpus.sessions,
                    files: &files,
                    recent_files: &recent_files,
                    projects: &corpus.projects,
                    changed: &|path| changed.contains(path),
                    home: &corpus.home,
                },
            )
        };
        if self.search_query_empty(cx) {
            self.search.searching = None;
            self.search.set(find());
            self.redraw_search(cx);
            return;
        }
        let executor = cx.background_executor().clone();
        self.search.searching = Some(cx.spawn(async move |this, cx| {
            executor.timer(SEARCH_PAUSE).await;
            let found = executor.spawn(async move { find() }).await;
            let landed = this.update(cx, |app, cx| {
                app.search.searching = None;
                app.search.set(found);
                app.redraw_search(cx);
            });
            if let Err(err) = landed {
                log::debug!("search results after app drop: {err:#}");
            }
        }));
    }

    fn set_search_scope(&mut self, scope: SearchScope, cx: &mut Context<Self>) {
        self.search.scope = scope;
        self.update_search_hits(cx);
    }

    /// The keys the query field gives Search; `true` when one was used.
    pub fn search_key(
        &mut self,
        key: &str,
        shift: bool,
        mention: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match key {
            "down" => self.search.step(1),
            "up" => self.search.step(-1),
            "enter" if mention => self.mention_search_hit(cx),
            "enter" => self.open_search_hit(self.search.active, window, cx),
            "escape" => {
                self.close_search_modal(cx);
                return true;
            }
            "tab" => {
                let at = SCOPES
                    .iter()
                    .position(|(s, ..)| *s == self.search.scope)
                    .unwrap_or(0);
                let next = if shift { at + SCOPES.len() - 1 } else { at + 1 };
                self.set_search_scope(SCOPES[next % SCOPES.len()].0, cx);
            }
            _ => return false,
        }
        if self.search.open {
            self.redraw_search(cx);
        }
        true
    }

    fn open_search_hit(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.search.found.hits.get(ix).cloned() else {
            return;
        };
        self.close_search_modal(cx);
        self.leave_surfaces(cx);
        match hit.kind {
            HitKind::Thread { .. } => self.open_session(hit.target_id, cx),
            HitKind::File { changed: true } => {
                let tab = PaneTab::Changes {
                    cwd: self.workspace(cx).cwd.clone(),
                    side: None,
                    focus: Some(hit.target_id),
                };
                self.open_pane_tab(tab, false, cx);
            }
            HitKind::File { changed: false } => {
                self.open_file_in_editor(&hit.target_id, window, cx)
            }
            HitKind::Project { .. } => self.switch_project(hit.target_id, cx),
        }
    }

    /// ⌘↵ on a file: `@path` into the prompt.
    fn mention_search_hit(&mut self, cx: &mut Context<Self>) {
        let Some(hit) = self.search.active_hit() else {
            return;
        };
        if !matches!(hit.kind, HitKind::File { .. }) {
            return;
        }
        let mention = format!("@{}", hit.target_id);
        self.close_search_modal(cx);
        self.leave_surfaces(cx);
        self.append_to_prompt(&mention, cx);
    }

    /// What Search opens shows in the workspace, so a view over it closes
    /// (Settings first goes back to the view it came from, which closes too).
    fn leave_surfaces(&mut self, cx: &mut Context<Self>) {
        while self.surface.is_some() {
            self.close_surface(cx);
        }
    }

    fn search_query_empty(&self, cx: &gpui::App) -> bool {
        self.search_modal_input.read(cx).text().trim().is_empty()
    }

    pub(crate) fn render_search_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.search.open {
            return None;
        }
        let colors = &cx.theme().colors;
        let panel = div()
            .id("search-dialog")
            .flex()
            .flex_col()
            .w(px(DIALOG_WIDTH))
            .h(px(DIALOG_HEIGHT))
            .max_h(gpui::relative(0.8))
            .overflow_hidden()
            .rounded(px(12.0))
            .border_1()
            .border_color(colors.fg.fill(Fill::Bold))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(crate::ui::frost::dialog_frost(
                crate::ui::frost::POPOVER_RADIUS,
                colors.surface,
                cx,
            ))
            .child(self.render_search_field(cx))
            .child(self.render_search_scopes(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .px_2()
                    .pb_1()
                    .child(self.render_search_results(cx)),
            )
            .child(self.render_search_hints(cx));
        Some(
            Backdrop::new("search-scrim")
                .on_dismiss(app_callback(cx, |this, cx| {
                    this.close_search_modal(cx);
                }))
                .child(panel)
                .into_any_element(),
        )
    }

    fn render_search_field(&self, cx: &Context<Self>) -> impl IntoElement {
        let fg = cx.theme().colors.fg;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h(px(52.0))
            .px_4()
            .border_b_1()
            .border_color(cx.theme().colors.border)
            .child(
                Icon::new(IconName::Search)
                    .size(IconSize::Md)
                    .color(fg.ink(Ink::Secondary)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(Text::Lg.px())
                    .child(self.search_modal_input.clone()),
            )
            .child(Kbd::new("escape"))
    }

    /// The kinds, each with how many matched.
    fn render_search_scopes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let counted = !self.search_query_empty(cx);
        let control = SCOPES.iter().fold(
            SegmentedControl::new("search-scope", scope_key(self.search.scope))
                .size(ControlSize::Sm),
            |control, (scope, key, label)| {
                let total = self.search.found.total(*scope);
                let shown = if counted && (*scope == self.search.scope || total > 0) {
                    format!("{label}  {total}")
                } else {
                    label.to_string()
                };
                control.segment(*key, shown, None)
            },
        );
        div()
            .flex()
            .flex_none()
            .px_3()
            .pt_2p5()
            .pb_1()
            .child(
                control.on_change(cx.listener(|this, key: &SharedString, _, cx| {
                    this.set_search_scope(scope_from_key(key).unwrap_or(SearchScope::All), cx)
                })),
            )
    }

    fn render_search_results(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.search.rows.is_empty() {
            let empty = if self.search_query_empty(cx) {
                EmptyState::new(
                    "search-idle",
                    IconName::Search,
                    "Find threads, files and projects",
                )
                .body("Words match titles, projects and what was said.")
            } else if self.search.scope == SearchScope::All {
                EmptyState::new("search-none", IconName::SearchX, "No results")
                    .body("Try fewer or other words.")
            } else {
                EmptyState::new("search-none", IconName::SearchX, "No results here")
                    .body("Tab back to All to look everywhere.")
            };
            return div().pt_12().child(empty).into_any_element();
        }
        let rows = list(
            self.search.list.clone(),
            cx.processor(|this, ix: usize, _, cx| this.render_search_row(ix, cx)),
        )
        .size_full()
        .pr(crate::ui::scrollbar::gutter(&self.search.list));
        crate::ui::scrollbar::framed("search-hits-scrollbar", &self.search.list, rows)
            .into_any_element()
    }

    fn render_search_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        match self.search.rows.get(ix) {
            Some(Row::Heading(kind)) => self.render_search_heading(*kind, cx),
            Some(Row::Hit(hit)) => self.render_search_hit(*hit, cx),
            None => div().into_any_element(),
        }
    }

    /// "Threads 24", and how many more the kind's own tab holds.
    fn render_search_heading(&self, kind: SearchScope, cx: &Context<Self>) -> AnyElement {
        let fg = cx.theme().colors.fg;
        let recent = self.search_query_empty(cx);
        let label = match kind {
            SearchScope::Conversations => "Threads",
            SearchScope::Files => "Files",
            _ => "Projects",
        };
        let total = self.search.found.total(kind);
        let shown = self
            .search
            .found
            .hits
            .iter()
            .filter(|h| h.scope() == kind)
            .count();
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .pt_4()
            .pb_1p5()
            .text_size(Text::Sm.px())
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg.ink(Ink::Secondary))
                    .child(if recent {
                        format!("Recent {}", label.to_lowercase())
                    } else {
                        label.to_string()
                    }),
            )
            .when(!recent, |el| {
                el.child(
                    div()
                        .text_color(fg.ink(Ink::Faint))
                        .child(total.to_string()),
                )
            })
            .child(div().flex_1())
            .when(!recent && total > shown, |el| {
                el.child(
                    div()
                        .text_color(fg.ink(Ink::Muted))
                        .child(format!("{} more", total - shown)),
                )
            })
            .into_any_element()
    }

    fn render_search_hit(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let hit = &self.search.found.hits[ix];
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let active = ix == self.search.active;
        let tile = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(30.0))
            .rounded(px(7.0))
            .bg(fg.fill(if active { Fill::Strong } else { Fill::Hover }));
        let tile = match &hit.kind {
            HitKind::Thread { harness, .. } => tile.child(HarnessIcon::new(harness).size(px(15.0))),
            HitKind::File { .. } => {
                tile.child(resolve_entry_icon(&hit.title.text, false, false).size(IconSize::Sm))
            }
            HitKind::Project { .. } => tile.child(
                Icon::new(IconName::Folder)
                    .size(IconSize::Sm)
                    .color(fg.ink(Ink::Secondary)),
            ),
        };
        let detail = div()
            .mt(px(2.0))
            .truncate()
            .text_size(Text::Sm.px())
            .text_color(fg.ink(Ink::Muted))
            .when(matches!(hit.kind, HitKind::File { .. }), |el| {
                el.font_family(cx.theme().mono_family.clone())
            })
            .child(marked(&hit.detail, cx));
        let meta: AnyElement = match &hit.kind {
            HitKind::Thread { updated_at, .. } => {
                let running = self.runs.contains_key(&hit.target_id);
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Faint))
                    .when(running, |el| {
                        el.child(
                            div()
                                .size(px(6.0))
                                .rounded_full()
                                .bg(Status::Working.color(fg)),
                        )
                    })
                    .child(crate::ui::relative_time::since(
                        *updated_at,
                        crate::app::now_ms(),
                    ))
                    .into_any_element()
            }
            HitKind::File { changed: true } => {
                Tag::new(("search-hit-changed", ix), "Changed").into_any_element()
            }
            HitKind::File { changed: false } => div().into_any_element(),
            HitKind::Project { threads } => div()
                .text_size(Text::Sm.px())
                .text_color(fg.ink(Ink::Faint))
                .child(match threads {
                    1 => "1 thread".to_string(),
                    n => format!("{n} threads"),
                })
                .into_any_element(),
        };
        div()
            .id(("search-hit", ix))
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .h(px(ROW_HEIGHT))
            .px_2()
            .rounded(px(8.0))
            .cursor_pointer()
            .map(|el| {
                if active {
                    el.bg(fg.fill(Fill::Selected))
                } else {
                    el.hover(|s| s.bg(fg.fill(Fill::Hover)))
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| this.open_search_hit(ix, window, cx)))
            .child(tile)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(Text::Md.px())
                            .text_color(fg.ink(Ink::Strong))
                            .child(marked(&hit.title, cx)),
                    )
                    .when(!hit.detail.text.is_empty(), |el| el.child(detail)),
            )
            .child(div().flex_none().child(meta))
            .into_any_element()
    }

    /// The keys Search answers, along its foot.
    fn render_search_hints(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let hint = |keys: &[&str], label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .children(keys.iter().map(|k| Kbd::new(k)))
                .child(div().text_color(fg.ink(Ink::Muted)).child(label))
        };
        let file = matches!(
            self.search.active_hit().map(|h| &h.kind),
            Some(HitKind::File { .. })
        );
        let count = self.search.found.total(self.search.scope);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_5()
            .h(px(34.0))
            .px_4()
            .border_t_1()
            .border_color(colors.border)
            .text_size(Text::Sm.px())
            .child(hint(&["up", "down"], "Move"))
            .child(hint(&["enter"], "Open"))
            .when(file, |el| el.child(hint(&["secondary-enter"], "Mention")))
            .child(hint(&["tab"], "Scope"))
            .child(div().flex_1())
            .when(!self.search_query_empty(cx), |el| {
                el.child(div().text_color(fg.ink(Ink::Faint)).child(match count {
                    1 => "1 result".to_string(),
                    n => format!("{n} results"),
                }))
            })
    }
}
