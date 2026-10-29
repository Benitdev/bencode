//! Provider usage for the usage chip (MonoCode `rateLimitsCache.ts` and
//! `UsageFooter`'s loading): each account is read once per run, again on
//! Refresh and after a turn that spent it, off the UI thread.

use std::collections::HashMap;
use std::time::Duration;

use gpui::Context;

use crate::app::accounts::SignIn;
use crate::app::{BenCodeApp, now_ms};
use crate::harness::accounts::{AccountProfile, DEFAULT_ACCOUNT_ID, supports_accounts};
use crate::rate_limits::{self, ProviderRateLimits, RateLimitProvider, RateLimitStatus};

/// MonoCode `CLOCK_MS`: how often the reset countdowns are redrawn.
const CLOCK_TICK: Duration = Duration::from_secs(30);

/// A turn's end reads its account again only when the last read is older
/// than this, so a burst of agents finishing does not hammer the provider.
const TURN_RELOAD_GAP_MS: i64 = 10_000;

static IDLE: ProviderRateLimits = ProviderRateLimits {
    session: None,
    weekly: None,
    monthly: None,
    updated_at: 0,
    error: None,
    status: RateLimitStatus::Idle,
};

/// A provider and one of its account ids.
type UsageKey = (RateLimitProvider, String);

/// The details popover's pages (MonoCode `accountView`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UsageView {
    #[default]
    Usage,
    Accounts,
    Add,
}

/// What the chip reports: the active thread's provider and account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageTarget {
    pub provider: RateLimitProvider,
    pub account_id: String,
    /// False when the thread's account was removed.
    pub available: bool,
}

/// Each account's last usage and the reads running: a model, followed by
/// the regions that show usage (`Region::Rail`, `CompactRail`, `Bot`,
/// `Surface`), so a read landing redraws those and not the app
/// (`docs/regions.md`).
#[derive(Default)]
pub struct UsageSnapshots {
    snapshots: HashMap<UsageKey, ProviderRateLimits>,
    /// Accounts being read, and whether Refresh asked for it.
    pending: HashMap<UsageKey, bool>,
}

/// The usage chip and its popover: what is open and in progress.
#[derive(Default)]
pub struct UsageState {
    /// The provider whose details popover is open, and its page.
    pub popover: Option<RateLimitProvider>,
    pub view: UsageView,
    /// The pointer is over the usage chip, so a press there is the chip's
    /// to toggle and not an outside click.
    pub chip_hovered: bool,
    /// The sign-in panel's run for the chip's account.
    pub sign_in: SignIn,
    /// The Add account form is waiting for the browser, or why it failed.
    pub adding: bool,
    pub add_error: Option<String>,
}

impl UsageSnapshots {
    pub fn limits(&self, provider: RateLimitProvider, account_id: &str) -> &ProviderRateLimits {
        self.cached(provider, account_id).unwrap_or(&IDLE)
    }

    /// The snapshot of an account that has been asked for, if any.
    pub fn cached(
        &self,
        provider: RateLimitProvider,
        account_id: &str,
    ) -> Option<&ProviderRateLimits> {
        self.snapshots.get(&(provider, account_id.to_string()))
    }

    /// Drops a removed account's snapshot (MonoCode `clearCachedRateLimits`).
    pub fn forget(&mut self, provider: RateLimitProvider, account_id: &str) {
        let key = (provider, account_id.to_string());
        self.snapshots.remove(&key);
        self.pending.remove(&key);
    }

    /// A Refresh is running (the first load only shows on the chip).
    pub fn refreshing(&self) -> bool {
        self.pending.values().any(|forced| *forced)
    }

    /// Pictures of the window (`app/snapshot.rs`) show an account's usage
    /// without reading it.
    #[cfg(feature = "snapshot")]
    pub(crate) fn stage(
        &mut self,
        provider: RateLimitProvider,
        account_id: &str,
        limits: ProviderRateLimits,
    ) {
        self.snapshots
            .insert((provider, account_id.to_string()), limits);
    }
}

impl BenCodeApp {
    /// The provider and account the chip reports, when the active
    /// thread's harness has readable usage.
    pub fn usage_target(&self) -> Option<UsageTarget> {
        let session = self.selected_session()?;
        let provider = RateLimitProvider::from_harness(&session.harness)?;
        if provider == RateLimitProvider::Antigravity {
            // One sign-in for the machine; the limits are the model group's.
            return Some(UsageTarget {
                provider,
                account_id: rate_limits::antigravity::group_of(&session.model).into(),
                available: true,
            });
        }
        if !supports_accounts(&session.harness) {
            return Some(UsageTarget {
                provider,
                account_id: DEFAULT_ACCOUNT_ID.into(),
                available: true,
            });
        }
        let account_id = self.session_account_id(session);
        Some(UsageTarget {
            available: self.account_exists(&session.harness, &account_id),
            provider,
            account_id,
        })
    }

    /// Reads an account's usage unless a snapshot exists; `force` reads it
    /// again. A read already running is left to finish.
    pub fn load_rate_limits(
        &mut self,
        provider: RateLimitProvider,
        account_id: &str,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let key = (provider, account_id.to_string());
        let data = self.usage_data.read(cx);
        let cached = data.snapshots.get(&key);
        if data.pending.contains_key(&key) || (cached.is_some() && !force) {
            return;
        }
        let fetching = ProviderRateLimits::fetching(cached);
        self.usage_data.update(cx, |data, cx| {
            data.snapshots.insert(key.clone(), fetching);
            data.pending.insert(key.clone(), force);
            cx.notify();
        });
        let profile = AccountProfile::resolve(provider.id(), Some(account_id));
        let account_id = account_id.to_string();
        let task = cx.background_executor().spawn(async move {
            rate_limits::fetch(provider, profile.as_ref(), &account_id, now_ms())
        });
        cx.spawn(async move |this, cx| {
            let fetched = task.await;
            let landed = this.update(cx, |app, cx| {
                app.usage_data.update(cx, |data, cx| {
                    let limits = fetched.into_limits(data.snapshots.get(&key), now_ms());
                    if let Some(error) = &limits.error {
                        log::info!("{} usage: {error}", provider.title());
                    }
                    data.snapshots.insert(key.clone(), limits);
                    data.pending.remove(&key);
                    cx.notify();
                });
            });
            if let Err(err) = landed {
                log::debug!("usage after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The usage chip's Refresh button.
    pub fn refresh_usage(&mut self, cx: &mut Context<Self>) {
        if let Some(target) = self.usage_target().filter(|target| target.available) {
            self.load_rate_limits(target.provider, &target.account_id, true, cx);
            cx.notify();
        }
    }

    /// Opens or closes the details popover. Opening it reads who each
    /// account is and the usage of the ones not asked yet, for the picker.
    pub fn toggle_usage_popover(&mut self, provider: RateLimitProvider, cx: &mut Context<Self>) {
        if self.close_usage_popover(cx) {
            return;
        }
        self.usage.popover = Some(provider);
        self.load_account_details(provider, false, cx);
        if provider == RateLimitProvider::Antigravity {
            // The popover shows both groups.
            for group in rate_limits::antigravity::GROUPS {
                self.load_rate_limits(provider, group, false, cx);
            }
        }
        cx.notify();
    }

    pub fn set_usage_view(&mut self, view: UsageView, cx: &mut Context<Self>) {
        self.usage.view = view;
        self.usage.add_error = None;
        if view == UsageView::Add {
            crate::ui::composer::menus::focus_later(
                gpui::Focusable::focus_handle(self.account_name_input.read(cx), cx),
                cx,
            );
        }
        cx.notify();
    }

    /// Closes the details popover; false when none was open. A sign-in
    /// that is still waiting for the browser carries on.
    pub fn close_usage_popover(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.usage.popover.take().is_some();
        if open {
            self.usage.view = UsageView::Usage;
            self.usage.add_error = None;
            if self.usage.sign_in != SignIn::Running {
                self.usage.sign_in = SignIn::Idle;
            }
            cx.notify();
        }
        open
    }

    /// The usage of the account a thread's turn just spent, read again when
    /// it has been shown (an account nobody looked at waits to be asked).
    /// Many agents ending together on one account read it once.
    pub(crate) fn reload_turn_usage(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        let Some(provider) = RateLimitProvider::from_harness(&session.harness) else {
            return;
        };
        if provider == RateLimitProvider::Antigravity {
            self.reload_antigravity_usage(cx);
            return;
        }
        let account_id = if supports_accounts(&session.harness) {
            self.session_account_id(session)
        } else {
            DEFAULT_ACCOUNT_ID.into()
        };
        let recent = self
            .usage_data
            .read(cx)
            .cached(provider, &account_id)
            .map(|limits| now_ms() - limits.updated_at < TURN_RELOAD_GAP_MS);
        if recent == Some(false) {
            self.load_rate_limits(provider, &account_id, true, cx);
        }
    }

    /// Antigravity's usage again: after a turn (which spent some and
    /// refreshed the token), and for another account after a switch.
    pub(crate) fn reload_antigravity_usage(&mut self, cx: &mut Context<Self>) {
        for group in rate_limits::antigravity::GROUPS {
            if self
                .usage_data
                .read(cx)
                .cached(RateLimitProvider::Antigravity, group)
                .is_some()
            {
                self.load_rate_limits(RateLimitProvider::Antigravity, group, true, cx);
            }
        }
    }

    /// Redraws the chip's countdowns while it shows usage.
    pub(crate) fn start_usage_clock(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK_TICK).await;
                let ticked = this.update(cx, |app, cx| {
                    if app.usage_target().is_some() {
                        // The countdowns move: the regions that follow
                        // usage redraw.
                        app.usage_data.update(cx, |_, cx| cx.notify());
                    }
                });
                if ticked.is_err() {
                    return; // app dropped
                }
            }
        })
        .detach();
    }
}
