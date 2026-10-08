//! MonoCode `app/model/updater.ts` and `updateNotice.ts`: the update state
//! the rail, Settings and the menu share. One probe at launch (MonoCode's
//! `SidebarUpdateFooter` mount), the manual flow behind "Check for
//! Updates…" with its dialogs, the install and the restart, and the
//! "Updated to" notice the next launch shows. The work is `crate::updater`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, PromptLevel, Window};

use super::BenCodeApp;
use crate::updater::{self, Check, NotConfigured, Progress, Update};

/// MonoCode `UpdaterPhase`, with `Ready` for an update installed while
/// chats were running and the restart was put off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Ready,
    Error,
}

#[derive(Default)]
pub struct UpdaterState {
    pub phase: Phase,
    /// The newer release, from `Available` on.
    pub available: Option<Update>,
    progress: Option<Arc<Progress>>,
    pub error: Option<String>,
    /// The bundle to reopen once BenCode quits (`Ready`).
    ready: Option<PathBuf>,
    /// The quit confirmation is the restart's: it says Restart.
    pub restart_on_quit: bool,
    /// MonoCode `updateNotice`: "Updated to" until dismissed.
    pub installed: Option<String>,
    /// The version whose notes the What's new dialog shows.
    pub whats_new: Option<String>,
}

impl UpdaterState {
    pub fn current_version(&self) -> &'static str {
        updater::current_version()
    }

    pub fn available_version(&self) -> Option<&str> {
        self.available
            .as_ref()
            .map(|update| update.version.as_str())
    }

    /// Whole percent while downloading, once the size is known.
    pub fn progress(&self) -> Option<u8> {
        self.progress
            .as_ref()
            .and_then(|progress| progress.percent())
    }

    /// MonoCode `isSidebarUpdateActionable` (plus a restart put off).
    pub fn actionable(&self) -> bool {
        matches!(
            self.phase,
            Phase::Available | Phase::Downloading | Phase::Ready
        )
    }

    /// The Settings row's line (MonoCode `UpdateRow`'s `status`).
    pub fn status_line(&self) -> String {
        match self.phase {
            Phase::Available => format!(
                "Version {} is available.",
                self.available_version().unwrap_or("?")
            ),
            Phase::Downloading => match self.progress() {
                Some(percent) => format!("Downloading {percent}%"),
                None => "Downloading…".to_string(),
            },
            Phase::Ready => format!(
                "Version {} is installed. Restart BenCode to use it.",
                self.available_version().unwrap_or("?")
            ),
            Phase::Checking => "Checking for updates…".to_string(),
            Phase::Current => "You're on the latest version.".to_string(),
            Phase::Error => self
                .error
                .clone()
                .unwrap_or_else(|| "Update check failed.".to_string()),
            Phase::Idle => match updater::configured() {
                Ok(_) => "BenCode updates itself from the release feed.".to_string(),
                Err(_) => "This build does not update itself.".to_string(),
            },
        }
    }
}

/// MonoCode's message for a build without an updater endpoint.
fn not_configured_detail(reason: NotConfigured) -> String {
    let why = match reason {
        NotConfigured::NoKey => "Automatic updates aren't configured for this build.",
        NotConfigured::NoBundle => "Automatic updates work only in the installed BenCode app.",
    };
    format!("{why}\n\nDownload releases at {}", updater::RELEASES_URL)
}

impl BenCodeApp {
    /// At launch: the "Updated to" note a restart left, then one silent
    /// check (MonoCode `SidebarUpdateFooter`'s probe).
    pub(crate) fn start_update_probe(&mut self, cx: &mut Context<Self>) {
        let configured = updater::configured().is_ok();
        let task = cx.background_executor().spawn(async move {
            let installed = updater::take_installed();
            let check = configured.then(updater::check);
            (installed, check)
        });
        if configured {
            self.updater.phase = Phase::Checking;
        }
        cx.spawn(async move |this, cx| {
            let (installed, check) = task.await;
            let landed = this.update(cx, |this, cx| {
                this.updater.installed = installed;
                match check {
                    Some(Ok(check)) => this.land_check(check),
                    Some(Err(err)) => {
                        // MonoCode: a failed probe stays silent.
                        log::warn!("update check failed: {err:#}");
                        this.updater.phase = Phase::Idle;
                    }
                    None => {}
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("update probe after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn land_check(&mut self, check: Check) {
        match check {
            Check::Current => {
                self.updater.phase = Phase::Current;
                self.updater.available = None;
            }
            Check::Available(update) => {
                self.updater.phase = Phase::Available;
                self.updater.available = Some(update);
            }
        }
        self.updater.error = None;
    }

    /// MonoCode `runUpdateFlow(true)`: BenCode › Check for Updates… and
    /// Settings' button. Says what it found, and offers to install.
    pub fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.updater.phase {
            Phase::Checking | Phase::Downloading => return,
            Phase::Ready => return self.restart_for_update(cx),
            _ => {}
        }
        if let Err(reason) = updater::configured() {
            self.updater.phase = Phase::Idle;
            cx.notify();
            drop(window.prompt(
                PromptLevel::Info,
                "BenCode",
                Some(&not_configured_detail(reason)),
                &["OK"],
                cx,
            ));
            return;
        }
        self.updater.phase = Phase::Checking;
        cx.notify();
        let task = cx.background_executor().spawn(async { updater::check() });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let landed = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(check) => {
                        this.land_check(check);
                        this.offer_update(window, cx);
                    }
                    Err(err) => {
                        let message = format!("{err:#}");
                        this.updater.phase = Phase::Error;
                        this.updater.error = Some(message.clone());
                        drop(window.prompt(
                            PromptLevel::Warning,
                            "Couldn't check for updates.",
                            Some(&message),
                            &["OK"],
                            cx,
                        ));
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("update check after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The manual check's answer: up to date, or "Install now?".
    fn offer_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(update) = self.updater.available.clone() else {
            drop(window.prompt(
                PromptLevel::Info,
                "You're on the latest version.",
                None,
                &["OK"],
                cx,
            ));
            return;
        };
        let notes = update
            .notes
            .as_deref()
            .map(|notes| format!("\n\n{notes}"))
            .unwrap_or_default();
        let detail = format!(
            "BenCode {} is available (you have {}).{notes}\n\nInstall now?",
            update.version,
            updater::current_version()
        );
        let answer = window.prompt(
            PromptLevel::Info,
            "Update available",
            Some(&detail),
            &["Install", "Not Now"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if !matches!(answer.await, Ok(0)) {
                return;
            }
            if let Err(err) = this.update(cx, |this, cx| this.install_update(cx)) {
                log::debug!("update install after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `installPendingUpdate`: download, check, put in place,
    /// restart. The rail button and Settings' Download run it.
    pub fn install_update(&mut self, cx: &mut Context<Self>) {
        if self.updater.phase == Phase::Ready {
            return self.restart_for_update(cx);
        }
        if self.updater.phase == Phase::Downloading {
            return;
        }
        let Some(update) = self.updater.available.clone() else {
            return;
        };
        let bundle = match updater::configured() {
            Ok(bundle) => bundle,
            Err(reason) => {
                self.fail_update(not_configured_detail(reason), cx);
                return;
            }
        };
        let progress = Arc::new(Progress::default());
        self.updater.progress = Some(progress.clone());
        self.updater.phase = Phase::Downloading;
        self.updater.error = None;
        cx.notify();

        let task = {
            let (update, bundle, progress) = (update.clone(), bundle.clone(), progress.clone());
            cx.background_executor()
                .spawn(async move { updater::install(&update, &bundle, &progress) })
        };
        // Redraws the percentage while the download runs.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let downloading = this.update(cx, |this, cx| {
                    cx.notify();
                    this.updater.phase == Phase::Downloading
                });
                if !matches!(downloading, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |this, cx| match result {
                Ok(()) => {
                    updater::remember_installed(&update.version);
                    this.updater.progress = None;
                    this.updater.ready = Some(bundle);
                    this.updater.phase = Phase::Ready;
                    cx.notify();
                    this.restart_for_update(cx);
                }
                Err(err) => {
                    log::error!("update install failed: {err:#}");
                    this.fail_update(format!("{err:#}"), cx);
                }
            });
            if let Err(err) = landed {
                log::debug!("update result after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode's "Couldn't install the update." message, and the error on
    /// the Settings row.
    fn fail_update(&mut self, message: String, cx: &mut Context<Self>) {
        self.updater.progress = None;
        self.updater.phase = Phase::Error;
        self.updater.error = Some(message.clone());
        cx.notify();
        // Next turn: a click that got here may still hold the window.
        cx.spawn(async move |_, cx| {
            let shown = cx.update(|cx| {
                let window = cx.windows().into_iter().next()?;
                Some(window.update(cx, |_, window, cx| {
                    drop(window.prompt(
                        PromptLevel::Warning,
                        "Couldn't install the update.",
                        Some(&message),
                        &["OK"],
                        cx,
                    ));
                }))
            });
            if let Some(Err(err)) = shown {
                log::error!("could not show the update failure: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `relaunch()`. With chats running, the quit confirmation
    /// asks first; turning it down keeps "Restart to update" on the rail.
    pub fn restart_for_update(&mut self, cx: &mut Context<Self>) {
        if self.updater.ready.is_none() {
            return;
        }
        if self.runs.is_empty() {
            self.relaunch_now(cx);
        } else {
            self.updater.restart_on_quit = true;
            self.quit_confirm_open = true;
            cx.notify();
        }
    }

    /// The quit confirmation's Restart, or a restart with nothing running.
    pub(crate) fn relaunch_now(&mut self, cx: &mut Context<Self>) {
        let Some(bundle) = self.updater.ready.clone() else {
            return cx.quit();
        };
        match updater::relaunch(&bundle) {
            Ok(()) => cx.quit(),
            Err(err) => {
                log::error!("could not restart BenCode: {err:#}");
                self.fail_update(format!("{err:#}"), cx);
            }
        }
    }

    /// MonoCode `onDismissUpdate`.
    pub fn dismiss_installed_update(&mut self, cx: &mut Context<Self>) {
        self.updater.installed = None;
        cx.notify();
    }

    /// MonoCode `onOpenWhatsNew`.
    pub fn open_whats_new(&mut self, version: String, cx: &mut Context<Self>) {
        self.updater.whats_new = Some(version);
        cx.notify();
    }

    pub fn close_whats_new(&mut self, cx: &mut Context<Self>) {
        self.updater.whats_new = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_update_to_act_on_shows_on_the_rail() {
        let mut state = UpdaterState::default();
        for phase in [Phase::Idle, Phase::Checking, Phase::Current, Phase::Error] {
            state.phase = phase;
            assert!(!state.actionable(), "{phase:?}");
        }
        for phase in [Phase::Available, Phase::Downloading, Phase::Ready] {
            state.phase = phase;
            assert!(state.actionable(), "{phase:?}");
        }
    }

    #[test]
    fn the_status_line_follows_the_phase() {
        let mut state = UpdaterState {
            phase: Phase::Checking,
            ..Default::default()
        };
        assert_eq!(state.status_line(), "Checking for updates…");
        state.phase = Phase::Current;
        assert_eq!(state.status_line(), "You're on the latest version.");
        state.phase = Phase::Downloading;
        assert_eq!(state.status_line(), "Downloading…");
        state.phase = Phase::Error;
        assert_eq!(state.status_line(), "Update check failed.");
        state.error = Some("offline".to_string());
        assert_eq!(state.status_line(), "offline");
    }

    #[test]
    fn a_build_without_a_key_says_where_releases_are() {
        let detail = not_configured_detail(NotConfigured::NoKey);
        assert!(detail.starts_with("Automatic updates aren't configured for this build."));
        assert!(detail.ends_with(updater::RELEASES_URL));
    }
}
