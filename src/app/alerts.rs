//! MonoCode `notifications.ts`, `notificationPreferences.ts` and the
//! callers of `playCue`: whether a cue plays and a banner is sent, and
//! what it says. Sounds are on by default; notifications are off until the
//! user turns them on, which asks macOS for permission. Both answer to the
//! project's mutes on the rail (`ui/rail/model.rs`).

use std::collections::{HashMap, HashSet};

use gpui::{Context, Task};
use serde_json::Value;

use crate::app::BenCodeApp;
use crate::db::SessionRow;
use crate::harness::{HarnessKind, PermissionRequest};
use crate::notifications::{self, Click, Permission};
use crate::sounds::{self, Cue};
use crate::ui::rail::model::{NotificationPreference, notification_id};
use crate::work_items::{Provider, WorkItem};

/// MonoCode `NOTIFICATION_CATEGORIES`, by the ids its preferences store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    PullRequests,
    Issues,
    AgentFinished,
    AgentInput,
    Reminders,
}

impl Category {
    fn id(self) -> &'static str {
        match self {
            Self::PullRequests => "pullRequests",
            Self::Issues => "issues",
            Self::AgentFinished => "agentFinished",
            Self::AgentInput => "agentInput",
            Self::Reminders => "reminders",
        }
    }
}

/// MonoCode `ProjectNotificationRule`: `after` is the last suppressed
/// millisecond.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub enabled: bool,
    pub after: i64,
}

/// MonoCode `getProjectNotificationRule`.
pub fn project_rule(pref: Option<&NotificationPreference>, category: Category) -> Rule {
    let Some(pref) = pref else {
        return Rule {
            enabled: true,
            after: 0,
        };
    };
    let enabled_after = pref
        .extra
        .get("enabledAfter")
        .and_then(|after| after.get(category.id()))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Rule {
        enabled: pref.muted_until != Some(None)
            && !pref.disabled.iter().any(|c| c == category.id()),
        after: pref
            .resumed_at
            .unwrap_or(0)
            // A scheduled event at the deadline belongs to the resumed interval.
            .max(pref.muted_until.flatten().unwrap_or(1) - 1)
            .max(enabled_after),
    }
}

/// MonoCode `allowsProjectNotification`.
pub fn allows(rule: Rule, now: i64, occurred_at: Option<i64>) -> bool {
    rule.enabled && now > rule.after && occurred_at.is_none_or(|at| at > rule.after)
}

/// MonoCode `shouldNotify`: a banner only earns its place while the user
/// looks elsewhere, at another app or another thread.
pub fn should_notify(
    enabled: bool,
    permission: Permission,
    window_active: bool,
    session_visible: bool,
) -> bool {
    if !enabled || (window_active && session_visible) {
        return false;
    }
    matches!(permission, Permission::Granted | Permission::Prompt)
}

/// What a thread's banner is about.
pub enum Event<'a> {
    Finished,
    Input(&'a PermissionRequest),
}

/// MonoCode `NotificationText`: the app, then the thread, then the reply.
#[derive(Debug, PartialEq)]
pub struct Text {
    pub title: String,
    pub subtitle: String,
    pub body: String,
}

const BODY_MAX: usize = 240;

/// MonoCode `notificationText`.
pub fn notification_text(session: &SessionRow, event: &Event) -> Text {
    let subtitle = crate::app::session_list::display_title(&session.title, &session.harness);
    let harness = HarnessKind::from_id(&session.harness).map_or("The agent", |k| k.label());
    let body = match event {
        Event::Input(request) if request.tool == crate::app::QUESTION_TOOL => {
            let questions = crate::ui::composer::question::parse_questions(&request.input);
            let prompt = questions
                .first()
                .map(|q| q.prompt.clone())
                .filter(|p| !p.trim().is_empty());
            prompt.unwrap_or_else(|| format!("{harness} has a question for you"))
        }
        Event::Input(request) => {
            let what = Some(request.description.trim())
                .filter(|d| !d.is_empty())
                .unwrap_or(request.tool.trim());
            if what.is_empty() {
                format!("{harness} needs your approval")
            } else {
                format!("Approve: {what}")
            }
        }
        Event::Finished => session
            .blocks
            .iter()
            .rev()
            .filter(|b| b.role == "assistant")
            .find_map(|b| b.text.as_deref().filter(|t| !t.trim().is_empty()))
            .map_or_else(|| format!("{harness} finished"), str::to_string),
    };
    Text {
        title: "BenCode".into(),
        subtitle,
        body: clip(&body),
    }
}

/// The first paragraph, whitespace collapsed; macOS wraps and truncates
/// the rest.
fn clip(text: &str) -> String {
    let paragraph = text
        .split("\n\n")
        .map(|part| part.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|part| !part.is_empty())
        .unwrap_or_default();
    if paragraph.chars().count() > BODY_MAX {
        let cut: String = paragraph.chars().take(BODY_MAX - 1).collect();
        format!("{cut}…")
    } else {
        paragraph
    }
}

/// MonoCode `InboxNotificationTracker`: each item's newest revision, so a
/// poll can tell which items changed. A provider's first list only primes it.
#[derive(Default)]
pub struct InboxTracker {
    revisions: HashMap<String, i64>,
    primed: HashSet<Provider>,
}

impl InboxTracker {
    /// The items new or updated since the last list.
    pub fn observe<'a>(&mut self, items: &'a [WorkItem], failed: bool) -> Vec<&'a WorkItem> {
        let mut changed = Vec::new();
        for item in items {
            let updated = crate::ui::inbox_view::updated_ms(item);
            if updated == 0 {
                continue;
            }
            let primed = self.primed.contains(&item.provider);
            let previous = self.revisions.insert(item.key(), updated);
            if primed && previous.is_none_or(|was| updated > was) {
                changed.push(item);
            }
            if let Some(was) = previous.filter(|was| *was > updated) {
                self.revisions.insert(item.key(), was);
            }
        }
        // A failed source lists nothing; it is primed by its first real list.
        for item in items {
            self.primed.insert(item.provider);
        }
        if !failed {
            self.primed.extend([Provider::GitHub, Provider::Backlog]);
        }
        changed
    }
}

/// MonoCode `inboxNotificationProject`: the folder an item was listed for,
/// else the tracker's own container.
fn inbox_project(item: &WorkItem) -> String {
    if item.project.is_empty() {
        format!("{:?}:{}", item.provider, item.repo).to_lowercase()
    } else {
        notification_id(&item.project)
    }
}

/// The app's alert state.
#[derive(Default)]
pub struct Alerts {
    /// MonoCode `monocode.sounds`; on by default.
    pub sounds: bool,
    /// MonoCode `monocode.notifications`; off until the user opts in.
    pub notifications: bool,
    /// MonoCode `monocode.soundsEnabledAt`: activity from before sounds were
    /// turned back on stays quiet.
    pub sounds_enabled_at: Option<i64>,
    /// What macOS last said; refreshed at launch and on focus.
    pub permission: Permission,
    /// Tracked from the window's activation: a banner is for when the user
    /// is looking elsewhere.
    pub window_active: bool,
    /// MonoCode `announcedUpdate`: one cue per available version.
    pub announced_update: Option<String>,
    pub inbox: InboxTracker,
}

impl BenCodeApp {
    fn project_allows(
        &self,
        project_id: &str,
        category: Category,
        occurred_at: Option<i64>,
    ) -> bool {
        let pref = self.settings.rail.project_notifications.get(project_id);
        allows(
            project_rule(pref, category),
            crate::app::now_ms(),
            occurred_at,
        )
    }

    /// MonoCode `playCue` for app-wide cues (switches, Copy, updates).
    pub fn play_cue(&self, cue: Cue) {
        if self.alerts.sounds {
            sounds::play(cue);
        }
    }

    /// MonoCode `playCue` for a project's activity: muted projects and
    /// activity from before sounds were turned on stay quiet.
    fn play_project_cue(
        &self,
        cue: Cue,
        project_id: &str,
        category: Category,
        occurred_at: Option<i64>,
    ) -> bool {
        if !self.alerts.sounds || !self.project_allows(project_id, category, occurred_at) {
            return false;
        }
        if occurred_at.is_some_and(|at| at < self.alerts.sounds_enabled_at.unwrap_or(0)) {
            return false;
        }
        sounds::play(cue);
        true
    }

    /// Whether the user can see `session_id` now: its thread is the one on
    /// screen and no surface covers the chat.
    fn session_visible(&self, session_id: &str) -> bool {
        self.surface.is_none() && self.selected_session_id.as_deref() == Some(session_id)
    }

    /// MonoCode `notifyProjectSession`: the banner, when policy allows.
    /// The task resolves true once macOS accepted it.
    fn notify_session(
        &self,
        session_id: &str,
        event: Event,
        category: Category,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return Task::ready(false);
        };
        let allowed = self.project_allows(&notification_id(&session.cwd), category, None)
            && should_notify(
                self.alerts.notifications,
                self.alerts.permission,
                self.alerts.window_active,
                self.session_visible(session_id),
            );
        if !allowed {
            return Task::ready(false);
        }
        let text = notification_text(session, &event);
        let identifier = notifications::session_identifier(session_id);
        let sound = self.alerts.sounds;
        cx.background_executor().spawn(async move {
            match notifications::show(&identifier, &text.title, &text.subtitle, &text.body, sound) {
                Ok(()) => true,
                Err(err) => {
                    log::info!("notification not sent: {err:#}");
                    false
                }
            }
        })
    }

    /// MonoCode `announceSessionFinished`: the banner, else the cue.
    pub fn announce_turn_finished(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(project) = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| notification_id(&s.cwd))
        else {
            return;
        };
        let occurred_at = crate::app::now_ms();
        let sent = self.notify_session(session_id, Event::Finished, Category::AgentFinished, cx);
        cx.spawn(async move |this, cx| {
            if sent.await {
                return;
            }
            let played = this.update(cx, |this, _| {
                this.play_project_cue(
                    Cue::TurnFinished,
                    &project,
                    Category::AgentFinished,
                    Some(occurred_at),
                );
            });
            if let Err(err) = played {
                log::debug!("turn cue after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `useInputNotifications`: an approval or a question waits.
    pub fn announce_input(
        &mut self,
        session_id: &str,
        request: &PermissionRequest,
        cx: &mut Context<Self>,
    ) {
        self.notify_session(session_id, Event::Input(request), Category::AgentInput, cx)
            .detach();
    }

    /// MonoCode's reminder banner (`reminders.rs`).
    pub fn announce_reminder(&self, reminder: &crate::db::Reminder, cx: &mut Context<Self>) {
        if !self.alerts.notifications
            || !self.project_allows(
                &notification_id(&reminder.cwd),
                Category::Reminders,
                Some(reminder.due_at),
            )
        {
            return;
        }
        let identifier = notifications::reminder_identifier(&reminder.session_id, reminder.due_at);
        let subtitle = crate::app::session_list::display_title(&reminder.title, &reminder.harness);
        let sound = self.alerts.sounds;
        cx.background_executor()
            .spawn(async move {
                if let Err(err) = notifications::show(
                    &identifier,
                    "BenCode",
                    &subtitle,
                    "Reminder: continue this conversation.",
                    sound,
                ) {
                    log::info!("reminder notification not sent: {err:#}");
                }
            })
            .detach();
    }

    /// MonoCode `announceUpdateAvailable`: one cue per available version.
    pub fn announce_update_available(&mut self, version: Option<&str>) {
        let Some(version) = version else {
            self.alerts.announced_update = None;
            return;
        };
        if self.alerts.announced_update.as_deref() == Some(version) {
            return;
        }
        self.alerts.announced_update = Some(version.to_string());
        self.play_cue(Cue::UpdateAvailable);
    }

    /// MonoCode `useInboxUnseen`: at most one cue per poll, for the first
    /// changed item whose project is not muted.
    pub fn announce_inbox_changes(&mut self, failed: bool) {
        let items = std::mem::take(&mut self.inbox.items);
        let changed = self.alerts.inbox.observe(&items, failed);
        for item in changed {
            let category = match item.kind {
                crate::work_items::Kind::Pr => Category::PullRequests,
                _ => Category::Issues,
            };
            let at = crate::ui::inbox_view::updated_ms(item);
            if self.play_project_cue(Cue::InboxUnseen, &inbox_project(item), category, Some(at)) {
                break;
            }
        }
        self.inbox.items = items;
    }

    /// Settings › General › "Sounds".
    pub fn set_sounds(&mut self, on: bool, cx: &mut Context<Self>) {
        if on && !self.alerts.sounds {
            self.alerts.sounds_enabled_at = Some(crate::app::now_ms());
        }
        self.alerts.sounds = on;
        self.save_settings(cx);
        cx.notify();
    }

    /// Settings › General › "Notifications": turning it on asks macOS.
    pub fn set_notifications(&mut self, on: bool, cx: &mut Context<Self>) {
        self.alerts.notifications = on;
        self.save_settings(cx);
        cx.notify();
        if on {
            self.load_notification_permission(true, cx);
        }
    }

    /// Re-reads (or, with `ask`, requests) the permission off the UI thread.
    pub fn load_notification_permission(&mut self, ask: bool, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async move {
            if ask {
                notifications::request_permission()
            } else {
                notifications::permission()
            }
        });
        cx.spawn(async move |this, cx| {
            let permission = task.await;
            let landed = this.update(cx, |this, cx| {
                if this.alerts.permission != permission {
                    this.alerts.permission = permission;
                    cx.notify();
                }
            });
            if let Err(err) = landed {
                log::debug!("notification permission after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The window came forward or went back. The user may have flipped the
    /// switch in System Settings, so the permission is read again.
    pub fn on_window_activation(&mut self, active: bool, cx: &mut Context<Self>) {
        self.alerts.window_active = active;
        if active && self.alerts.notifications {
            self.load_notification_permission(false, cx);
        }
    }

    /// Settings › General › "Open System Settings".
    pub fn open_notification_settings(&self) {
        if let Err(err) = notifications::open_settings() {
            log::warn!("could not open notification settings: {err:#}");
        }
    }

    /// Becomes the notification center's delegate and opens what a click
    /// on a banner names.
    pub fn listen_for_notification_clicks(&mut self, cx: &mut Context<Self>) {
        let Some(mut clicks) = notifications::install_delegate() else {
            return;
        };
        if self.alerts.notifications {
            self.load_notification_permission(false, cx);
        }
        cx.spawn(async move |this, cx| {
            while let Some(identifier) = clicks.recv().await {
                let Some(click) = notifications::parse_click(&identifier) else {
                    continue;
                };
                let opened = this.update(cx, |this, cx| {
                    cx.activate(true);
                    match click {
                        Click::Session(id) => this.select_live_agent(&id, cx),
                        Click::Reminder { session_id, due_at } => {
                            this.open_reminder(&session_id, due_at, cx)
                        }
                    }
                });
                if opened.is_err() {
                    return;
                }
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pref(value: Value) -> NotificationPreference {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_project_without_preferences_allows_everything() {
        let rule = project_rule(None, Category::AgentFinished);
        assert_eq!(
            rule,
            Rule {
                enabled: true,
                after: 0
            }
        );
        assert!(allows(rule, 10, Some(5)));
    }

    #[test]
    fn a_mute_until_resumed_or_a_disabled_category_blocks() {
        let muted = pref(json!({ "disabled": [], "mutedUntil": null }));
        assert!(!project_rule(Some(&muted), Category::Issues).enabled);
        let disabled = pref(json!({ "disabled": ["agentInput"] }));
        assert!(!project_rule(Some(&disabled), Category::AgentInput).enabled);
        assert!(project_rule(Some(&disabled), Category::AgentFinished).enabled);
    }

    #[test]
    fn a_timed_mute_holds_until_its_deadline() {
        let muted = pref(json!({ "disabled": [], "mutedUntil": 1000 }));
        let rule = project_rule(Some(&muted), Category::Issues);
        assert_eq!(rule.after, 999);
        assert!(!allows(rule, 999, None));
        assert!(allows(rule, 1000, None));
        // Activity from inside the mute stays quiet after it ends.
        assert!(!allows(rule, 2000, Some(500)));
    }

    #[test]
    fn a_resume_or_a_reenabled_category_moves_the_line() {
        let resumed =
            pref(json!({ "disabled": [], "resumedAt": 50, "enabledAfter": { "issues": 80 } }));
        assert_eq!(project_rule(Some(&resumed), Category::Issues).after, 80);
        assert_eq!(
            project_rule(Some(&resumed), Category::PullRequests).after,
            50
        );
    }

    #[test]
    fn a_banner_is_for_when_the_user_looks_elsewhere() {
        assert!(!should_notify(false, Permission::Granted, false, false));
        assert!(!should_notify(true, Permission::Granted, true, true));
        assert!(should_notify(true, Permission::Granted, true, false));
        assert!(should_notify(true, Permission::Prompt, false, true));
        assert!(!should_notify(true, Permission::Denied, false, false));
        assert!(!should_notify(true, Permission::Unsupported, false, false));
    }

    fn session(blocks: Vec<crate::db::Block>) -> SessionRow {
        SessionRow {
            id: "s".into(),
            title: "claude · Fix the rail".into(),
            harness: "claude".into(),
            blocks,
            ..Default::default()
        }
    }

    fn block(role: &str, text: &str) -> crate::db::Block {
        serde_json::from_value(json!({ "id": text, "role": role, "text": text })).unwrap()
    }

    #[test]
    fn a_finished_banner_quotes_the_last_reply() {
        let s = session(vec![
            block("assistant", "First."),
            block("assistant", "Done.\n\nMore detail   here."),
            block("tool", "ran"),
        ]);
        let text = notification_text(&s, &Event::Finished);
        assert_eq!(text.title, "BenCode");
        assert_eq!(text.body, "Done.");
        assert_eq!(
            notification_text(&session(vec![]), &Event::Finished).body,
            "Claude Code finished"
        );
    }

    #[test]
    fn an_input_banner_names_the_question_or_the_approval() {
        let s = session(vec![]);
        let ask = PermissionRequest {
            request_id: "1".into(),
            tool: crate::app::QUESTION_TOOL.into(),
            description: String::new(),
            input: json!({ "questions": [{ "question": "Which database?", "options": ["SQLite"] }] }),
        };
        assert_eq!(
            notification_text(&s, &Event::Input(&ask)).body,
            "Which database?"
        );
        let approve = PermissionRequest {
            request_id: "2".into(),
            tool: "Bash".into(),
            description: "Run cargo test".into(),
            input: json!({}),
        };
        assert_eq!(
            notification_text(&s, &Event::Input(&approve)).body,
            "Approve: Run cargo test"
        );
    }

    #[test]
    fn a_long_reply_is_clipped() {
        let body = clip(&"word ".repeat(100));
        assert_eq!(body.chars().count(), BODY_MAX);
        assert!(body.ends_with('…'));
    }

    fn item(number: i64, updated_at: &str) -> WorkItem {
        WorkItem {
            provider: Provider::GitHub,
            number,
            updated_at: updated_at.into(),
            repo: "o/r".into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_first_list_primes_and_later_ones_report_changes() {
        let mut tracker = InboxTracker::default();
        let first = [item(1, "2026-01-01T00:00:00Z")];
        assert!(tracker.observe(&first, false).is_empty());
        let next = [
            item(1, "2026-01-02T00:00:00Z"),
            item(2, "2026-01-01T00:00:00Z"),
        ];
        let changed: Vec<i64> = tracker
            .observe(&next, false)
            .iter()
            .map(|i| i.number)
            .collect();
        assert_eq!(changed, [1, 2]);
        assert!(tracker.observe(&next, false).is_empty());
    }
}
