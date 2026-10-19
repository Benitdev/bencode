//! Keeps the model catalogs live (MonoCode `refreshHarnessCatalogs`): each
//! installed CLI is asked for its models at startup, and again when the
//! picker opens on its provider. One probe per harness runs at a time, and
//! hovering across provider tabs does not respawn a CLI that just answered.

use std::time::{Duration, Instant};

use gpui::Context;

use crate::app::BenCodeApp;
use crate::harness::{ALL_HARNESSES, HarnessKind, HarnessResolver, catalog, discovery};

const PROBE_COOLDOWN: Duration = Duration::from_secs(60);

impl BenCodeApp {
    /// Probes every installed harness.
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

    /// After the CLI changed (an update): skips the cooldown, so the picker
    /// gets the new version's models.
    pub fn force_refresh_model_catalog(&mut self, kind: HarnessKind, cx: &mut Context<Self>) {
        if let Some(Some(_)) = self.catalog_probes.get(&kind) {
            self.catalog_probes.remove(&kind);
        }
        self.refresh_model_catalog(kind, cx);
    }

    /// Replaces `kind`'s models with what its CLI reports; a failed probe
    /// keeps the current list (MonoCode logs it and moves on).
    pub fn refresh_model_catalog(&mut self, kind: HarnessKind, cx: &mut Context<Self>) {
        let installed = self
            .harnesses
            .iter()
            .any(|h| h.available && h.id == kind.id());
        let busy = match self.catalog_probes.get(&kind) {
            Some(None) => true,
            Some(Some(ended)) => ended.elapsed() < PROBE_COOLDOWN,
            None => false,
        };
        if !installed || busy {
            return;
        }
        self.catalog_probes.insert(kind, None);
        let task = cx
            .background_executor()
            .spawn(async move { discovery::discover(kind) });
        cx.spawn(async move |this, cx| {
            let models = task.await;
            let updated = this.update(cx, |app, cx| {
                app.catalog_probes.insert(kind, Some(Instant::now()));
                // Startup lists the first Codex found; the probe has now
                // picked the newest copy, which is the one turns run.
                if kind == HarnessKind::Codex
                    && let Some(path) = HarnessResolver::resolved_codex()
                    && let Some(info) = app.harnesses.iter_mut().find(|h| h.id == kind.id())
                {
                    info.binary_path = Some(path);
                }
                match models {
                    Ok(models) => {
                        log::info!("{} lists {} models", kind.label(), models.len());
                        catalog::set_harness_models(kind, models);
                        cx.notify();
                    }
                    Err(err) => log::warn!("{} model catalog: {err:#}", kind.label()),
                }
            });
            if let Err(err) = updated {
                log::debug!("model catalog after app drop: {err:#}");
            }
        })
        .detach();
    }
}
