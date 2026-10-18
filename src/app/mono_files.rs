//! A Mono's files on the app (MonoCode `useMonoFiles`, `FileField`,
//! `NameField`, `MemoryPage`'s `FactEditor`): each Mono's `SOUL.md` and
//! `MEMORY.md` as last read, the settings dialog's soul field (saved when
//! it loses focus, over the version it opened, so a change made on disk
//! meanwhile asks "Use theirs / Keep mine"), its name field and the fact
//! being added or reworded. Reads and writes run off the UI thread; the
//! model is `crate::monos::files` and `crate::monos::memory`.

use std::collections::HashMap;
use std::path::PathBuf;

use ely_gpui_component::forms::{InputEvent, TextInput};
use gpui::{AppContext as _, Context, Entity, Focusable as _, Subscription, Window};

use crate::app::BenCodeApp;
use crate::monos::files::{self, MonoFiles, SaveError};
use crate::monos::memory::{self, MemoryLine};

/// The fact field: a new one, or one being reworded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryEdit {
    Adding,
    Editing(MemoryLine),
}

pub struct FilesState {
    /// Each Mono's files as last read or saved, by Mono.
    pub loaded: HashMap<String, MonoFiles>,
    /// Why the open Mono's files could not be read.
    pub load_error: Option<String>,
    /// Bumped by each read, by Mono: a read that lands after a newer one
    /// is dropped.
    generations: HashMap<String, u64>,
    pub soul_input: Entity<TextInput>,
    /// The Mono whose soul the field holds.
    soul_of: Option<String>,
    /// MonoCode `FileField`'s `base`: the text and hash the draft started
    /// from. A newer version replaces a draft still equal to it.
    soul_base: Option<(String, String)>,
    /// The file changed on disk since the draft started (MonoCode
    /// `conflict`).
    pub soul_conflict: bool,
    pub soul_error: Option<String>,
    pub name_input: Entity<TextInput>,
    name_of: Option<String>,
    pub memory_input: Entity<TextInput>,
    pub memory_edit: Option<MemoryEdit>,
    pub memory_error: Option<String>,
    /// A fact is being written; the field waits for it.
    pub memory_saving: bool,
}

impl FilesState {
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> (Self, Vec<Subscription>) {
        let soul_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line(12, usize::MAX)
                .placeholder("Who it is, and what it should always keep in mind")
        });
        let name_input = cx.new(|cx| TextInput::new(window, cx).max_len(crate::monos::NAME_MAX));
        let memory_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .max_len(memory::MEMORY_ENTRY_MAX)
                .placeholder("Something it should remember")
        });
        let subscriptions = vec![
            cx.subscribe(
                &soul_input,
                |this: &mut BenCodeApp, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Blur) {
                        this.save_mono_soul(false, cx);
                    }
                },
            ),
            cx.subscribe(
                &name_input,
                |this: &mut BenCodeApp, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Blur | InputEvent::Submit) {
                        this.commit_mono_name(cx);
                    }
                },
            ),
            cx.subscribe(
                &memory_input,
                |this: &mut BenCodeApp, _, event: &InputEvent, cx| match event {
                    // Enter saves and, for a new fact, leaves the field open
                    // for the next one; leaving the field saves and closes.
                    InputEvent::Submit => this.commit_memory(false, cx),
                    InputEvent::Blur => this.commit_memory(true, cx),
                    _ => {}
                },
            ),
        ];
        let state = Self {
            loaded: HashMap::new(),
            load_error: None,
            generations: HashMap::new(),
            soul_input,
            soul_of: None,
            soul_base: None,
            soul_conflict: false,
            soul_error: None,
            name_input,
            name_of: None,
            memory_input,
            memory_edit: None,
            memory_error: None,
            memory_saving: false,
        };
        (state, subscriptions)
    }
}

/// The folder a Mono's files are in.
pub(crate) fn mono_files_dir(id: &str) -> Option<PathBuf> {
    crate::storage::data_dir().and_then(|data| crate::monos::mono_dir(&data, id))
}

impl BenCodeApp {
    /// The open Mono's files, once read.
    pub fn mono_files(&self, id: &str) -> Option<&MonoFiles> {
        self.monos.files.loaded.get(id)
    }

    /// The Mono's files changed outside the dialog (its agent, through
    /// `bencode app`): read again where they are shown.
    pub(crate) fn mono_files_changed(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.monos.files.loaded.contains_key(id) || self.monos.view.as_deref() == Some(id) {
            self.load_mono_files(id, cx);
        }
    }

    /// Its files are what its turns carry: read off the UI thread once,
    /// before the first turn, so it finds them read (`mono_turn_files`).
    pub(crate) fn warm_mono_files(&mut self, id: &str, cx: &mut Context<Self>) {
        let files = &self.monos.files;
        if !files.loaded.contains_key(id) && !files.generations.contains_key(id) {
            self.load_mono_files(id, cx);
        }
    }

    /// MonoCode `loadMonoFiles` off the UI thread (seeding `SOUL.md` the
    /// first time); the dialog's fields follow when they hold no edit.
    pub fn load_mono_files(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(dir) = mono_files_dir(id) else {
            return;
        };
        let generation = {
            let slot = self
                .monos
                .files
                .generations
                .entry(id.to_string())
                .or_default();
            *slot += 1;
            *slot
        };
        let id = id.to_string();
        let task = cx
            .background_executor()
            .spawn(async move { files::load(&dir) });
        cx.spawn(async move |this, cx| {
            let loaded = task.await;
            let landed = this.update(cx, |this, cx| {
                if this.monos.files.generations.get(&id) != Some(&generation) {
                    return;
                }
                match loaded {
                    Ok(files) => this.land_mono_files(&id, files, cx),
                    Err(err) => {
                        log::error!("could not read the Mono's files: {err:#}");
                        if this.monos.view.as_deref() == Some(id.as_str()) {
                            this.monos.files.load_error =
                                Some(format!("Could not read its files: {err}"));
                        }
                        cx.notify();
                    }
                }
            });
            if let Err(err) = landed {
                log::debug!("Mono files after app drop: {err:#}");
            }
        })
        .detach();
    }

    fn land_mono_files(&mut self, id: &str, loaded: MonoFiles, cx: &mut Context<Self>) {
        if self.monos.view.as_deref() == Some(id) {
            self.monos.files.load_error = None;
        }
        self.monos
            .files
            .loaded
            .insert(id.to_string(), loaded.clone());
        if self.monos.files.soul_of.as_deref() == Some(id) {
            let clean = self
                .monos
                .files
                .soul_base
                .as_ref()
                .is_none_or(|(value, _)| value == self.monos.files.soul_input.read(cx).text());
            let moved = self
                .monos
                .files
                .soul_base
                .as_ref()
                .is_none_or(|(_, hash)| *hash != loaded.soul_hash);
            if clean && moved {
                self.set_soul_field(loaded.soul, loaded.soul_hash, cx);
            }
        }
        cx.notify();
    }

    /// The soul and name fields take this Mono's text (the settings dialog
    /// opening on it).
    pub(crate) fn show_mono_fields(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.monos.files.soul_of.as_deref() != Some(id) {
            self.monos.files.soul_of = Some(id.to_string());
            self.monos.files.soul_base = None;
            self.monos.files.soul_conflict = false;
            self.monos.files.soul_error = None;
            match self.mono_files(id).cloned() {
                Some(files) => self.set_soul_field(files.soul, files.soul_hash, cx),
                None => self
                    .monos
                    .files
                    .soul_input
                    .update(cx, |input, cx| input.set_text("", cx)),
            }
        }
        self.sync_mono_name_field(id, cx);
        self.load_mono_files(id, cx);
    }

    fn set_soul_field(&mut self, text: String, hash: String, cx: &mut Context<Self>) {
        if self.monos.files.soul_input.read(cx).text() != text {
            let shown = text.clone();
            self.monos
                .files
                .soul_input
                .update(cx, |input, cx| input.set_text(shown, cx));
        }
        self.monos.files.soul_base = Some((text, hash));
        self.monos.files.soul_conflict = false;
    }

    fn soul_draft_dirty(&self, cx: &gpui::App) -> bool {
        self.monos.files.soul_of.is_some()
            && self
                .monos
                .files
                .soul_base
                .as_ref()
                .is_some_and(|(value, _)| value != self.monos.files.soul_input.read(cx).text())
    }

    /// MonoCode `FileField`'s save on blur: over the version the draft
    /// started from, or over whatever is there (`overwrite`, Keep mine).
    pub(crate) fn save_mono_soul(&mut self, overwrite: bool, cx: &mut Context<Self>) {
        if !self.soul_draft_dirty(cx) && !overwrite {
            return;
        }
        let (Some(id), Some((_, base_hash))) = (
            self.monos.files.soul_of.clone(),
            self.monos.files.soul_base.clone(),
        ) else {
            return;
        };
        let Some(dir) = mono_files_dir(&id) else {
            return;
        };
        let text = self.monos.files.soul_input.read(cx).text().to_string();
        let expected = (!overwrite).then_some(base_hash);
        let task = cx.background_executor().spawn({
            let text = text.clone();
            async move { files::save(&dir, files::SOUL_FILE, &text, expected.as_deref()) }
        });
        cx.spawn(async move |this, cx| {
            let saved = task.await;
            let landed = this.update(cx, |this, cx| {
                let current = this.monos.files.soul_of.as_deref() == Some(id.as_str());
                match saved {
                    Ok(hash) => {
                        if let Some(files) = this.monos.files.loaded.get_mut(&id) {
                            files.soul = text.clone();
                            files.soul_hash = hash.clone();
                        }
                        if current {
                            this.monos.files.soul_base = Some((text, hash));
                            this.monos.files.soul_conflict = false;
                            this.monos.files.soul_error = None;
                        }
                    }
                    Err(SaveError::Conflict) => {
                        if current {
                            this.monos.files.soul_conflict = true;
                        }
                        // What "Use theirs" goes back to.
                        this.load_mono_files(&id, cx);
                    }
                    Err(SaveError::Failed(err)) => {
                        log::error!("could not save the Mono's soul: {err:#}");
                        if current {
                            this.monos.files.soul_error =
                                Some(format!("Could not save SOUL.md: {err}"));
                        }
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("soul saved after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// MonoCode "Use theirs": the draft goes back to the file as it is now.
    pub(crate) fn use_their_soul(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.monos.files.soul_of.clone() else {
            return;
        };
        self.monos.files.soul_conflict = false;
        if let Some(files) = self.mono_files(&id).cloned() {
            self.set_soul_field(files.soul, files.soul_hash, cx);
        }
        cx.notify();
    }

    /// MonoCode `NameField`: the Mono's name, or empty for its mascot's.
    pub(crate) fn sync_mono_name_field(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(mono) = self.mono(id) else {
            return;
        };
        let name = mono.name.clone().unwrap_or_default();
        let placeholder = crate::monos::default_name(&mono.mascot);
        self.monos.files.name_of = Some(id.to_string());
        self.monos.files.name_input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, cx);
            if input.text() != name {
                input.set_text(name, cx);
            }
        });
    }

    /// The name field's text, saved when it is left or Enter is pressed.
    pub(crate) fn commit_mono_name(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.monos.files.name_of.clone() else {
            return;
        };
        let draft: String = self
            .monos
            .files
            .name_input
            .read(cx)
            .text()
            .trim()
            .chars()
            .take(crate::monos::NAME_MAX)
            .collect();
        let saved = self
            .mono(&id)
            .and_then(|mono| mono.name.clone())
            .unwrap_or_default();
        if draft != saved {
            self.set_mono_name(&id, &draft, cx);
        }
        self.sync_mono_name_field(&id, cx);
    }

    /// MonoCode's "Add memory": an empty fact field at the top of the list.
    pub(crate) fn start_adding_memory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_memory_field(MemoryEdit::Adding, String::new(), window, cx);
    }

    /// A fact clicked: its words in the field, to reword.
    pub(crate) fn start_editing_memory(
        &mut self,
        fact: MemoryLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = fact.text.clone();
        self.open_memory_field(MemoryEdit::Editing(fact), text, window, cx);
    }

    fn open_memory_field(
        &mut self,
        edit: MemoryEdit,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.monos.files.memory_saving {
            return;
        }
        self.monos.files.memory_edit = Some(edit);
        self.monos.files.memory_error = None;
        self.monos
            .files
            .memory_input
            .update(cx, |input, cx| input.set_text(text, cx));
        let focus = self.monos.files.memory_input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Escape: the fact field closes as it was. Whether one was open.
    pub(crate) fn close_memory_field(&mut self, cx: &mut Context<Self>) -> bool {
        if self.monos.files.memory_edit.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    /// MonoCode `FactEditor`'s `commit`: a new fact appended as a dated
    /// entry, or the fact reworded in place; an empty field just closes.
    fn commit_memory(&mut self, close: bool, cx: &mut Context<Self>) {
        if self.monos.files.memory_saving {
            return;
        }
        let Some(edit) = self.monos.files.memory_edit.clone() else {
            return;
        };
        let Some(id) = self.monos.view.clone() else {
            return;
        };
        let fact = self
            .monos
            .files
            .memory_input
            .read(cx)
            .text()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let unchanged = matches!(&edit, MemoryEdit::Editing(line) if line.text == fact);
        if fact.is_empty() || unchanged {
            self.monos.files.memory_edit = None;
            cx.notify();
            return;
        }
        let keep_open = !close && edit == MemoryEdit::Adding;
        let date = memory::memory_date();
        let change = move |text: &str| -> Result<String, String> {
            match &edit {
                MemoryEdit::Adding => {
                    let entry = memory::memory_entry(&fact, &date, None)?;
                    Ok(memory::add_memory_entry(text, &entry).0)
                }
                MemoryEdit::Editing(line) => {
                    let index = memory::fact_index(text, &line.line)?;
                    memory::with_line_edited(text, index, &fact, &date)
                }
            }
        };
        self.edit_mono_memory(
            &id,
            change,
            move |this, cx| {
                if keep_open {
                    this.monos
                        .files
                        .memory_input
                        .update(cx, |input, cx| input.set_text("", cx));
                } else {
                    this.monos.files.memory_edit = None;
                }
            },
            cx,
        );
    }

    /// MonoCode's Forget: the fact's line taken out.
    pub(crate) fn forget_memory(&mut self, fact: MemoryLine, cx: &mut Context<Self>) {
        let Some(id) = self.monos.view.clone() else {
            return;
        };
        let change = move |text: &str| -> Result<String, String> {
            let index = memory::fact_index(text, &fact.line)?;
            Ok(memory::without_line(text, index))
        };
        self.edit_mono_memory(&id, change, |_, _| {}, cx);
    }

    /// MonoCode `editMonoFile` on `MEMORY.md` off the UI thread, then the
    /// files read again; `done` runs once it is saved.
    fn edit_mono_memory(
        &mut self,
        id: &str,
        change: impl Fn(&str) -> Result<String, String> + Send + 'static,
        done: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = mono_files_dir(id) else {
            return;
        };
        self.monos.files.memory_saving = true;
        let task = cx.background_executor().spawn(async move {
            files::edit(&dir, files::MEMORY_FILE, change)?;
            files::load(&dir).map_err(|err| format!("{err:#}"))
        });
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let saved = task.await;
            let landed = this.update(cx, |this, cx| {
                this.monos.files.memory_saving = false;
                match saved {
                    Ok(files) => {
                        this.monos.files.memory_error = None;
                        this.land_mono_files(&id, files, cx);
                        done(this, cx);
                    }
                    Err(err) => {
                        log::warn!("could not save the Mono's memory: {err}");
                        this.monos.files.memory_error = Some(err);
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("memory saved after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// MonoCode `resetMonoDefaults` (Settings › Monos, Reset Mono): the
    /// default soul and its mascot's name; its conversation, projects,
    /// memory and habits stay.
    pub fn reset_mono_defaults(&mut self, id: &str, cx: &mut Context<Self>) {
        self.monos.pending_defaults = None;
        let Some(dir) = mono_files_dir(id) else {
            return;
        };
        self.set_mono_name(id, "", cx);
        if self.monos.files.name_of.as_deref() == Some(id) {
            self.sync_mono_name_field(id, cx);
        }
        let task = cx.background_executor().spawn(async move {
            files::save(&dir, files::SOUL_FILE, &crate::monos::default_soul(), None)
                .map_err(|err| err.to_string())?;
            files::load(&dir).map_err(|err| format!("{err:#}"))
        });
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let reset = task.await;
            let landed = this.update(cx, |this, cx| match reset {
                Ok(files) => {
                    // The field gives up any edit: the soul was reset.
                    if this.monos.files.soul_of.as_deref() == Some(id.as_str()) {
                        this.set_soul_field(files.soul.clone(), files.soul_hash.clone(), cx);
                    }
                    this.land_mono_files(&id, files, cx);
                }
                Err(err) => {
                    log::error!("could not reset the Mono: {err}");
                    this.monos.reset_error = Some(format!("Could not reset the Mono. {err}"));
                    cx.notify();
                }
            });
            if let Err(err) = landed {
                log::debug!("Mono reset after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// What a Mono's turn reads: its files as last read (opening its chat
    /// reads them, `ensure_mono_session`), else read now, when that read
    /// failed or has not landed yet. Small files.
    pub(crate) fn mono_turn_files(&mut self, id: &str) -> MonoFiles {
        if let Some(files) = self.monos.files.loaded.get(id) {
            return files.clone();
        }
        let read = mono_files_dir(id).and_then(|dir| {
            files::load(&dir)
                .map_err(|err| log::error!("could not read the Mono's files: {err:#}"))
                .ok()
        });
        match read {
            Some(files) => {
                self.monos
                    .files
                    .loaded
                    .insert(id.to_string(), files.clone());
                files
            }
            None => {
                let soul = crate::monos::default_soul();
                MonoFiles {
                    soul_hash: files::content_hash(&soul),
                    soul,
                    memory_hash: files::content_hash(""),
                    ..Default::default()
                }
            }
        }
    }
}
