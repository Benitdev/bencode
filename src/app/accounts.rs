//! Provider accounts in the app (MonoCode `providerAccounts.ts`,
//! `UsageFooter`'s account callbacks and `App.tsx` `onSelectProviderAccount`):
//! which profile a thread runs under, switching, adding one through the
//! provider's browser sign-in, and who each profile is signed in as.

use std::collections::{HashMap, HashSet};

use gpui::Context;

use crate::app::tab_scope::is_blank_session;
use crate::app::{BenCodeApp, normalize_project_path};
use crate::db::SessionRow;
use crate::harness::HarnessKind;
use crate::harness::account_identity::{self, AccountIdentity};
use crate::harness::accounts::{
    self, AccountProfile, DEFAULT_ACCOUNT_ID, ProviderAccount, supports_accounts,
};
use crate::rate_limits::RateLimitProvider;

/// MonoCode's notice when a thread's account no longer exists.
const REMOVED_ACCOUNT: &str = "This conversation uses a removed provider account. Switch accounts from the usage control to start a new conversation.";

/// A provider's browser sign-in, as the popover shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SignIn {
    #[default]
    Idle,
    Running,
    Complete,
    Failed(String),
}

#[derive(Default)]
pub struct AccountsState {
    /// MonoCode's own accounts, read once at startup.
    shared: Vec<ProviderAccount>,
    /// `(provider, id)` of the profile directories found on disk.
    on_disk: HashSet<(String, String)>,
    /// False until the startup read lands; nothing is called removed before.
    loaded: bool,
    /// Who each `(provider, id)` is signed in as; None is signed out.
    identities: HashMap<(String, String), Option<AccountIdentity>>,
}

impl AccountsState {
    pub fn identity(&self, account: &ProviderAccount) -> Option<&AccountIdentity> {
        self.identities
            .get(&(account.provider.clone(), account.id.clone()))?
            .as_ref()
    }
}

impl BenCodeApp {
    /// Reads MonoCode's account list and the profiles on disk, off the UI
    /// thread.
    pub(crate) fn load_shared_accounts(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async move {
            (crate::db::monocode_accounts::load(), accounts::profiles_on_disk())
        });
        cx.spawn(async move |this, cx| {
            let (shared, on_disk) = task.await;
            let landed = this.update(cx, |app, cx| {
                app.accounts.shared = shared;
                app.accounts.on_disk = on_disk.into_iter().collect();
                app.accounts.loaded = true;
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("accounts after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// `provider`'s accounts, the default profile first. A profile found
    /// on disk that no list names still shows, so its threads keep running.
    pub fn provider_accounts(&self, provider: &str) -> Vec<ProviderAccount> {
        let mut listed =
            accounts::provider_accounts(provider, &self.settings.provider_accounts, &self.accounts.shared);
        let mut unnamed: Vec<&String> = self
            .accounts
            .on_disk
            .iter()
            .filter(|(owner, id)| owner == provider && !listed.iter().any(|a| &a.id == id))
            .map(|(_, id)| id)
            .collect();
        unnamed.sort();
        listed.extend(unnamed.into_iter().map(|id| ProviderAccount {
            id: id.clone(),
            provider: provider.to_string(),
            label: "Unnamed account".into(),
        }));
        listed
    }

    /// The key a project's account choice is saved under.
    fn account_project_key(&self, session: Option<&SessionRow>) -> String {
        normalize_project_path(session.map_or(self.current_cwd.as_str(), |s| s.cwd.as_str()))
    }

    /// The account `session` runs under: its own once it has one, else the
    /// one chosen for its project. Threads from before accounts existed
    /// belong to the default profile.
    pub fn session_account_id(&self, session: &SessionRow) -> String {
        if let Some(id) = &session.provider_account_id {
            return id.clone();
        }
        if session.blocks.iter().any(|block| block.role == "user") {
            return DEFAULT_ACCOUNT_ID.into();
        }
        accounts::selected_account_id(
            &self.settings.provider_account_selections,
            &self.account_project_key(Some(session)),
            &session.harness,
            &self.provider_accounts(&session.harness),
        )
    }

    /// Whether `provider` still has the account `id`.
    pub fn account_exists(&self, provider: &str, id: &str) -> bool {
        !self.accounts.loaded || self.provider_accounts(provider).iter().any(|a| a.id == id)
    }

    /// Gives a thread about to run its account, for good: provider thread
    /// ids are account-owned. Fails when that account was removed.
    pub(crate) fn pin_session_account(&mut self, session_id: &str) -> Result<(), String> {
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return Ok(());
        };
        if !supports_accounts(&session.harness) {
            return Ok(());
        }
        let account_id = self.session_account_id(session);
        if !self.account_exists(&session.harness, &account_id) {
            return Err(REMOVED_ACCOUNT.into());
        }
        if session.provider_account_id.as_deref() != Some(account_id.as_str()) {
            if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
                session.provider_account_id = Some(account_id);
            }
            self.persist_session(session_id);
        }
        Ok(())
    }

    /// MonoCode `selectAccount` + `onSelectProviderAccount`: remembers the
    /// choice for the project; an empty active thread takes the account, a
    /// started one stays on its own and a new thread opens for the choice.
    pub fn select_provider_account(&mut self, provider: &str, account_id: &str, cx: &mut Context<Self>) {
        if !self.provider_accounts(provider).iter().any(|a| a.id == account_id) {
            return;
        }
        let project = self.account_project_key(self.selected_session());
        self.update_provider_accounts(
            |_, selections| {
                selections
                    .entry(project)
                    .or_default()
                    .insert(provider.to_string(), account_id.to_string());
            },
            cx,
        );
        let Some(active) = self.selected_session().filter(|s| s.harness == provider) else {
            return;
        };
        let current = active
            .provider_account_id
            .clone()
            .unwrap_or_else(|| DEFAULT_ACCOUNT_ID.into());
        let (active_id, cwd) = (active.id.clone(), active.cwd.clone());
        let (model, harness) = (active.model.clone(), active.harness.clone());
        let empty = active.blocks.is_empty() && is_blank_session(active, self.is_agent_running_in(&active_id));
        if current == account_id && active.provider_account_id.is_some() {
            return;
        }
        let target = if empty {
            active_id
        } else if current == account_id {
            return;
        } else {
            let id = self.create_session_row(&cwd);
            if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
                session.model = model;
                session.harness = harness;
            }
            self.tabs.open(&id);
            self.sync_selection(cx);
            self.refocus_prompt(cx);
            id
        };
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == target) {
            session.provider_account_id = Some(account_id.to_string());
        }
        self.persist_session(&target);
        cx.notify();
    }

    /// The Add account form: names a new profile, runs the provider's
    /// sign-in into it, and on success lists and selects it.
    pub fn add_provider_account(&mut self, provider: RateLimitProvider, cx: &mut Context<Self>) {
        let label = self.account_name_input.read(cx).text().to_string();
        let Some(kind) = HarnessKind::from_id(provider.id()) else {
            return;
        };
        if label.trim().is_empty() || self.usage.adding {
            return;
        }
        let account = accounts::new_account(provider.id(), &label, self.provider_accounts(provider.id()).len());
        let Some(profile) = AccountProfile::resolve(&account.provider, Some(&account.id)) else {
            return;
        };
        self.usage.adding = true;
        self.usage.add_error = None;
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            let result = crate::harness::login::login(kind, Some(&profile));
            if result.is_err() {
                profile.discard();
            }
            result
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                app.usage.adding = false;
                match result {
                    Ok(()) => {
                        let saved = account.clone();
                        app.update_provider_accounts(
                            |stored, _| stored.entry(saved.provider.clone()).or_default().push(saved),
                            cx,
                        );
                        app.accounts
                            .on_disk
                            .insert((account.provider.clone(), account.id.clone()));
                        app.select_provider_account(&account.provider, &account.id, cx);
                        app.account_name_input.update(cx, |input, cx| input.set_text("", cx));
                        app.close_usage_popover(cx);
                    }
                    Err(error) => app.usage.add_error = Some(error),
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("account sign-in after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The sign-in panel's button: signs the footer's account in again,
    /// then reads its usage.
    pub fn reconnect_provider(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.usage_target().filter(|target| target.available) else {
            return;
        };
        let Some(kind) = HarnessKind::from_id(target.provider.id()) else {
            return;
        };
        if self.usage.sign_in == SignIn::Running {
            return;
        }
        self.usage.sign_in = SignIn::Running;
        cx.notify();
        let profile = AccountProfile::resolve(target.provider.id(), Some(&target.account_id));
        let task = cx
            .background_executor()
            .spawn(async move { crate::harness::login::login(kind, profile.as_ref()) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                match result {
                    Ok(()) => {
                        app.usage.sign_in = SignIn::Complete;
                        app.load_rate_limits(target.provider, &target.account_id, true, cx);
                        app.load_account_identities(target.provider.id(), cx);
                    }
                    Err(error) => app.usage.sign_in = SignIn::Failed(error),
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("sign-in after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Reads who each of `provider`'s accounts is signed in as.
    pub fn load_account_identities(&mut self, provider: &str, cx: &mut Context<Self>) {
        let accounts = self.provider_accounts(provider);
        let provider = provider.to_string();
        let task = cx.background_executor().spawn(async move {
            accounts
                .into_iter()
                .map(|account| {
                    let profile = AccountProfile::resolve(&provider, Some(&account.id));
                    let identity = account_identity::read(&provider, profile.as_ref());
                    ((provider.clone(), account.id), identity)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let identities = task.await;
            let landed = this.update(cx, |app, cx| {
                app.accounts.identities.extend(identities);
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("account identities after app drop: {err:#}");
            }
        })
        .detach();
    }
}
