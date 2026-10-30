//! Bots on the app (MonoCode `App.tsx`'s `monoViewId`, `monoWorkspace.ts`,
//! `monoFiles.ts`): the roster in `settings.json`, the Bot whose
//! conversation covers the workspace, its Overview beside the chat and its
//! settings dialog, and the standing context a turn carries (its soul and
//! its memory). Its files are `bot_files.rs`, its Overview's state
//! `bot_overview.rs`; the model is in `crate::bots`.

use std::collections::HashMap;
use std::time::Instant;

use gpui::{Context, Hsla, PathPromptOptions, Subscription, Window};

use crate::app::bot_files::FilesState;
use crate::app::bot_overview::OverviewState;
use crate::app::session_folders::parse_hex;
use crate::app::{BenCodeApp, same_project_path, unique_id};
use crate::bots::figure::{Ended, Figure, Mood, idle_mood};
use crate::bots::{Bot, BotState, BotStatus, Handed};
use crate::harness::DoneStatus;
use crate::ui::figure::Look;

/// MonoCode `ensureMonoSession`: a Bot's chat starts with Auto permissions.
const BOT_RUNTIME_MODE: &str = "auto";

/// MonoCode `MonoSettingsSection`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsSection {
    #[default]
    General,
    Soul,
    Memory,
}

/// What a running turn carried, recorded as handed over once it ends.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Carried {
    soul_hash: String,
    memory_hash: String,
}

pub struct BotsState {
    /// The Bot whose conversation covers the workspace.
    pub view: Option<String>,
    /// Replies each Bot's conversation has that the user has not seen.
    pub unread: HashMap<String, usize>,
    /// How each conversation's last turn ended and when, while its bot's
    /// figure shows it (BenCode's own).
    pub ended: HashMap<String, (Ended, Instant)>,
    /// What the user set aside of each bot's digest, by bot
    /// (`bots/digest.rs`'s keys).
    pub digest_aside: HashMap<String, std::collections::HashSet<String>>,
    /// MonoCode `monoOverviewOpen`: the title bar's Overview pill. A Bot
    /// opens with its Overview beside the chat until it is hidden.
    pub overview_open: bool,
    /// MonoCode `monoSettingsFor`: the settings dialog is open, on this
    /// section.
    pub settings_open: bool,
    pub settings_section: SettingsSection,
    /// The dialog's face opens its figure's parts and colours (MonoCode
    /// `LookButton`).
    pub look_open: bool,
    /// MonoCode `lastMonoIdRef`: the last Bot opened, which ⌘I brings back.
    pub last_bot: Option<String>,
    /// The Bot composer's emoji picker (MonoCode `EmojiPickerPopover`).
    pub emoji: crate::ui::emoji_picker::EmojiPicker,
    pub files: FilesState,
    pub overview: OverviewState,
    pub changes: crate::app::bot_changes::ChangesState,
    /// Bot id awaiting delete confirmation.
    pub pending_delete: Option<String>,
    /// A Bot the rail's + made, kept only once its settings say Create:
    /// closing them otherwise takes it away.
    pub creating: Option<String>,
    /// The open Bot's chat is awaiting a reset confirmation.
    pub pending_reset: bool,
    /// Bot id awaiting Settings › Bots' Reset Bot confirmation.
    pub pending_defaults: Option<String>,
    /// Why the last reset could not finish.
    pub reset_error: Option<String>,
    /// Why a Bot's conversation could not be read, by Bot; the floating
    /// chat shows it in place of "Loading conversation…".
    pub load_errors: HashMap<String, String>,
    /// The floating chat, while its window is open.
    float: Option<gpui::WindowHandle<crate::ui::bot_float::BotFloat>>,
    /// The context a running turn carries, by conversation: recorded as
    /// handed over once the turn ends with a provider session to name.
    carrying: HashMap<String, Carried>,
    _subscriptions: Vec<Subscription>,
}

/// MonoCode `onToggleMono`'s pick: the last Bot opened while it is still
/// on the roster, else the first.
pub fn toggle_target(last: Option<&str>, roster: &[Bot]) -> Option<String> {
    last.filter(|id| roster.iter().any(|bot| bot.id == *id))
        .map(str::to_string)
        .or_else(|| roster.first().map(|bot| bot.id.clone()))
}

impl BotsState {
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> Self {
        let (files, subscriptions) = FilesState::new(window, cx);
        let changes = crate::app::bot_changes::ChangesState::new(window, cx);
        Self {
            view: None,
            overview_open: true,
            settings_open: false,
            settings_section: SettingsSection::General,
            look_open: false,
            last_bot: None,
            emoji: Default::default(),
            files,
            overview: OverviewState::default(),
            changes,
            pending_delete: None,
            creating: None,
            pending_reset: false,
            pending_defaults: None,
            reset_error: None,
            load_errors: HashMap::new(),
            float: None,
            carrying: HashMap::new(),
            unread: HashMap::new(),
            ended: HashMap::new(),
            digest_aside: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }
}

impl BenCodeApp {
    pub fn bot(&self, id: &str) -> Option<&Bot> {
        self.settings.bots.iter().find(|bot| bot.id == id)
    }

    /// MonoCode `monoForSession`: the Bot whose conversation this is.
    pub fn bot_for_session(&self, session_id: &str) -> Option<&Bot> {
        self.settings
            .bots
            .iter()
            .find(|bot| bot.session_id.as_deref() == Some(session_id))
    }

    pub fn is_bot_session(&self, session_id: &str) -> bool {
        self.bot_for_session(session_id).is_some()
    }

    /// MonoCode `useUnseenReplyCounts`: replies a Bot finished (or its
    /// habits posted) while the user was not looking at it.
    pub fn bot_unread(&self, bot: &Bot) -> Option<usize> {
        let session = bot.session_id.as_deref()?;
        self.bots.unread.get(session).copied().filter(|n| *n > 0)
    }

    /// MonoCode `noteReply`: one more unread reply, unless it is in view.
    pub(crate) fn note_bot_reply(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if !self.is_bot_session(session_id) || self.bot_view_session() == Some(session_id) {
            return;
        }
        *self.bots.unread.entry(session_id.to_string()).or_default() += 1;
        cx.notify();
    }

    /// The Bot open in the main area.
    pub fn bot_in_view(&self) -> Option<&Bot> {
        self.bot(self.bots.view.as_deref()?)
    }

    /// The conversation the open Bot shows, once it is loaded.
    pub fn bot_view_session(&self) -> Option<&str> {
        let id = self.bot_in_view()?.session_id.as_deref()?;
        self.sessions.iter().any(|s| s.id == id).then_some(id)
    }

    /// A Bot's colour: its custom one, else its palette pick.
    pub fn bot_color(&self, bot: &Bot) -> Hsla {
        bot.custom_color
            .as_deref()
            .and_then(parse_hex)
            .or_else(|| crate::bots::bot_palette_color(bot.color_index))
            .or_else(|| crate::bots::bot_palette_color(1))
            .unwrap_or_default()
    }

    /// How a bot's figure looks now: its status, and while it is idle how
    /// its last turn ended or that one of its habits is coming up.
    pub fn bot_look(&self, bot: &Bot) -> Look {
        let mood = match self.bot_state(bot).status {
            BotStatus::Working => Mood::Working,
            BotStatus::NeedsYou => Mood::NeedsYou,
            BotStatus::Idle => {
                let now = Instant::now();
                let ended = bot
                    .session_id
                    .as_deref()
                    .and_then(|id| self.bots.ended.get(id))
                    .map(|&(how, at)| (how, now.saturating_duration_since(at)));
                let ahead = self.habits.next_run(&bot.id).is_some();
                let (mood, until) = idle_mood(ended, ahead);
                return Look {
                    until: until.map(|(left, after)| (now + left, after)),
                    ..Look::new(mood, &bot.id)
                };
            }
        };
        Look::new(mood, &bot.id)
    }

    /// The status beside a bot's name, from its look: what it is doing, a
    /// failed last turn, or the habit it sleeps until. A finished turn's
    /// moment passes with nothing redrawn, so the words skip it.
    pub fn bot_status_label(&self, bot: &Bot, look: Look) -> String {
        let mood = match look.until {
            Some((_, after)) => after,
            None => look.mood,
        };
        match mood {
            Mood::Working => BotStatus::Working.label().to_string(),
            Mood::NeedsYou => BotStatus::NeedsYou.label().to_string(),
            Mood::Failed => "Last turn failed".to_string(),
            Mood::Asleep => match self.habits.next_run(&bot.id) {
                Some((at, name)) => format!(
                    "Asleep until {} · {name}",
                    crate::bots::habits::wake_time(at, crate::app::now_ms())
                ),
                None => "Asleep".to_string(),
            },
            Mood::Idle | Mood::Done => BotStatus::Idle.label().to_string(),
        }
    }

    /// A bot's turn ended: its figure shows how until the next one (a
    /// finished turn only for a moment). One stopped shows nothing.
    pub(crate) fn note_bot_turn_end(&mut self, session_id: &str, outcome: Option<DoneStatus>) {
        if !self.is_bot_session(session_id) {
            return;
        }
        let how = match outcome {
            Some(DoneStatus::Completed) => Ended::Done,
            Some(DoneStatus::Failed) => Ended::Failed,
            Some(DoneStatus::Cancelled) | None => {
                self.bots.ended.remove(session_id);
                return;
            }
        };
        self.bots
            .ended
            .insert(session_id.to_string(), (how, Instant::now()));
    }

    /// MonoCode `monoLook().projects`: its projects as (rail label, folder).
    pub fn bot_projects(&self, bot: &Bot) -> Vec<(String, String)> {
        bot.projects
            .iter()
            .map(|path| (self.rail_project_label(path), path.clone()))
            .collect()
    }

    /// MonoCode `monoState`: working, waiting on the user, or idle.
    pub fn bot_state(&self, bot: &Bot) -> BotState {
        let Some(session) = bot
            .session_id
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| s.id == id))
        else {
            return BotState::IDLE;
        };
        if let Some(request) = self.pending_permission_for(&session.id) {
            let activity = if request.tool == crate::app::QUESTION_TOOL {
                crate::bots::question_activity(&request.input)
            } else {
                Some(crate::bots::approval_activity(
                    &crate::harness::summarize_tool_input(&request.tool, &request.input),
                ))
            };
            return BotState {
                status: BotStatus::NeedsYou,
                activity,
            };
        }
        if self
            .thread(&session.id)
            .is_some_and(|thread| thread.usage_limit.is_some())
        {
            return BotState {
                status: BotStatus::NeedsYou,
                activity: Some("Usage limit reached".to_string()),
            };
        }
        if !self.is_agent_running_in(&session.id) {
            return BotState::IDLE;
        }
        BotState {
            status: BotStatus::Working,
            activity: Some(crate::bots::working_activity(&session.blocks)),
        }
    }

    /// MonoCode `createMono` then `onOpenMono`: a new Bot, opened.
    pub fn create_bot(&mut self, cx: &mut Context<Self>) {
        let id = self.add_bot(cx);
        self.open_bot(&id, cx);
        // Its settings open with it, ready to be told what it works on (a
        // new Bot has no conversation to read, so it is in view already).
        if self.bots.view.as_deref() == Some(id.as_str()) {
            self.open_bot_settings(SettingsSection::General, cx);
            self.bots.creating = Some(id);
        }
    }

    /// The new Bot's Create: it stays, with what its settings hold.
    pub fn confirm_new_bot(&mut self, cx: &mut Context<Self>) {
        self.bots.creating = None;
        self.close_bot_settings(cx);
    }

    /// MonoCode `createMono`: a new Bot on the roster; its id, which gives
    /// it a name none of the others goes by.
    pub fn add_bot(&mut self, cx: &mut Context<Self>) -> String {
        let id = crate::bots::fresh_id(&self.settings.bots, || unique_id("bot"));
        let bot = crate::bots::new_bot(&self.settings.bots, id.clone());
        self.update_bots(|roster| roster.push(bot), cx);
        id
    }

    /// MonoCode `onOpenMono`: the Bot's conversation takes the main area,
    /// read from the database or started if it has none yet.
    pub fn open_bot(&mut self, id: &str, cx: &mut Context<Self>) {
        self.close_rail_menu(cx);
        self.ensure_bot_session(id, true, cx);
    }

    /// MonoCode `ensureMonoSession`: the Bot's conversation in memory,
    /// without showing it (the floating chat reads it from there).
    pub fn prepare_bot_session(&mut self, id: &str, cx: &mut Context<Self>) {
        self.ensure_bot_session(id, false, cx);
    }

    fn ensure_bot_session(&mut self, id: &str, show: bool, cx: &mut Context<Self>) {
        let Some(session) = self.bot(id).map(|bot| bot.session_id.clone()) else {
            return;
        };
        self.warm_bot_files(id, cx);
        let Some(session_id) = session else {
            self.start_bot_session(id, show, cx);
            return;
        };
        if self.sessions.iter().any(|s| s.id == session_id) {
            if show {
                self.show_bot(id, cx);
            }
            return;
        }
        let (bot_id, row) = (id.to_string(), session_id.clone());
        self.db_then(
            cx,
            move |db| db.get_session(&row),
            move |this, loaded, cx| {
                // Deleted or reset while it loaded: no longer its chat, and
                // nothing to bring back or start over.
                let Some(bot) = this.bot(&bot_id) else {
                    return;
                };
                if bot.session_id.as_deref() != Some(session_id.as_str()) {
                    if show {
                        this.open_bot(&bot_id, cx);
                    }
                    return;
                }
                if loaded.is_ok() {
                    this.bots.load_errors.remove(&bot_id);
                }
                match loaded {
                    Ok(Some(session)) => {
                        if !this.sessions.iter().any(|s| s.id == session.id) {
                            this.sessions.push(session);
                        }
                        if show {
                            this.show_bot(&bot_id, cx);
                        }
                        cx.notify();
                    }
                    // Deleted since: it starts over, as a Bot first opened does.
                    Ok(None) => this.start_bot_session(&bot_id, show, cx),
                    Err(err) => {
                        log::error!("could not load the bot's conversation: {err:#}");
                        this.bots
                            .load_errors
                            .insert(bot_id, format!("Could not load the conversation: {err}"));
                        cx.notify();
                    }
                }
            },
        );
    }

    /// A new conversation for the Bot, in the home folder: no single
    /// project is a Bot's own.
    fn start_bot_session(&mut self, id: &str, show: bool, cx: &mut Context<Self>) {
        let Some(home) = super::projects::home_dir() else {
            log::error!("no home folder to start a bot in");
            return;
        };
        let session_id = self.create_session_row(&home, cx);
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            session.runtime_mode = Some(BOT_RUNTIME_MODE.to_string());
            session.worktree_cwd = None;
            session.branch = None;
        }
        self.persist_session(&session_id);
        self.update_bots(
            |roster| {
                if let Some(bot) = roster.iter_mut().find(|bot| bot.id == id) {
                    bot.session_id = Some(session_id.clone());
                    bot.handed = None;
                    bot.rotation = None;
                }
            },
            cx,
        );
        if show {
            self.show_bot(id, cx);
        }
    }

    /// MonoCode `FloatingMonoChat`: the Bot's chat in the window that
    /// floats above the others, opened or brought forward on this Bot.
    pub fn open_bot_float(&mut self, id: &str, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.open_bot_float_over(id, window.window_handle(), cx);
    }

    /// `open_bot_float` from outside BenCode's window (the menu bar), with
    /// the window the float belongs to.
    pub fn open_bot_float_over(
        &mut self,
        id: &str,
        main: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if self.bot(id).is_none() {
            return;
        }
        self.close_rail_menu(cx);
        self.prepare_bot_session(id, cx);
        let (app, bot_id) = (cx.entity(), id.to_string());
        let existing = self.bots.float.take();
        // Opening a window updates the app it is drawn from: after this one.
        cx.defer(move |cx| {
            let shown = existing.and_then(|float| {
                float
                    .update(cx, |float, window, cx| {
                        float.show(&bot_id, window, cx);
                        window.activate_window();
                    })
                    .ok()
                    .map(|()| float)
            });
            let float = match shown {
                Some(float) => Some(float),
                // Never opened, or closed since.
                None => crate::ui::bot_float::open(app.clone(), main, bot_id, cx)
                    .map_err(|err| log::error!("could not open the floating chat: {err:#}"))
                    .ok(),
            };
            app.update(cx, |this, _| this.bots.float = float);
        });
    }

    /// A message typed in the floating chat: into the running turn where
    /// the agent takes follow-ups, queued behind it otherwise, else a turn
    /// of its own (the composer's own rules, `submit_prompt`).
    /// Whether it was taken: a thread that cannot run keeps the message
    /// and its files where they were (MonoCode's "could not be sent").
    pub fn send_bot_message(
        &mut self,
        session_id: &str,
        text: String,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if let Some(why) = self.turn_blocked(session_id) {
            log::warn!("bot thread {session_id} {why}; message not sent");
            return Err("The message could not be sent. Try again in a moment.".to_string());
        }
        // With the files and a thread asked about waiting in the thread's
        // composer, as Send takes them.
        let (attachments, card) = self
            .threads
            .get_mut(session_id)
            .map(|thread| {
                (
                    std::mem::take(&mut thread.attachments),
                    thread.composer_card.take(),
                )
            })
            .unwrap_or_default();
        let input = crate::app::TurnInput {
            text,
            attachments,
            card: card.map(Box::new),
            ..Default::default()
        };
        if self.is_agent_running_in(session_id) {
            if let Err(input) = self.steer(session_id, input, cx) {
                self.thread_mut(session_id).queue.push(*input);
            }
            cx.notify();
            return Ok(());
        }
        self.send_turn(session_id, input, cx);
        Ok(())
    }

    fn show_bot(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.surface == Some(crate::app::Surface::Notes) {
            self.save_note_if_dirty(cx);
        }
        self.surface = None;
        self.settings_return = None;
        let another = self.bots.view.as_deref() != Some(id);
        if another {
            // Another Bot's view closes the last one's dialog.
            self.close_bot_settings(cx);
            self.bots.overview.bot_changed();
        }
        self.bots.view = Some(id.to_string());
        self.bots.last_bot = Some(id.to_string());
        if let Some(session) = self.bot(id).and_then(|m| m.session_id.clone()) {
            self.bots.unread.remove(&session);
        }
        self.bots.pending_reset = false;
        self.bots.files.load_error = None;
        self.load_bot_files(id, cx);
        self.load_bot_overview(id, cx);
        self.load_bot_changes(cx);
        self.load_bot_project_sessions(cx);
        self.save_open_bot(cx);
        self.sync_selection(cx);
    }

    /// MonoCode `closeMonoView`: back to the project's tabs. Returns whether
    /// a Bot was open.
    pub fn close_bot_view(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.leave_bot_view(cx) {
            return false;
        }
        self.sync_selection(cx);
        true
    }

    /// Leaves the Bot's view without choosing what shows next; the caller
    /// does, and syncs the selection.
    pub(crate) fn leave_bot_view(&mut self, cx: &mut Context<Self>) -> bool {
        if self.bots.view.is_none() {
            return false;
        }
        self.close_bot_settings(cx);
        self.bots.view = None;
        self.bots.overview.shown_session = None;
        self.bots.overview.shown_focused = false;
        self.bots.pending_reset = false;
        self.bots.overview.activity = None;
        self.save_open_bot(cx);
        true
    }

    /// MonoCode `onToggleMono` (⌘I): out of the Bot's view, else into the
    /// last Bot opened, or the first on the rail.
    pub fn toggle_bot(&mut self, cx: &mut Context<Self>) {
        if self.bot_covers() {
            self.close_bot_view(cx);
            self.refocus_prompt(cx);
            return;
        }
        if !self.bots_enabled() {
            return;
        }
        let id = toggle_target(self.bots.last_bot.as_deref(), &self.settings.bots);
        if let Some(id) = id {
            self.open_bot(&id, cx);
        }
    }

    /// MonoCode's reopened Bot: the one in view when BenCode last closed
    /// opens again at launch; one deleted since is forgotten.
    pub(crate) fn reopen_saved_bot(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.settings.open_bot.clone() else {
            return;
        };
        if self.bots_enabled() && self.bot(&id).is_some() {
            self.open_bot(&id, cx);
        } else {
            // Nothing is in view: the saved one is cleared.
            self.save_open_bot(cx);
        }
    }

    /// MonoCode `onToggleMonoOverview`: the title bar's Overview pill.
    pub fn toggle_bot_overview(&mut self, cx: &mut Context<Self>) {
        if self.bots.overview_open {
            // Hiding it leaves what was open over its list.
            self.hide_bot_launch(cx);
            self.bots.overview.document = None;
        }
        self.bots.overview_open = !self.bots.overview_open;
        cx.notify();
    }

    /// MonoCode `onOpenMonoSettings`: the title bar's gear opens the
    /// dialog on General.
    pub fn open_bot_settings(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        let Some(id) = self.bots.view.clone() else {
            return;
        };
        self.bots.settings_open = true;
        self.bots.settings_section = section;
        self.bots.look_open = false;
        self.show_bot_fields(&id, cx);
        cx.notify();
    }

    /// The dialog's close: what its fields hold is saved first. Whether it
    /// was open.
    pub fn close_bot_settings(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.bots.settings_open {
            return false;
        }
        // A new Bot's settings closed without Create: it was never made.
        if let Some(id) = self.bots.creating.take() {
            self.bots.look_open = false;
            self.delete_bot(&id, cx);
            self.discard_bot_files(&id, cx);
            cx.notify();
            return true;
        }
        self.save_bot_soul(false, cx);
        self.commit_bot_name(cx);
        self.bots.settings_open = false;
        self.bots.look_open = false;
        self.bots.files.memory_edit = None;
        cx.notify();
        true
    }

    /// Escape in the dialog: the look popover or the fact field closes
    /// first, then the dialog.
    pub fn close_bot_settings_layer(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.bots.settings_open {
            return false;
        }
        if std::mem::take(&mut self.bots.look_open) {
            cx.notify();
            return true;
        }
        if self.close_memory_field(cx) {
            return true;
        }
        self.close_bot_settings(cx)
    }

    /// A section of the dialog: the soul is saved on the way out of it.
    pub fn select_bot_settings_section(
        &mut self,
        section: SettingsSection,
        cx: &mut Context<Self>,
    ) {
        if self.bots.settings_section == SettingsSection::Soul && section != SettingsSection::Soul {
            self.save_bot_soul(false, cx);
        }
        self.bots.settings_section = section;
        self.bots.files.memory_edit = None;
        cx.notify();
    }

    /// MonoCode `onRuntimeModeChange` for the Bot: its thread's access,
    /// not the default new threads start with.
    pub fn set_bot_runtime_mode(
        &mut self,
        mode: crate::app::PermissionMode,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.bot_view_session().map(str::to_string) else {
            return;
        };
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
            session.runtime_mode = Some(mode.id().to_string());
        }
        self.persist_session(&id);
        cx.notify();
    }

    /// MonoCode `switchMonoUsageLimitAccount`: a Bot stopped by its
    /// account's limit carries on as another account. The provider session
    /// belongs to the old account's profile, so a new one starts, which is
    /// handed the Bot's standing context again; then it resumes.
    pub fn switch_bot_usage_limit_account(
        &mut self,
        session_id: &str,
        account_id: &str,
        cx: &mut Context<Self>,
    ) {
        if self.bot_usage_limited(session_id) {
            self.switch_bot_account(session_id, account_id, cx);
        }
    }

    fn bot_usage_limited(&self, session_id: &str) -> bool {
        self.thread(session_id)
            .is_some_and(|thread| thread.usage_limit.is_some())
    }

    /// The Bot's composer chip picks another account (BenCode's own;
    /// MonoCode switches only at a usage limit). A Claude session's files
    /// are carried to the account's profile and resumed there; otherwise
    /// the next message starts a new provider session, briefed and handed
    /// the standing context again (`plan_bot_rotation`). A Bot stopped by
    /// its limit resumes. Not while a turn runs: its process is the old
    /// account's.
    pub(crate) fn switch_bot_account(
        &mut self,
        session_id: &str,
        account_id: &str,
        cx: &mut Context<Self>,
    ) {
        if self.is_agent_running_in(session_id) || !self.is_bot_session(session_id) {
            return;
        }
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        if !self.account_exists(&session.harness, account_id) {
            return;
        }
        let same = self.session_account_id(session) == account_id;
        if same && session.provider_account_id.is_some() {
            return;
        }
        if same {
            self.move_bot_account(session_id, account_id, true, false, cx);
            return;
        }
        let (sid, account) = (session_id.to_string(), account_id.to_string());
        self.carry_session_to_account(session_id, account_id, cx, move |app, carried, cx| {
            app.move_bot_account(&sid, &account, carried, true, cx)
        });
    }

    /// `switch_bot_account` once the session's files are or are not on
    /// the account: without them the provider session starts afresh, and
    /// the rotation briefs it.
    fn move_bot_account(
        &mut self,
        session_id: &str,
        account_id: &str,
        carried: bool,
        moved: bool,
        cx: &mut Context<Self>,
    ) {
        if self.is_agent_running_in(session_id) {
            return;
        }
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else {
            return;
        };
        session.provider_account_id = Some(account_id.to_string());
        if !carried {
            session.provider_session_id = None;
            session.context_used = None;
            session.context_window = None;
        }
        self.persist_session(session_id);
        if moved && self.bot_usage_limited(session_id) {
            self.resume_after_limit(session_id, cx);
        }
        cx.notify();
    }

    /// The Bot's greeting fills its chat: it has no message yet. Its
    /// figure bobs, so the clock wakes for it (`start_clock`).
    pub fn bot_greeting_shown(&self) -> bool {
        self.bot_covers()
            && self
                .bot_view_session()
                .and_then(|id| self.sessions.iter().find(|s| s.id == id))
                .is_some_and(|session| !session.blocks.iter().any(|b| b.role == "user"))
    }

    /// The floating chat's window, while it is open.
    pub(crate) fn bot_float_window(
        &self,
    ) -> Option<gpui::WindowHandle<crate::ui::bot_float::BotFloat>> {
        self.bots.float
    }

    /// The figures drawn now that move with no agent running (one that
    /// needs you, a finished turn's sparks, the greeting's and the settings
    /// face's breath): when the app's clock must wake for them next, if at
    /// all.
    pub fn idle_figure_wake(&self) -> Option<std::time::Duration> {
        let mut looks = Vec::new();
        if let Some(bot) = self.bot_in_view().filter(|_| self.bot_covers()) {
            let look = self.bot_look(bot);
            if self.bot_greeting_shown() {
                looks.push(look.lively());
            }
            if self.bots.settings_open {
                looks.push(Look::new(Mood::Idle, &bot.id).lively());
            }
        }
        if self.bots_enabled() {
            looks.extend(self.settings.bots.iter().map(|bot| self.bot_look(bot)));
        }
        looks
            .into_iter()
            .filter(|look| look.moves_at_rest())
            .map(Look::next_change)
            .min()
    }

    /// MonoCode `loadMonosEnabled`.
    pub fn bots_enabled(&self) -> bool {
        !self.settings.bots_off
    }

    /// MonoCode `removeMono`: its conversation goes with it; its folder of
    /// files stays on disk.
    pub fn delete_bot(&mut self, id: &str, cx: &mut Context<Self>) {
        self.bots.pending_delete = None;
        if self.bots.creating.as_deref() == Some(id) {
            self.bots.creating = None;
        }
        let Some(bot) = self.bot(id).cloned() else {
            return;
        };
        if self.bots.view.as_deref() == Some(id) {
            // Its fields are not saved on the way out: the Bot is going.
            self.bots.settings_open = false;
            self.bots.files.memory_edit = None;
            self.bots.view = None;
            self.save_open_bot(cx);
        }
        self.bots.files.loaded.remove(id);
        if self.bot_backgrounds.dialog.as_deref() == Some(id) {
            self.bot_backgrounds.dialog = None;
        }
        if bot.background.is_some() {
            self.remove_bot_background(id, cx);
        }
        self.update_bots(|roster| roster.retain(|bot| bot.id != id), cx);
        match bot.session_id {
            Some(session_id) => self.drop_bot_session(&session_id, cx),
            None => self.sync_selection(cx),
        }
    }

    /// Stops and deletes a conversation that was a Bot's.
    fn drop_bot_session(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if self.is_agent_running_in(session_id) {
            self.stop_agent(session_id, cx);
        }
        if self.sessions.iter().any(|s| s.id == session_id) {
            self.delete_session(session_id, cx);
            return;
        }
        // Never loaded this launch: only its row is left.
        let row = session_id.to_string();
        self.db_write("delete bot session", move |db| db.delete_session(&row));
        self.sync_selection(cx);
    }

    /// MonoCode `resetMonoSession`: a fresh chat with the same harness,
    /// model and permissions; its name, look, projects and soul stay.
    pub fn reset_bot_session(&mut self, cx: &mut Context<Self>) {
        self.bots.pending_reset = false;
        let Some(bot) = self.bot_in_view().cloned() else {
            return;
        };
        // What carries over, not the whole row: its blocks can be huge.
        let Some((old_id, cwd, harness, model, runtime_mode, model_settings, account)) = bot
            .session_id
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| s.id == id))
            .map(|s| {
                (
                    s.id.clone(),
                    s.cwd.clone(),
                    s.harness.clone(),
                    s.model.clone(),
                    s.runtime_mode.clone(),
                    s.model_settings.clone(),
                    s.provider_account_id.clone(),
                )
            })
        else {
            return;
        };
        let fresh = self.create_session_row(&cwd, cx);
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == fresh) {
            session.harness = harness;
            session.model = model;
            session.runtime_mode = runtime_mode;
            session.model_settings = model_settings;
            session.provider_account_id = account;
            session.worktree_cwd = None;
            session.branch = None;
        }
        self.persist_session(&fresh);
        self.update_bots(
            |roster| {
                if let Some(entry) = roster.iter_mut().find(|entry| entry.id == bot.id) {
                    entry.session_id = Some(fresh.clone());
                    entry.handed = None;
                    entry.rotation = None;
                }
            },
            cx,
        );
        self.drop_bot_session(&old_id, cx);
    }

    pub fn set_bot_name(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        let name: String = name.trim().chars().take(crate::bots::NAME_MAX).collect();
        self.update_bots(
            |roster| {
                if let Some(bot) = roster.iter_mut().find(|bot| bot.id == id) {
                    bot.name = Some(name).filter(|name| !name.is_empty());
                }
            },
            cx,
        );
        // Its composer says "Message {name}".
        self.sync_prompt_placeholder(cx);
    }

    /// MonoCode `saveMonoMascot`, for a figure.
    pub fn set_bot_figure(&mut self, id: &str, figure: Figure, cx: &mut Context<Self>) {
        self.update_bots(
            |roster| {
                if let Some(bot) = roster.iter_mut().find(|bot| bot.id == id) {
                    bot.figure = Some(figure.code());
                }
            },
            cx,
        );
    }

    /// A palette pick; it replaces a custom colour.
    pub fn set_bot_color(&mut self, id: &str, index: usize, cx: &mut Context<Self>) {
        self.update_bots(
            |roster| {
                if let Some(bot) = roster.iter_mut().find(|bot| bot.id == id) {
                    bot.color_index = index;
                    bot.custom_color = None;
                }
            },
            cx,
        );
    }

    /// The projects on the rail the Bot has not been given yet.
    pub fn bot_project_choices(&self, bot: &Bot) -> Vec<String> {
        self.rail_order()
            .into_iter()
            .filter(|path| !bot.works_on(path))
            .collect()
    }

    /// MonoCode `addMonoProject`.
    pub fn add_bot_project(&mut self, id: &str, path: &str, cx: &mut Context<Self>) {
        let path = crate::app::normalize_project_path(path);
        if path.is_empty() || path == "~" {
            return;
        }
        self.update_bots(
            |roster| {
                if let Some(bot) = roster.iter_mut().find(|bot| bot.id == id)
                    && !bot.works_on(&path)
                {
                    bot.projects.push(path);
                }
            },
            cx,
        );
    }

    /// MonoCode `removeMonoProject`.
    pub fn remove_bot_project(&mut self, id: &str, path: &str, cx: &mut Context<Self>) {
        self.update_bots(
            |roster| {
                if let Some(bot) = roster.iter_mut().find(|bot| bot.id == id) {
                    bot.projects.retain(|p| !same_project_path(p, path));
                }
            },
            cx,
        );
    }

    /// "Choose folder…": any folder becomes one of the Bot's projects.
    pub fn pick_bot_project(&mut self, id: &str, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add Project".into()),
        });
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(err)) => {
                    log::error!("folder picker failed: {err:#}");
                    return;
                }
            };
            let added = this.update(cx, |this, cx| {
                for path in paths {
                    this.add_bot_project(&id, &path.to_string_lossy(), cx);
                }
            });
            if let Err(err) = added {
                log::debug!("folder picked after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `monoTurn` over `monoContext`: a Bot's turn carries who it
    /// is, its standing instructions and its memory, each unless the
    /// provider session behind its chat already holds that version. Other
    /// threads' prompts pass through.
    pub(crate) fn apply_bot_context(
        &mut self,
        session_id: &str,
        prompt: String,
        cx: &mut Context<Self>,
    ) -> String {
        // MonoCode keeps a raw slash command first only for harnesses that
        // take them typed (`isNativeCommandPrompt`); none of BenCode's do, so
        // every Bot turn carries it.
        let Some(bot) = self.bot_for_session(session_id).cloned() else {
            return prompt;
        };
        let brief = self.plan_bot_rotation(&bot, session_id, cx);
        let bot = self.bot(&bot.id).cloned().unwrap_or(bot);
        let provider = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .and_then(|s| s.provider_session_id.clone());
        let provider = provider.as_deref();
        let files = self.bot_turn_files(&bot.id);
        let (soul, memory) = self.bot_context_parts(&bot, &files, self.has_app_access(session_id));
        let carried = Carried {
            soul_hash: crate::bots::context_hash(&soul),
            memory_hash: files.memory_hash.clone(),
        };
        let provider = provider.filter(|id| !id.is_empty());
        let plan = crate::bots::plan_context(
            bot.handed.as_ref(),
            provider,
            &carried.soul_hash,
            &carried.memory_hash,
        );
        // MonoCode `unseenHabitPosts`: what its habits posted since the
        // user last wrote, which the provider session never saw. Read
        // before the message just sent, which would end the search at once.
        let away = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| {
                let before = s.blocks.len().saturating_sub(1);
                crate::bots::habit_posts_context(&s.blocks[..before])
            })
            .unwrap_or_default();
        if !plan.any() && away.is_empty() && brief.is_none() {
            return prompt;
        }
        if plan.any() {
            self.bots.carrying.insert(session_id.to_string(), carried);
        }
        let context = [
            brief.unwrap_or_default(),
            away,
            crate::bots::turn_context(plan, &soul, &memory),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
        crate::bots::turn_with_context(&prompt, &context)
    }

    /// MonoCode's rotation, before a Bot's turn: past 80% of its window the
    /// provider session is let go, and a fresh one (that, or one a switch
    /// of account started) is briefed on the conversation so far.
    fn plan_bot_rotation(
        &mut self,
        bot: &crate::bots::Bot,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let session = self.sessions.iter().find(|s| s.id == session_id)?;
        let rotate = crate::bots::rotation::should_rotate(
            session.provider_session_id.as_deref(),
            session.context_used,
            session.context_window,
        );
        // A provider session that went for another reason (a switch of
        // account or model) had seen the chat: the new one has not.
        let fresh = session
            .provider_session_id
            .as_deref()
            .is_none_or(str::is_empty);
        let replaced = rotate || (fresh && bot.handed.is_some());
        // Without the turn just started.
        let before = session.blocks.len().saturating_sub(1);
        if !replaced || !session.blocks[..before].iter().any(|b| b.role == "user") {
            return None;
        }
        let reason = if rotate { "context" } else { "restart" };
        let (rotation, brief) = crate::bots::rotation::plan_rotation(
            &session.blocks[..before],
            bot.rotation.as_ref(),
            reason,
            crate::app::now_ms(),
            session_id,
        );
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            session.provider_session_id = None;
            session.context_used = None;
            session.context_window = None;
        }
        let id = bot.id.clone();
        self.update_bots(
            |roster| {
                if let Some(entry) = roster.iter_mut().find(|m| m.id == id) {
                    entry.rotation = Some(rotation);
                    // The fresh session holds none of it yet.
                    entry.handed = None;
                }
            },
            cx,
        );
        Some(brief)
    }

    /// MonoCode `monoContext`'s two parts for `bot`: who it is (with the
    /// app CLI's rules when its turns get the CLI) and its memory block.
    pub(crate) fn bot_context_parts(
        &self,
        bot: &crate::bots::Bot,
        files: &crate::bots::files::BotFiles,
        app: bool,
    ) -> (String, String) {
        // The CLI's path is part of who it is: a move hands it over again.
        let app_cli = app.then(crate::app_cli::command);
        let soul = crate::bots::soul_context(
            &bot.display_name(),
            &self.bot_projects(bot),
            &files.soul,
            app_cli.as_deref(),
        );
        let topics_dir = crate::app::bot_files::bot_files_dir(&bot.id)
            .map(|dir| dir.join("memory").to_string_lossy().into_owned())
            .filter(|_| !app);
        let memory =
            crate::bots::memory_context(&files.memory, &files.topics, topics_dir.as_deref());
        (soul, memory)
    }

    /// A turn that never finished (stopped, or never started) hands nothing over.
    pub(crate) fn drop_bot_context(&mut self, session_id: &str) {
        self.bots.carrying.remove(session_id);
    }

    /// After a compaction the provider session holds a summary, which may
    /// have dropped who the Bot is: the next turn carries it again.
    pub(crate) fn forget_bot_context(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(id) = self.bot_for_session(session_id).map(|bot| bot.id.clone()) else {
            return;
        };
        self.update_bots(
            |roster| {
                if let Some(entry) = roster.iter_mut().find(|entry| entry.id == id) {
                    entry.handed = None;
                }
            },
            cx,
        );
    }

    /// MonoCode `recordAgentContext`, once a Bot's turn has ended: the
    /// provider session it left behind holds what the turn carried. A turn
    /// that never reached a provider session records nothing, so the next
    /// one carries the context again.
    pub(crate) fn record_bot_context(
        &mut self,
        session_id: &str,
        accepted: bool,
        cx: &mut Context<Self>,
    ) {
        let carried = self.bots.carrying.remove(session_id);
        // MonoCode `useMonoFiles`: a turn ending is when its files, and the
        // projects it works in, may have changed. The files are read again
        // wherever they are held, as the next turn carries them (a Bot
        // without the app CLI edits them on disk); the Overview only while
        // it is shown.
        if let Some(id) = self.bot_for_session(session_id).map(|m| m.id.clone()) {
            self.bot_files_changed(&id, cx);
            if self.bots.view.as_deref() == Some(id.as_str()) {
                self.load_bot_overview(&id, cx);
                self.load_bot_changes(cx);
            }
        }
        // MonoCode records what a turn the provider accepted carried: one
        // that failed or was cut short may never have reached it.
        if !accepted {
            return;
        }
        let Some(bot) = self.bot_for_session(session_id) else {
            return;
        };
        let Some(provider) = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .and_then(|s| s.provider_session_id.clone())
            .filter(|id| !id.is_empty())
        else {
            return;
        };
        // A provider that renames its session each turn keeps what it held.
        let Some(carried) = carried.or_else(|| {
            bot.handed.as_ref().map(|h| Carried {
                soul_hash: h.context_hash.clone(),
                memory_hash: h.memory_hash.clone(),
            })
        }) else {
            return;
        };
        let (id, handed) = (
            bot.id.clone(),
            Handed {
                provider: Some(provider),
                context_hash: carried.soul_hash,
                memory_hash: carried.memory_hash,
            },
        );
        self.update_bots(
            |roster| {
                if let Some(entry) = roster.iter_mut().find(|entry| entry.id == id) {
                    entry.handed = Some(handed);
                }
            },
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortcut_opens_the_last_bot_or_the_first() {
        let bot = |id: &str| Bot {
            id: id.into(),
            ..Default::default()
        };
        let roster = [bot("a"), bot("b")];
        assert_eq!(toggle_target(Some("b"), &roster).as_deref(), Some("b"));
        assert_eq!(toggle_target(Some("gone"), &roster).as_deref(), Some("a"));
        assert_eq!(toggle_target(None, &roster).as_deref(), Some("a"));
        assert_eq!(toggle_target(None, &[]), None);
    }
}
