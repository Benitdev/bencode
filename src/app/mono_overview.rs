//! A Mono's Overview on the app (MonoCode `MonoOverview`, `MonoSidebar`,
//! `App.tsx`'s `monoOverviewTab` and `monoActivity`): the tab it shows, its
//! width, the turn whose activity it shows, the artifact open over its
//! Documents (`artifacts.rs`), the habits read from the Mono's folder, and
//! the Changes tab's per-project state (`mono_changes.rs`). The views are
//! `ui/mono_overview/`.

use gpui::Context;

use crate::app::BenCodeApp;
use crate::app::mono_files::mono_files_dir;
use crate::monos::habits::Habit;

/// MonoCode `MonoSidebar`: `MIN_WIDTH`, `DEFAULT_WIDTH`, and the widest it
/// may grow (`Math.min(960, …)`).
pub const MIN_WIDTH: f32 = 340.0;
pub const DEFAULT_WIDTH: f32 = 400.0;
pub const MAX_WIDTH: f32 = 960.0;
/// MonoCode keeps the chat a good share: the Overview takes at most 62% of
/// the window.
const MAX_SHARE: f32 = 0.62;

/// MonoCode `MonoOverviewTab`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverviewTab {
    #[default]
    Sessions,
    Documents,
    Habits,
    Changes,
    /// The work of the turn picked from the chat (MonoCode's Activity
    /// button, apart from the tabs since it follows the chat).
    Activity,
}

/// MonoCode `MonoActivitySelection`: the turn whose steps the Activity tab
/// shows, by its first block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivitySelection {
    pub session_id: String,
    pub turn_id: String,
}

pub struct OverviewState {
    pub tab: OverviewTab,
    /// The tab the Activity button goes back to.
    last_tab: OverviewTab,
    /// Before the interface scale.
    pub width: f32,
    /// (pointer x, width) when the resize started.
    pub resizing: Option<(f32, f32)>,
    pub activity: Option<ActivitySelection>,
    /// The Mono whose habits these are.
    of: Option<String>,
    /// MonoCode `shownDocumentId`: the artifact open over the Documents
    /// list until Back.
    pub document: Option<String>,
    /// `None` until read.
    pub habits: Option<Vec<Habit>>,
    generation: u64,
}

impl Default for OverviewState {
    fn default() -> Self {
        Self {
            tab: OverviewTab::Sessions,
            last_tab: OverviewTab::Sessions,
            width: DEFAULT_WIDTH,
            resizing: None,
            activity: None,
            of: None,
            document: None,
            habits: None,
            generation: 0,
        }
    }
}

impl OverviewState {
    /// Another Mono starts at its own front page.
    pub fn mono_changed(&mut self) {
        self.activity = None;
        self.document = None;
        if self.tab == OverviewTab::Activity {
            self.tab = self.last_tab;
        }
    }

    /// MonoCode `TabButton`'s Activity: back to the tab it was opened from.
    pub fn toggle_activity(&mut self) {
        if self.tab == OverviewTab::Activity {
            self.tab = self.last_tab;
        } else {
            self.last_tab = self.tab;
            self.tab = OverviewTab::Activity;
        }
    }

    pub fn select(&mut self, tab: OverviewTab) {
        if tab != OverviewTab::Activity {
            self.last_tab = tab;
        }
        self.tab = tab;
        self.document = None;
    }
}

/// The Overview's width for a drag that started at (`start_x`, `start`)
/// and is at `x` in a window `window` wide: it grows leftwards, between
/// [`MIN_WIDTH`] and the smaller of [`MAX_WIDTH`] and its share.
pub fn resized_width(start_x: f32, start: f32, x: f32, window: f32) -> f32 {
    let max = (window * MAX_SHARE).round().clamp(MIN_WIDTH, MAX_WIDTH);
    (start + start_x - x).clamp(MIN_WIDTH, max)
}

impl BenCodeApp {
    /// The Mono's habits, read off the UI thread.
    pub(crate) fn load_mono_overview(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(dir) = mono_files_dir(id) else {
            return;
        };
        let overview = &mut self.monos.overview;
        if overview.of.as_deref() != Some(id) {
            overview.of = Some(id.to_string());
            overview.habits = None;
        }
        overview.generation += 1;
        let generation = overview.generation;
        let task = cx.background_executor().spawn(async move {
            match crate::monos::files::read(&dir, crate::monos::files::HABITS_FILE) {
                Ok((text, _)) => crate::monos::habits::parse_habits(text.as_deref().unwrap_or("")),
                Err(err) => {
                    log::warn!("could not read the Mono's habits: {err:#}");
                    Vec::new()
                }
            }
        });
        cx.spawn(async move |this, cx| {
            let habits = task.await;
            let landed = this.update(cx, |this, cx| {
                let overview = &mut this.monos.overview;
                if overview.generation == generation {
                    overview.habits = Some(habits);
                    cx.notify();
                }
            });
            if let Err(err) = landed {
                log::debug!("Mono overview after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// A tab of the Overview, which opens if it was hidden.
    pub fn select_mono_overview_tab(&mut self, tab: OverviewTab, cx: &mut Context<Self>) {
        self.monos.overview_open = true;
        self.monos.overview.select(tab);
        if let Some(id) = self.monos.view.clone() {
            match tab {
                OverviewTab::Habits => self.load_mono_overview(&id, cx),
                OverviewTab::Documents => {}
                OverviewTab::Changes => self.load_mono_changes(cx),
                OverviewTab::Sessions => self.load_mono_project_sessions(cx),
                OverviewTab::Activity => {}
            }
        }
        cx.notify();
    }

    /// MonoCode lists the sessions a Mono started; BenCode's Monos start
    /// none, so its Sessions tab lists the ones in its projects: those
    /// projects' recent threads are read in if they are not in memory yet.
    pub(crate) fn load_mono_project_sessions(&mut self, cx: &mut Context<Self>) {
        let Some(projects) = self.mono_in_view().map(|mono| mono.projects.clone()) else {
            return;
        };
        for project in projects {
            self.load_sessions_in_background(Some(project), cx);
        }
    }

    pub fn toggle_mono_activity(&mut self, cx: &mut Context<Self>) {
        self.monos.overview.toggle_activity();
        self.monos.overview.document = None;
        cx.notify();
    }

    /// MonoCode `onShowMonoActivity`: a turn's work on the Activity tab;
    /// picking the shown one again clears it.
    pub fn show_mono_activity(&mut self, session_id: &str, turn_id: &str, cx: &mut Context<Self>) {
        let overview = &mut self.monos.overview;
        let shown = self.monos.overview_open && overview.tab == OverviewTab::Activity;
        let picked = ActivitySelection {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
        };
        if overview.tab != OverviewTab::Activity {
            overview.last_tab = overview.tab;
        }
        overview.tab = OverviewTab::Activity;
        overview.document = None;
        overview.activity = if shown && overview.activity.as_ref() == Some(&picked) {
            None
        } else {
            Some(picked)
        };
        self.monos.overview_open = true;
        cx.notify();
    }

    /// The turn the Overview's Activity tab shows, which the chat marks.
    pub fn mono_activity_shown(&self, session_id: &str) -> Option<&str> {
        let overview = &self.monos.overview;
        (self.monos_overview_shown() && overview.tab == OverviewTab::Activity)
            .then_some(overview.activity.as_ref())
            .flatten()
            .filter(|picked| picked.session_id == session_id)
            .map(|picked| picked.turn_id.as_str())
    }

    /// The Overview is beside the Mono's chat.
    pub fn monos_overview_shown(&self) -> bool {
        self.mono_covers() && self.monos.overview_open
    }

    /// The end of a drag on the Overview's edge: its width is kept.
    pub(crate) fn end_mono_overview_resize(&mut self, cx: &mut Context<Self>) {
        if self.monos.overview.resizing.take().is_some() {
            self.save_settings(cx);
            cx.notify();
        }
    }
}

/// The threads a Mono can see: those in its projects (a worktree's too),
/// newest first, without drafts, archived ones or Monos' own chats.
/// MonoCode's Sessions tab lists the ones a Mono started; BenCode's start
/// none.
pub fn visible_sessions<'a>(
    sessions: &'a [crate::db::SessionRow],
    projects: &[String],
    is_mono: impl Fn(&str) -> bool,
) -> Vec<&'a crate::db::SessionRow> {
    let mut out: Vec<&crate::db::SessionRow> = sessions
        .iter()
        .filter(|s| !s.archived && !s.is_draft() && !is_mono(&s.id))
        .filter(|s| {
            projects
                .iter()
                .any(|project| crate::app::same_project_path(project, &s.cwd))
        })
        .collect();
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mono_sees_the_threads_in_its_projects() {
        let row = |id: &str, cwd: &str, at: i64| crate::db::SessionRow {
            id: id.into(),
            cwd: cwd.into(),
            title: format!("t {id}"),
            updated_at: at,
            blocks: vec![crate::db::Block {
                id: format!("b-{id}"),
                role: "user".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut archived = row("a", "/app", 9);
        archived.archived = true;
        let sessions = vec![
            row("1", "/app", 1),
            row("2", "/other", 5),
            row("3", "/app/", 3),
            archived,
            row("mono", "/app", 7),
        ];
        let seen: Vec<&str> = visible_sessions(&sessions, &["/app".into()], |id| id == "mono")
            .into_iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(seen, ["3", "1"]);
    }

    #[test]
    fn the_overview_grows_leftwards_within_its_bounds() {
        // Dragged 100px left in a wide window.
        assert_eq!(resized_width(1000.0, 400.0, 900.0, 2000.0), 500.0);
        assert_eq!(resized_width(1000.0, 400.0, 1200.0, 2000.0), MIN_WIDTH);
        assert_eq!(resized_width(1000.0, 400.0, 0.0, 3000.0), MAX_WIDTH);
        // The chat keeps its share of a narrow window.
        assert_eq!(resized_width(1000.0, 400.0, 0.0, 1000.0), 620.0);
        assert_eq!(resized_width(1000.0, 400.0, 0.0, 400.0), MIN_WIDTH);
    }

    #[test]
    fn activity_goes_back_to_the_tab_it_came_from() {
        let mut overview = OverviewState::default();
        overview.select(OverviewTab::Habits);
        overview.toggle_activity();
        assert_eq!(overview.tab, OverviewTab::Activity);
        overview.toggle_activity();
        assert_eq!(overview.tab, OverviewTab::Habits);
        overview.toggle_activity();
        overview.mono_changed();
        assert_eq!(overview.tab, OverviewTab::Habits);
    }
}
