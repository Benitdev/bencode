//! Monos on the app (MonoCode `App.tsx`'s `monoViewId`, `monoWorkspace.ts`,
//! `monoFiles.ts`): the roster in `settings.json`, the Mono whose
//! conversation covers the workspace, its Overview beside the chat and its
//! settings dialog, and the standing context a turn carries (its soul and
//! its memory). Its files are `mono_files.rs`, its Overview's state
//! `mono_overview.rs`; the model is in `crate::monos`.

use std::collections::HashMap;

use gpui::{Context, Hsla, PathPromptOptions, Subscription, Window};

use crate::app::mono_files::FilesState;
use crate::app::mono_overview::OverviewState;
use crate::app::session_folders::parse_hex;
use crate::app::{BenCodeApp, same_project_path, unique_id};
use crate::monos::{Handed, Mono, MonoState, MonoStatus};
use crate::ui::mascot::{Mascot, mascot_for};

/// MonoCode `ensureMonoSession`: a Mono's chat starts with Auto permissions.
const MONO_RUNTIME_MODE: &str = "auto";

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

pub struct MonosState {
    /// The Mono whose conversation covers the workspace.
    pub view: Option<String>,
    /// Replies each Mono's conversation has that the user has not seen.
    pub unread: HashMap<String, usize>,
    /// MonoCode `monoOverviewOpen`: the title bar's Overview pill. A Mono
    /// opens with its Overview beside the chat until it is hidden.
    pub overview_open: bool,
    /// MonoCode `monoSettingsFor`: the settings dialog is open, on this
    /// section.
    pub settings_open: bool,
    pub settings_section: SettingsSection,
    /// The dialog's face opens its mascots and colours (MonoCode
    /// `LookButton`).
    pub look_open: bool,
    /// MonoCode `lastMonoIdRef`: the last Mono opened, which ⌘I brings back.
    pub last_mono: Option<String>,
    /// The Mono composer's emoji picker (MonoCode `EmojiPickerPopover`).
    pub emoji: crate::ui::emoji_picker::EmojiPicker,
    pub files: FilesState,
    pub overview: OverviewState,
    pub changes: crate::app::mono_changes::ChangesState,
    /// Mono id awaiting delete confirmation.
    pub pending_delete: Option<String>,
    /// A Mono the rail's + made, kept only once its settings say Create:
    /// closing them otherwise takes it away.
    pub creating: Option<String>,
    /// The open Mono's chat is awaiting a reset confirmation.
    pub pending_reset: bool,
    /// Mono id awaiting Settings › Monos' Reset Mono confirmation.
    pub pending_defaults: Option<String>,
    /// Why the last reset could not finish.
    pub reset_error: Option<String>,
    /// Why a Mono's conversation could not be read, by Mono; the floating
    /// chat shows it in place of "Loading conversation…".
    pub load_errors: HashMap<String, String>,
    /// The floating chat, while its window is open.
    float: Option<gpui::WindowHandle<crate::ui::mono_float::MonoFloat>>,
    /// The context a running turn carries, by conversation: recorded as
    /// handed over once the turn ends with a provider session to name.
    carrying: HashMap<String, Carried>,
    _subscriptions: Vec<Subscription>,
}

/// MonoCode `onToggleMono`'s pick: the last Mono opened while it is still
/// on the roster, else the first.
pub fn toggle_target(last: Option<&str>, roster: &[Mono]) -> Option<String> {
    last.filter(|id| roster.iter().any(|mono| mono.id == *id))
        .map(str::to_string)
        .or_else(|| roster.first().map(|mono| mono.id.clone()))
}

impl MonosState {
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> Self {
        let (files, subscriptions) = FilesState::new(window, cx);
        let changes = crate::app::mono_changes::ChangesState::new(window, cx);
        Self {
            view: None,
            overview_open: true,
            settings_open: false,
            settings_section: SettingsSection::General,
            look_open: false,
            last_mono: None,
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
            _subscriptions: subscriptions,
        }
    }
}

impl BenCodeApp {
    pub fn mono(&self, id: &str) -> Option<&Mono> {
        self.settings.monos.iter().find(|mono| mono.id == id)
    }

    /// MonoCode `monoForSession`: the Mono whose conversation this is.
    pub fn mono_for_session(&self, session_id: &str) -> Option<&Mono> {
        self.settings
            .monos
            .iter()
            .find(|mono| mono.session_id.as_deref() == Some(session_id))
    }

    pub fn is_mono_session(&self, session_id: &str) -> bool {
        self.mono_for_session(session_id).is_some()
    }

    /// MonoCode `useUnseenReplyCounts`: replies a Mono finished (or its
    /// habits posted) while the user was not looking at it.
    pub fn mono_unread(&self, mono: &Mono) -> Option<usize> {
        let session = mono.session_id.as_deref()?;
        self.monos.unread.get(session).copied().filter(|n| *n > 0)
    }

    /// MonoCode `noteReply`: one more unread reply, unless it is in view.
    pub(crate) fn note_mono_reply(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if !self.is_mono_session(session_id) || self.mono_view_session() == Some(session_id) {
            return;
        }
        *self.monos.unread.entry(session_id.to_string()).or_default() += 1;
        cx.notify();
    }

    /// The Mono open in the main area.
    pub fn mono_in_view(&self) -> Option<&Mono> {
        self.mono(self.monos.view.as_deref()?)
    }

    /// The conversation the open Mono shows, once it is loaded.
    pub fn mono_view_session(&self) -> Option<&str> {
        let id = self.mono_in_view()?.session_id.as_deref()?;
        self.sessions.iter().any(|s| s.id == id).then_some(id)
    }

    /// A Mono's colour: its custom one, else its palette pick.
    pub fn mono_color(&self, mono: &Mono) -> Hsla {
        mono.custom_color
            .as_deref()
            .and_then(parse_hex)
            .or_else(|| crate::monos::mono_palette_color(mono.color_index))
            .or_else(|| crate::monos::mono_palette_color(1))
            .unwrap_or_default()
    }

    pub fn mono_mascot(&self, mono: &Mono) -> &'static Mascot {
        mascot_for(&mono.id, Some(&mono.mascot))
    }

    /// MonoCode `monoLook().projects`: its projects as (rail label, folder).
    pub fn mono_projects(&self, mono: &Mono) -> Vec<(String, String)> {
        mono.projects
            .iter()
            .map(|path| (self.rail_project_label(path), path.clone()))
            .collect()
    }

    /// MonoCode `monoState`: working, waiting on the user, or idle.
    pub fn mono_state(&self, mono: &Mono) -> MonoState {
        let Some(session) = mono
            .session_id
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| s.id == id))
        else {
            return MonoState::IDLE;
        };
        if let Some(request) = self.pending_permission_for(&session.id) {
            let activity = if request.tool == crate::app::QUESTION_TOOL {
                crate::monos::question_activity(&request.input)
            } else {
                Some(crate::monos::approval_activity(
                    &crate::harness::summarize_tool_input(&request.tool, &request.input),
                ))
            };
            return MonoState {
                status: MonoStatus::NeedsYou,
                activity,
            };
        }
        if self
            .thread(&session.id)
            .is_some_and(|thread| thread.usage_limit.is_some())
        {
            return MonoState {
                status: MonoStatus::NeedsYou,
                activity: Some("Usage limit reached".to_string()),
            };
        }
        if !self.is_agent_running_in(&session.id) {
            return MonoState::IDLE;
        }
        MonoState {
            status: MonoStatus::Working,
            activity: Some(crate::monos::working_activity(&session.blocks)),
        }
    }

    /// MonoCode `createMono` then `onOpenMono`: a new Mono, opened.
    pub fn create_mono(&mut self, cx: &mut Context<Self>) {
        let id = self.add_mono(cx);
        self.open_mono(&id, cx);
        // Its settings open with it, ready to be told what it works on (a
        // new Mono has no conversation to read, so it is in view already).
        if self.monos.view.as_deref() == Some(id.as_str()) {
            self.open_mono_settings(SettingsSection::General, cx);
            self.monos.creating = Some(id);
        }
    }

    /// The new Mono's Create: it stays, with what its settings hold.
    pub fn confirm_new_mono(&mut self, cx: &mut Context<Self>) {
        self.monos.creating = None;
        self.close_mono_settings(cx);
    }

    /// MonoCode `createMono`: a new Mono on the roster; its id.
    pub fn add_mono(&mut self, cx: &mut Context<Self>) -> String {
        let id = unique_id("mono");
        let mono = crate::monos::new_mono(&self.settings.monos, id.clone());
        self.update_monos(|roster| roster.push(mono), cx);
        id
    }

    /// MonoCode `onOpenMono`: the Mono's conversation takes the main area,
    /// read from the database or started if it has none yet.
    pub fn open_mono(&mut self, id: &str, cx: &mut Context<Self>) {
        self.close_rail_menu(cx);
        self.ensure_mono_session(id, true, cx);
    }

    /// MonoCode `ensureMonoSession`: the Mono's conversation in memory,
    /// without showing it (the floating chat reads it from there).
    pub fn prepare_mono_session(&mut self, id: &str, cx: &mut Context<Self>) {
        self.ensure_mono_session(id, false, cx);
    }

    fn ensure_mono_session(&mut self, id: &str, show: bool, cx: &mut Context<Self>) {
        let Some(session) = self.mono(id).map(|mono| mono.session_id.clone()) else {
            return;
        };
        self.warm_mono_files(id, cx);
        let Some(session_id) = session else {
            self.start_mono_session(id, show, cx);
            return;
        };
        if self.sessions.iter().any(|s| s.id == session_id) {
            if show {
                self.show_mono(id, cx);
            }
            return;
        }
        let (mono_id, row) = (id.to_string(), session_id.clone());
        self.db_then(
            cx,
            move |db| db.get_session(&row),
            move |this, loaded, cx| {
                // Deleted or reset while it loaded: no longer its chat, and
                // nothing to bring back or start over.
                let Some(mono) = this.mono(&mono_id) else {
                    return;
                };
                if mono.session_id.as_deref() != Some(session_id.as_str()) {
                    if show {
                        this.open_mono(&mono_id, cx);
                    }
                    return;
                }
                if loaded.is_ok() {
                    this.monos.load_errors.remove(&mono_id);
                }
                match loaded {
                    Ok(Some(session)) => {
                        if !this.sessions.iter().any(|s| s.id == session.id) {
                            this.sessions.push(session);
                        }
                        if show {
                            this.show_mono(&mono_id, cx);
                        }
                        cx.notify();
                    }
                    // Deleted since: it starts over, as a Mono first opened does.
                    Ok(None) => this.start_mono_session(&mono_id, show, cx),
                    Err(err) => {
                        log::error!("could not load the Mono's conversation: {err:#}");
                        this.monos
                            .load_errors
                            .insert(mono_id, format!("Could not load the conversation: {err}"));
                        cx.notify();
                    }
                }
            },
        );
    }

    /// A new conversation for the Mono, in the home folder: no single
    /// project is a Mono's own.
    fn start_mono_session(&mut self, id: &str, show: bool, cx: &mut Context<Self>) {
        let Some(home) = super::projects::home_dir() else {
            log::error!("no home folder to start a Mono in");
            return;
        };
        let session_id = self.create_session_row(&home, cx);
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            session.runtime_mode = Some(MONO_RUNTIME_MODE.to_string());
            session.worktree_cwd = None;
            session.branch = None;
        }
        self.persist_session(&session_id);
        self.update_monos(
            |roster| {
                if let Some(mono) = roster.iter_mut().find(|mono| mono.id == id) {
                    mono.session_id = Some(session_id.clone());
                    mono.handed = None;
                    mono.rotation = None;
                }
            },
            cx,
        );
        if show {
            self.show_mono(id, cx);
        }
    }

    /// MonoCode `FloatingMonoChat`: the Mono's chat in the window that
    /// floats above the others, opened or brought forward on this Mono.
    pub fn open_mono_float(&mut self, id: &str, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.open_mono_float_over(id, window.window_handle(), cx);
    }

    /// `open_mono_float` from outside BenCode's window (the menu bar), with
    /// the window the float belongs to.
    pub fn open_mono_float_over(
        &mut self,
        id: &str,
        main: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if self.mono(id).is_none() {
            return;
        }
        self.close_rail_menu(cx);
        self.prepare_mono_session(id, cx);
        let (app, mono_id) = (cx.entity(), id.to_string());
        let existing = self.monos.float.take();
        // Opening a window updates the app it is drawn from: after this one.
        cx.defer(move |cx| {
            let shown = existing.and_then(|float| {
                float
                    .update(cx, |float, window, cx| {
                        float.show(&mono_id, window, cx);
                        window.activate_window();
                    })
                    .ok()
                    .map(|()| float)
            });
            let float = match shown {
                Some(float) => Some(float),
                // Never opened, or closed since.
                None => crate::ui::mono_float::open(app.clone(), main, mono_id, cx)
                    .map_err(|err| log::error!("could not open the floating chat: {err:#}"))
                    .ok(),
            };
            app.update(cx, |this, _| this.monos.float = float);
        });
    }

    /// A message typed in the floating chat: into the running turn where
    /// the agent takes follow-ups, queued behind it otherwise, else a turn
    /// of its own (the composer's own rules, `submit_prompt`).
    /// Whether it was taken: a thread that cannot run keeps the message
    /// and its files where they were (MonoCode's "could not be sent").
    pub fn send_mono_message(
        &mut self,
        session_id: &str,
        text: String,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if let Some(why) = self.turn_blocked(session_id) {
            log::warn!("Mono thread {session_id} {why}; message not sent");
            return Err("The message could not be sent. Try again in a moment.".to_string());
        }
        // With the files waiting in the thread's composer, as Send takes them.
        let attachments = self
            .threads
            .get_mut(session_id)
            .map(|thread| std::mem::take(&mut thread.attachments))
            .unwrap_or_default();
        let input = crate::app::TurnInput {
            text,
            attachments,
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

    fn show_mono(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.surface == Some(crate::app::Surface::Notes) {
            self.save_note_if_dirty(cx);
        }
        self.surface = None;
        self.settings_return = None;
        let another = self.monos.view.as_deref() != Some(id);
        if another {
            // Another Mono's view closes the last one's dialog.
            self.close_mono_settings(cx);
            self.monos.overview.mono_changed();
        }
        self.monos.view = Some(id.to_string());
        self.monos.last_mono = Some(id.to_string());
        if let Some(session) = self.mono(id).and_then(|m| m.session_id.clone()) {
            self.monos.unread.remove(&session);
        }
        self.monos.pending_reset = false;
        self.monos.files.load_error = None;
        self.load_mono_files(id, cx);
        self.load_mono_overview(id, cx);
        self.load_mono_changes(cx);
        self.load_mono_project_sessions(cx);
        self.save_open_mono(cx);
        self.sync_selection(cx);
    }

    /// MonoCode `closeMonoView`: back to the project's tabs. Returns whether
    /// a Mono was open.
    pub fn close_mono_view(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.leave_mono_view(cx) {
            return false;
        }
        self.sync_selection(cx);
        true
    }

    /// Leaves the Mono's view without choosing what shows next; the caller
    /// does, and syncs the selection.
    pub(crate) fn leave_mono_view(&mut self, cx: &mut Context<Self>) -> bool {
        if self.monos.view.is_none() {
            return false;
        }
        self.close_mono_settings(cx);
        self.monos.view = None;
        self.monos.overview.shown_session = None;
        self.monos.overview.shown_focused = false;
        self.monos.pending_reset = false;
        self.monos.overview.activity = None;
        self.save_open_mono(cx);
        true
    }

    /// MonoCode `onToggleMono` (⌘I): out of the Mono's view, else into the
    /// last Mono opened, or the first on the rail.
    pub fn toggle_mono(&mut self, cx: &mut Context<Self>) {
        if self.mono_covers() {
            self.close_mono_view(cx);
            self.refocus_prompt(cx);
            return;
        }
        if !self.monos_enabled() {
            return;
        }
        let id = toggle_target(self.monos.last_mono.as_deref(), &self.settings.monos);
        if let Some(id) = id {
            self.open_mono(&id, cx);
        }
    }

    /// MonoCode's reopened Mono: the one in view when BenCode last closed
    /// opens again at launch; one deleted since is forgotten.
    pub(crate) fn reopen_saved_mono(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.settings.open_mono.clone() else {
            return;
        };
        if self.monos_enabled() && self.mono(&id).is_some() {
            self.open_mono(&id, cx);
        } else {
            // Nothing is in view: the saved one is cleared.
            self.save_open_mono(cx);
        }
    }

    /// MonoCode `onToggleMonoOverview`: the title bar's Overview pill.
    pub fn toggle_mono_overview(&mut self, cx: &mut Context<Self>) {
        if self.monos.overview_open {
            // Hiding it leaves what was open over its list.
            self.hide_mono_launch(cx);
            self.monos.overview.document = None;
        }
        self.monos.overview_open = !self.monos.overview_open;
        cx.notify();
    }

    /// MonoCode `onOpenMonoSettings`: the title bar's gear opens the
    /// dialog on General.
    pub fn open_mono_settings(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        let Some(id) = self.monos.view.clone() else {
            return;
        };
        self.monos.settings_open = true;
        self.monos.settings_section = section;
        self.monos.look_open = false;
        self.show_mono_fields(&id, cx);
        cx.notify();
    }

    /// The dialog's close: what its fields hold is saved first. Whether it
    /// was open.
    pub fn close_mono_settings(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.monos.settings_open {
            return false;
        }
        // A new Mono's settings closed without Create: it was never made.
        if let Some(id) = self.monos.creating.take() {
            self.monos.look_open = false;
            self.delete_mono(&id, cx);
            self.discard_mono_files(&id, cx);
            cx.notify();
            return true;
        }
        self.save_mono_soul(false, cx);
        self.commit_mono_name(cx);
        self.monos.settings_open = false;
        self.monos.look_open = false;
        self.monos.files.memory_edit = None;
        cx.notify();
        true
    }

    /// Escape in the dialog: the look popover or the fact field closes
    /// first, then the dialog.
    pub fn close_mono_settings_layer(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.monos.settings_open {
            return false;
        }
        if std::mem::take(&mut self.monos.look_open) {
            cx.notify();
            return true;
        }
        if self.close_memory_field(cx) {
            return true;
        }
        self.close_mono_settings(cx)
    }

    /// A section of the dialog: the soul is saved on the way out of it.
    pub fn select_mono_settings_section(
        &mut self,
        section: SettingsSection,
        cx: &mut Context<Self>,
    ) {
        if self.monos.settings_section == SettingsSection::Soul && section != SettingsSection::Soul
        {
            self.save_mono_soul(false, cx);
        }
        self.monos.settings_section = section;
        self.monos.files.memory_edit = None;
        cx.notify();
    }

    /// MonoCode `onRuntimeModeChange` for the Mono: its thread's access,
    /// not the default new threads start with.
    pub fn set_mono_runtime_mode(
        &mut self,
        mode: crate::app::PermissionMode,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.mono_view_session().map(str::to_string) else {
            return;
        };
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
            session.runtime_mode = Some(mode.id().to_string());
        }
        self.persist_session(&id);
        cx.notify();
    }

    /// MonoCode `switchMonoUsageLimitAccount`: a Mono stopped by its
    /// account's limit carries on as another account. The provider session
    /// belongs to the old account's profile, so a new one starts, which is
    /// handed the Mono's standing context again; then it resumes.
    pub fn switch_mono_usage_limit_account(
        &mut self,
        session_id: &str,
        account_id: &str,
        cx: &mut Context<Self>,
    ) {
        if self.mono_usage_limited(session_id) {
            self.switch_mono_account(session_id, account_id, cx);
        }
    }

    fn mono_usage_limited(&self, session_id: &str) -> bool {
        self.thread(session_id)
            .is_some_and(|thread| thread.usage_limit.is_some())
    }

    /// The Mono's composer chip picks another account (BenCode's own;
    /// MonoCode switches only at a usage limit). A Claude session's files
    /// are carried to the account's profile and resumed there; otherwise
    /// the next message starts a new provider session, briefed and handed
    /// the standing context again (`plan_mono_rotation`). A Mono stopped by
    /// its limit resumes. Not while a turn runs: its process is the old
    /// account's.
    pub(crate) fn switch_mono_account(
        &mut self,
        session_id: &str,
        account_id: &str,
        cx: &mut Context<Self>,
    ) {
        if self.is_agent_running_in(session_id) || !self.is_mono_session(session_id) {
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
            self.move_mono_account(session_id, account_id, true, false, cx);
            return;
        }
        let (sid, account) = (session_id.to_string(), account_id.to_string());
        self.carry_session_to_account(session_id, account_id, cx, move |app, carried, cx| {
            app.move_mono_account(&sid, &account, carried, true, cx)
        });
    }

    /// `switch_mono_account` once the session's files are or are not on
    /// the account: without them the provider session starts afresh, and
    /// the rotation briefs it.
    fn move_mono_account(
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
        if moved && self.mono_usage_limited(session_id) {
            self.resume_after_limit(session_id, cx);
        }
        cx.notify();
    }

    /// The Mono's greeting fills its chat: it has no message yet. Its
    /// mascot bobs, so the clock wakes for it (`start_clock`).
    pub fn mono_greeting_shown(&self) -> bool {
        self.mono_covers()
            && self
                .mono_view_session()
                .and_then(|id| self.sessions.iter().find(|s| s.id == id))
                .is_some_and(|session| !session.blocks.iter().any(|b| b.role == "user"))
    }

    /// The mascots drawn now that move with no agent running (MonoCode
    /// `PixelMascot`'s idle bob and blink, the bulb over one that needs
    /// you): when the app's clock must wake for them next, if at all.
    /// The floating chat's window, while it is open.
    pub(crate) fn mono_float_window(
        &self,
    ) -> Option<gpui::WindowHandle<crate::ui::mono_float::MonoFloat>> {
        self.monos.float
    }

    pub fn idle_mascot_wake(&self) -> Option<std::time::Duration> {
        use crate::ui::mascot::MascotLook;
        let mut looks = Vec::new();
        if let Some(mono) = self.mono_in_view().filter(|_| self.mono_covers()) {
            let status = self.mono_state(mono).status;
            looks.push(MascotLook::for_status(status, &mono.mascot));
            if self.mono_greeting_shown() {
                looks.push(MascotLook::greeting(status, &mono.mascot));
            }
            if self.monos.settings_open {
                looks.push(MascotLook::idle(&mono.mascot));
            }
        }
        if self.monos_enabled() {
            looks.extend(self.settings.monos.iter().filter_map(|mono| {
                let status = self.mono_state(mono).status;
                (status == MonoStatus::NeedsYou)
                    .then(|| MascotLook::for_status(status, &mono.mascot))
            }));
        }
        looks
            .into_iter()
            .filter(|look| look.moves_at_rest())
            .map(MascotLook::next_change)
            .min()
    }

    /// MonoCode `loadMonosEnabled`.
    pub fn monos_enabled(&self) -> bool {
        !self.settings.monos_off
    }

    /// MonoCode `removeMono`: its conversation goes with it; its folder of
    /// files stays on disk.
    pub fn delete_mono(&mut self, id: &str, cx: &mut Context<Self>) {
        self.monos.pending_delete = None;
        if self.monos.creating.as_deref() == Some(id) {
            self.monos.creating = None;
        }
        let Some(mono) = self.mono(id).cloned() else {
            return;
        };
        if self.monos.view.as_deref() == Some(id) {
            // Its fields are not saved on the way out: the Mono is going.
            self.monos.settings_open = false;
            self.monos.files.memory_edit = None;
            self.monos.view = None;
            self.save_open_mono(cx);
        }
        self.monos.files.loaded.remove(id);
        if self.mono_backgrounds.dialog.as_deref() == Some(id) {
            self.mono_backgrounds.dialog = None;
        }
        if mono.background.is_some() {
            self.remove_mono_background(id, cx);
        }
        self.update_monos(|roster| roster.retain(|mono| mono.id != id), cx);
        match mono.session_id {
            Some(session_id) => self.drop_mono_session(&session_id, cx),
            None => self.sync_selection(cx),
        }
    }

    /// Stops and deletes a conversation that was a Mono's.
    fn drop_mono_session(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if self.is_agent_running_in(session_id) {
            self.stop_agent(session_id, cx);
        }
        if self.sessions.iter().any(|s| s.id == session_id) {
            self.delete_session(session_id, cx);
            return;
        }
        // Never loaded this launch: only its row is left.
        let row = session_id.to_string();
        self.db_write("delete mono session", move |db| db.delete_session(&row));
        self.sync_selection(cx);
    }

    /// MonoCode `resetMonoSession`: a fresh chat with the same harness,
    /// model and permissions; its name, look, projects and soul stay.
    pub fn reset_mono_session(&mut self, cx: &mut Context<Self>) {
        self.monos.pending_reset = false;
        let Some(mono) = self.mono_in_view().cloned() else {
            return;
        };
        // What carries over, not the whole row: its blocks can be huge.
        let Some((old_id, cwd, harness, model, runtime_mode, model_settings, account)) = mono
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
        self.update_monos(
            |roster| {
                if let Some(entry) = roster.iter_mut().find(|entry| entry.id == mono.id) {
                    entry.session_id = Some(fresh.clone());
                    entry.handed = None;
                    entry.rotation = None;
                }
            },
            cx,
        );
        self.drop_mono_session(&old_id, cx);
    }

    pub fn set_mono_name(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        let name: String = name.trim().chars().take(crate::monos::NAME_MAX).collect();
        self.update_monos(
            |roster| {
                if let Some(mono) = roster.iter_mut().find(|mono| mono.id == id) {
                    mono.name = Some(name).filter(|name| !name.is_empty());
                }
            },
            cx,
        );
        // Its composer says "Message {name}".
        self.sync_prompt_placeholder(cx);
    }

    /// MonoCode `saveMonoMascot`.
    pub fn set_mono_mascot(&mut self, id: &str, mascot: &str, cx: &mut Context<Self>) {
        self.update_monos(
            |roster| {
                if let Some(mono) = roster.iter_mut().find(|mono| mono.id == id) {
                    mono.mascot = mascot.to_string();
                }
            },
            cx,
        );
    }

    /// A palette pick; it replaces a custom colour.
    pub fn set_mono_color(&mut self, id: &str, index: usize, cx: &mut Context<Self>) {
        self.update_monos(
            |roster| {
                if let Some(mono) = roster.iter_mut().find(|mono| mono.id == id) {
                    mono.color_index = index;
                    mono.custom_color = None;
                }
            },
            cx,
        );
    }

    /// The projects on the rail the Mono has not been given yet.
    pub fn mono_project_choices(&self, mono: &Mono) -> Vec<String> {
        self.rail_order()
            .into_iter()
            .filter(|path| !mono.works_on(path))
            .collect()
    }

    /// MonoCode `addMonoProject`.
    pub fn add_mono_project(&mut self, id: &str, path: &str, cx: &mut Context<Self>) {
        let path = crate::app::normalize_project_path(path);
        if path.is_empty() || path == "~" {
            return;
        }
        self.update_monos(
            |roster| {
                if let Some(mono) = roster.iter_mut().find(|mono| mono.id == id)
                    && !mono.works_on(&path)
                {
                    mono.projects.push(path);
                }
            },
            cx,
        );
    }

    /// MonoCode `removeMonoProject`.
    pub fn remove_mono_project(&mut self, id: &str, path: &str, cx: &mut Context<Self>) {
        self.update_monos(
            |roster| {
                if let Some(mono) = roster.iter_mut().find(|mono| mono.id == id) {
                    mono.projects.retain(|p| !same_project_path(p, path));
                }
            },
            cx,
        );
    }

    /// "Choose folder…": any folder becomes one of the Mono's projects.
    pub fn pick_mono_project(&mut self, id: &str, cx: &mut Context<Self>) {
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
                    this.add_mono_project(&id, &path.to_string_lossy(), cx);
                }
            });
            if let Err(err) = added {
                log::debug!("folder picked after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `monoTurn` over `monoContext`: a Mono's turn carries who it
    /// is, its standing instructions and its memory, each unless the
    /// provider session behind its chat already holds that version. Other
    /// threads' prompts pass through.
    pub(crate) fn apply_mono_context(
        &mut self,
        session_id: &str,
        prompt: String,
        cx: &mut Context<Self>,
    ) -> String {
        // MonoCode keeps a raw slash command first only for harnesses that
        // take them typed (`isNativeCommandPrompt`); none of BenCode's do, so
        // every Mono turn carries it.
        let Some(mono) = self.mono_for_session(session_id).cloned() else {
            return prompt;
        };
        let brief = self.plan_mono_rotation(&mono, session_id, cx);
        let mono = self.mono(&mono.id).cloned().unwrap_or(mono);
        let provider = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .and_then(|s| s.provider_session_id.clone());
        let provider = provider.as_deref();
        let files = self.mono_turn_files(&mono.id);
        let (soul, memory) =
            self.mono_context_parts(&mono, &files, self.has_app_access(session_id));
        let carried = Carried {
            soul_hash: crate::monos::context_hash(&soul),
            memory_hash: files.memory_hash.clone(),
        };
        let provider = provider.filter(|id| !id.is_empty());
        let plan = crate::monos::plan_context(
            mono.handed.as_ref(),
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
                crate::monos::habit_posts_context(&s.blocks[..before])
            })
            .unwrap_or_default();
        if !plan.any() && away.is_empty() && brief.is_none() {
            return prompt;
        }
        if plan.any() {
            self.monos.carrying.insert(session_id.to_string(), carried);
        }
        let context = [
            brief.unwrap_or_default(),
            away,
            crate::monos::turn_context(plan, &soul, &memory),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
        crate::monos::turn_with_context(&prompt, &context)
    }

    /// MonoCode's rotation, before a Mono's turn: past 80% of its window the
    /// provider session is let go, and a fresh one (that, or one a switch
    /// of account started) is briefed on the conversation so far.
    fn plan_mono_rotation(
        &mut self,
        mono: &crate::monos::Mono,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let session = self.sessions.iter().find(|s| s.id == session_id)?;
        let rotate = crate::monos::rotation::should_rotate(
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
        let replaced = rotate || (fresh && mono.handed.is_some());
        // Without the turn just started.
        let before = session.blocks.len().saturating_sub(1);
        if !replaced || !session.blocks[..before].iter().any(|b| b.role == "user") {
            return None;
        }
        let reason = if rotate { "context" } else { "restart" };
        let (rotation, brief) = crate::monos::rotation::plan_rotation(
            &session.blocks[..before],
            mono.rotation.as_ref(),
            reason,
            crate::app::now_ms(),
            session_id,
        );
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            session.provider_session_id = None;
            session.context_used = None;
            session.context_window = None;
        }
        let id = mono.id.clone();
        self.update_monos(
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

    /// MonoCode `monoContext`'s two parts for `mono`: who it is (with the
    /// app CLI's rules when its turns get the CLI) and its memory block.
    pub(crate) fn mono_context_parts(
        &self,
        mono: &crate::monos::Mono,
        files: &crate::monos::files::MonoFiles,
        app: bool,
    ) -> (String, String) {
        // The CLI's path is part of who it is: a move hands it over again.
        let app_cli = app.then(crate::app_cli::command);
        let soul = crate::monos::soul_context(
            &mono.display_name(),
            &self.mono_projects(mono),
            &files.soul,
            app_cli.as_deref(),
        );
        let topics_dir = crate::app::mono_files::mono_files_dir(&mono.id)
            .map(|dir| dir.join("memory").to_string_lossy().into_owned())
            .filter(|_| !app);
        let memory =
            crate::monos::memory_context(&files.memory, &files.topics, topics_dir.as_deref());
        (soul, memory)
    }

    /// A turn that never finished (stopped, or never started) hands nothing over.
    pub(crate) fn drop_mono_context(&mut self, session_id: &str) {
        self.monos.carrying.remove(session_id);
    }

    /// After a compaction the provider session holds a summary, which may
    /// have dropped who the Mono is: the next turn carries it again.
    pub(crate) fn forget_mono_context(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(id) = self
            .mono_for_session(session_id)
            .map(|mono| mono.id.clone())
        else {
            return;
        };
        self.update_monos(
            |roster| {
                if let Some(entry) = roster.iter_mut().find(|entry| entry.id == id) {
                    entry.handed = None;
                }
            },
            cx,
        );
    }

    /// MonoCode `recordAgentContext`, once a Mono's turn has ended: the
    /// provider session it left behind holds what the turn carried. A turn
    /// that never reached a provider session records nothing, so the next
    /// one carries the context again.
    pub(crate) fn record_mono_context(
        &mut self,
        session_id: &str,
        accepted: bool,
        cx: &mut Context<Self>,
    ) {
        let carried = self.monos.carrying.remove(session_id);
        // MonoCode `useMonoFiles`: a turn ending is when its files, and the
        // projects it works in, may have changed. The files are read again
        // wherever they are held, as the next turn carries them (a Mono
        // without the app CLI edits them on disk); the Overview only while
        // it is shown.
        if let Some(id) = self.mono_for_session(session_id).map(|m| m.id.clone()) {
            self.mono_files_changed(&id, cx);
            if self.monos.view.as_deref() == Some(id.as_str()) {
                self.load_mono_overview(&id, cx);
                self.load_mono_changes(cx);
            }
        }
        // MonoCode records what a turn the provider accepted carried: one
        // that failed or was cut short may never have reached it.
        if !accepted {
            return;
        }
        let Some(mono) = self.mono_for_session(session_id) else {
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
            mono.handed.as_ref().map(|h| Carried {
                soul_hash: h.context_hash.clone(),
                memory_hash: h.memory_hash.clone(),
            })
        }) else {
            return;
        };
        let (id, handed) = (
            mono.id.clone(),
            Handed {
                provider: Some(provider),
                context_hash: carried.soul_hash,
                memory_hash: carried.memory_hash,
            },
        );
        self.update_monos(
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
    fn the_shortcut_opens_the_last_mono_or_the_first() {
        let mono = |id: &str| Mono {
            id: id.into(),
            ..Default::default()
        };
        let roster = [mono("a"), mono("b")];
        assert_eq!(toggle_target(Some("b"), &roster).as_deref(), Some("b"));
        assert_eq!(toggle_target(Some("gone"), &roster).as_deref(), Some("a"));
        assert_eq!(toggle_target(None, &roster).as_deref(), Some("a"));
        assert_eq!(toggle_target(None, &[]), None);
    }
}
