//! A GitHub issue or pull request's editable properties (`properties.rs`):
//! assignees (with Assign to me) and labels on any item, plus requested
//! reviewers, completed reviewers (with one-click re-request) and review
//! submission (Approve / Request changes / Comment) on pull requests.
//!
//! Multi-select pickers batch toggles while open (`commit_on_close`) and
//! apply changes optimistically with a per-field busy mark, rolling back
//! cleanly if `gh` rejects the edit.

use std::path::Path;

use bencode_ui::buttons::{Button, ButtonVariant, IconButton, SegmentedControl};
use bencode_ui::data_display::Avatar;
use bencode_ui::forms::{Choice, MultiSelect, Select};
use bencode_ui::menus::{DropdownMenu, Menu, MenuItem};
use bencode_ui::overlays::Dialog;
use bencode_ui::primitives::{Icon, IconName, Tooltip};
use bencode_ui::theme::{ActiveTheme, AvatarSize, ControlSize, IconSize, Radius};
use gpui::{
    AnyElement, Context, Focusable, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::*, transparent_black,
};

use super::blocks::Described;
use super::comments::review_tint;
use super::properties::{ROW_HEIGHT, none, plain, property, property_with};
use super::{DetailView, InboxState, loading_icon};
use crate::app::BenCodeApp;
use crate::github::{self, ItemChange, Kind, Label, RepoOptions, ReviewAction, Thread, WorkItem};
use crate::ui::app_callback::{app_callback, on_value};
use crate::ui::composer::inbox_card::{label_chip_within, label_color};
use crate::ui::scale::px;
use crate::ui::tokens::{Fill, Ink, Text, Tone as _};
use crate::work_items::Provider;

/// The controls' size: their text matches the properties' body size.
const SIZE: ControlSize = ControlSize::Md;
/// An empty field's placeholder.
const EMPTY: &str = "—";

/// Which GitHub property is currently saving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GithubEditField {
    Assignees,
    Labels,
    Reviewers,
}

impl GithubEditField {
    fn from_change(change: &ItemChange) -> Self {
        match change {
            ItemChange::Assignees { .. } => Self::Assignees,
            ItemChange::Labels { .. } => Self::Labels,
            ItemChange::Reviewers { .. } => Self::Reviewers,
        }
    }
}

/// An in-flight GitHub metadata edit, for per-field busy marks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingGithubEdit {
    pub key: String,
    pub field: GithubEditField,
}

/// Snapshot kept before an optimistic edit so a failed `gh` call rolls back.
enum Rollback {
    Assignees(Vec<String>),
    Labels(Vec<Label>),
    Reviewers(Vec<String>),
}

/// The pull request review dialog, while open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrReviewDialog {
    pub key: String,
    pub action: ReviewAction,
    pub submitting: bool,
    pub error: Option<String>,
}

/// A reviewer who already submitted a review on a pull request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewVerdict {
    Approved,
    ChangesRequested,
    Commented,
}

impl ReviewVerdict {
    fn label(self) -> &'static str {
        match self {
            Self::Approved => "Approved",
            Self::ChangesRequested => "Changes requested",
            Self::Commented => "Commented",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Approved => IconName::CircleCheck,
            Self::ChangesRequested => IconName::CircleAlert,
            Self::Commented => IconName::MessageSquare,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SettledReviewer {
    login: String,
    verdict: ReviewVerdict,
}

/// Latest review verdict per non-author reviewer from `thread`, omitting
/// anyone currently in `pending` (`reviewRequests`) or dismissed.
fn settled_reviews(thread: &Thread, author: &str, pending: &[String]) -> Vec<SettledReviewer> {
    let mut states: Vec<(String, Option<ReviewVerdict>)> = Vec::new();
    for comment in &thread.comments {
        if comment.kind != "review" {
            continue;
        }
        let login = comment.author.trim();
        if login.is_empty() || (!author.is_empty() && login.eq_ignore_ascii_case(author)) {
            continue;
        }
        let next = match comment.state.as_str() {
            "APPROVED" => Some(Some(ReviewVerdict::Approved)),
            "CHANGES_REQUESTED" => Some(Some(ReviewVerdict::ChangesRequested)),
            "DISMISSED" => Some(None),
            "COMMENTED" => None,
            _ => continue,
        };
        if let Some((_, slot)) = states
            .iter_mut()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(login))
        {
            match next {
                Some(decisive) => *slot = decisive,
                // A follow-up comment does not clear an earlier approval or change request.
                None if slot.is_none() => *slot = Some(ReviewVerdict::Commented),
                None => {}
            }
        } else {
            let initial = match next {
                Some(decisive) => decisive,
                None => Some(ReviewVerdict::Commented),
            };
            states.push((login.to_string(), initial));
        }
    }
    states
        .into_iter()
        .filter(|(login, _)| !pending.iter().any(|p| p.eq_ignore_ascii_case(login)))
        .filter_map(|(login, verdict)| verdict.map(|verdict| SettledReviewer { login, verdict }))
        .collect()
}

/// Applies `add`/`remove` case-insensitively to a list of strings.
fn apply_names(current: &[String], add: &[String], remove: &[String]) -> Vec<String> {
    let mut next: Vec<String> = current
        .iter()
        .filter(|val| !remove.iter().any(|rem| rem.eq_ignore_ascii_case(val)))
        .cloned()
        .collect();
    for val in add {
        if !val.trim().is_empty() && !next.iter().any(|cur| cur.eq_ignore_ascii_case(val)) {
            next.push(val.clone());
        }
    }
    next
}

/// Applies `add`/`remove` to an item's labels, pulling colors from `options`.
fn apply_labels(
    current: &[Label],
    add: &[String],
    remove: &[String],
    options: Option<&RepoOptions>,
) -> Vec<Label> {
    let mut next: Vec<Label> = current
        .iter()
        .filter(|l| !remove.iter().any(|rem| rem.eq_ignore_ascii_case(&l.name)))
        .cloned()
        .collect();
    for name in add {
        if name.trim().is_empty() || next.iter().any(|l| l.name.eq_ignore_ascii_case(name)) {
            continue;
        }
        let color = options
            .and_then(|o| o.labels.iter().find(|l| l.name.eq_ignore_ascii_case(name)))
            .map(|l| l.color.clone())
            .unwrap_or_default();
        next.push(Label {
            name: name.clone(),
            color,
        });
    }
    next
}

/// Collaborator choices from `options`, plus any currently selected logins
/// absent from the first page so they remain visible and removable.
fn user_choices(options: &RepoOptions, current: &[String], exclude: Option<&str>) -> Vec<Choice> {
    let excluded =
        |login: &str| exclude.is_some_and(|ex| !ex.is_empty() && ex.eq_ignore_ascii_case(login));
    let mut choices: Vec<Choice> = options
        .users
        .iter()
        .filter(|user| !excluded(&user.login))
        .map(|user| {
            let mut choice =
                Choice::new(user.login.clone(), user.login.clone()).avatar(user.login.clone());
            if !user.name.is_empty() && !user.name.eq_ignore_ascii_case(&user.login) {
                choice = choice.note(user.name.clone());
            }
            choice
        })
        .collect();
    for login in current {
        if !login.is_empty()
            && !choices
                .iter()
                .any(|existing| existing.value.eq_ignore_ascii_case(login))
        {
            choices.push(Choice::new(login.clone(), login.clone()).avatar(login.clone()));
        }
    }
    choices
}

/// Label choices from `options`, plus any labels already on the item that were
/// outside the first page, each tinted with its repository color.
fn label_choices(options: &RepoOptions, current: &[Label]) -> Vec<Choice> {
    let make = |label: &Label| {
        let mut choice = Choice::new(label.name.clone(), label.name.clone());
        if let Some(color) = label_color(&label.color) {
            choice = choice.color(color);
        }
        choice
    };
    let mut choices: Vec<Choice> = options.labels.iter().map(make).collect();
    for label in current {
        if !label.name.is_empty()
            && !choices
                .iter()
                .any(|existing| existing.value.eq_ignore_ascii_case(&label.name))
        {
            choices.push(make(label));
        }
    }
    choices
}

/// Selected values aligned to each matching choice's exact value string
/// (GitHub handles logins and labels case-insensitively).
fn selected_values(choices: &[Choice], current: &[String]) -> Vec<SharedString> {
    current
        .iter()
        .filter_map(|val| {
            choices
                .iter()
                .find(|c| c.value.eq_ignore_ascii_case(val))
                .map(|c| c.value.clone())
        })
        .collect()
}

/// A closed read-only placeholder select shown while options load or when a
/// repository has no choices for a field.
fn closed_select(id: &'static str, text: String) -> AnyElement {
    Select::new(id, Vec::<Choice>::new())
        .quiet()
        .size(SIZE)
        .disabled(true)
        .placeholder(text)
        .into_any_element()
}

fn repo_key(repo: &str) -> String {
    repo.to_lowercase()
}

fn review_decision_label(decision: &str) -> &'static str {
    match decision {
        "APPROVED" => "Approved",
        "CHANGES_REQUESTED" => "Changes requested",
        "REVIEW_REQUIRED" => "Review required",
        _ => "",
    }
}

fn is_own_pr(inbox: &InboxState, key: &str, repo: &str) -> bool {
    let author = inbox
        .details
        .get(key)
        .and_then(|d| d.as_ref().ok())
        .map(|d| d.details.author.as_str())
        .unwrap_or("");
    let me = inbox
        .github_options
        .get(&repo_key(repo))
        .and_then(|opt| opt.as_ref().ok())
        .map(|opt| opt.me.as_str())
        .unwrap_or("");
    !author.is_empty() && !me.is_empty() && author.eq_ignore_ascii_case(me)
}

impl BenCodeApp {
    /// Lists assignable users, labels and the signed-in viewer for `repo`,
    /// once per session (cleared on explicit refresh).
    pub(super) fn load_github_repo_options(
        &mut self,
        project: &str,
        repo: &str,
        cx: &mut Context<Self>,
    ) {
        if repo.is_empty() {
            return;
        }
        let key = repo_key(repo);
        let ready = matches!(self.inbox(cx).github_options.get(&key), Some(Ok(_)));
        if ready || self.inbox(cx).github_options.is_loading(&key) {
            return;
        }
        let generation = self.inbox(cx).generation;
        self.inbox_state.update(cx, |inbox, cx| {
            inbox.github_options.loading.insert(key.clone());
            cx.notify();
        });
        let project = project.to_string();
        let repo_name = repo.to_string();
        let task = cx
            .background_executor()
            .spawn(async move { github::repo_options(Path::new(&project), &repo_name) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            if let Err(err) = &result {
                log::warn!("github: no repo options for {key}: {err}");
            }
            let landed = this.update(cx, |app, cx| {
                app.inbox_state.update(cx, |inbox, cx| {
                    inbox.github_options.loading.remove(&key);
                    if inbox.generation == generation {
                        inbox.github_options.value.insert(key, result);
                        cx.notify();
                    }
                });
            });
            if let Err(err) = landed {
                log::debug!("github repo options after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Clears a failed repo options load and tries again.
    fn retry_github_repo_options(&mut self, project: &str, repo: &str, cx: &mut Context<Self>) {
        let key = repo_key(repo);
        self.inbox_mut(cx, |inbox| {
            inbox.github_options.value.remove(&key);
        });
        self.load_github_repo_options(project, repo, cx);
    }

    /// Applies `change` optimistically to GitHub item `key`, runs `gh`, then
    /// reconciles with the authoritative response (or rolls back on error).
    pub(super) fn update_github_item(
        &mut self,
        key: &str,
        change: ItemChange,
        cx: &mut Context<Self>,
    ) {
        if change.is_empty() || self.inbox(cx).github_edit_busy.is_some() {
            return;
        }
        let Some(item) = self
            .inbox(cx)
            .item(key)
            .filter(|item| item.provider == Provider::GitHub)
            .cloned()
        else {
            return;
        };
        let key = key.to_string();
        let field = GithubEditField::from_change(&change);
        let rollback = self.inbox_state.update(cx, |inbox, cx| {
            let options = inbox
                .github_options
                .get(&repo_key(&item.repo))
                .and_then(|opt| opt.as_ref().ok())
                .cloned();
            let rollback = match &change {
                ItemChange::Assignees { add, remove } => {
                    let prev = item.assignees.clone();
                    if let Some(slot) = inbox.item_mut(&key) {
                        slot.assignees = apply_names(&slot.assignees, add, remove);
                    }
                    Rollback::Assignees(prev)
                }
                ItemChange::Labels { add, remove } => {
                    let prev = item.labels.clone();
                    if let Some(slot) = inbox.item_mut(&key) {
                        slot.labels = apply_labels(&slot.labels, add, remove, options.as_ref());
                    }
                    Rollback::Labels(prev)
                }
                ItemChange::Reviewers { add, remove } => {
                    let prev = inbox
                        .details
                        .value
                        .get(&key)
                        .and_then(|d| d.as_ref().ok())
                        .map(|d| d.details.reviewers.clone())
                        .unwrap_or_default();
                    if let Some(Ok(shown)) = inbox.details.value.get_mut(&key) {
                        shown.details.reviewers =
                            apply_names(&shown.details.reviewers, add, remove);
                    }
                    Rollback::Reviewers(prev)
                }
            };
            inbox.github_edit_busy = Some(PendingGithubEdit {
                key: key.clone(),
                field,
            });
            inbox.github_edit_error = None;
            cx.notify();
            rollback
        });
        let project = item.project.clone();
        let repo = item.repo.clone();
        let kind = item.kind;
        let number = item.number;
        let task = cx.background_executor().spawn(async move {
            github::edit_item(Path::new(&project), &repo, kind, number, &change).map(|read| {
                read.map(|(mut next, details)| {
                    next.project = project;
                    (next, details)
                })
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                app.inbox_state.update(cx, |inbox, _| {
                    inbox.github_edit_busy = None;
                });
                match result {
                    Ok(read) => app.land_github_read_back(&key, read, cx),
                    Err(err) => {
                        log::warn!("could not edit GitHub item {key}: {err}");
                        app.inbox_state.update(cx, |inbox, _| {
                            match rollback {
                                Rollback::Assignees(prev) => {
                                    if let Some(slot) = inbox.item_mut(&key) {
                                        slot.assignees = prev;
                                    }
                                }
                                Rollback::Labels(prev) => {
                                    if let Some(slot) = inbox.item_mut(&key) {
                                        slot.labels = prev;
                                    }
                                }
                                Rollback::Reviewers(prev) => {
                                    if let Some(Ok(shown)) = inbox.details.value.get_mut(&key) {
                                        shown.details.reviewers = prev;
                                    }
                                }
                            }
                            inbox.github_edit_error = Some((key.clone(), err));
                        });
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("github edit after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// What a change that went through reads back replaces what was shown;
    /// when it cannot be read, the change stays shown as made and the item
    /// loads again.
    fn land_github_read_back(&mut self, key: &str, read: github::ReadBack, cx: &mut Context<Self>) {
        match read {
            Ok((next, details)) => {
                self.inbox_state.update(cx, |inbox, _| {
                    if let Some(slot) = inbox.item_mut(key) {
                        *slot = next;
                    }
                    inbox
                        .details
                        .value
                        .insert(key.to_string(), Ok(Described::from(details)));
                });
                self.apply_inbox_translations(key, cx);
            }
            Err(err) => {
                log::warn!("GitHub item {key} changed but could not be read back: {err}");
                self.inbox_mut(cx, |inbox| {
                    inbox.details.value.remove(key);
                });
                self.load_inbox_item(key, cx);
            }
        }
        // The user's own change is not news to them.
        self.mark_inbox_key_seen(key, cx);
    }

    /// The signed-in account added to item `key`'s assignees: the button by
    /// Assignees, and `a` in the list.
    pub(super) fn assign_github_item_to_me(&mut self, key: &str, cx: &mut Context<Self>) {
        let inbox = self.inbox(cx);
        let Some(item) = inbox.item(key) else {
            return;
        };
        let me = (inbox.github_options.get(&repo_key(&item.repo)))
            .and_then(|options| options.as_ref().ok())
            .map(|options| options.me.clone())
            .unwrap_or_default();
        if me.is_empty() || item.assignees.iter().any(|a| a.eq_ignore_ascii_case(&me)) {
            return;
        }
        let change = ItemChange::Assignees {
            add: vec![me],
            remove: Vec::new(),
        };
        self.update_github_item(key, change, cx);
    }

    /// Opens the pull request review dialog for `key` with `action` selected.
    pub(super) fn open_pr_review_dialog(
        &mut self,
        key: &str,
        action: ReviewAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pr_review_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.inbox_state.as_mut(cx).pr_review = Some(PrReviewDialog {
            key: key.to_string(),
            action,
            submitting: false,
            error: None,
        });
        let focus = self.pr_review_input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        // The dialog sits in the shell.
        cx.notify();
    }

    fn close_pr_review_dialog(&mut self, cx: &mut Context<Self>) {
        if self
            .inbox(cx)
            .pr_review
            .as_ref()
            .is_some_and(|d| d.submitting)
        {
            return;
        }
        self.inbox_state.as_mut(cx).pr_review = None;
        cx.notify();
    }

    /// Submits the open pull request review dialog (`gh pr review`).
    pub(crate) fn submit_pr_review(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.inbox(cx).pr_review.clone() else {
            return;
        };
        if dialog.submitting {
            return;
        }
        let Some(item) = self
            .inbox(cx)
            .item(&dialog.key)
            .filter(|item| item.provider == Provider::GitHub && item.kind == Kind::Pr)
            .cloned()
        else {
            self.inbox_state.as_mut(cx).pr_review = None;
            cx.notify();
            return;
        };
        let own = is_own_pr(self.inbox(cx), &dialog.key, &item.repo);
        if own && dialog.action != ReviewAction::Comment {
            if let Some(d) = &mut self.inbox_state.as_mut(cx).pr_review {
                d.error = Some(
                    "GitHub does not allow approving or requesting changes on your own pull request."
                        .into(),
                );
            }
            cx.notify();
            return;
        }
        let body = self.pr_review_input.read(cx).text().trim().to_string();
        if dialog.action.needs_body() && body.is_empty() {
            if let Some(d) = &mut self.inbox_state.as_mut(cx).pr_review {
                d.error = Some(match dialog.action {
                    ReviewAction::RequestChanges => {
                        "Explain the requested changes before submitting.".into()
                    }
                    ReviewAction::Comment => "Review comment cannot be empty.".into(),
                    ReviewAction::Approve => unreachable!(),
                });
            }
            cx.notify();
            return;
        }
        if let Some(d) = &mut self.inbox_state.as_mut(cx).pr_review {
            d.submitting = true;
            d.error = None;
        }
        cx.notify();
        let key = dialog.key.clone();
        let action = dialog.action;
        let project = item.project.clone();
        let repo = item.repo.clone();
        let number = item.number;
        let task = cx.background_executor().spawn(async move {
            github::submit_review(Path::new(&project), &repo, number, action, &body).map(|read| {
                read.map(|(mut next, details)| {
                    next.project = project;
                    (next, details)
                })
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let landed = this.update(cx, |app, cx| {
                match result {
                    // Submitted: its dialog closes even when the pull request
                    // does not read back, so the review is not sent twice.
                    Ok(read) => {
                        let ours = app.inbox_mut(cx, |inbox| {
                            let ours = (inbox.pr_review.as_ref()).is_some_and(|d| d.key == key);
                            if ours {
                                inbox.pr_review = None;
                            }
                            ours
                        });
                        if ours {
                            app.pr_review_input
                                .update(cx, |input, cx| input.set_text("", cx));
                        }
                        app.land_github_read_back(&key, read, cx);
                        app.load_inbox_thread(&key, true, cx);
                    }
                    Err(err) => {
                        log::warn!("could not submit pull request review on {key}: {err}");
                        let mut inbox = app.inbox_state.as_mut(cx);
                        if let Some(d) = inbox.pr_review.as_mut().filter(|d| d.key == key) {
                            d.submitting = false;
                            d.error = Some(err);
                        }
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("pr review after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The Approve / Request changes / Comment menu for an open pull request,
    /// shared by the header actions bar (visible in Files changed too) and the
    /// sidebar `Review` row.
    pub(super) fn pr_review_menu(&self, item: &WorkItem, cx: &Context<Self>) -> Menu {
        let key = item.key();
        let own = is_own_pr(self.inbox(cx), &key, &item.repo);
        let row = |action: ReviewAction, icon: IconName, disabled: bool| {
            let key = key.clone();
            let entity = cx.entity().downgrade();
            MenuItem::new(action.label())
                .icon(icon)
                .disabled(disabled)
                .on_click(move |window, cx| {
                    if let Err(err) = entity.update(cx, |this, cx| {
                        this.open_pr_review_dialog(&key, action, window, cx)
                    }) {
                        log::debug!("pr review menu after app drop: {err:#}");
                    }
                })
        };
        Menu::new()
            .item(row(ReviewAction::Approve, IconName::CircleCheck, own))
            .item(row(
                ReviewAction::RequestChanges,
                IconName::CircleAlert,
                own,
            ))
            .item(row(ReviewAction::Comment, IconName::MessageSquare, false))
    }

    /// Editable Assignees, Labels, and (for pull requests) Reviewers (with
    /// completed review statuses) and Review, plus inline error / retry.
    pub(super) fn render_github_fields(
        &self,
        item: &WorkItem,
        _view: DetailView,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let inbox = self.inbox(cx);
        let key = item.key();
        let repo = repo_key(&item.repo);
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let details = inbox
            .details
            .get(&key)
            .and_then(|d| d.as_ref().ok())
            .map(|d| &d.details);
        let options_entry = inbox.github_options.get(&repo);
        let options = options_entry.and_then(|opt| opt.as_ref().ok());
        let options_loading = inbox.github_options.is_loading(&repo);
        let options_failed = matches!(options_entry, Some(Err(_)));
        let busy = inbox.github_edit_busy.is_some();
        let saving = |field: GithubEditField| {
            inbox
                .github_edit_busy
                .as_ref()
                .is_some_and(|pending| pending.key == key && pending.field == field)
        };
        let spinner = |id: &'static str| loading_icon(id, fg.ink(Ink::Muted)).into_any_element();

        // Assignees (with "Assign to me" icon or saving spinner beside the name).
        let me = options.map(|o| o.me.as_str()).unwrap_or("");
        let assigned_to_me =
            !me.is_empty() && item.assignees.iter().any(|a| a.eq_ignore_ascii_case(me));
        let assignee_accessory = if saving(GithubEditField::Assignees) || options_loading {
            Some(spinner("github-assignees-loading"))
        } else if !assigned_to_me && !busy && !me.is_empty() {
            let assign_key = key.clone();
            Some(
                IconButton::new("github-assign-me", IconName::User)
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .tooltip("Assign to me")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.assign_github_item_to_me(&assign_key, cx)
                    }))
                    .into_any_element(),
            )
        } else {
            None
        };
        let assignees_picker = match options {
            Some(opts) => {
                let choices = user_choices(opts, &item.assignees, None);
                if choices.is_empty() {
                    closed_select(
                        "github-assignees",
                        if item.assignees.is_empty() {
                            EMPTY.into()
                        } else {
                            item.assignees.join(", ")
                        },
                    )
                } else {
                    let selected = selected_values(&choices, &item.assignees);
                    let current = item.assignees.clone();
                    let edit_key = key.clone();
                    let entity = cx.entity().downgrade();
                    MultiSelect::new("github-assignees", choices)
                        .quiet()
                        .commit_on_close()
                        .size(SIZE)
                        .disabled(busy)
                        .placeholder(EMPTY)
                        .selected(selected)
                        .on_change(move |values, _, cx| {
                            let next: Vec<String> = values.iter().map(|v| v.to_string()).collect();
                            let (add, remove) = github::diff_values(&current, &next);
                            let change = ItemChange::Assignees { add, remove };
                            let edit_key = edit_key.clone();
                            if let Err(err) = entity.update(cx, |this, cx| {
                                this.update_github_item(&edit_key, change, cx)
                            }) {
                                log::debug!("github assignees after app drop: {err:#}");
                            }
                        })
                        .into_any_element()
                }
            }
            None => closed_select(
                "github-assignees",
                if item.assignees.is_empty() {
                    EMPTY.into()
                } else {
                    item.assignees.join(", ")
                },
            ),
        };
        let mut rows = vec![property_with(
            "Assignees",
            assignee_accessory,
            assignees_picker,
            cx,
        )];

        // Labels.
        let labels_picker = match options {
            Some(opts) => {
                let choices = label_choices(opts, &item.labels);
                if choices.is_empty() {
                    closed_select("github-labels", EMPTY.into())
                } else {
                    let current: Vec<String> = item.labels.iter().map(|l| l.name.clone()).collect();
                    let selected = selected_values(&choices, &current);
                    let edit_key = key.clone();
                    let entity = cx.entity().downgrade();
                    MultiSelect::new("github-labels", choices)
                        .quiet()
                        .commit_on_close()
                        .size(SIZE)
                        .disabled(busy)
                        .placeholder(EMPTY)
                        .selected(selected)
                        .on_change(move |values, _, cx| {
                            let next: Vec<String> = values.iter().map(|v| v.to_string()).collect();
                            let (add, remove) = github::diff_values(&current, &next);
                            let change = ItemChange::Labels { add, remove };
                            let edit_key = edit_key.clone();
                            if let Err(err) = entity.update(cx, |this, cx| {
                                this.update_github_item(&edit_key, change, cx)
                            }) {
                                log::debug!("github labels after app drop: {err:#}");
                            }
                        })
                        .into_any_element()
                }
            }
            None => {
                if item.labels.is_empty() {
                    closed_select("github-labels", EMPTY.into())
                } else {
                    plain(cx)
                        .children(
                            item.labels
                                .iter()
                                .map(|l| label_chip_within(l, gpui::relative(1.0), cx)),
                        )
                        .into_any_element()
                }
            }
        };
        rows.push(property_with(
            "Labels",
            saving(GithubEditField::Labels).then(|| spinner("github-labels-loading")),
            labels_picker,
            cx,
        ));

        // Pull request: Reviewers (pending + completed) + Review decision / submission.
        if item.kind == Kind::Pr {
            let reviewers = details.map(|d| d.reviewers.as_slice()).unwrap_or(&[]);
            let author = details.map(|d| d.author.as_str()).unwrap_or("");
            let settled = inbox
                .threads
                .get(&key)
                .and_then(|t| t.as_ref().ok())
                .map(|thread| settled_reviews(thread, author, reviewers))
                .unwrap_or_default();
            let placeholder = if reviewers.is_empty() && !settled.is_empty() {
                "Request reviewer…"
            } else {
                EMPTY
            };
            let reviewers_picker = match (options, details) {
                (Some(opts), Some(_)) => {
                    let choices = user_choices(opts, reviewers, Some(author));
                    if choices.is_empty() {
                        closed_select(
                            "github-reviewers",
                            if reviewers.is_empty() {
                                EMPTY.into()
                            } else {
                                reviewers.join(", ")
                            },
                        )
                    } else {
                        let selected = selected_values(&choices, reviewers);
                        let current = reviewers.to_vec();
                        let edit_key = key.clone();
                        let entity = cx.entity().downgrade();
                        MultiSelect::new("github-reviewers", choices)
                            .quiet()
                            .commit_on_close()
                            .size(SIZE)
                            .disabled(busy || item.closed)
                            .placeholder(placeholder)
                            .selected(selected)
                            .on_change(move |values, _, cx| {
                                let next: Vec<String> =
                                    values.iter().map(|v| v.to_string()).collect();
                                let (add, remove) = github::diff_values(&current, &next);
                                let change = ItemChange::Reviewers { add, remove };
                                let edit_key = edit_key.clone();
                                if let Err(err) = entity.update(cx, |this, cx| {
                                    this.update_github_item(&edit_key, change, cx)
                                }) {
                                    log::debug!("github reviewers after app drop: {err:#}");
                                }
                            })
                            .into_any_element()
                    }
                }
                _ => closed_select(
                    "github-reviewers",
                    if reviewers.is_empty() {
                        EMPTY.into()
                    } else {
                        reviewers.join(", ")
                    },
                ),
            };
            let reviewers_value = if settled.is_empty() {
                reviewers_picker
            } else {
                let can_rerequest = !item.closed && !busy;
                let settled_rows = settled.into_iter().enumerate().map(|(ix, rev)| {
                    self.render_settled_reviewer(&key, ix, rev, can_rerequest, cx)
                });
                div()
                    .child(reviewers_picker)
                    .child(div().mt_0p5().children(settled_rows))
                    .into_any_element()
            };
            rows.push(property_with(
                "Reviewers",
                saving(GithubEditField::Reviewers).then(|| spinner("github-reviewers-loading")),
                reviewers_value,
                cx,
            ));

            rows.push(self.render_review_property(item, details, cx));
        }

        // Inline repo options retry or edit error right where the user acted.
        if options_failed {
            let project = item.project.clone();
            let repo_name = item.repo.clone();
            rows.push(
                div()
                    .mt_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .text_size(Text::Sm.px())
                    .text_color(fg.ink(Ink::Muted))
                    .child("Could not load repo options")
                    .child(
                        Button::new("github-options-retry", "Retry")
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.retry_github_repo_options(&project, &repo_name, cx);
                            })),
                    )
                    .into_any_element(),
            );
        }
        // Only under the item whose edit failed.
        if let Some((_, error)) = (inbox.github_edit_error.as_ref()).filter(|(at, _)| *at == key) {
            rows.push(
                div()
                    .mt_1()
                    .text_size(Text::Sm.px())
                    .text_color(colors.danger.opacity(0.9))
                    .child(SharedString::from(error.clone()))
                    .into_any_element(),
            );
        }

        rows
    }

    /// One completed reviewer row under `Reviewers`: avatar, login, verdict
    /// icon (✓ Approved / ✗ Changes requested / 💬 Commented), and a hover
    /// re-request button while the PR is open.
    fn render_settled_reviewer(
        &self,
        key: &str,
        ix: usize,
        reviewer: SettledReviewer,
        can_rerequest: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let fg = colors.fg;
        let tint = match reviewer.verdict {
            ReviewVerdict::Approved => colors.success,
            ReviewVerdict::ChangesRequested => colors.danger,
            ReviewVerdict::Commented => fg.ink(Ink::Muted),
        };
        let group = SharedString::from(format!("settled-reviewer-{ix}"));
        let rerequest = can_rerequest.then(|| {
            let key = key.to_string();
            let login = reviewer.login.clone();
            div()
                .flex_none()
                .invisible()
                .group_hover(group.clone(), |s| s.visible())
                .child(
                    IconButton::new(
                        SharedString::from(format!("rerequest-reviewer-{ix}")),
                        IconName::RefreshCw,
                    )
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .tooltip(format!("Re-request review from {}", reviewer.login))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_github_item(
                            &key,
                            ItemChange::Reviewers {
                                add: vec![login.clone()],
                                remove: Vec::new(),
                            },
                            cx,
                        );
                    })),
                )
        });
        let tip = format!("{} · {}", reviewer.login, reviewer.verdict.label());
        div()
            .id(group.clone())
            .group(group)
            .h(px(26.0))
            .pl_2()
            .pr(theme.control_padding(SIZE))
            .flex()
            .items_center()
            .gap_1p5()
            .text_size(Text::Sm.px())
            .child(
                Avatar::new(
                    SharedString::from(format!("settled-avatar-{ix}")),
                    reviewer.login.clone(),
                )
                .size(AvatarSize::Xs),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(fg.ink(Ink::Body))
                    .child(SharedString::from(reviewer.login)),
            )
            .children(rerequest)
            .child(
                div()
                    .id(SharedString::from(format!("settled-verdict-{ix}")))
                    .flex_none()
                    .tooltip(Tooltip::text(tip))
                    .child(
                        Icon::new(reviewer.verdict.icon())
                            .size(IconSize::Xs)
                            .color(tint),
                    ),
            )
            .into_any_element()
    }

    /// The `Review` property row for a pull request: displays the current
    /// decision and, while the PR is open, stays clearly clickable to submit
    /// a review (Approve / Request changes / Comment).
    fn render_review_property(
        &self,
        item: &WorkItem,
        details: Option<&crate::work_items::Details>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let decision = details.map_or("", |d| d.review_decision.as_str());
        let review = review_decision_label(decision);
        if item.closed {
            return property(
                "Review",
                if review.is_empty() {
                    none(cx)
                } else {
                    let tint = review_tint(decision, colors).unwrap_or(colors.fg.ink(Ink::Body));
                    plain(cx).text_color(tint).child(review)
                },
                cx,
            );
        }
        let menu = self.pr_review_menu(item, cx);
        let (text, color) = if review.is_empty() {
            ("Submit review", colors.fg.ink(Ink::Faint))
        } else {
            (
                review,
                review_tint(decision, colors).unwrap_or(colors.fg.ink(Ink::Body)),
            )
        };
        let (border_strong, surface) = (colors.border_strong, colors.surface);
        let trigger = div()
            .id("github-review-trigger")
            .w_full()
            .h(px(ROW_HEIGHT))
            .px(theme.control_padding(SIZE))
            .rounded(theme.radius(Radius::Md))
            .border_1()
            .border_color(transparent_black())
            .hover(move |s| s.border_color(border_strong).bg(surface))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .text_size(Text::Base.px())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(color)
                    .child(text),
            )
            .child(
                Icon::new(IconName::ChevronsUpDown)
                    .size(IconSize::Xs)
                    .color(colors.fg.ink(Ink::Faint)),
            );
        property(
            "Review",
            DropdownMenu::new("github-pr-review", "Review", menu).trigger(trigger),
            cx,
        )
    }

    /// The pull request review dialog, over the window (`app.rs` `dialogs`).
    pub fn render_pr_review_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let dialog = self.inbox(cx).pr_review.clone()?;
        let item = self.inbox(cx).item(&dialog.key)?;
        let colors = &cx.theme().colors;
        let fg = colors.fg;
        let own = is_own_pr(self.inbox(cx), &dialog.key, &item.repo);
        let typed = !self.pr_review_input.read(cx).text().trim().is_empty();
        let blocked_own = own && dialog.action != ReviewAction::Comment;
        let can_submit =
            !dialog.submitting && !blocked_own && (!dialog.action.needs_body() || typed);
        let submit_label = if dialog.submitting {
            "Submitting…"
        } else {
            dialog.action.label()
        };
        let (submit_variant, submit_icon) = match dialog.action {
            ReviewAction::Approve => (ButtonVariant::Success, IconName::CircleCheck),
            ReviewAction::RequestChanges => (ButtonVariant::Danger, IconName::CircleAlert),
            ReviewAction::Comment => (ButtonVariant::Primary, IconName::MessageSquare),
        };
        let cancel = app_callback(cx, |this, cx| this.close_pr_review_dialog(cx));
        let submit = app_callback(cx, |this, cx| this.submit_pr_review(cx));
        let switch_action = on_value(cx, |this, value, cx| {
            let Some(action) = ReviewAction::from_key(value) else {
                return;
            };
            let changed = this.inbox_mut(cx, |inbox| {
                let Some(d) = &mut inbox.pr_review else {
                    return false;
                };
                if d.submitting {
                    return false;
                }
                d.action = action;
                d.error = None;
                true
            });
            if changed {
                cx.notify();
            }
        });
        let hint = if blocked_own {
            Some("GitHub does not allow approving or requesting changes on your own pull request.")
        } else if dialog.action.needs_body() && !typed {
            Some("A comment is required for this review.")
        } else if dialog.action == ReviewAction::Approve {
            Some("Comment is optional when approving. Press ⌘↩ to submit.")
        } else {
            None
        };
        let submitting = dialog.submitting;
        let input_focus = self.pr_review_input.read(cx).focus_handle(cx);
        let modal = Dialog::new(
            "inbox-pr-review-dialog",
            format!("Review #{}", item.number),
            cancel,
        )
        .detail(item.title.clone())
        .focus_first(input_focus)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    SegmentedControl::new("pr-review-action", dialog.action.key())
                        .segment(
                            ReviewAction::Approve.key(),
                            ReviewAction::Approve.label(),
                            Some(IconName::CircleCheck),
                        )
                        .segment(
                            ReviewAction::RequestChanges.key(),
                            ReviewAction::RequestChanges.label(),
                            Some(IconName::CircleAlert),
                        )
                        .segment(
                            ReviewAction::Comment.key(),
                            ReviewAction::Comment.label(),
                            Some(IconName::MessageSquare),
                        )
                        .on_change(switch_action),
                )
                .child(
                    div()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(fg.fill(Fill::Strong))
                        .bg(fg.fill(Fill::Hover))
                        .px_3()
                        .py_2()
                        .text_size(Text::Md.px())
                        .when(submitting, |el| el.opacity(0.4))
                        .child(self.pr_review_input.clone()),
                )
                .children(hint.map(|text| {
                    div()
                        .text_size(Text::Sm.px())
                        .text_color(fg.ink(Ink::Muted))
                        .child(text)
                }))
                .children(dialog.error.map(|err| {
                    div()
                        .text_size(Text::Sm.px())
                        .text_color(colors.danger.opacity(0.9))
                        .child(SharedString::from(err))
                })),
        )
        .action(move |close| {
            Button::new("pr-review-cancel", "Cancel")
                .variant(ButtonVariant::Ghost)
                .disabled(submitting)
                .on_click(move |_, window, cx| close(window, cx))
        })
        .action(move |_| {
            Button::new("pr-review-submit", submit_label)
                .variant(submit_variant)
                .icon(submit_icon)
                .disabled(!can_submit)
                .on_click(move |_, window, cx| submit(window, cx))
        });
        Some(modal.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{Comment, RepoUser};

    #[test]
    fn user_choices_exclude_author_and_keep_unlisted_current() {
        let opts = RepoOptions {
            me: "ben".into(),
            users: vec![
                RepoUser {
                    login: "alice".into(),
                    name: "Alice".into(),
                },
                RepoUser {
                    login: "bob".into(),
                    name: "bob".into(),
                },
            ],
            labels: Vec::new(),
        };
        let choices = user_choices(&opts, &["ghost".into()], Some("Alice"));
        let values: Vec<_> = choices.iter().map(|c| c.value.as_ref()).collect();
        assert_eq!(values, ["bob", "ghost"]);
        // A full name identical to login is not repeated as a note.
        assert_eq!(choices[0].note, None);
    }

    #[test]
    fn label_choices_preserve_unlisted_labels_case_insensitively() {
        let opts = RepoOptions {
            me: String::new(),
            users: Vec::new(),
            labels: vec![Label {
                name: "Bug".into(),
                color: "d73a4a".into(),
            }],
        };
        let current = vec![
            Label {
                name: "bug".into(),
                color: "d73a4a".into(),
            },
            Label {
                name: "urgent".into(),
                color: "ff0000".into(),
            },
        ];
        let choices = label_choices(&opts, &current);
        let values: Vec<_> = choices.iter().map(|c| c.value.as_ref()).collect();
        assert_eq!(values, ["Bug", "urgent"]);
        assert!(choices[0].color.is_some());
        assert!(choices[1].color.is_some());
        let selected = selected_values(&choices, &["bug".into(), "urgent".into()]);
        assert_eq!(
            selected,
            vec![SharedString::from("Bug"), SharedString::from("urgent")]
        );
    }

    #[test]
    fn settled_reviews_keep_latest_decisive_state_and_exclude_pending() {
        let review = |author: &str, state: &str| Comment {
            kind: "review".into(),
            author: author.into(),
            state: state.into(),
            ..Default::default()
        };
        let thread = Thread {
            comments: vec![
                // Author's own comment review is excluded.
                review("ann", "COMMENTED"),
                // Bob approved, then commented: stays Approved.
                review("bob", "APPROVED"),
                review("bob", "COMMENTED"),
                // Cat requested changes, then was re-requested (in pending).
                review("cat", "CHANGES_REQUESTED"),
                // Dan approved then got dismissed.
                review("dan", "APPROVED"),
                review("dan", "DISMISSED"),
                // Eve only commented.
                review("eve", "COMMENTED"),
            ],
            ..Default::default()
        };
        let settled = settled_reviews(&thread, "ann", &["Cat".into()]);
        assert_eq!(
            settled,
            vec![
                SettledReviewer {
                    login: "bob".into(),
                    verdict: ReviewVerdict::Approved
                },
                SettledReviewer {
                    login: "eve".into(),
                    verdict: ReviewVerdict::Commented
                },
            ]
        );
    }

    #[test]
    fn optimistic_helpers_apply_adds_and_removes() {
        let names = apply_names(
            &["Alice".into(), "bob".into()],
            &["cat".into(), "alice".into()],
            &["BOB".into()],
        );
        assert_eq!(names, vec!["Alice", "cat"]);

        let opts = RepoOptions {
            me: String::new(),
            users: Vec::new(),
            labels: vec![Label {
                name: "bug".into(),
                color: "d73a4a".into(),
            }],
        };
        let labels = apply_labels(
            &[Label {
                name: "old".into(),
                color: "111111".into(),
            }],
            &["bug".into()],
            &["old".into()],
            Some(&opts),
        );
        assert_eq!(
            labels,
            vec![Label {
                name: "bug".into(),
                color: "d73a4a".into()
            }]
        );
    }
}
