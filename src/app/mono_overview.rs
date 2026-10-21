//! A Mono's Overview on the app (MonoCode `MonoOverview`, `MonoSidebar`,
//! `App.tsx`'s `monoOverviewTab` and `monoActivity`): the tab it shows, its
//! width, the turn whose activity it shows, the artifact open over its
//! Documents (`artifacts.rs`), the habits and board read from the Mono's
//! folder, and the Changes tab's per-project state (`mono_changes.rs`). The
//! views are `ui/mono_overview/`.

use std::collections::HashSet;

use ely_gpui_component::forms::{InputEvent, TextInput};
use gpui::{AppContext as _, Context, Entity, Focusable as _, Window};

use crate::app::BenCodeApp;
use crate::app::mono_files::mono_files_dir;
use crate::monos::board::Task;
use crate::monos::habits::{Habit, HabitSchedule};

/// MonoCode `MonoSidebar`: `MIN_WIDTH`, `DEFAULT_WIDTH`, and the widest it
/// may grow (`Math.min(960, …)`).
pub const MIN_WIDTH: f32 = 340.0;
pub const DEFAULT_WIDTH: f32 = 400.0;
pub const MAX_WIDTH: f32 = 960.0;
/// MonoCode keeps the chat a good share: the Overview takes at most 62% of
/// the window.
const MAX_SHARE: f32 = 0.62;

/// MonoCode `MonoOverviewTab`, with Board where MonoCode lists Sessions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverviewTab {
    /// The Mono's tasks by column, then the other sessions it started.
    #[default]
    Board,
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

/// MonoCode `Route` over the Habits list: a habit's page, or New habit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HabitRoute {
    New,
    Habit(String),
}

/// MonoCode `NewHabitPage`'s fields.
pub struct HabitForm {
    pub name: Entity<TextInput>,
    pub instructions: Entity<TextInput>,
    pub schedule: HabitSchedule,
    pub saving: bool,
    pub error: Option<String>,
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
    /// A habit's page, or New habit, over the Habits list until Back.
    pub habit_route: Option<HabitRoute>,
    /// New habit's fields, while it is open.
    pub habit_form: Option<HabitForm>,
    /// Why the last change to its habits failed.
    pub habit_error: Option<String>,
    /// `None` until read.
    pub board: Option<Vec<Task>>,
    /// Why the last change to its board failed.
    pub board_error: Option<String>,
    /// The Board's Done column shown.
    pub done_open: bool,
    /// The runs opened on a habit's page, by (habit, when).
    pub open_runs: HashSet<(String, i64)>,
    /// MonoCode `monoShownSession`: a session the Mono started, open over
    /// everything in the Overview until Back.
    pub shown_session: Option<String>,
    /// MonoCode `monoShownSessionFocused`: its composer, rather than the
    /// Mono's, holds the prompt.
    pub shown_focused: bool,
    generation: u64,
}

impl Default for OverviewState {
    fn default() -> Self {
        Self {
            tab: OverviewTab::Board,
            last_tab: OverviewTab::Board,
            width: DEFAULT_WIDTH,
            resizing: None,
            activity: None,
            of: None,
            document: None,
            habits: None,
            habit_route: None,
            habit_form: None,
            habit_error: None,
            board: None,
            board_error: None,
            done_open: false,
            open_runs: HashSet::new(),
            shown_session: None,
            shown_focused: false,
            generation: 0,
        }
    }
}

impl OverviewState {
    /// Another Mono starts at its own front page.
    pub fn mono_changed(&mut self) {
        self.shown_session = None;
        self.shown_focused = false;
        self.activity = None;
        self.document = None;
        self.habit_route = None;
        self.habit_form = None;
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
        self.habit_route = None;
        self.habit_form = None;
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
    /// The Mono's habits and board, read off the UI thread.
    pub(crate) fn load_mono_overview(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(dir) = mono_files_dir(id) else {
            return;
        };
        let overview = &mut self.monos.overview;
        if overview.of.as_deref() != Some(id) {
            overview.of = Some(id.to_string());
            overview.habits = None;
            overview.board = None;
            overview.board_error = None;
        }
        overview.generation += 1;
        let generation = overview.generation;
        let task = cx.background_executor().spawn(async move {
            let habits = match crate::monos::files::read(&dir, crate::monos::files::HABITS_FILE) {
                Ok((text, _)) => crate::monos::habits::parse_habits(text.as_deref().unwrap_or("")),
                Err(err) => {
                    log::warn!("could not read the Mono's habits: {err:#}");
                    Vec::new()
                }
            };
            let board = crate::monos::board::load_board(&dir).unwrap_or_else(|err| {
                log::warn!("could not read the Mono's board: {err}");
                Vec::new()
            });
            (habits, board)
        });
        cx.spawn(async move |this, cx| {
            let (habits, board) = task.await;
            let landed = this.update(cx, |this, cx| {
                let overview = &mut this.monos.overview;
                if overview.generation == generation {
                    overview.habits = Some(habits);
                    overview.board = Some(board);
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
        self.hide_mono_launch(cx);
        self.monos.overview_open = true;
        self.monos.overview.select(tab);
        if let Some(id) = self.monos.view.clone() {
            match tab {
                OverviewTab::Habits => self.load_mono_overview(&id, cx),
                OverviewTab::Documents => {}
                OverviewTab::Changes => self.load_mono_changes(cx),
                OverviewTab::Board => {
                    self.load_mono_overview(&id, cx);
                    self.load_mono_project_sessions(cx);
                }
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

    /// MonoCode `showMonoLaunchedSession`: a session the Mono started, open
    /// over the Overview beside its chat, where it can be followed, answered
    /// or stopped; the one already shown hides again. One this launch has
    /// not read yet comes from the database first.
    pub fn show_mono_launch(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.monos.overview_open && self.monos.overview.shown_session.as_deref() == Some(id) {
            self.hide_mono_launch(cx);
            return;
        }
        let Some(mono) = self.monos.view.clone() else {
            return;
        };
        self.with_mono_launch(id, cx, move |this, id, cx| {
            // A Mono left or switched meanwhile keeps what it shows.
            if this.monos.view.as_deref() != Some(mono.as_str()) {
                return;
            }
            this.monos.overview_open = true;
            this.monos.overview.select(OverviewTab::Board);
            this.monos.overview.document = None;
            this.monos.overview.shown_session = Some(id.to_string());
            this.monos.overview.shown_focused = true;
            this.sync_selection(cx);
            cx.notify();
        });
    }

    /// Back from the shown session to the sessions the Mono started; the
    /// Mono's chat takes the prompt again.
    pub fn hide_mono_launch(&mut self, cx: &mut Context<Self>) {
        let overview = &mut self.monos.overview;
        if overview.shown_session.take().is_none() {
            return;
        }
        let focused = std::mem::take(&mut overview.shown_focused);
        if focused {
            self.sync_selection(cx);
        }
        cx.notify();
    }

    /// The shown session's "Open in workspace": out of the Mono, into its
    /// tab.
    pub fn open_mono_launch(&mut self, id: &str, cx: &mut Context<Self>) {
        self.with_mono_launch(id, cx, |this, id, cx| {
            this.monos.overview.shown_session = None;
            this.monos.overview.shown_focused = false;
            this.open_launched_session(id, cx);
        });
    }

    /// Before a file or review of the shown session opens beside the chat:
    /// that session moves to the workspace first, or the tab would open
    /// behind the Mono's view.
    pub(crate) fn surface_shown_session(&mut self, session: Option<&str>, cx: &mut Context<Self>) {
        let Some(shown) = self.mono_shown_session().map(str::to_string) else {
            return;
        };
        let ours = match session {
            Some(id) => id == shown,
            None => self.monos.overview.shown_focused,
        };
        if ours && self.mono_covers() {
            self.monos.overview.shown_session = None;
            self.monos.overview.shown_focused = false;
            self.open_launched_session(&shown, cx);
        }
    }

    /// The session the Overview shows over everything, while it is open
    /// and the session is in memory.
    pub fn mono_shown_session(&self) -> Option<&str> {
        self.monos.view.as_ref()?;
        if !self.monos.overview_open {
            return None;
        }
        let id = self.monos.overview.shown_session.as_deref()?;
        self.sessions.iter().any(|s| s.id == id).then_some(id)
    }

    /// The thread the Mono's view gives the prompt to: the shown session
    /// while it holds focus, else the Mono's chat.
    pub fn mono_focus_session(&self) -> Option<&str> {
        self.mono_shown_session()
            .filter(|_| self.monos.overview.shown_focused)
            .or_else(|| self.mono_view_session())
    }

    /// MonoCode `onFocusPane` in a Mono's view: a click on its chat or on
    /// the shown session moves the prompt there. False for any other pane.
    pub(crate) fn focus_mono_pane(&mut self, pane_id: &str, cx: &mut Context<Self>) -> bool {
        let shown = self.mono_shown_session() == Some(pane_id);
        if !shown && self.mono_view_session() != Some(pane_id) {
            return false;
        }
        if self.monos.overview.shown_focused != shown {
            self.monos.overview.shown_focused = shown;
            self.sync_selection(cx);
        }
        true
    }

    /// `f` with session `id` in memory, read from the database first when
    /// it is not.
    fn with_mono_launch(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &str, &mut Context<Self>) + 'static,
    ) {
        if self.sessions.iter().any(|s| s.id == id) {
            f(self, id, cx);
            return;
        }
        let row = id.to_string();
        self.db_then(
            cx,
            move |db| db.get_session(&row),
            move |this, loaded, cx| match loaded {
                Ok(Some(session)) => {
                    let id = session.id.clone();
                    if !this.sessions.iter().any(|s| s.id == id) {
                        this.sessions.push(session);
                    }
                    f(this, &id, cx);
                }
                Ok(None) => log::info!("a Mono's session was deleted since"),
                Err(err) => log::error!("could not read a Mono's session: {err:#}"),
            },
        );
    }

    pub fn toggle_mono_activity(&mut self, cx: &mut Context<Self>) {
        self.hide_mono_launch(cx);
        self.monos.overview.toggle_activity();
        self.monos.overview.document = None;
        cx.notify();
    }

    /// MonoCode `onShowMonoActivity`: a turn's work on the Activity tab;
    /// picking the shown one again clears it.
    pub fn show_mono_activity(&mut self, session_id: &str, turn_id: &str, cx: &mut Context<Self>) {
        self.hide_mono_launch(cx);
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

impl BenCodeApp {
    /// MonoCode `push({ kind: "habit", id })`.
    pub fn open_mono_habit(&mut self, id: &str, cx: &mut Context<Self>) {
        self.monos.overview.habit_route = Some(HabitRoute::Habit(id.to_string()));
        cx.notify();
    }

    /// MonoCode `push({ kind: "new-habit" })`: the page with its fields,
    /// the name focused.
    pub fn open_new_habit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| TextInput::new(window, cx).max_len(80).placeholder("Name"));
        let instructions = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line(3, usize::MAX)
                .max_len(4_000)
                .placeholder(
                    "What it should do on each run, and when it's worth telling you about.",
                )
        });
        cx.subscribe(&name, |this: &mut BenCodeApp, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Submit) {
                this.create_mono_habit(cx);
            }
        })
        .detach();
        cx.observe(&name, |_, _, cx| cx.notify()).detach();
        cx.observe(&instructions, |_, _, cx| cx.notify()).detach();
        let focus = name.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.monos.overview.habit_form = Some(HabitForm {
            name,
            instructions,
            schedule: HabitSchedule {
                schedule_kind: "daily".into(),
                minute: 0,
                time: "09:00".into(),
                day_of_week: 1,
            },
            saving: false,
            error: None,
        });
        self.monos.overview.habit_route = Some(HabitRoute::New);
        cx.notify();
    }

    /// Back from a habit's page or New habit to the list.
    pub fn close_mono_habit_page(&mut self, cx: &mut Context<Self>) {
        self.monos.overview.habit_route = None;
        self.monos.overview.habit_form = None;
        cx.notify();
    }

    /// New habit's schedule, one field at a time.
    pub fn set_new_habit_schedule(
        &mut self,
        change: impl FnOnce(&mut HabitSchedule),
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.monos.overview.habit_form.as_mut() {
            change(&mut form.schedule);
            cx.notify();
        }
    }

    /// Whether New habit can be created: a name and instructions, not saving.
    pub fn new_habit_ready(&self, cx: &gpui::App) -> bool {
        self.monos.overview.habit_form.as_ref().is_some_and(|form| {
            !form.saving
                && !form.name.read(cx).text().trim().is_empty()
                && !form.instructions.read(cx).text().trim().is_empty()
        })
    }

    /// MonoCode `addHabit` from New habit, then back to the list.
    pub fn create_mono_habit(&mut self, cx: &mut Context<Self>) {
        if !self.new_habit_ready(cx) {
            return;
        }
        let (Some(mono), Some(form)) = (
            self.monos.view.clone(),
            self.monos.overview.habit_form.as_mut(),
        ) else {
            return;
        };
        form.saving = true;
        form.error = None;
        let name: String = form.name.read(cx).text().trim().chars().take(80).collect();
        let instructions: String = form
            .instructions
            .read(cx)
            .text()
            .trim()
            .chars()
            .take(4_000)
            .collect();
        let schedule = form.schedule.clone();
        let Some(dir) = mono_files_dir(&mono) else {
            return;
        };
        let task = cx.background_executor().spawn(async move {
            crate::monos::habits::update_habits(&dir, |mut list| {
                if list.len() >= crate::monos::habits::HABITS_MAX {
                    return Err(format!(
                        "A Mono can have at most {} habits",
                        crate::monos::habits::HABITS_MAX
                    ));
                }
                let habit = crate::monos::habits::new_habit(
                    &name,
                    &instructions,
                    schedule.clone(),
                    crate::app::now_ms(),
                );
                list.push(habit);
                Ok((list, ()))
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.monos.overview.habit_route = None;
                        this.monos.overview.habit_form = None;
                        this.load_mono_overview(&mono, cx);
                    }
                    Err(err) => {
                        if let Some(form) = this.monos.overview.habit_form.as_mut() {
                            form.saving = false;
                            form.error = Some(err);
                        }
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("habit created after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// MonoCode `habitActions.runNow`.
    pub fn run_mono_habit_now(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(mono) = self.monos.view.clone() else {
            return;
        };
        let id = id.to_string();
        self.edit_mono_habits(
            &mono,
            move |list, _| {
                Ok(list
                    .into_iter()
                    .map(|mut habit| {
                        if habit.id == id {
                            habit.run_requested = true;
                        }
                        habit
                    })
                    .collect())
            },
            true,
            cx,
        );
    }

    /// MonoCode `habitActions.toggle`: pause, or resume counting from now.
    pub fn toggle_mono_habit(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(mono) = self.monos.view.clone() else {
            return;
        };
        let id = id.to_string();
        self.edit_mono_habits(
            &mono,
            move |list, now| {
                Ok(list
                    .into_iter()
                    .map(|mut habit| {
                        if habit.id == id {
                            habit.enabled = !habit.enabled;
                            if habit.enabled {
                                habit.next_run_at = habit.schedule.next_run_at(now);
                            }
                        }
                        habit
                    })
                    .collect())
            },
            false,
            cx,
        );
    }

    /// MonoCode `habitActions.remove`; its page, if open, closes.
    pub fn remove_mono_habit(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(mono) = self.monos.view.clone() else {
            return;
        };
        if self.monos.overview.habit_route == Some(HabitRoute::Habit(id.to_string())) {
            self.monos.overview.habit_route = None;
        }
        let id = id.to_string();
        self.edit_mono_habits(
            &mono,
            move |list, _| Ok(list.into_iter().filter(|habit| habit.id != id).collect()),
            false,
            cx,
        );
    }

    /// A run's report on a habit's page, opened or closed.
    pub fn toggle_habit_run(&mut self, habit: &str, at: i64, cx: &mut Context<Self>) {
        let key = (habit.to_string(), at);
        let open = &mut self.monos.overview.open_runs;
        if !open.remove(&key) {
            open.insert(key);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
