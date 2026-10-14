//! Monos on the app (MonoCode `App.tsx`'s `monoViewId`, `monoWorkspace.ts`,
//! `monoFiles.ts`): the roster in `settings.json`, the Mono whose
//! conversation covers the workspace, its details panel and `SOUL.md`, and
//! the standing context a turn carries. The model is in `crate::monos`.

use std::collections::HashMap;
use std::time::Duration;

use ely_gpui_component::forms::{InputEvent, TextInput};
use gpui::{AppContext as _, Context, Entity, Hsla, PathPromptOptions, Subscription, Window};

use crate::app::live_agents::{RunFlags, live_agent};
use crate::app::session_folders::{palette_color, parse_hex};
use crate::app::{BenCodeApp, same_project_path, unique_id};
use crate::monos::{Handed, Mono, MonoState, MonoStatus};
use crate::ui::mascot::{Mascot, mascot_for};
use crate::ui::transcript::turns::is_tool;

/// How long the soul field rests before it is saved.
const SOUL_SAVE_DELAY: Duration = Duration::from_millis(600);
/// MonoCode `ensureMonoSession`: a Mono's chat starts with Auto permissions.
const MONO_RUNTIME_MODE: &str = "auto";

pub struct MonosState {
    /// The Mono whose conversation covers the workspace.
    pub view: Option<String>,
    /// MonoCode `monoOverviewOpen`: the title bar's Overview pill is on.
    /// Until the Overview panel is ported it shows the details panel.
    pub overview_open: bool,
    /// MonoCode `monoSettingsOpen`: the title bar's gear. Until the
    /// settings dialog is ported it shows the details panel too.
    pub settings_open: bool,
    /// The Mono composer's emoji picker (MonoCode `EmojiPickerPopover`).
    pub emoji: crate::ui::emoji_picker::EmojiPicker,
    /// Each Mono's `SOUL.md` as last read or saved.
    souls: HashMap<String, String>,
    pub soul_input: Entity<TextInput>,
    /// The Mono whose soul the field holds.
    soul_of: Option<String>,
    pub soul_error: Option<String>,
    /// Bumped on every edit; a pending save only runs if it still matches.
    soul_generation: u64,
    /// Mono id awaiting delete confirmation.
    pub pending_delete: Option<String>,
    /// The open Mono's chat is awaiting a reset confirmation.
    pub pending_reset: bool,
    /// Why a Mono's conversation could not be read, by Mono; the floating
    /// chat shows it in place of "Loading conversation…".
    pub load_errors: HashMap<String, String>,
    /// The floating chat, while its window is open.
    float: Option<gpui::WindowHandle<crate::ui::mono_float::MonoFloat>>,
    /// The context a running turn carries, by conversation: recorded as
    /// handed over once the turn ends with a provider session to name.
    carrying: HashMap<String, String>,
    _subscriptions: Vec<Subscription>,
}

impl MonosState {
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> Self {
        let soul_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line(8, usize::MAX)
                .placeholder("Anything it should always keep in mind…")
        });
        let subscriptions = vec![cx.subscribe(
            &soul_input,
            |this: &mut BenCodeApp, _, event: &InputEvent, cx| match event {
                InputEvent::Changed => this.schedule_soul_save(cx),
                InputEvent::Blur => this.save_soul_if_dirty(cx),
                _ => {}
            },
        )];
        Self {
            view: None,
            overview_open: false,
            settings_open: false,
            emoji: Default::default(),
            souls: HashMap::new(),
            soul_input,
            soul_of: None,
            soul_error: None,
            soul_generation: 0,
            pending_delete: None,
            pending_reset: false,
            load_errors: HashMap::new(),
            float: None,
            carrying: HashMap::new(),
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
            .or_else(|| palette_color(mono.color_index))
            .or_else(|| palette_color(1))
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
            let waiting = if request.tool == crate::app::QUESTION_TOOL {
                "Waiting for your answer".to_string()
            } else {
                format!("Approve {}", request.tool)
            };
            return MonoState {
                status: MonoStatus::NeedsYou,
                activity: Some(waiting),
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
        let flags = RunFlags {
            busy: self.is_agent_running_in(&session.id),
            ..Default::default()
        };
        match live_agent(session, flags) {
            Some(agent) => {
                // Its one conversation runs for days: a tool call from an
                // earlier turn is not what it is on now. With none in this
                // turn it is just "Working", said once.
                let this_turn = session
                    .blocks
                    .iter()
                    .rev()
                    .take_while(|block| block.role != "user")
                    .any(is_tool);
                MonoState {
                    status: MonoStatus::Working,
                    activity: this_turn.then_some(agent.activity),
                }
            }
            None => MonoState::IDLE,
        }
    }

    /// MonoCode `createMono` then `onOpenMono`: a new Mono, opened.
    pub fn create_mono(&mut self, cx: &mut Context<Self>) {
        let id = self.add_mono(cx);
        // Its settings open with it, ready to be told what it works on.
        self.monos.settings_open = true;
        self.open_mono(&id, cx);
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
        let session_id = self.create_session_row(&home);
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
        if self.mono(id).is_none() {
            return;
        }
        self.close_rail_menu(cx);
        self.prepare_mono_session(id, cx);
        let (app, main, mono_id) = (cx.entity(), window.window_handle(), id.to_string());
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
                self.thread_mut(session_id).queue.push(input);
            }
            cx.notify();
            return Ok(());
        }
        self.send_turn(session_id, input, cx);
        Ok(())
    }

    fn show_mono(&mut self, id: &str, cx: &mut Context<Self>) {
        self.save_soul_if_dirty(cx);
        if self.surface == Some(crate::app::Surface::Notes) {
            self.save_note_if_dirty(cx);
        }
        self.surface = None;
        self.settings_return = None;
        self.monos.view = Some(id.to_string());
        self.monos.pending_reset = false;
        self.load_mono_soul(id, cx);
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
        self.save_soul_if_dirty(cx);
        self.monos.view = None;
        self.monos.pending_reset = false;
        true
    }

    /// The panel beside the Mono's chat shows: its Overview or its
    /// settings, both the details panel for now.
    pub fn mono_details_shown(&self) -> bool {
        self.monos.overview_open || self.monos.settings_open
    }

    /// MonoCode `onToggleMonoOverview`: the title bar's Overview pill.
    pub fn toggle_mono_overview(&mut self, cx: &mut Context<Self>) {
        self.monos.overview_open = !self.monos.overview_open;
        self.monos.settings_open = false;
        self.mono_details_changed(cx);
    }

    /// MonoCode `onOpenMonoSettings`: the title bar's gear.
    pub fn toggle_mono_settings(&mut self, cx: &mut Context<Self>) {
        self.monos.settings_open = !self.monos.settings_open;
        self.monos.overview_open = false;
        self.mono_details_changed(cx);
    }

    /// The details panel's own Hide.
    pub fn close_mono_details(&mut self, cx: &mut Context<Self>) {
        self.monos.overview_open = false;
        self.monos.settings_open = false;
        self.mono_details_changed(cx);
    }

    fn mono_details_changed(&mut self, cx: &mut Context<Self>) {
        if !self.mono_details_shown() {
            self.save_soul_if_dirty(cx);
        }
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
        let limited = self
            .thread(session_id)
            .is_some_and(|thread| thread.usage_limit.is_some());
        if !limited || self.is_agent_running_in(session_id) || !self.is_mono_session(session_id) {
            return;
        }
        let Some(session) = self.sessions.iter().find(|s| s.id == session_id) else {
            return;
        };
        if self.session_account_id(session) == account_id
            || !self.account_exists(&session.harness, account_id)
        {
            return;
        }
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            session.provider_account_id = Some(account_id.to_string());
            session.provider_session_id = None;
        }
        self.persist_session(session_id);
        self.resume_after_limit(session_id, cx);
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

    /// MonoCode `removeMono`: its conversation goes with it; its folder of
    /// files stays on disk.
    pub fn delete_mono(&mut self, id: &str, cx: &mut Context<Self>) {
        self.monos.pending_delete = None;
        let Some(mono) = self.mono(id).cloned() else {
            return;
        };
        if self.monos.view.as_deref() == Some(id) {
            // Its soul is not saved on the way out: the Mono is going.
            self.monos.soul_generation += 1;
            self.monos.soul_of = None;
            self.monos.view = None;
        }
        self.monos.souls.remove(id);
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
        let Some((old_id, cwd, harness, model, runtime_mode, model_settings)) = mono
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
                )
            })
        else {
            return;
        };
        let fresh = self.create_session_row(&cwd);
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == fresh) {
            session.harness = harness;
            session.model = model;
            session.runtime_mode = runtime_mode;
            session.model_settings = model_settings;
            session.worktree_cwd = None;
            session.branch = None;
        }
        self.persist_session(&fresh);
        self.update_monos(
            |roster| {
                if let Some(entry) = roster.iter_mut().find(|entry| entry.id == mono.id) {
                    entry.session_id = Some(fresh.clone());
                    entry.handed = None;
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

    /// Reads the Mono's `SOUL.md` off the UI thread (seeding it the first
    /// time) and shows it in the details field.
    fn load_mono_soul(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(dir) =
            crate::storage::data_dir().and_then(|data| crate::monos::mono_dir(&data, id))
        else {
            return;
        };
        self.monos.soul_error = None;
        match self.monos.souls.get(id).cloned() {
            Some(soul) => self.show_soul(id, soul, cx),
            // The field is nobody's until the read lands.
            None => self.monos.soul_of = None,
        }
        let id = id.to_string();
        let task = cx
            .background_executor()
            .spawn(async move { crate::monos::load_soul(&dir) });
        cx.spawn(async move |this, cx| {
            let loaded = task.await;
            let landed = this.update(cx, |this, cx| match loaded {
                Ok(soul) => {
                    // Edits made while it loaded win over what was on disk.
                    let editing = this.monos.soul_of.as_deref() == Some(id.as_str())
                        && this.soul_field_dirty(cx);
                    if !editing {
                        this.monos.souls.insert(id.clone(), soul.clone());
                        if this.monos.view.as_deref() == Some(id.as_str()) {
                            this.show_soul(&id, soul, cx);
                        }
                    }
                }
                Err(err) => {
                    log::error!("could not read the Mono's soul: {err:#}");
                    this.monos.soul_error = Some(format!("Could not read SOUL.md: {err}"));
                    cx.notify();
                }
            });
            if let Err(err) = landed {
                log::debug!("soul loaded after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn show_soul(&mut self, id: &str, soul: String, cx: &mut Context<Self>) {
        self.monos.soul_of = Some(id.to_string());
        // Not an edit: nothing is saved for it.
        self.monos.soul_generation += 1;
        if self.monos.soul_input.read(cx).text() != soul {
            self.monos
                .soul_input
                .update(cx, |input, cx| input.set_text(soul, cx));
        }
        cx.notify();
    }

    fn soul_field_dirty(&self, cx: &gpui::App) -> bool {
        let Some(id) = self.monos.soul_of.as_deref() else {
            return false;
        };
        self.monos
            .souls
            .get(id)
            .is_some_and(|saved| saved != self.monos.soul_input.read(cx).text())
    }

    fn schedule_soul_save(&mut self, cx: &mut Context<Self>) {
        self.monos.soul_generation += 1;
        let generation = self.monos.soul_generation;
        let timer = cx.background_executor().timer(SOUL_SAVE_DELAY);
        cx.spawn(async move |this, cx| {
            timer.await;
            let saved = this.update(cx, |this, cx| {
                if this.monos.soul_generation == generation {
                    this.save_soul_if_dirty(cx);
                }
            });
            if let Err(err) = saved {
                log::debug!("soul autosave after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// Saves the soul field when it differs from what is stored, so closing
    /// the panel or switching Monos never drops edits.
    pub(crate) fn save_soul_if_dirty(&mut self, cx: &mut Context<Self>) {
        if !self.soul_field_dirty(cx) {
            return;
        }
        let Some(id) = self.monos.soul_of.clone() else {
            return;
        };
        let Some(dir) =
            crate::storage::data_dir().and_then(|data| crate::monos::mono_dir(&data, &id))
        else {
            return;
        };
        let text = self.monos.soul_input.read(cx).text().to_string();
        // What the next turn carries, whether or not the write has landed.
        self.monos.souls.insert(id, text.clone());
        let task = cx
            .background_executor()
            .spawn(async move { crate::monos::save_soul(&dir, &text) });
        cx.spawn(async move |this, cx| {
            let saved = task.await;
            let landed = this.update(cx, |this, cx| {
                this.monos.soul_error = saved
                    .err()
                    .map(|err| format!("Could not save SOUL.md: {err}"));
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("soul saved after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode `monoTurn` over `monoContext`: a Mono's turn carries who it
    /// is and its standing instructions, unless the provider session behind
    /// its chat already holds them. Other threads' prompts pass through.
    pub(crate) fn apply_mono_context(
        &mut self,
        session_id: &str,
        provider: Option<&str>,
        prompt: String,
    ) -> String {
        // MonoCode keeps a raw slash command first only for harnesses that
        // take them typed (`isNativeCommandPrompt`); none of BenCode's do, so
        // every Mono turn carries it.
        let Some(mono) = self.mono_for_session(session_id).cloned() else {
            return prompt;
        };
        let soul = match self.monos.souls.get(&mono.id) {
            Some(soul) => soul.clone(),
            // Not opened this launch (a queued turn, a resumed one, the
            // floating chat): a small file, read as skill bodies are right
            // before the turn starts, and kept for the turns after it.
            None => {
                let read = crate::storage::data_dir()
                    .and_then(|data| crate::monos::mono_dir(&data, &mono.id))
                    .and_then(|dir| {
                        crate::monos::load_soul(&dir)
                            .map_err(|err| log::error!("could not read the Mono's soul: {err:#}"))
                            .ok()
                    });
                if let Some(soul) = &read {
                    self.monos.souls.insert(mono.id.clone(), soul.clone());
                }
                read.unwrap_or_else(crate::monos::default_soul)
            }
        };
        let context =
            crate::monos::standing_context(&mono.display_name(), &self.mono_projects(&mono), &soul);
        let hash = crate::monos::context_hash(&context);
        let provider = provider.filter(|id| !id.is_empty());
        if !crate::monos::needs_context(mono.handed.as_ref(), provider, &hash) {
            return prompt;
        }
        self.monos.carrying.insert(session_id.to_string(), hash);
        crate::monos::turn_with_context(&prompt, &context)
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
        let Some(context_hash) =
            carried.or_else(|| mono.handed.as_ref().map(|h| h.context_hash.clone()))
        else {
            return;
        };
        let (id, handed) = (
            mono.id.clone(),
            Handed {
                provider: Some(provider),
                context_hash,
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
