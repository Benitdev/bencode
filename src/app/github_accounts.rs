//! GitHub accounts as the app holds them (Settings › Integrations): the
//! accounts `gh` is signed in with, and the one picked for each project.
//! What runs `gh` as them is `github_accounts.rs`.

use std::collections::BTreeMap;

use gpui::Context;

use crate::app::BenCodeApp;
use crate::github_accounts::{self, Account};

#[derive(Default)]
pub struct GithubAccountsState {
    /// The accounts `gh` is signed in with, once listed.
    pub accounts: Option<Result<Vec<Account>, String>>,
    /// Saved: the account picked for a project, by project folder. A
    /// project left out is on Automatic.
    pub choices: BTreeMap<String, String>,
    /// Bumped by each listing: an older answer is dropped.
    generation: u64,
}

impl BenCodeApp {
    /// Lists `gh`'s accounts off the UI thread (Settings › Integrations
    /// opened): one signed in since then shows.
    pub fn load_github_accounts(&mut self, cx: &mut Context<Self>) {
        self.github.generation += 1;
        let generation = self.github.generation;
        let task = cx.background_executor().spawn(async move { github_accounts::list() });
        cx.spawn(async move |this, cx| {
            let accounts = task.await;
            let landed = this.update(cx, |app, cx| {
                if app.github.generation == generation {
                    app.github.accounts = Some(accounts);
                    cx.notify();
                }
            });
            if let Err(err) = landed {
                log::debug!("github accounts after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Picks the account `project`'s GitHub commands run as (`None` is
    /// Automatic) and reads the Inbox again as it.
    pub fn set_project_github_account(&mut self, project: &str, login: Option<String>, cx: &mut Context<Self>) {
        let before = self.github.choices.get(project).cloned();
        if before == login {
            return;
        }
        match login {
            Some(login) => self.github.choices.insert(project.to_string(), login),
            None => self.github.choices.remove(project),
        };
        github_accounts::set_choices(self.github.choices.clone());
        self.save_settings(cx);
        self.refresh_inbox(cx);
        cx.notify();
    }
}
