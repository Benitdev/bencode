//! Keeps the model catalogs live (MonoCode `refreshHarnessCatalogs`): each
//! installed CLI is asked for its models at startup, when switching accounts,
//! and again when the picker opens on its provider. One probe per
//! `(harness, account)` runs at a time, and hovering across provider tabs does
//! not respawn a CLI that just answered for that account.

use std::time::{Duration, Instant};

use gpui::Context;

use crate::app::{BenCodeApp, normalize_project_path};
use crate::harness::accounts::{self, AccountProfile, DEFAULT_ACCOUNT_ID, supports_accounts};
use crate::harness::{ALL_HARNESSES, HarnessKind, HarnessResolver, catalog, discovery};

const PROBE_COOLDOWN: Duration = Duration::from_secs(60);
/// Claude's probe is a whole Claude Code, which may refresh (and rotate)
/// the sign-in as it starts; its list changes only with the CLI, whose
/// update skips the cooldown (MonoCode caches it for 6h).
const CLAUDE_PROBE_COOLDOWN: Duration = Duration::from_secs(6 * 60 * 60);

fn cooldown(kind: HarnessKind) -> Duration {
    match kind {
        HarnessKind::Claude => CLAUDE_PROBE_COOLDOWN,
        _ => PROBE_COOLDOWN,
    }
}

fn normalized_account_id(kind: HarnessKind, account_id: &str) -> String {
    if supports_accounts(kind.id()) && !account_id.is_empty() {
        account_id.to_string()
    } else {
        DEFAULT_ACCOUNT_ID.into()
    }
}

impl BenCodeApp {
    /// The account profile `kind`'s model catalog belongs to in the current
    /// view: the focused thread's own account when it runs on `kind`, else the
    /// account chosen for the active project (or automation / translation).
    pub(crate) fn catalog_account_id(&self, kind: HarnessKind) -> String {
        if !supports_accounts(kind.id()) {
            return DEFAULT_ACCOUNT_ID.into();
        }
        if self.models_for_translation() {
            return DEFAULT_ACCOUNT_ID.into();
        }
        if self.models_for_automation()
            && let Some(draft) = self.automations.draft.as_ref()
        {
            return self.project_account_id(kind, &draft.cwd);
        }
        if let Some(session) = self.selected_session()
            && session.harness == kind.id()
        {
            return self.session_account_id(session);
        }
        self.project_account_id(kind, &self.account_project_key(self.selected_session()))
    }

    /// The account a new `kind` thread in `cwd` runs on: the project's choice.
    pub(crate) fn project_account_id(&self, kind: HarnessKind, cwd: &str) -> String {
        if !supports_accounts(kind.id()) {
            return DEFAULT_ACCOUNT_ID.into();
        }
        accounts::selected_account_id(
            &self.settings.provider_account_selections,
            &normalize_project_path(cwd),
            kind.id(),
            &self.provider_accounts(kind.id()),
        )
    }

    /// Whether a model catalog probe for `kind`'s active account is currently
    /// running.
    pub(crate) fn is_catalog_loading(&self, kind: HarnessKind) -> bool {
        let key = (kind, self.catalog_account_id(kind));
        matches!(self.catalog_probes.get(&key), Some(None))
    }

    /// Probes every installed harness under its active account.
    pub fn refresh_installed_catalogs(&mut self, cx: &mut Context<Self>) {
        for kind in ALL_HARNESSES {
            self.refresh_model_catalog(kind, cx);
        }
    }

    /// The Providers page's refresh: looks for the agent CLIs again (one
    /// installed or removed since launch), then asks each installed one
    /// for its models, past the cooldown.
    pub fn rescan_harnesses(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.harness_rescan, true) {
            return;
        }
        cx.notify();
        // A probe keeps the newest Codex it picked over the first one on
        // PATH, while that copy is still there.
        let codex = self
            .harnesses
            .iter()
            .find(|h| h.id == HarnessKind::Codex.id())
            .and_then(|h| h.binary_path.clone());
        let task = cx.background_executor().spawn(async move {
            let mut found = HarnessResolver::discover();
            if let Some(path) = codex.filter(|path| path.is_file())
                && let Some(info) = found
                    .iter_mut()
                    .find(|h| h.available && h.id == HarnessKind::Codex.id())
            {
                info.binary_path = Some(path);
            }
            found
        });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let updated = this.update(cx, |app, cx| {
                app.harness_rescan = false;
                app.harnesses = found;
                for kind in ALL_HARNESSES {
                    app.force_refresh_model_catalog(kind, cx);
                }
                cx.notify();
            });
            if let Err(err) = updated {
                log::debug!("harness rescan after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// After the CLI changed (an update or rescan): skips every account's
    /// cooldown for `kind` and probes the active account, so the picker gets
    /// the new version's models.
    pub fn force_refresh_model_catalog(&mut self, kind: HarnessKind, cx: &mut Context<Self>) {
        // A probe still running answers for the CLI or sign-in that was.
        self.catalog_stale.extend(
            self.catalog_probes
                .iter()
                .filter(|((k, _), state)| *k == kind && state.is_none())
                .map(|(key, _)| key.clone()),
        );
        self.catalog_probes
            .retain(|(k, _), state| *k != kind || state.is_none());
        self.refresh_model_catalog(kind, cx);
    }

    /// Skips the cooldown for `(kind, account_id)` and probes it now (for
    /// example after signing in or reconnecting that account).
    pub fn force_refresh_model_catalog_for(
        &mut self,
        kind: HarnessKind,
        account_id: &str,
        cx: &mut Context<Self>,
    ) {
        let account_id = normalized_account_id(kind, account_id);
        let key = (kind, account_id.clone());
        match self.catalog_probes.get(&key) {
            Some(Some(_)) => {
                self.catalog_probes.remove(&key);
            }
            Some(None) => {
                self.catalog_stale.insert(key);
            }
            None => {}
        }
        self.refresh_model_catalog_for(kind, &account_id, cx);
    }

    /// Called when switching to `account_id`: probes immediately if that
    /// account has no loaded models yet (even past a prior failed cooldown),
    /// otherwise refreshes when its cooldown has elapsed.
    pub fn refresh_account_catalog_on_switch(
        &mut self,
        kind: HarnessKind,
        account_id: &str,
        cx: &mut Context<Self>,
    ) {
        let account_id = normalized_account_id(kind, account_id);
        if !catalog::has_account_models(kind, Some(&account_id)) {
            self.force_refresh_model_catalog_for(kind, &account_id, cx);
        } else {
            self.refresh_model_catalog_for(kind, &account_id, cx);
        }
    }

    /// Drops a removed account's cached models and probe cooldown.
    pub fn forget_account_catalog(&mut self, kind: HarnessKind, account_id: &str) {
        let account_id = normalized_account_id(kind, account_id);
        let key = (kind, account_id);
        self.catalog_probes.remove(&key);
        self.catalog_stale.remove(&key);
        catalog::forget_account_models(kind, &key.1);
    }

    /// Replaces `kind`'s active account models with what its CLI reports; a
    /// failed probe keeps the current list (MonoCode logs it and moves on).
    pub fn refresh_model_catalog(&mut self, kind: HarnessKind, cx: &mut Context<Self>) {
        let account_id = self.catalog_account_id(kind);
        self.refresh_model_catalog_for(kind, &account_id, cx);
    }

    /// Replaces `(kind, account_id)`'s models with what its CLI reports under
    /// that account profile.
    pub fn refresh_model_catalog_for(
        &mut self,
        kind: HarnessKind,
        account_id: &str,
        cx: &mut Context<Self>,
    ) {
        let account_id = normalized_account_id(kind, account_id);
        let installed = self
            .harnesses
            .iter()
            .any(|h| h.available && h.id == kind.id());
        if !installed {
            return;
        }
        if supports_accounts(kind.id()) && !self.account_exists(kind.id(), &account_id) {
            return;
        }
        let key = (kind, account_id.clone());
        let busy = match self.catalog_probes.get(&key) {
            Some(None) => true,
            Some(Some(next)) => Instant::now() < *next,
            None => false,
        };
        if busy {
            return;
        }
        self.catalog_probes.insert(key.clone(), None);
        let profile = AccountProfile::resolve(kind.id(), Some(&account_id));
        let task = cx
            .background_executor()
            .spawn(async move { discovery::discover_for(kind, profile.as_ref()) });
        cx.spawn(async move |this, cx| {
            let models = task.await;
            let updated = this.update(cx, |app, cx| {
                // Startup lists the first Codex found; the probe has now
                // picked the newest copy, which is the one turns run.
                if kind == HarnessKind::Codex
                    && let Some(path) = HarnessResolver::resolved_codex()
                    && let Some(info) = app.harnesses.iter_mut().find(|h| h.id == kind.id())
                {
                    info.binary_path = Some(path);
                }
                if supports_accounts(kind.id()) && !app.account_exists(kind.id(), &account_id) {
                    app.catalog_probes.remove(&key);
                    app.catalog_stale.remove(&key);
                    cx.notify();
                    return;
                }
                if app.catalog_stale.remove(&key) {
                    log::info!(
                        "{} ({account_id}) model catalog changed while probing; asking again",
                        kind.label()
                    );
                    app.catalog_probes.remove(&key);
                    app.refresh_model_catalog_for(kind, &account_id, cx);
                    return;
                }
                // A failed probe tries again soon, whatever the harness.
                let wait = if models.is_ok() {
                    cooldown(kind)
                } else {
                    PROBE_COOLDOWN
                };
                app.catalog_probes.insert(key, Some(Instant::now() + wait));
                match models {
                    Ok(models) => {
                        log::info!(
                            "{} ({account_id}) lists {} models",
                            kind.label(),
                            models.len()
                        );
                        catalog::set_account_models(kind, Some(&account_id), models);
                    }
                    Err(err) => {
                        log::warn!("{} ({account_id}) model catalog: {err:#}", kind.label())
                    }
                }
                cx.notify();
            });
            if let Err(err) = updated {
                log::debug!("model catalog after app drop: {err:#}");
            }
        })
        .detach();
    }
}
