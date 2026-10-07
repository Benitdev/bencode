//! Taking a project off the rail: MonoCode `onRemoveProject` (Archive,
//! or Delete with its `RemoveProjectDialog`) and `onRestoreProject`.

use ely_gpui_component::overlays::ConfirmDialog;
use gpui::{AnyElement, Context, IntoElement};

use super::state::RemoveProject;
use crate::app::BenCodeApp;
use crate::ui::app_callback::app_callback;

/// Every session of a project, for Delete (far above any real count).
const ALL_SESSIONS: usize = 1_000_000;

impl BenCodeApp {
    /// MonoCode `onRemoveProject(path, { purgeData: false })`: off the rail
    /// and into Settings › Archive; an open project gives way to the next.
    pub(super) fn archive_rail_project(&mut self, path: &str, cx: &mut Context<Self>) {
        let now = crate::app::now_ms();
        self.update_rail_prefs(|prefs| prefs.with_archived(path, now), cx);
        self.leave_removed_project(path, cx);
    }

    /// MonoCode `onRestoreProject`: back on the rail, and opened.
    pub(crate) fn restore_rail_project(&mut self, path: &str, cx: &mut Context<Self>) {
        self.update_rail_prefs(|prefs| prefs.with_restored(path), cx);
        self.close_surface(cx);
        self.switch_project(path.to_string(), cx);
    }

    /// The next project on the rail takes over from a removed open one.
    pub(super) fn leave_removed_project(&mut self, path: &str, cx: &mut Context<Self>) {
        if !crate::app::same_project_path(path, &self.current_cwd) {
            return;
        }
        let next = self
            .recent_projects
            .iter()
            .find(|p| !crate::app::same_project_path(p, path) && !self.settings.rail.is_archived(p))
            .cloned();
        if let Some(next) = next {
            self.switch_project(next, cx);
        }
    }

    /// The ids of every saved conversation of `path`, read on a background
    /// connection (the in-memory DB of tests is read in place).
    fn read_project_session_ids(&self, path: &str, cx: &Context<Self>) -> gpui::Task<anyhow::Result<Vec<String>>> {
        let ids = |db: &crate::db::AppDb, path: &str| -> anyhow::Result<Vec<String>> {
            Ok(db
                .list_sessions_for_cwd(path, ALL_SESSIONS, &[])?
                .into_iter()
                .map(|row| row.id)
                .collect())
        };
        let Some(file) = self.db.file_path() else {
            return gpui::Task::ready(ids(&self.db, path));
        };
        let path = path.to_string();
        cx.background_executor().spawn(async move {
            let db = crate::db::AppDb::open_reader(&file)?;
            ids(&db, &path)
        })
    }

    /// Opens MonoCode `RemoveProjectDialog`; the count of what it will
    /// delete arrives from a background read.
    pub(crate) fn request_remove_project(&mut self, path: &str, cx: &mut Context<Self>) {
        self.rail_ui.removing = Some(RemoveProject {
            path: path.to_string(),
            name: self.rail_project_label(path),
            sessions: None,
        });
        cx.notify();
        let task = self.read_project_session_ids(path, cx);
        let path = path.to_string();
        cx.spawn(async move |this, cx| {
            let count = match task.await {
                Ok(ids) => ids.len(),
                Err(err) => {
                    log::error!("could not count sessions of {path}: {err:#}");
                    return;
                }
            };
            let shown = this.update(cx, |this, cx| {
                if let Some(removing) = this.rail_ui.removing.as_mut()
                    && removing.path == path
                {
                    removing.sessions = Some(count);
                    cx.notify();
                }
            });
            if let Err(err) = shown {
                log::debug!("session count after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `onRemoveProject(path, { purgeData: true })`: every saved
    /// conversation of the project goes, and so does all the rail kept
    /// about it. The folder on disk is left alone.
    pub(super) fn remove_rail_project(&mut self, path: &str, cx: &mut Context<Self>) {
        let task = self.read_project_session_ids(path, cx);
        let path = path.to_string();
        cx.spawn(async move |this, cx| {
            let ids = match task.await {
                Ok(ids) => ids,
                Err(err) => {
                    log::error!("could not list sessions of {path}: {err:#}");
                    return;
                }
            };
            let removed = this.update(cx, |this, cx| this.forget_rail_project(&path, &ids, cx));
            if let Err(err) = removed {
                log::debug!("project removal after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn forget_rail_project(&mut self, path: &str, ids: &[String], cx: &mut Context<Self>) {
        // The next project takes over first, and the project's panes close
        // outright: `delete_session` would otherwise keep its last tab open
        // on a fresh thread, saved in the very folder being removed.
        self.leave_removed_project(path, cx);
        let left = !crate::app::same_project_path(path, &self.current_cwd);
        for id in ids {
            if left && !self.is_agent_running_in(id) {
                self.tabs.remove_session(id);
            }
            self.delete_session(id, cx);
        }
        // `recent_projects` is only rebuilt from the sessions at startup.
        let emptied = !self
            .sessions
            .iter()
            .any(|s| crate::app::same_project_path(&s.cwd, path));
        if left && emptied {
            self.recent_projects
                .retain(|p| !crate::app::same_project_path(p, path));
        }
        let pins: Vec<String> = self
            .settings
            .pinned_projects
            .iter()
            .filter(|p| !crate::app::same_project_path(p, path))
            .cloned()
            .collect();
        self.set_pinned_projects(pins, cx);
        self.update_rail_prefs(|prefs| prefs.without_project(path), cx);
        cx.notify();
    }

    /// MonoCode `RemoveProjectDialog`, as a `ConfirmDialog`.
    pub(super) fn render_remove_project_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let removing = self.rail_ui.removing.clone()?;
        let close = app_callback(cx, |this, cx| {
            this.rail_ui.removing = None;
            cx.notify();
        });
        let path = removing.path.clone();
        let confirm = app_callback(cx, move |this, cx| {
            this.rail_ui.removing = None;
            this.remove_rail_project(&path, cx);
        });
        let count = match removing.sessions {
            None | Some(0) => String::new(),
            Some(1) => "\n\n1 saved conversation will be removed.".into(),
            Some(n) => format!("\n\n{n} saved conversations will be removed."),
        };
        Some(
            ConfirmDialog::new(
                "remove-project",
                format!("Delete “{}”?", removing.name),
                // Broken by hand: Ely's dialog does not wrap its message.
                format!(
                    "All conversations for this project will be deleted.\n\
                     It also leaves the sidebar. The folder on disk stays put,\n\
                     and opening it again brings the project back empty.{count}\n\n{}",
                    removing.path
                ),
                close,
            )
            .confirm("Delete")
            .destructive()
            .on_confirm(confirm)
            .into_any_element(),
        )
    }
}
