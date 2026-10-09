//! Antigravity accounts in the app (BenCode's own; MonoCode has none):
//! the saved sign-ins of `harness/agy_accounts.rs`, which one `agy` uses
//! now, switching, Add account through `agy`'s own sign-in in the terminal
//! dock, rename and remove. The account is the machine's, not a thread's:
//! every Antigravity turn runs as the one switched to.

use std::time::{Duration, Instant};

use gpui::Context;

use crate::app::BenCodeApp;
use crate::harness::accounts::{self, ProviderAccount};
use crate::harness::agy_accounts::{self, AgyProfile, PROVIDER, Snapshot};

/// How often a sign-in under way is looked for in the Keychain.
const SIGN_IN_POLL: Duration = Duration::from_secs(2);
/// A sign-in nobody finished is given up, and the old one put back.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(600);

const TURNS_RUNNING: &str = "Stop the running Antigravity turns before changing its account.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgyWork {
    Switching(String),
    Removing(String),
    SigningIn,
}

#[derive(Default)]
pub struct AgyAccountsState {
    pub profiles: Vec<AgyProfile>,
    /// Who `agy` is signed in as; None is signed out.
    pub live_email: Option<String>,
    /// The first read was asked for.
    requested: bool,
    /// Settings waits on this; its buttons are off meanwhile.
    pub working: Option<AgyWork>,
    pub error: Option<String>,
    /// The account whose name field is open.
    pub renaming: Option<String>,
    /// The account the Remove confirmation asks about.
    pub pending_remove: Option<String>,
    /// Counts sign-ins, so a cancelled one's poll stops.
    sign_in_run: u64,
    /// The Keychain item a sign-in replaced, to put back if it is cancelled.
    previous: Option<String>,
}

impl AgyAccountsState {
    /// Whether `agy` is signed in as `profile`.
    pub fn is_active(&self, profile: &AgyProfile) -> bool {
        match (&self.live_email, &profile.email) {
            (Some(live), Some(email)) => live.eq_ignore_ascii_case(email),
            _ => false,
        }
    }
}

impl BenCodeApp {
    /// The saved Antigravity accounts under their names: the one given in
    /// Settings, else the email.
    pub fn agy_accounts(&self) -> Vec<(ProviderAccount, &AgyProfile)> {
        let stored = self.settings.provider_accounts.get(PROVIDER);
        self.agy_accounts
            .profiles
            .iter()
            .map(|profile| {
                let label = stored
                    .and_then(|list| list.iter().find(|account| account.id == profile.id))
                    .map(|account| accounts::clean_label(&account.label))
                    .filter(|label| !label.is_empty())
                    .or_else(|| profile.email.clone())
                    .unwrap_or_else(|| "Unnamed account".into());
                let account = ProviderAccount {
                    id: profile.id.clone(),
                    provider: PROVIDER.into(),
                    label,
                };
                (account, profile)
            })
            .collect()
    }

    /// The name of the account `agy` is signed in as, else its email when it
    /// is not a saved one; None when signed out.
    pub fn agy_live_label(&self) -> Option<String> {
        let live = self.agy_accounts.live_email.as_ref()?;
        let saved = self
            .agy_accounts()
            .into_iter()
            .find(|(_, profile)| self.agy_accounts.is_active(profile))
            .map(|(account, _)| account.label);
        Some(saved.unwrap_or_else(|| live.clone()))
    }

    fn agy_turn_running(&self) -> bool {
        self.runs.keys().any(|id| {
            self.sessions
                .iter()
                .any(|session| &session.id == id && session.harness == PROVIDER)
        })
    }

    /// Whether the account may change now; says why not otherwise.
    fn agy_account_free(&mut self, cx: &mut Context<Self>) -> bool {
        if self.agy_accounts.working.is_some() {
            return false;
        }
        if self.agy_turn_running() {
            self.agy_accounts.error = Some(TURNS_RUNNING.into());
            cx.notify();
            return false;
        }
        true
    }

    /// Reads the accounts the first time something shows them.
    pub fn ensure_agy_accounts(&mut self, cx: &mut Context<Self>) {
        if !self.agy_accounts.requested {
            self.load_agy_accounts(cx);
        }
    }

    /// Reads the saved accounts and who `agy` is signed in as.
    pub fn load_agy_accounts(&mut self, cx: &mut Context<Self>) {
        self.agy_accounts.requested = true;
        self.run_agy_job(None, || Ok(()), cx);
    }

    /// Runs `job` then reads the accounts again, off the UI thread.
    fn run_agy_job(
        &mut self,
        work: Option<AgyWork>,
        job: impl FnOnce() -> Result<(), String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let waited = work.is_some();
        if waited {
            self.agy_accounts.working = work;
            self.agy_accounts.error = None;
            cx.notify();
        }
        let task = cx.background_executor().spawn(async move {
            let done = job();
            (done, agy_accounts::load())
        });
        cx.spawn(async move |this, cx| {
            let (done, snapshot) = task.await;
            let landed = this.update(cx, |app, cx| {
                if waited {
                    app.agy_accounts.working = None;
                }
                if let Err(error) = done {
                    log::warn!("antigravity account: {error}");
                    app.agy_accounts.error = Some(error);
                }
                match snapshot {
                    Ok(snapshot) => app.land_agy_snapshot(snapshot, cx),
                    Err(error) => log::warn!("antigravity accounts: {error}"),
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("antigravity accounts after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn land_agy_snapshot(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.agy_accounts.profiles = snapshot.profiles;
        if self.agy_accounts.live_email != snapshot.live_email {
            self.agy_accounts.live_email = snapshot.live_email;
            self.reload_antigravity_usage(cx);
        }
        if !snapshot.imported.is_empty() {
            // The script profiles keep the names their folders had.
            self.update_provider_accounts(
                |stored, _| {
                    stored
                        .entry(PROVIDER.into())
                        .or_default()
                        .extend(snapshot.imported)
                },
                cx,
            );
        }
    }

    /// `agy-switch`: every Antigravity turn from now on runs as `id`.
    pub fn switch_agy_account(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.agy_account_free(cx) {
            return;
        }
        let id = id.to_string();
        let target = id.clone();
        self.run_agy_job(
            Some(AgyWork::Switching(id)),
            move || agy_accounts::activate(&target),
            cx,
        );
    }

    /// Add account: signs `agy` out (keeping the account it had), then runs
    /// it in the terminal dock, where it asks for a sign-in. The account it
    /// ends in is saved when it shows up in the Keychain.
    pub fn add_agy_account(&mut self, cx: &mut Context<Self>) {
        if !self.agy_account_free(cx) {
            return;
        }
        self.agy_accounts.working = Some(AgyWork::SigningIn);
        self.agy_accounts.error = None;
        self.agy_accounts.sign_in_run += 1;
        let run = self.agy_accounts.sign_in_run;
        cx.notify();
        let new_id = accounts::new_account(PROVIDER, "", 0).id;
        let begin = cx
            .background_executor()
            .spawn(async move { agy_accounts::begin_sign_in(&new_id) });
        cx.spawn(async move |this, cx| {
            let started = begin.await;
            let opened = this.update(cx, |app, cx| match started {
                Ok(start) => {
                    app.agy_accounts.previous = start.previous;
                    app.open_agy_sign_in_terminal(cx);
                    true
                }
                Err(error) => {
                    app.agy_accounts.working = None;
                    app.agy_accounts.error = Some(error);
                    cx.notify();
                    false
                }
            });
            if !matches!(opened, Ok(true)) {
                return;
            }
            let deadline = Instant::now() + SIGN_IN_TIMEOUT;
            loop {
                cx.background_executor().timer(SIGN_IN_POLL).await;
                let waiting = this.update(cx, |app, _| {
                    app.agy_accounts.working == Some(AgyWork::SigningIn)
                        && app.agy_accounts.sign_in_run == run
                });
                if !matches!(waiting, Ok(true)) {
                    return; // cancelled, or the app is gone
                }
                let new_id = accounts::new_account(PROVIDER, "", 0).id;
                let polled = cx
                    .background_executor()
                    .spawn(async move { agy_accounts::finish_sign_in(&new_id) })
                    .await;
                let landed = match polled {
                    Ok(None) if Instant::now() < deadline => continue,
                    Ok(None) => this.update(cx, |app, cx| {
                        app.cancel_agy_sign_in(cx);
                        app.agy_accounts.error =
                            Some("The Antigravity sign-in was not finished.".into());
                    }),
                    Ok(Some(_)) => this.update(cx, |app, cx| {
                        app.agy_accounts.working = None;
                        app.agy_accounts.previous = None;
                        app.load_agy_accounts(cx);
                        // Back to the list, which now has the account.
                        app.manage_accounts(cx);
                    }),
                    Err(error) => this.update(cx, |app, cx| {
                        app.cancel_agy_sign_in(cx);
                        app.agy_accounts.error = Some(error);
                    }),
                };
                if let Err(err) = landed {
                    log::debug!("antigravity sign-in after app drop: {err:#}");
                }
                return;
            }
        })
        .detach();
    }

    /// Shows the workspace with a new terminal running `agy`.
    fn open_agy_sign_in_terminal(&mut self, cx: &mut Context<Self>) {
        // A view covers the dock.
        while self.surface.is_some() {
            self.close_surface(cx);
        }
        let program = self
            .harnesses
            .iter()
            .find(|info| info.id == PROVIDER)
            .and_then(|info| info.binary_path.as_ref())
            .map_or_else(
                || "agy".to_string(),
                |path| path.to_string_lossy().into_owned(),
            );
        self.new_terminal(cx);
        let Some(tab) = self
            .terminals
            .dock(&self.current_cwd)
            .and_then(|dock| dock.tabs.last())
        else {
            return;
        };
        // The shell reads this once it is up.
        let line = format!("'{}'\n", program.replace('\'', "'\\''"));
        tab.entity
            .update(cx, |terminal, _| terminal.write(line.into_bytes()));
    }

    /// Gives a sign-in up: `agy` is signed back in as it was.
    pub fn cancel_agy_sign_in(&mut self, cx: &mut Context<Self>) {
        if self.agy_accounts.working != Some(AgyWork::SigningIn) {
            return;
        }
        self.agy_accounts.working = None;
        let previous = self.agy_accounts.previous.take();
        self.run_agy_job(
            None,
            move || agy_accounts::cancel_sign_in(previous.as_deref()),
            cx,
        );
        cx.notify();
    }

    /// Opens the name field of account `id`.
    pub fn rename_agy_account(&mut self, account: &ProviderAccount, cx: &mut Context<Self>) {
        if self.agy_accounts.working.is_some() || self.accounts.working.is_some() {
            return;
        }
        self.accounts.editor = None;
        self.agy_accounts.error = None;
        self.agy_accounts.renaming = Some(account.id.clone());
        let label = account.label.clone();
        self.account_editor_input
            .update(cx, |input, cx| input.set_text(label, cx));
        crate::ui::composer::menus::focus_later(
            gpui::Focusable::focus_handle(self.account_editor_input.read(cx), cx),
            cx,
        );
        cx.notify();
    }

    /// The name field's Save. False when no Antigravity account is being
    /// renamed, so the field is another provider's.
    pub(super) fn submit_agy_rename(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.agy_accounts.renaming.clone() else {
            return false;
        };
        let label = self.account_editor_input.read(cx).text().to_string();
        if label.trim().is_empty() {
            return true;
        }
        let account = ProviderAccount {
            id,
            provider: PROVIDER.into(),
            label: String::new(),
        };
        self.update_provider_accounts(
            |stored, _| {
                accounts::rename_account(stored, &account, &label);
            },
            cx,
        );
        self.agy_accounts.renaming = None;
        cx.notify();
        true
    }

    /// Remove asks first.
    pub fn request_remove_agy_account(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.agy_accounts.working.is_some() {
            return;
        }
        self.agy_accounts.pending_remove = Some(id.to_string());
        cx.notify();
    }

    pub fn cancel_remove_agy_account(&mut self, cx: &mut Context<Self>) {
        self.agy_accounts.pending_remove = None;
        cx.notify();
    }

    /// Forgets a saved account and its name. `agy` stays signed in.
    pub fn confirm_remove_agy_account(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.agy_accounts.pending_remove.take() else {
            return;
        };
        if self.agy_accounts.working.is_some() {
            cx.notify();
            return;
        }
        if self.agy_accounts.renaming.as_deref() == Some(id.as_str()) {
            self.agy_accounts.renaming = None;
        }
        self.update_provider_accounts(
            |stored, selections| accounts::remove_account(stored, selections, PROVIDER, &id),
            cx,
        );
        let target = id.clone();
        self.run_agy_job(
            Some(AgyWork::Removing(id)),
            move || agy_accounts::remove(&target),
            cx,
        );
    }
}
