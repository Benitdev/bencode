//! MonoCode `HarnessUpdateNotice`'s logic: once per launch, each installed
//! CLI with a version feed is compared with its latest release; the ones behind
//! are offered, and Update runs the CLI's own updater. Nothing is remembered
//! between launches: a harness still behind is offered again next time.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::Context;

use crate::app::BenCodeApp;
use crate::harness::updates::{self, HarnessUpdate};
use crate::harness::{ALL_HARNESSES, HarnessKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowState {
    Idle,
    Updating,
    Updated(String),
    Failed(String),
}

#[derive(Default)]
pub struct HarnessUpdates {
    /// The harnesses behind, in picker order; empty hides the notice.
    pub updates: Vec<HarnessUpdate>,
    rows: HashMap<HarnessKind, RowState>,
}

impl HarnessUpdates {
    pub fn state(&self, kind: HarnessKind) -> &RowState {
        self.rows.get(&kind).unwrap_or(&RowState::Idle)
    }

    pub fn busy(&self) -> bool {
        self.updates
            .iter()
            .any(|u| *self.state(u.harness) == RowState::Updating)
    }

    /// Rows that can still be started: never tried, or failed.
    pub fn pending(&self) -> Vec<HarnessKind> {
        self.updates
            .iter()
            .map(|u| u.harness)
            .filter(|kind| matches!(self.state(*kind), RowState::Idle | RowState::Failed(_)))
            .collect()
    }

    pub fn any_updated(&self) -> bool {
        self.updates
            .iter()
            .any(|u| matches!(self.state(u.harness), RowState::Updated(_)))
    }
}

impl BenCodeApp {
    /// Where startup found `kind`'s CLI. `updates` works out from it the
    /// copy turns run, off the UI thread.
    fn harness_program(&self, kind: HarnessKind) -> Option<PathBuf> {
        self.harnesses
            .iter()
            .find(|h| h.available && h.id == kind.id())
            .and_then(|h| h.binary_path.clone())
    }

    /// The launch check: every installed, updatable harness at once.
    pub fn start_harness_update_check(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<(HarnessKind, PathBuf)> = ALL_HARNESSES
            .into_iter()
            .filter(|kind| updates::is_updatable(*kind))
            .filter_map(|kind| Some((kind, self.harness_program(kind)?)))
            .collect();
        if targets.is_empty() {
            return;
        }
        // All at once; awaited in picker order.
        let checks: Vec<_> = targets
            .into_iter()
            .map(|(kind, program)| {
                cx.background_executor()
                    .spawn(async move { updates::find_update(kind, &program) })
            })
            .collect();
        cx.spawn(async move |this, cx| {
            let mut found = Vec::new();
            for check in checks {
                found.extend(check.await);
            }
            if found.is_empty() {
                return;
            }
            let landed = this.update(cx, |this, cx| {
                for update in &found {
                    log::info!(
                        "{} {} → {}",
                        update.harness.label(),
                        update.installed,
                        update.latest
                    );
                }
                this.harness_updates.updates = found;
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("harness updates after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Runs each harness's updater. Some updaters exit cleanly without
    /// installing anything, so success is the version the CLI reports
    /// afterwards, not the exit code; its models are reloaded with it.
    pub fn update_harnesses(&mut self, kinds: Vec<HarnessKind>, cx: &mut Context<Self>) {
        for kind in kinds {
            let Some(update) = self
                .harness_updates
                .updates
                .iter()
                .find(|u| u.harness == kind)
                .cloned()
            else {
                continue;
            };
            if *self.harness_updates.state(kind) == RowState::Updating {
                continue;
            }
            let Some(program) = self.harness_program(kind) else {
                continue;
            };
            self.harness_updates.rows.insert(kind, RowState::Updating);
            let task = cx
                .background_executor()
                .spawn(async move { updates::run_update(kind, &program) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let landed = this.update(cx, |this, cx| {
                    let state = match result {
                        Ok(version)
                            if updates::compare_semver(&version, &update.latest).is_ge() =>
                        {
                            log::info!("{} updated to {version}", kind.label());
                            this.force_refresh_model_catalog(kind, cx);
                            RowState::Updated(version)
                        }
                        Ok(version) => {
                            RowState::Failed(format!("Still on {version} after updating."))
                        }
                        Err(err) => {
                            log::warn!("{} update: {err:#}", kind.label());
                            RowState::Failed(format!("{err:#}"))
                        }
                    };
                    this.harness_updates.rows.insert(kind, state);
                    cx.notify();
                });
                if let Err(err) = landed {
                    log::debug!("harness update after app drop: {err:#}");
                }
            })
            .detach();
        }
        cx.notify();
    }

    /// Only for this run: the next launch checks and offers again.
    pub fn dismiss_harness_updates(&mut self, cx: &mut Context<Self>) {
        if self.harness_updates.busy() {
            return;
        }
        self.harness_updates = HarnessUpdates::default();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(harness: HarnessKind) -> HarnessUpdate {
        HarnessUpdate {
            harness,
            installed: "1.0.0".into(),
            latest: "1.0.1".into(),
        }
    }

    #[test]
    fn pending_rows_are_the_untried_and_the_failed() {
        let mut state = HarnessUpdates {
            updates: vec![
                update(HarnessKind::Claude),
                update(HarnessKind::Codex),
                update(HarnessKind::OpenCode),
            ],
            rows: HashMap::new(),
        };
        state.rows.insert(HarnessKind::Codex, RowState::Updating);
        state
            .rows
            .insert(HarnessKind::OpenCode, RowState::Failed("no".into()));
        assert_eq!(
            state.pending(),
            [HarnessKind::Claude, HarnessKind::OpenCode]
        );
        assert!(state.busy());
        assert!(!state.any_updated());
        state
            .rows
            .insert(HarnessKind::Codex, RowState::Updated("1.0.1".into()));
        assert!(!state.busy());
        assert!(state.any_updated());
    }
}
