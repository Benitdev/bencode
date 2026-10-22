//! The Backlog connection as the app holds it (Settings › Integrations and
//! the Inbox): connect and disconnect, which projects the Inbox lists, the
//! folder each Backlog project's threads start in, and changes to an
//! issue's status and fields. The API itself is `backlog.rs`.

use std::collections::{BTreeMap, HashMap, HashSet};

use gpui::Context;

use crate::app::BenCodeApp;
use crate::backlog::{self, Account, Change, FieldOptions, Project, Statuses};
use crate::work_items::Provider;

#[derive(Default)]
pub struct BacklogState {
    /// The saved connection, once `loaded`.
    pub account: Option<Account>,
    pub loaded: bool,
    /// Connecting or disconnecting.
    pub busy: bool,
    pub error: Option<String>,
    /// The space's projects, for the list of what the Inbox shows.
    pub projects: Option<Result<Vec<Project>, String>>,
    pub projects_loading: bool,
    /// Saved: ids of the projects left out of the Inbox.
    pub hidden_projects: Vec<String>,
    /// Saved: the folder "Send to agent" starts in, by Backlog project key.
    pub project_folders: BTreeMap<String, String>,
    /// Each project's workflow, as the last Inbox fetch found it.
    pub statuses: Statuses,
    /// The issue (by Inbox key) whose status or a field is being changed.
    pub issue_busy: Option<String>,
    pub issue_error: Option<String>,
    /// What each project's issues can be set to, by project id.
    pub field_options: HashMap<String, Result<FieldOptions, String>>,
    field_options_loading: HashSet<String>,
    /// The issue and hours the hours inputs were last given.
    pub hours_shown: Option<(String, Option<f64>, Option<f64>)>,
    /// A fresh connection: its first list counts as read.
    pub seed_seen: bool,
    /// Bumped by connect and disconnect: older answers are dropped.
    generation: u64,
}

impl BenCodeApp {
    /// Reads the saved connection off the UI thread (once at launch).
    pub fn load_backlog_account(&mut self, cx: &mut Context<Self>) {
        let generation = self.backlog.generation;
        let task = cx
            .background_executor()
            .spawn(async move { backlog::account() });
        cx.spawn(async move |this, cx| {
            let account = task.await;
            let landed = this.update(cx, |app, cx| {
                if app.backlog.generation == generation {
                    app.backlog.account = account;
                    app.backlog.loaded = true;
                    cx.notify();
                }
            });
            if let Err(err) = landed {
                log::debug!("backlog account after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The connection form: Connect follows what is typed, Enter submits.
    pub(crate) fn on_backlog_form_input(
        &mut self,
        _: gpui::Entity<ely_gpui_component::forms::TextInput>,
        event: &ely_gpui_component::forms::InputEvent,
        cx: &mut Context<Self>,
    ) {
        use ely_gpui_component::forms::InputEvent;
        match event {
            InputEvent::Submit => self.connect_backlog(cx),
            InputEvent::Changed => cx.notify(),
            _ => {}
        }
    }

    /// Settings › Integrations opened: the projects are listed again.
    pub fn open_integrations_page(&mut self, cx: &mut Context<Self>) {
        self.load_github_accounts(cx);
        self.backlog.error = None;
        if self.backlog.account.is_some() {
            self.load_backlog_projects(cx);
        }
    }

    /// Checks the typed space and key, saves them, and fills the Inbox.
    pub fn connect_backlog(&mut self, cx: &mut Context<Self>) {
        if self.backlog.busy {
            return;
        }
        let space = self.backlog_space_input.read(cx).text().trim().to_string();
        let key = self.backlog_key_input.read(cx).text().trim().to_string();
        self.backlog.busy = true;
        self.backlog.error = None;
        self.backlog.generation += 1;
        let generation = self.backlog.generation;
        let task = cx
            .background_executor()
            .spawn(async move { backlog::connect(&space, &key) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                if app.backlog.generation != generation {
                    return;
                }
                app.backlog.busy = false;
                app.backlog.loaded = true;
                match result {
                    Ok(account) => {
                        app.backlog.account = Some(account);
                        app.backlog.seed_seen = true;
                        // The key is saved; it need not stay in the field.
                        app.backlog_key_input
                            .update(cx, |input, cx| input.set_text("", cx));
                        app.load_backlog_projects(cx);
                        app.refresh_inbox(cx);
                    }
                    Err(err) => {
                        log::warn!("could not connect Backlog: {err}");
                        app.backlog.error = Some(err);
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("backlog connected after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// Forgets the key; Backlog's issues leave the Inbox.
    pub fn disconnect_backlog(&mut self, cx: &mut Context<Self>) {
        if self.backlog.busy {
            return;
        }
        self.backlog.busy = true;
        self.backlog.error = None;
        self.backlog.generation += 1;
        let generation = self.backlog.generation;
        let task = cx
            .background_executor()
            .spawn(async move { backlog::disconnect() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                if app.backlog.generation != generation {
                    return;
                }
                app.backlog.busy = false;
                match result {
                    Ok(()) => {
                        app.backlog.account = None;
                        app.backlog.projects = None;
                        app.backlog.statuses.clear();
                        app.backlog.field_options.clear();
                        app.refresh_inbox(cx);
                    }
                    Err(err) => {
                        log::warn!("could not disconnect Backlog: {err}");
                        app.backlog.error = Some(err);
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("backlog disconnected after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    fn load_backlog_projects(&mut self, cx: &mut Context<Self>) {
        if self.backlog.projects_loading {
            return;
        }
        self.backlog.projects_loading = true;
        let generation = self.backlog.generation;
        let task = cx
            .background_executor()
            .spawn(async move { backlog::projects() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                app.backlog.projects_loading = false;
                if app.backlog.generation == generation {
                    if let Err(err) = &result {
                        log::warn!("could not list Backlog projects: {err}");
                    }
                    app.backlog.projects = Some(result);
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("backlog projects after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// Shows or hides a Backlog project's issues in the Inbox.
    pub fn set_backlog_project_shown(&mut self, id: &str, shown: bool, cx: &mut Context<Self>) {
        let hidden = &mut self.backlog.hidden_projects;
        let was_hidden = hidden.iter().any(|h| h == id);
        if shown == !was_hidden {
            return;
        }
        if shown {
            hidden.retain(|h| h != id);
        } else {
            hidden.push(id.to_string());
        }
        self.save_settings(cx);
        self.refresh_inbox(cx);
    }

    /// Remembers the folder a Backlog project's threads start in.
    pub(crate) fn set_backlog_project_folder(
        &mut self,
        project_key: &str,
        folder: &str,
        cx: &mut Context<Self>,
    ) {
        let folders = &mut self.backlog.project_folders;
        if folders.get(project_key).map(String::as_str) != Some(folder) {
            folders.insert(project_key.to_string(), folder.to_string());
            self.save_settings(cx);
        }
    }

    /// Lists what `project_id`'s issues can be set to, once a session
    /// (the members, milestones, priorities and resolutions).
    pub fn load_backlog_field_options(&mut self, project_id: &str, cx: &mut Context<Self>) {
        let ready = matches!(self.backlog.field_options.get(project_id), Some(Ok(_)));
        if ready
            || !self
                .backlog
                .field_options_loading
                .insert(project_id.to_string())
        {
            return;
        }
        let generation = self.backlog.generation;
        let project = project_id.to_string();
        let task = cx.background_executor().spawn({
            let project = project.clone();
            async move { backlog::options(&project) }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            if let Err(err) = &result {
                log::warn!("backlog: no field options for project {project}: {err}");
            }
            let landed = this.update(cx, |app, cx| {
                app.backlog.field_options_loading.remove(&project);
                if app.backlog.generation == generation {
                    app.backlog.field_options.insert(project, result);
                    cx.notify();
                }
            });
            if let Err(err) = landed {
                log::debug!("backlog field options after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Applies `change` to the Inbox's Backlog issue `key`; the list, its
    /// fields and the conversation (which records the change) follow.
    pub fn update_backlog_issue(&mut self, key: &str, change: Change, cx: &mut Context<Self>) {
        if self.backlog.issue_busy.is_some() {
            return;
        }
        let Some(item) = self
            .inbox(cx)
            .item(key)
            .filter(|item| item.provider == Provider::Backlog)
            .cloned()
        else {
            return;
        };
        self.backlog.issue_busy = Some(key.to_string());
        self.backlog.issue_error = None;
        let issue = item.identifier.clone();
        let task = cx
            .background_executor()
            .spawn(async move { backlog::update(&issue, &change) });
        let key = key.to_string();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                app.backlog.issue_busy = None;
                match result {
                    Ok((next, fields)) => {
                        app.inbox_state.update(cx, |inbox, _| {
                            if let Some(slot) = inbox.items.iter_mut().find(|i| i.key() == key) {
                                *slot = next;
                            }
                            if let Some(Ok(shown)) = inbox.details.value.get_mut(&key) {
                                shown.details.backlog = Some(fields);
                            }
                        });
                        // The user's own change is not news to them.
                        app.mark_inbox_key_seen(&key, cx);
                        app.load_inbox_thread(&key, true, cx);
                    }
                    Err(err) => {
                        log::warn!("could not change the Backlog issue: {err}");
                        app.backlog.issue_error = Some(err);
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("backlog change after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }
}
