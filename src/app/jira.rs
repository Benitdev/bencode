//! The Jira connection as the app holds it (Settings › Integrations and
//! the Inbox): connect and disconnect, which projects the Inbox lists, the
//! folder each Jira project's threads start in, and workflow status
//! transitions. The API itself is `jira.rs`.

use std::collections::{BTreeMap, HashMap, HashSet};

use gpui::Context;

use crate::app::BenCodeApp;
use crate::jira::{self, Account, Project, Transition};
use crate::work_items::Provider;

#[derive(Default)]
pub struct JiraState {
    /// The saved connection, once `loaded`.
    pub account: Option<Account>,
    pub loaded: bool,
    /// Connecting or disconnecting.
    pub busy: bool,
    pub error: Option<String>,
    /// The site's projects, for the list of what the Inbox shows.
    pub projects: Option<Result<Vec<Project>, String>>,
    pub projects_loading: bool,
    /// Saved: ids of the projects left out of the Inbox.
    pub hidden_projects: Vec<String>,
    /// Saved: the folder "Send to agent" starts in, by Jira project key.
    pub project_folders: BTreeMap<String, String>,
    /// Available workflow transitions per issue key (`jira:eng-42`).
    pub transitions: HashMap<String, Vec<Transition>>,
    transitions_loading: HashSet<String>,
    /// The issue (by Inbox key) whose status is being transitioned.
    pub issue_busy: Option<String>,
    pub issue_error: Option<String>,
    /// A fresh connection: its first list counts as read.
    pub seed_seen: bool,
    /// Bumped by connect and disconnect: older answers are dropped.
    generation: u64,
}

impl BenCodeApp {
    /// How a turn's agent reaches Jira: only while it is connected, so
    /// the tools cost no tokens otherwise.
    pub(crate) fn jira_mcp_launch(&self) -> Option<bencode_core::mcp::McpLaunch> {
        self.jira.account.as_ref()?;
        jira::mcp::launch()
            .map_err(|err| log::warn!("Jira tools are off for this turn: {err:#}"))
            .ok()
    }

    /// Reads the saved connection off the UI thread (once at launch).
    pub fn load_jira_account(&mut self, cx: &mut Context<Self>) {
        let generation = self.jira.generation;
        let task = cx
            .background_executor()
            .spawn(async move { jira::account() });
        cx.spawn(async move |this, cx| {
            let account = task.await;
            let landed = this.update(cx, |app, cx| {
                if app.jira.generation == generation {
                    let me = account.as_ref().map(|a| a.display_name.clone());
                    app.jira.account = account;
                    app.jira.loaded = true;
                    app.inbox_mut(cx, |inbox| inbox.attention.set_jira_me(me));
                    app.backfill_jira_links(cx);
                    cx.notify();
                }
            });
            if let Err(err) = landed {
                log::debug!("jira account after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The connection form: Connect follows what is typed, Enter submits.
    pub(crate) fn on_jira_form_input(
        &mut self,
        _: gpui::Entity<bencode_ui::forms::TextInput>,
        event: &bencode_ui::forms::InputEvent,
        cx: &mut Context<Self>,
    ) {
        use bencode_ui::forms::InputEvent;
        match event {
            InputEvent::Submit => self.connect_jira(cx),
            InputEvent::Changed => cx.notify(),
            _ => {}
        }
    }

    /// Checks the typed site, email and API token, saves them, and fills the Inbox.
    pub fn connect_jira(&mut self, cx: &mut Context<Self>) {
        if self.jira.busy {
            return;
        }
        let site = self.jira_site_input.read(cx).text().trim().to_string();
        let email = self.jira_email_input.read(cx).text().trim().to_string();
        let token = self.jira_token_input.read(cx).text().trim().to_string();
        self.jira.busy = true;
        self.jira.error = None;
        self.jira.generation += 1;
        let generation = self.jira.generation;
        let task = cx
            .background_executor()
            .spawn(async move { jira::connect(&site, &email, &token) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                if app.jira.generation != generation {
                    return;
                }
                app.jira.busy = false;
                app.jira.loaded = true;
                match result {
                    Ok(account) => {
                        let me = account.display_name.clone();
                        app.jira.account = Some(account);
                        app.jira.seed_seen = true;
                        app.inbox_mut(cx, |inbox| inbox.attention.set_jira_me(Some(me)));
                        // The token is saved; it need not stay in the field.
                        app.jira_token_input
                            .update(cx, |input, cx| input.set_text("", cx));
                        app.load_jira_projects(false, cx);
                        app.refresh_inbox(cx);
                        app.backfill_jira_links(cx);
                    }
                    Err(err) => {
                        log::warn!("could not connect Jira: {err}");
                        app.jira.error = Some(err);
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("jira connected after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// Forgets the token; Jira's issues leave the Inbox.
    pub fn disconnect_jira(&mut self, cx: &mut Context<Self>) {
        if self.jira.busy {
            return;
        }
        self.jira.busy = true;
        self.jira.error = None;
        self.jira.generation += 1;
        let generation = self.jira.generation;
        let task = cx
            .background_executor()
            .spawn(async move { jira::disconnect() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                if app.jira.generation != generation {
                    return;
                }
                app.jira.busy = false;
                match result {
                    Ok(()) => {
                        app.jira.account = None;
                        app.jira.projects = None;
                        app.jira.transitions.clear();
                        app.jira.transitions_loading.clear();
                        app.inbox_mut(cx, |inbox| inbox.attention.set_jira_me(None));
                        app.refresh_inbox(cx);
                    }
                    Err(err) => {
                        log::warn!("could not disconnect Jira: {err}");
                        app.jira.error = Some(err);
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("jira disconnected after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn load_jira_projects(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.jira.projects_loading {
            return;
        }
        self.jira.projects_loading = true;
        let generation = self.jira.generation;
        let task = cx.background_executor().spawn(async move {
            if force {
                jira::refresh_projects()
            } else {
                jira::projects()
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                app.jira.projects_loading = false;
                if app.jira.generation == generation {
                    if let Err(err) = &result {
                        log::warn!("could not list Jira projects: {err}");
                    }
                    app.jira.projects = Some(result);
                } else if app.jira.account.is_some() && app.jira.projects.is_none() {
                    // A reconnect's own load was turned away while this
                    // one ran: start it now.
                    app.load_jira_projects(false, cx);
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("jira projects after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// Shows or hides a Jira project's issues in the Inbox.
    pub fn set_jira_project_shown(&mut self, id: &str, shown: bool, cx: &mut Context<Self>) {
        let hidden = &mut self.jira.hidden_projects;
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

    /// Remembers the folder a Jira project's threads start in.
    pub(crate) fn set_jira_project_folder(
        &mut self,
        project_key: &str,
        folder: &str,
        cx: &mut Context<Self>,
    ) {
        let folders = &mut self.jira.project_folders;
        if folders.get(project_key).map(String::as_str) != Some(folder) {
            folders.insert(project_key.to_string(), folder.to_string());
            self.save_settings(cx);
        }
    }

    /// Loads available workflow transitions for Jira issue `key` (`jira:eng-42`).
    pub fn load_jira_transitions(
        &mut self,
        key: &str,
        identifier: &str,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        if !force && self.jira.transitions.contains_key(key) {
            return;
        }
        if !self.jira.transitions_loading.insert(key.to_string()) {
            return;
        }
        let generation = self.jira.generation;
        let issue = identifier.to_string();
        let key = key.to_string();
        let task = cx
            .background_executor()
            .spawn(async move { jira::transitions(&issue) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                app.jira.transitions_loading.remove(&key);
                if app.jira.generation != generation {
                    return;
                }
                match result {
                    Ok(list) => {
                        app.jira.transitions.insert(key, list);
                        cx.notify();
                    }
                    Err(err) => {
                        log::warn!("jira: no transitions for {key}: {err}");
                    }
                }
            });
            if let Err(err) = landed {
                log::debug!("jira transitions after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Transitions Inbox Jira issue `key` to workflow transition `transition_id`.
    pub fn transition_jira_issue(
        &mut self,
        key: &str,
        transition_id: String,
        cx: &mut Context<Self>,
    ) {
        if self.jira.issue_busy.is_some() {
            return;
        }
        let Some(item) = self
            .inbox(cx)
            .item(key)
            .filter(|item| item.provider == Provider::Jira)
            .cloned()
        else {
            return;
        };
        self.jira.issue_busy = Some(key.to_string());
        self.jira.issue_error = None;
        let issue = item.identifier.clone();
        let task = cx
            .background_executor()
            .spawn(async move { jira::transition(&issue, &transition_id) });
        let key = key.to_string();
        let identifier = item.identifier.clone();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                app.jira.issue_busy = None;
                match result {
                    Ok(next) => {
                        app.inbox_state.update(cx, |inbox, _| {
                            if let Some(slot) = inbox.item_mut(&key) {
                                *slot = next;
                            }
                        });
                        // The user's own change is not news to them.
                        app.mark_inbox_key_seen(&key, cx);
                        app.load_jira_transitions(&key, &identifier, true, cx);
                    }
                    Err(err) => {
                        log::warn!("could not transition the Jira issue: {err}");
                        app.jira.issue_error = Some(err);
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("jira transition after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }
}
