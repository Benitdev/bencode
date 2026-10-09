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
