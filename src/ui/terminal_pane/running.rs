//! What runs in the dock's terminals (MonoCode `terminal/model/terminalTab.ts`
//! and `terminalClose.ts`): a poll of each terminal's shell that names its
//! tab, the footer chip's list of running jobs, and the "Close anyway?"
//! asked before closing a terminal a job still runs in.

use std::time::Duration;

use ely_gpui_component::overlays::ConfirmDialog;
use gpui::{AnyElement, Context, IntoElement};

use crate::app::BenCodeApp;
use crate::terminal_process::{self, Foreground, Probe};
use crate::ui::app_callback::app_callback;

/// MonoCode reads each terminal's job every second; 800ms also steps the
/// footer's live mark, a quarter of its 3.2s cycle per read.
pub const POLL_EVERY: Duration = Duration::from_millis(800);

/// MonoCode `RunningTerminal`: a terminal whose foreground is not its shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningTerminal {
    pub id: u64,
    pub process: String,
    /// The folder its shell is in.
    pub label: String,
}

/// Terminals to close once "Close anyway?" is answered.
pub(super) struct CloseConfirm {
    project: String,
    ids: Vec<u64>,
    /// Close Others: the terminal that stays, selected.
    keep: Option<u64>,
    message: String,
}

/// MonoCode `defaultTerminalTitle`: the folder's name, `Terminal` at `/`.
pub(super) fn default_title(cwd: &str) -> String {
    match std::path::Path::new(cwd)
        .file_name()
        .and_then(|name| name.to_str())
    {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => "Terminal".to_string(),
    }
}

/// MonoCode `runningTerminalChipLabel`: `vite`, `vite · jest`, `vite ×2`.
pub fn chip_label(terminals: &[RunningTerminal]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for terminal in terminals {
        match counts
            .iter_mut()
            .find(|(name, _)| *name == terminal.process)
        {
            Some((_, n)) => *n += 1,
            None => counts.push((&terminal.process, 1)),
        }
    }
    counts
        .into_iter()
        .map(|(name, n)| {
            if n > 1 {
                format!("{name} ×{n}")
            } else {
                name.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// MonoCode `confirmCloseTerminal(s)`'s question.
fn close_message(running: &[RunningTerminal]) -> String {
    match running {
        [one] => format!(
            "\"{}\" is still running in {}. Close this terminal anyway?",
            one.process, one.label
        ),
        many => {
            let lines = many
                .iter()
                .map(|t| format!("• {} ({})", t.label, t.process))
                .collect::<Vec<_>>()
                .join("\n");
            format!("These terminals are still running:\n{lines}\n\nClose them anyway?")
        }
    }
}

/// MonoCode's `[process exited (code)]`, without the code when a signal
/// ended it.
pub(super) fn exited_line(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("[process exited ({code})]"),
        None => "[process exited]".to_string(),
    }
}

impl BenCodeApp {
    /// MonoCode `listRunningTerminals` for the current project's dock.
    pub fn running_terminals(&self) -> Vec<RunningTerminal> {
        let Some(dock) = self.terminals.dock(&self.current_cwd) else {
            return Vec::new();
        };
        dock.tabs
            .iter()
            .filter_map(|tab| {
                let job = tab.foreground.as_ref()?;
                Some(RunningTerminal {
                    id: tab.id,
                    process: job.process.clone(),
                    label: tab.folder(),
                })
            })
            .collect()
    }

    /// Reads every live terminal's shell each `POLL_EVERY` until none is
    /// left; started by each new terminal.
    pub(super) fn start_terminal_poll(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.terminals.polling, true) {
            return;
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_EVERY).await;
                let jobs = this.update(cx, |this, _| this.terminal_probe_jobs());
                let jobs = match jobs {
                    Ok(Some(jobs)) => jobs,
                    Ok(None) => return,
                    Err(err) => {
                        log::debug!("terminal poll after app drop: {err:#}");
                        return;
                    }
                };
                let probes = cx
                    .background_executor()
                    .spawn(async move {
                        jobs.into_iter()
                            .map(|(id, root, shell, last)| {
                                (id, terminal_process::probe(root, shell, last.as_ref()))
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                if let Err(err) = this.update(cx, |this, cx| this.apply_terminal_probes(probes, cx))
                {
                    log::debug!("terminal probes after app drop: {err:#}");
                    return;
                }
            }
        })
        .detach();
    }

    /// What the next poll reads; `None` stops it, no shell being left.
    fn terminal_probe_jobs(&mut self) -> Option<Vec<(u64, u32, Option<u32>, Option<Foreground>)>> {
        let jobs: Vec<_> = self
            .terminals
            .docks
            .values()
            .flat_map(|dock| &dock.tabs)
            .filter_map(|tab| Some((tab.id, tab.process?, tab.shell, tab.foreground.clone())))
            .collect();
        if jobs.is_empty() {
            self.terminals.polling = false;
            return None;
        }
        Some(jobs)
    }

    fn apply_terminal_probes(&mut self, probes: Vec<(u64, Probe)>, cx: &mut Context<Self>) {
        let mut changed = false;
        for (id, probe) in probes {
            let Some(tab) = self
                .terminals
                .docks
                .values_mut()
                .flat_map(|dock| &mut dock.tabs)
                .find(|tab| tab.id == id)
            else {
                continue;
            };
            // The shell ended while this was read.
            if tab.process.is_none() {
                continue;
            }
            let dir = probe.cwd.or_else(|| tab.dir.clone());
            if tab.shell != probe.shell || tab.foreground != probe.foreground || tab.dir != dir {
                tab.shell = probe.shell;
                tab.foreground = probe.foreground;
                tab.dir = dir;
                changed = true;
            }
        }
        // The footer's live mark steps with each read while it shows.
        let live_mark = !cx.reduce_motion() && !self.running_terminals().is_empty();
        if changed || live_mark {
            cx.notify();
        }
    }

    /// The shell of `id` ended: its tab stays with MonoCode's exit line.
    pub(super) fn terminal_exited(&mut self, project: &str, id: u64, cx: &mut Context<Self>) {
        if let Some(tab) = self
            .terminals
            .docks
            .get_mut(project)
            .and_then(|dock| dock.tabs.iter_mut().find(|tab| tab.id == id))
        {
            tab.process = None;
            tab.shell = None;
            tab.foreground = None;
        }
        cx.notify();
    }

    /// MonoCode `onCloseProjectTerminal` / `onCloseOtherProjectTerminals`:
    /// closes `ids`, asking first when a job runs in any of them. The job
    /// is read again now, not taken from the last poll.
    pub(super) fn request_close_terminals(
        &mut self,
        project: &str,
        ids: Vec<u64>,
        keep: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if ids.is_empty() {
            return;
        }
        let running: Vec<RunningTerminal> = self
            .terminals
            .dock(project)
            .map(|dock| {
                dock.tabs
                    .iter()
                    .filter(|tab| ids.contains(&tab.id) && tab.process.is_some())
                    .filter_map(|tab| {
                        let job =
                            terminal_process::foreground_now(tab.shell?, tab.foreground.as_ref())?;
                        Some(RunningTerminal {
                            id: tab.id,
                            process: job.process,
                            label: tab.folder(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if running.is_empty() {
            self.finish_close_terminals(project, &ids, keep, cx);
            return;
        }
        self.terminals.close_confirm = Some(CloseConfirm {
            project: project.to_string(),
            message: close_message(&running),
            ids,
            keep,
        });
        cx.notify();
    }

    fn finish_close_terminals(
        &mut self,
        project: &str,
        ids: &[u64],
        keep: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        match keep {
            Some(keep) => self.close_terminals_but(project, ids, keep, cx),
            None => {
                for &id in ids {
                    self.close_terminal(project, id, cx);
                }
            }
        }
    }

    /// MonoCode `onToggleRunningTerminal`: hides the dock when it shows,
    /// else shows it on terminal `id` with the keyboard.
    pub(crate) fn toggle_running_terminal(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.is_terminal_open() {
            self.set_terminal_open(false, cx);
            self.refocus_prompt(cx);
            return;
        }
        let Some(dock) = self.terminals.docks.get_mut(&self.current_cwd) else {
            return;
        };
        let Some(tab) = dock.tabs.iter().find(|tab| tab.id == id) else {
            return;
        };
        let focus = gpui::Focusable::focus_handle(tab.entity.read(cx), cx);
        dock.active = id;
        self.set_terminal_open(true, cx);
        crate::ui::composer::focus_later(focus, cx);
    }

    pub(crate) fn render_terminal_close_confirm(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let confirm = self.terminals.close_confirm.as_ref()?;
        let cancel = app_callback(cx, |this, cx| {
            this.terminals.close_confirm = None;
            cx.notify();
        });
        let close = app_callback(cx, |this, cx| {
            if let Some(confirm) = this.terminals.close_confirm.take() {
                this.finish_close_terminals(&confirm.project, &confirm.ids, confirm.keep, cx);
            }
            cx.notify();
        });
        Some(
            ConfirmDialog::new(
                "terminal-close-confirm",
                "BenCode",
                confirm.message.clone(),
                cancel,
            )
            .confirm("Close")
            .destructive()
            .on_confirm(close)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(id: u64, process: &str, label: &str) -> RunningTerminal {
        RunningTerminal {
            id,
            process: process.into(),
            label: label.into(),
        }
    }

    #[test]
    fn chip_label_counts_repeats_in_first_seen_order() {
        assert_eq!(chip_label(&[running(1, "vite", "web")]), "vite");
        assert_eq!(
            chip_label(&[running(1, "vite", "web"), running(2, "jest", "web")]),
            "vite · jest"
        );
        assert_eq!(
            chip_label(&[
                running(1, "vite", "a"),
                running(2, "jest", "b"),
                running(3, "vite", "c")
            ]),
            "vite ×2 · jest"
        );
        assert_eq!(chip_label(&[]), "");
    }

    #[test]
    fn close_message_names_one_or_lists_many() {
        assert_eq!(
            close_message(&[running(1, "vite", "web")]),
            "\"vite\" is still running in web. Close this terminal anyway?"
        );
        assert_eq!(
            close_message(&[running(1, "vite", "web"), running(2, "cargo", "api")]),
            "These terminals are still running:\n• web (vite)\n• api (cargo)\n\nClose them anyway?"
        );
    }

    #[test]
    fn titles_and_exit_lines() {
        assert_eq!(default_title("/Users/me/project"), "project");
        assert_eq!(default_title("/"), "Terminal");
        assert_eq!(default_title(""), "Terminal");
        assert_eq!(exited_line(Some(0)), "[process exited (0)]");
        assert_eq!(exited_line(None), "[process exited]");
    }
}
