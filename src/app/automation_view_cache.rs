//! What the Automations overview and the cards show that costs a schedule
//! walk or a pass over the week's runs (Upcoming, each card's run strip,
//! Recent results), worked out once per list, runs, pause and minute
//! instead of on every frame the Surface is drawn. The pure functions
//! are `automation_overview.rs`'s; this only keeps their answers.

use std::cell::RefCell;
use std::collections::HashMap;

use jiff::tz::TimeZone;

use super::BenCodeApp;
use super::automation_overview::{
    ResultGroup, Upcoming, group_results, recent_results, run_strips, upcoming,
};
use crate::db::{AutomationRunRow, RunStatus};

const MINUTE_MS: i64 = 60_000;

/// What the answers were worked out from: the list and runs as loaded
/// (`generation`), Pause all, and the minute ("Today", the week's edge
/// and what is upcoming move with the clock).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    generation: u64,
    paused_until: Option<i64>,
    minute: i64,
}

#[derive(Default)]
struct Answers {
    key: Option<Key>,
    upcoming: Vec<Upcoming>,
    strips: HashMap<String, Vec<RunStatus>>,
    /// Recent results by day, each group the indices of its runs in
    /// `AutomationsState::recent`, newest first.
    results: Vec<(String, Vec<Vec<usize>>)>,
}

/// Kept on `AutomationsState`; `invalidate` whenever its list or runs
/// change.
#[derive(Default)]
pub struct AutomationViewCache {
    generation: u64,
    answers: RefCell<Answers>,
}

impl AutomationViewCache {
    pub fn invalidate(&mut self) {
        self.generation += 1;
    }
}

impl BenCodeApp {
    /// Brings the answers up to date for `now`. Skipped while a caller
    /// still reads them (it reads the ones of a moment ago).
    fn refresh_view_cache(&self, now: i64, tz: &TimeZone) {
        let state = &self.automations;
        let key = Key {
            generation: state.view_cache.generation,
            paused_until: self.automations_paused_until(),
            minute: now / MINUTE_MS,
        };
        let Ok(mut answers) = state.view_cache.answers.try_borrow_mut() else {
            log::debug!("automation view cache read while it refreshed");
            return;
        };
        if answers.key == Some(key) {
            return;
        }
        // From the start of the minute, so every frame in it agrees.
        let at = key.minute * MINUTE_MS;
        let index: HashMap<&str, usize> = state
            .recent
            .iter()
            .enumerate()
            .map(|(ix, run)| (run.id.as_str(), ix))
            .collect();
        let shown: Vec<&AutomationRunRow> = recent_results(&state.recent, at)
            .into_iter()
            .filter(|run| state.items.iter().any(|a| a.id == run.automation_id))
            .collect();
        let results = group_results(&shown, at, tz)
            .into_iter()
            .map(|(day, groups)| {
                let groups = groups
                    .iter()
                    .map(|group| {
                        group
                            .runs
                            .iter()
                            .map(|run| index[run.id.as_str()])
                            .collect()
                    })
                    .collect();
                (day, groups)
            })
            .collect();
        *answers = Answers {
            key: Some(key),
            upcoming: upcoming(&state.items, at, key.paused_until, tz),
            strips: run_strips(&state.recent),
            results,
        };
    }

    /// Upcoming, still ahead of `now`.
    pub(crate) fn upcoming_automation_runs(&self, now: i64, tz: &TimeZone) -> Vec<Upcoming> {
        self.refresh_view_cache(now, tz);
        let answers = self.automations.view_cache.answers.borrow();
        answers
            .upcoming
            .iter()
            .filter(|row| row.at > now)
            .cloned()
            .collect()
    }

    /// The card's run strip, oldest first.
    pub(crate) fn automation_run_strip(&self, id: &str, now: i64, tz: &TimeZone) -> Vec<RunStatus> {
        self.refresh_view_cache(now, tz);
        let answers = self.automations.view_cache.answers.borrow();
        answers.strips.get(id).cloned().unwrap_or_default()
    }

    /// Recent results: day labels and their groups, newest first.
    pub(crate) fn automation_result_days(
        &self,
        now: i64,
        tz: &TimeZone,
    ) -> Vec<(String, Vec<ResultGroup<'_>>)> {
        self.refresh_view_cache(now, tz);
        let answers = self.automations.view_cache.answers.borrow();
        let recent = &self.automations.recent;
        answers
            .results
            .iter()
            .map(|(day, groups)| {
                let groups = groups
                    .iter()
                    .map(|runs| ResultGroup {
                        runs: runs.iter().map(|ix| &recent[*ix]).collect(),
                    })
                    .collect();
                (day.clone(), groups)
            })
            .collect()
    }
}
