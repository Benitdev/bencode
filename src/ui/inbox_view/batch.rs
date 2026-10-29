//! BenCode's own: the picked issues (`picks.rs`) sent to agents at once.
//! A confirmation lists them and says how many turns start, on which
//! model; each issue then gets its own thread in its start project,
//! linked to it, its card sent at once as the first message, so the
//! agents run side by side under the navigator's Agents. Issues that
//! already have a thread are skipped unless asked; pull requests are left
//! out (a review is started one at a time, from its detail). A Backlog
//! issue's description goes in its card (the agent cannot open Backlog),
//! read first where the Inbox has not loaded it.

use bencode_ui::buttons::{Button, ButtonVariant};
use bencode_ui::forms::Checkbox;
use bencode_ui::overlays::Dialog;
use bencode_ui::primitives::Icon;
use bencode_ui::theme::{ActiveTheme, IconSize};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::*, relative,
};

use super::blocks::Described;
use super::properties::title_shown;
use super::{StartWith, short_key, status_mark};
use crate::app::{BenCodeApp, TurnInput};
use crate::github::{Kind, WorkItem};
use crate::harness::catalog;
use crate::ui::app_callback::{app_callback, app_callback_with};
use crate::ui::composer::inbox_card::{CardTask, compose_inbox_message};
use crate::ui::scale::px;
use crate::ui::tokens::{Ink, Status, Text, Tone as _};
use crate::work_items::Provider;

/// The confirmation's state, while it is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchSend {
    /// The picked items, in list order.
    pub keys: Vec<String>,
    pub start: StartWith,
    /// Issues that already have a thread get another one too.
    pub include_linked: bool,
    /// Descriptions are being read before the threads start.
    pub starting: bool,
}

/// What a batch send does with one picked item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fate {
    Start,
    /// It has a thread already: skipped unless asked.
    HasThread,
    /// Pull requests are reviewed one at a time.
    PullRequest,
}

impl Fate {
    pub fn of(kind: Kind, linked: bool, include_linked: bool) -> Self {
        match kind {
            Kind::Pr => Self::PullRequest,
            Kind::Issue if linked && !include_linked => Self::HasThread,
            Kind::Issue => Self::Start,
        }
    }

    fn note(self) -> Option<&'static str> {
        match self {
            Self::Start => None,
            Self::HasThread => Some("Has a thread · skipped"),
            Self::PullRequest => Some("Pull request · left out"),
        }
    }
}

/// "2 agent turns" / "1 agent turn".
fn turns(count: usize) -> String {
    match count {
        1 => "1 agent turn".into(),
        n => format!("{n} agent turns"),
    }
}

/// Where and how the threads start, as the confirmation says it.
fn how(start: StartWith) -> String {
    let mut how = "Each issue gets its own thread in its project".to_string();
    if start.worktree {
        how.push_str(", in a new worktree");
    }
    if start.plan {
        how.push_str(", in Plan mode");
    }
    how.push_str(", linked to it, and shows under Agents in the navigator.");
    how
}

impl BenCodeApp {
    /// Send to agents: the confirmation over the picked items.
    pub(super) fn open_inbox_batch(&mut self, start: StartWith, cx: &mut Context<Self>) {
        let keys = self.inbox_picked_keys(cx);
        if keys.is_empty() {
            return;
        }
        self.inbox_state.as_mut(cx).batch = Some(BatchSend {
            keys,
            start,
            include_linked: false,
            starting: false,
        });
        // The dialog is the shell's.
        cx.notify();
    }

    fn close_inbox_batch(&mut self, cx: &mut Context<Self>) {
        self.inbox_state.as_mut(cx).batch = None;
        cx.notify();
    }

    /// The picked items still listed, each with what the send does with it.
    fn batch_fates(&self, batch: &BatchSend, cx: &gpui::App) -> Vec<(WorkItem, Fate)> {
        let inbox = self.inbox(cx);
        (batch.keys.iter())
            .filter_map(|key| inbox.item(key))
            .map(|item| {
                let linked = !self.item_threads(&item.key()).is_empty();
                let fate = Fate::of(item.kind, linked, batch.include_linked);
                (item.clone(), fate)
            })
            .collect()
    }

    /// Confirmed: the Backlog descriptions not yet loaded are read off the
    /// UI thread, then every thread starts.
    fn confirm_inbox_batch(&mut self, cx: &mut Context<Self>) {
        let Some(batch) = self.inbox(cx).batch.clone().filter(|b| !b.starting) else {
            return;
        };
        let items: Vec<WorkItem> = (self.batch_fates(&batch, cx).into_iter())
            .filter(|(_, fate)| *fate == Fate::Start)
            .map(|(item, _)| item)
            .collect();
        if items.is_empty() {
            return;
        }
        let unread: Vec<(String, String)> = (items.iter())
            .filter(|item| item.provider == Provider::Backlog)
            .filter(|item| {
                (self.inbox(cx).details.get(&item.key())).is_none_or(|loaded| loaded.is_err())
            })
            .map(|item| (item.key(), item.identifier.clone()))
            .collect();
        if unread.is_empty() {
            self.start_inbox_batch(&items, batch.start, cx);
            return;
        }
        if let Some(batch) = &mut self.inbox_state.as_mut(cx).batch {
            batch.starting = true;
        }
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            (unread.into_iter())
                .map(|(key, identifier)| (key, crate::backlog::details(&identifier)))
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let read = task.await;
            let updated = this.update(cx, |app, cx| {
                // Cancelled while reading: nothing starts.
                if app.inbox(cx).batch.is_none() {
                    return;
                }
                app.inbox_state.update(cx, |inbox, cx| {
                    for (key, details) in read {
                        match details {
                            Ok(details) => {
                                (inbox.details.value).insert(key, Ok(Described::from(details)));
                            }
                            // The card goes without it, as an issue opened
                            // with no description loaded would.
                            Err(err) => log::warn!("inbox: could not read {key}: {err}"),
                        }
                    }
                    cx.notify();
                });
                app.start_inbox_batch(&items, batch.start, cx);
            });
            if let Err(err) = updated {
                log::debug!("inbox batch after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// A thread per item, linked to it, its card sent at once; the Inbox
    /// stays, its picks let go.
    fn start_inbox_batch(&mut self, items: &[WorkItem], start: StartWith, cx: &mut Context<Self>) {
        for item in items {
            let Some(card) = self.inbox_start_card(item, CardTask::Work, cx) else {
                continue;
            };
            let title = self.inbox_thread_title(item, CardTask::Work, cx);
            let link = crate::app::work_item_link::link_for(item);
            let cwd = self.inbox_start_project(item, cx);
            let id = self.create_session_row(&cwd, cx);
            if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
                session.title = title;
                session.linked_work_item = link.clone();
            }
            self.persist_session(&id);
            if link.is_some() {
                self.set_session_link(&id, link, cx);
            }
            let input = TurnInput {
                // A card sends without a note.
                text: compose_inbox_message(&card, ""),
                plan: start.plan,
                ..TurnInput::default()
            };
            if start.worktree {
                let base = self.new_thread_worktree_base(&cwd);
                self.new_worktrees.insert(id.clone(), base);
                self.send_in_new_worktree(&id, input, String::new(), cx);
            } else {
                self.send_turn(&id, input, cx);
            }
        }
        self.links_changed();
        self.inbox_state.update(cx, |inbox, cx| {
            inbox.batch = None;
            inbox.picks.clear();
            cx.notify();
        });
        cx.notify();
    }

    /// The confirmation, while open: the issues, which start and which do
    /// not and why, the model the turns start on.
    pub fn render_inbox_batch(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let batch = self.inbox(cx).batch.clone()?;
        let fates = self.batch_fates(&batch, cx);
        let count = fates.iter().filter(|(_, f)| *f == Fate::Start).count();
        // Issues with a thread, skipped or included by the checkbox.
        let linked = (fates.iter())
            .filter(|(item, f)| {
                *f != Fate::PullRequest && !self.item_threads(&item.key()).is_empty()
            })
            .count();
        let prs = (fates.iter())
            .filter(|(_, f)| *f == Fate::PullRequest)
            .count();
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let model = &self.selected_model;
        let on = match catalog::find(model) {
            Some(found) => format!("{} · {}", found.harness.label(), catalog::label_for(model)),
            None => catalog::label_for(model),
        };
        let inbox = self.inbox(cx);
        let rows = fates.iter().map(|(item, fate)| {
            let (icon, tint, _) = status_mark(item, cx);
            let key = match item.provider {
                Provider::Backlog => short_key(&item.identifier, &inbox.key_prefix).to_string(),
                Provider::GitHub => item.identifier.clone(),
            };
            div()
                .h(px(26.0))
                .flex()
                .items_center()
                .gap_2()
                .text_size(Text::Base.px())
                .child(Icon::new(icon).size(IconSize::Xs).color(tint))
                .child(
                    div()
                        .flex_none()
                        .text_color(fg.ink(Ink::Secondary))
                        .child(SharedString::from(key)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(match fate {
                            Fate::Start => fg.ink(Ink::Body),
                            _ => fg.ink(Ink::Muted),
                        })
                        .child(title_shown(inbox, item)),
                )
                .children(fate.note().map(|note| {
                    div()
                        .flex_none()
                        .text_size(Text::Sm.px())
                        .text_color(match fate {
                            Fate::HasThread => Status::Attention.color(fg),
                            _ => fg.ink(Ink::Muted),
                        })
                        .child(note)
                }))
        });
        let list = div()
            .id("inbox-batch-items")
            .max_h(px(280.0))
            .overflow_y_scroll()
            .children(rows);
        let note = |text: String| {
            div()
                .text_size(Text::Sm.px())
                .text_color(fg.ink(Ink::Muted))
                .child(text)
        };
        let include = batch.include_linked;
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .text_size(Text::Md.px())
            .line_height(relative(1.5))
            .child(
                div()
                    .text_color(fg.ink(Ink::Secondary))
                    .child(how(batch.start)),
            )
            .child(list)
            .when(linked > 0, |el| {
                el.child(
                    Checkbox::new("inbox-batch-linked", include)
                        .label(match linked {
                            1 => "Also start a thread for the issue that has one".to_string(),
                            n => format!("Also start threads for the {n} issues that have one"),
                        })
                        .on_change(app_callback_with(cx, |this, on: bool, cx| {
                            if let Some(batch) = &mut this.inbox_state.as_mut(cx).batch {
                                batch.include_linked = on;
                            }
                            cx.notify();
                        })),
                )
            })
            .when(prs > 0, |el| {
                el.child(note(
                    "Pull requests are left out: start a review from each one's detail.".into(),
                ))
            });
        let close = app_callback(cx, |this, cx| this.close_inbox_batch(cx));
        let confirm = app_callback(cx, |this, cx| this.confirm_inbox_batch(cx));
        let starting = batch.starting;
        let label = match count {
            1 => "Start 1 agent".to_string(),
            n => format!("Start {n} agents"),
        };
        let issues = match fates.len() {
            1 => "1 issue".to_string(),
            n => format!("{n} issues"),
        };
        let dialog = Dialog::new("inbox-batch", format!("Send {issues} to agents?"), close)
            .detail(format!("{} will start at once on {on}.", turns(count)))
            .child(body)
            .action(|close| {
                Button::new("inbox-batch-cancel", "Cancel")
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, window, cx| close(window, cx))
            })
            .action(move |_| {
                Button::new("inbox-batch-confirm", label)
                    .variant(ButtonVariant::Primary)
                    .disabled(count == 0 || starting)
                    .loading(starting)
                    .on_click(move |_, window, cx| confirm(window, cx))
            });
        Some(dialog.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_skips_linked_issues_and_leaves_pull_requests_out() {
        let picked = [
            (Kind::Issue, false),
            (Kind::Issue, true),
            (Kind::Pr, false),
            (Kind::Pr, true),
        ];
        let plan = |include| -> Vec<Fate> {
            (picked.iter())
                .map(|(kind, linked)| Fate::of(*kind, *linked, include))
                .collect()
        };
        use Fate::*;
        assert_eq!(plan(false), [Start, HasThread, PullRequest, PullRequest]);
        assert_eq!(plan(true), [Start, Start, PullRequest, PullRequest]);
    }

    #[test]
    fn the_confirmation_says_how_the_threads_start() {
        assert_eq!(turns(1), "1 agent turn");
        assert_eq!(turns(3), "3 agent turns");
        assert_eq!(
            how(StartWith::default()),
            "Each issue gets its own thread in its project, linked to it, and shows under Agents in the navigator."
        );
        let both = StartWith {
            worktree: true,
            plan: true,
            ..Default::default()
        };
        assert!(how(both).contains("its project, in a new worktree, in Plan mode, linked"));
    }
}
