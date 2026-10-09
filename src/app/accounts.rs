//! Provider accounts in the app (MonoCode `providerAccounts.ts`,
//! `UsageFooter`'s account callbacks and `App.tsx` `onSelectProviderAccount`):
//! which profile a thread runs under, switching, adding one through the
//! provider's browser sign-in, and who each profile is signed in as; and
//! Settings › Accounts (`SettingsView.tsx` `ProviderAccountsSettings`):
//! rename and remove.

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

/// The providers Settings › Accounts lists (MonoCode
/// `PROVIDER_ACCOUNT_PROVIDERS`).
pub const ACCOUNT_PROVIDERS: [RateLimitProvider; 2] =
    [RateLimitProvider::Claude, RateLimitProvider::Codex];

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

/// Where an Add account form was sent from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccountForm {
    Popover,
    Settings,
}

/// Settings › Accounts' inline name field (MonoCode `AccountEditor`): a new
/// account of `provider`, or the rename of `account_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountEditor {
    pub provider: RateLimitProvider,
    pub account_id: Option<String>,
}

#[derive(Default)]
pub struct AccountsState {
    /// `(provider, id)` of the profile directories found on disk.
    on_disk: HashSet<(String, String)>,
    /// False until the startup read lands; nothing is called removed before.
    loaded: bool,
    /// Who each `(provider, id)` is signed in as; None is signed out.
    identities: HashMap<(String, String), Option<AccountIdentity>>,
    /// Settings › Accounts: the name field, while open.
    pub editor: Option<AccountEditor>,
    /// Settings is signing an account in or removing one; its buttons wait.
    pub working: Option<Working>,
    /// Why Settings' last add, rename or remove failed.
    pub error: Option<String>,
    /// The account the Remove confirmation asks about.
    pub pending_remove: Option<ProviderAccount>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Working {
    Adding(RateLimitProvider),
    Removing(String, String),
}

impl AccountsState {
    pub fn identity(&self, account: &ProviderAccount) -> Option<&AccountIdentity> {
        self.identities
            .get(&(account.provider.clone(), account.id.clone()))?
            .as_ref()
    }
}

impl BenCodeApp {
    /// Reads the profiles on disk, off the UI thread.
    pub(crate) fn load_account_profiles(&mut self, cx: &mut Context<Self>) {
        let task = cx
            .background_executor()
            .spawn(async move { accounts::profiles_on_disk() });
        cx.spawn(async move |this, cx| {
            let on_disk = task.await;
            let landed = this.update(cx, |app, cx| {
                app.accounts.on_disk = on_disk.into_iter().collect();
                app.accounts.loaded = true;
                // Profiles no list names show up now; a page already open
                // reads them too, or they would stay "Checking…".
                if let Some(provider) = app.usage.popover {
                    app.load_account_details(provider, false, cx);
                }
                if app.surface_open(crate::app::surfaces::Surface::Settings)
                    && app.settings_tab == crate::ui::settings_modal::SettingsTab::Providers
                {
                    app.load_accounts_page(false, cx);
                }
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
        let mut listed = accounts::provider_accounts(provider, &self.settings.provider_accounts);
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
    pub fn select_provider_account(
        &mut self,
        provider: &str,
        account_id: &str,
        cx: &mut Context<Self>,
    ) {
        if !self
            .provider_accounts(provider)
            .iter()
            .any(|a| a.id == account_id)
        {
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
        let empty = active.blocks.is_empty()
            && is_blank_session(active, self.is_agent_running_in(&active_id));
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

    /// The popover's Add account form: names a new profile, runs the
    /// provider's sign-in into it, and on success lists and selects it.
    pub fn add_provider_account(&mut self, provider: RateLimitProvider, cx: &mut Context<Self>) {
        let label = self.account_name_input.read(cx).text().to_string();
        self.sign_in_new_account(provider, &label, AccountForm::Popover, cx);
    }

    fn sign_in_new_account(
        &mut self,
        provider: RateLimitProvider,
        label: &str,
        form: AccountForm,
        cx: &mut Context<Self>,
    ) {
        let Some(kind) = HarnessKind::from_id(provider.id()) else {
            return;
        };
        if label.trim().is_empty() || self.usage.adding || self.accounts.working.is_some() {
            return;
        }
        let account = accounts::new_account(
            provider.id(),
            label,
            self.provider_accounts(provider.id()).len(),
        );
        let Some(profile) = AccountProfile::resolve(&account.provider, Some(&account.id)) else {
            return;
        };
        match form {
            AccountForm::Popover => {
                self.usage.adding = true;
                self.usage.add_error = None;
            }
            AccountForm::Settings => {
                self.accounts.working = Some(Working::Adding(provider));
                self.accounts.error = None;
            }
        }
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
                match form {
                    AccountForm::Popover => app.usage.adding = false,
                    AccountForm::Settings => app.accounts.working = None,
                }
                match result {
                    Ok(()) => {
                        let saved = account.clone();
                        app.update_provider_accounts(
                            |stored, _| {
                                stored
                                    .entry(saved.provider.clone())
                                    .or_default()
                                    .push(saved)
                            },
                            cx,
                        );
                        app.accounts
                            .on_disk
                            .insert((account.provider.clone(), account.id.clone()));
                        app.load_account_identities(&account.provider, cx);
                        app.load_rate_limits(provider, &account.id, false, cx);
                        match form {
                            AccountForm::Popover => {
                                app.select_provider_account(&account.provider, &account.id, cx);
                                app.account_name_input
                                    .update(cx, |input, cx| input.set_text("", cx));
                                app.close_usage_popover(cx);
                            }
                            AccountForm::Settings => app.accounts.editor = None,
                        }
                    }
                    Err(error) => match form {
                        AccountForm::Popover => app.usage.add_error = Some(error),
                        AccountForm::Settings => app.accounts.error = Some(error),
                    },
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("account sign-in after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Settings › Accounts opened: who each account is and its usage, read
    /// once; `force` (Refresh) reads the usage again.
    pub fn load_accounts_page(&mut self, force: bool, cx: &mut Context<Self>) {
        for provider in ACCOUNT_PROVIDERS {
            self.load_account_details(provider, force, cx);
        }
        if force {
            self.load_agy_accounts(cx);
        } else {
            self.ensure_agy_accounts(cx);
        }
    }

    /// Who each of `provider`'s accounts is, and the usage of the ones not
    /// read yet (all of them with `force`).
    pub fn load_account_details(
        &mut self,
        provider: RateLimitProvider,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        if !supports_accounts(provider.id()) {
            return;
        }
        self.load_account_identities(provider.id(), cx);
        for account in self.provider_accounts(provider.id()) {
            self.load_rate_limits(provider, &account.id, force, cx);
        }
    }

    /// Opens Settings' name field to add an account of `provider`, or to
    /// rename `account`.
    pub fn open_account_editor(
        &mut self,
        provider: RateLimitProvider,
        account: Option<&ProviderAccount>,
        cx: &mut Context<Self>,
    ) {
        if self.accounts.working.is_some() {
            return;
        }
        self.accounts.error = None;
        self.agy_accounts.renaming = None;
        self.accounts.editor = Some(AccountEditor {
            provider,
            account_id: account.map(|account| account.id.clone()),
        });
        let label = account
            .map_or("", |account| account.label.as_str())
            .to_string();
        self.account_editor_input
            .update(cx, |input, cx| input.set_text(label, cx));
        crate::ui::composer::menus::focus_later(
            gpui::Focusable::focus_handle(self.account_editor_input.read(cx), cx),
            cx,
        );
        cx.notify();
    }

    pub fn close_account_editor(&mut self, cx: &mut Context<Self>) {
        if self.accounts.working.is_some() {
            return;
        }
        self.accounts.editor = None;
        self.accounts.error = None;
        self.agy_accounts.renaming = None;
        cx.notify();
    }

    /// The name field's Save, or Sign in and add.
    pub fn submit_account_editor(&mut self, cx: &mut Context<Self>) {
        if self.submit_agy_rename(cx) {
            return;
        }
        let Some(editor) = self.accounts.editor.clone() else {
            return;
        };
        let label = self.account_editor_input.read(cx).text().to_string();
        if label.trim().is_empty() || self.accounts.working.is_some() {
            return;
        }
        let Some(account_id) = editor.account_id else {
            self.sign_in_new_account(editor.provider, &label, AccountForm::Settings, cx);
            return;
        };
        let Some(account) = self
            .provider_accounts(editor.provider.id())
            .into_iter()
            .find(|account| account.id == account_id)
        else {
            self.accounts.editor = None;
            cx.notify();
            return;
        };
        self.update_provider_accounts(
            |stored, _| {
                accounts::rename_account(stored, &account, &label);
            },
            cx,
        );
        self.accounts.editor = None;
        cx.notify();
    }

    /// Remove asks first (MonoCode's warning dialog).
    pub fn request_remove_account(&mut self, account: ProviderAccount, cx: &mut Context<Self>) {
        if account.is_default() || self.accounts.working.is_some() {
            return;
        }
        self.accounts.pending_remove = Some(account);
        cx.notify();
    }

    pub fn cancel_remove_account(&mut self, cx: &mut Context<Self>) {
        self.accounts.pending_remove = None;
        cx.notify();
    }

    /// MonoCode `removeAccount`: stops the account's running turns, deletes
    /// its sign-in and profile directory, then forgets it. Its threads stay
    /// and say their account was removed.
    pub fn confirm_remove_account(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.accounts.pending_remove.take() else {
            return;
        };
        let Some(profile) = AccountProfile::resolve(&account.provider, Some(&account.id)) else {
            cx.notify();
            return;
        };
        let running: Vec<String> = self
            .runs
            .keys()
            .filter(|id| {
                self.sessions.iter().any(|session| {
                    &session.id == *id
                        && session.harness == account.provider
                        && self.session_account_id(session) == account.id
                })
            })
            .cloned()
            .collect();
        for session_id in running {
            self.stop_agent(&session_id, cx);
        }
        self.accounts.working = Some(Working::Removing(
            account.provider.clone(),
            account.id.clone(),
        ));
        self.accounts.error = None;
        cx.notify();
        let task = cx
            .background_executor()
            .spawn(async move { profile.remove() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                app.accounts.working = None;
                match result {
                    Ok(()) => app.forget_account(&account, cx),
                    Err(error) => {
                        log::warn!("removing account {}: {error}", account.id);
                        app.accounts.error = Some(error);
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("account removal after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn forget_account(&mut self, account: &ProviderAccount, cx: &mut Context<Self>) {
        self.update_provider_accounts(
            |stored, selections| {
                accounts::remove_account(stored, selections, &account.provider, &account.id)
            },
            cx,
        );
        let key = (account.provider.clone(), account.id.clone());
        self.accounts.on_disk.remove(&key);
        self.accounts.identities.remove(&key);
        if let Some(provider) = RateLimitProvider::from_harness(&account.provider) {
            self.usage.forget(provider, &account.id);
        }
        if self
            .accounts
            .editor
            .as_ref()
            .is_some_and(|editor| editor.account_id.as_deref() == Some(account.id.as_str()))
        {
            self.accounts.editor = None;
        }
    }

    /// The popover's Manage accounts…: Settings › Providers.
    pub fn manage_accounts(&mut self, cx: &mut Context<Self>) {
        use crate::app::surfaces::Surface;
        use crate::ui::settings_modal::SettingsTab;
        self.close_usage_popover(cx);
        if self.surface_open(Surface::Settings) {
            self.select_settings_tab(SettingsTab::Providers, cx);
        } else {
            // Opening Settings reads the page's accounts.
            self.settings_tab = SettingsTab::Providers;
            self.open_settings(cx);
        }
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
