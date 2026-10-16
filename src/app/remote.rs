//! Machines (MonoCode `connections/model/connections.ts`,
//! `ConnectionsSettings.tsx`, `AddRemoteProjectDialog.tsx`): the list in
//! `settings.json`, each one's connection status, Settings › Connections'
//! Add machine / Reconnect / Update Host / Remove, and opening a folder on
//! a machine as a project. The connections themselves are `crate::remote`.

use std::collections::HashMap;
use std::sync::Arc;

use bencode_core::harness::HarnessKind;
use bencode_core::harness::catalog::ModelOption;
use bencode_core::remote::protocol::{EntryKind, EntryWire, HarnessWire, Request};
use ely_gpui_component::forms::{InputEvent, TextInput};
use gpui::{AppContext as _, Context, Entity, Window};

use crate::app::BenCodeApp;
use crate::remote::setup::{self, Job, Purpose};
use crate::remote::{Machine, Status};

/// The most folders the picker lists (MonoCode `browseHostDirectories`).
const MAX_FOLDERS: usize = 500;

/// What a setup in progress is for, so its end says the right thing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobKind {
    Add,
    Reconnect(String),
    Update(String),
}

/// "Open folder on a machine…" (MonoCode `AddRemoteProjectDialog`).
#[derive(Default)]
pub struct FolderDialog {
    /// The machine picked; the first one until another is.
    pub machine: Option<String>,
    /// The folder listed below the path, and its subfolders.
    pub listing: Option<Listing>,
    pub loading: bool,
    pub opening: bool,
    pub error: Option<String>,
    /// Bumped per request: an answer for an older one is dropped.
    generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub path: String,
    pub parent: Option<String>,
    pub folders: Vec<String>,
}

/// What the host has, for the composer's pickers in its projects.
#[derive(Clone, Debug, Default)]
pub struct MachineCatalog {
    pub harnesses: Vec<HarnessWire>,
    pub models: HashMap<HarnessKind, Vec<ModelOption>>,
    pub loading: bool,
}

pub struct RemoteUi {
    /// Settings' Add machine form is open.
    pub adding: bool,
    pub target_input: Entity<TextInput>,
    pub name_input: Entity<TextInput>,
    pub port_input: Entity<TextInput>,
    /// SSH's password or passphrase, while it asks.
    pub answer_input: Entity<TextInput>,
    /// The folder dialog's path field.
    pub path_input: Entity<TextInput>,
    pub job: Option<(Arc<Job>, JobKind)>,
    /// The machine whose Remove awaits confirmation.
    pub removing: Option<String>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub folder: Option<FolderDialog>,
    pub catalogs: HashMap<String, MachineCatalog>,
    /// Settings asked for each machine's status once this visit.
    pub checked: bool,
    _subscriptions: Vec<gpui::Subscription>,
}

impl RemoteUi {
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> Self {
        let input = |placeholder: &str, window: &mut Window, cx: &mut Context<BenCodeApp>| {
            crate::app::text_input(window, cx, placeholder)
        };
        let target_input = input("user@my-mac-mini or an SSH alias", window, cx);
        let name_input = input("Optional, e.g. Home Mac mini", window, cx);
        let port_input = input("From SSH config", window, cx);
        let answer_input = cx.new(|cx| TextInput::new(window, cx).masked());
        let path_input = input("/home/me/code/my-app", window, cx);
        let subscriptions = vec![
            cx.subscribe(&target_input, Self::on_form_input),
            cx.subscribe(&name_input, Self::on_form_input),
            cx.subscribe(&port_input, Self::on_form_input),
            cx.subscribe(
                &answer_input,
                |this: &mut BenCodeApp, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.answer_ssh_prompt(None, cx),
                    InputEvent::Changed => cx.notify(),
                    _ => {}
                },
            ),
            cx.subscribe(
                &path_input,
                |this: &mut BenCodeApp, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit => this.open_remote_folder(cx),
                    InputEvent::Changed => cx.notify(),
                    _ => {}
                },
            ),
        ];
        Self {
            adding: false,
            target_input,
            name_input,
            port_input,
            answer_input,
            path_input,
            job: None,
            removing: None,
            error: None,
            notice: None,
            folder: None,
            catalogs: HashMap::new(),
            checked: false,
            _subscriptions: subscriptions,
        }
    }

    fn on_form_input(
        this: &mut BenCodeApp,
        _: Entity<TextInput>,
        event: &InputEvent,
        cx: &mut Context<BenCodeApp>,
    ) {
        match event {
            InputEvent::Submit => this.add_machine(cx),
            InputEvent::Changed => cx.notify(),
            _ => {}
        }
    }
}

/// A connection's change, from any thread, as a redraw of the app.
fn change_hook(cx: &mut Context<BenCodeApp>) -> Arc<dyn Fn() + Send + Sync> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    cx.spawn(async move |this, cx| {
        while rx.recv().await.is_some() {
            // One redraw for a burst of changes.
            while rx.try_recv().is_ok() {}
            if this.update(cx, |app, cx| app.on_remote_change(cx)).is_err() {
                return;
            }
        }
    })
    .detach();
    Arc::new(move || {
        let _ = tx.send(());
    })
}

impl BenCodeApp {
    /// At launch: the saved machines get their connections (opened when
    /// first used) and the app redraws when one changes.
    pub(crate) fn init_remote(&mut self, cx: &mut Context<Self>) {
        crate::remote::set_on_change(change_hook(cx));
        crate::remote::sync_machines(&self.settings.remote_machines);
        // The rail shows each machine's status (MonoCode watches it while a
        // project of it shows); the link is opened now, quietly.
        for machine in self.machines().to_vec() {
            self.connect_machine_quietly(&machine.id, cx);
        }
    }

    pub fn machines(&self) -> &[Machine] {
        &self.settings.remote_machines
    }

    pub fn machine_status(&self, id: &str) -> Status {
        crate::remote::connection(id).map_or(Status::Idle, |conn| conn.status())
    }

    /// A rail project on a machine: the machine's name (`None` when this
    /// BenCode does not know it), whether it is connected (`None` then
    /// too), and the folder's path there. `None` for a folder here.
    pub fn remote_project_status(
        &self,
        path: &str,
    ) -> Option<(Option<String>, Option<bool>, String)> {
        let parsed = bencode_core::remote::path::parse(path)?;
        let machine = self.machines().iter().find(|m| m.id == parsed.machine);
        let online = machine.map(|m| matches!(self.machine_status(&m.id), Status::Connected(_)));
        Some((machine.map(|m| m.name.clone()), online, parsed.host_path))
    }

    /// The machine the composer's thread runs on: the selected thread's
    /// folder, else the current project's.
    pub(crate) fn composer_machine_id(&self) -> Option<String> {
        let path = self
            .selected_session()
            .map_or(self.current_cwd.as_str(), |s| s.work_dir());
        bencode_core::remote::path::parse(path).map(|parsed| parsed.machine.to_string())
    }

    /// That machine's name, and what its host listed once it answered.
    pub(crate) fn composer_machine(&self) -> Option<(String, Option<&MachineCatalog>)> {
        let id = self.composer_machine_id()?;
        let name = self
            .machines()
            .iter()
            .find(|m| m.id == id)
            .map_or_else(|| "this machine".to_string(), |m| m.name.clone());
        let catalog = self.remote.catalogs.get(&id).filter(|c| !c.loading);
        Some((name, catalog))
    }

    fn on_remote_change(&mut self, cx: &mut Context<Self>) {
        self.land_machine_setup(cx);
        cx.notify();
    }

    /// Settings › Connections opened: each machine is asked how it is,
    /// once a visit (MonoCode checks every 10 seconds while it shows).
    pub(crate) fn check_machines(&mut self, cx: &mut Context<Self>) {
        if self.remote.checked {
            return;
        }
        self.remote.checked = true;
        for machine in self.machines().to_vec() {
            self.connect_machine_quietly(&machine.id, cx);
        }
    }

    /// Opens a machine's link in the background, never asking anything.
    pub(crate) fn connect_machine_quietly(&self, id: &str, cx: &mut Context<Self>) {
        let Some(conn) = crate::remote::connection(id) else {
            return;
        };
        if matches!(conn.status(), Status::Connected(_) | Status::Connecting) {
            return;
        }
        cx.background_executor()
            .spawn(async move {
                if let Err(err) = conn.connect(None, None) {
                    log::info!("remote: {err}");
                }
            })
            .detach();
    }

    // ---- Settings › Connections ------------------------------------------

    pub fn open_add_machine(&mut self, cx: &mut Context<Self>) {
        self.remote.adding = true;
        self.remote.error = None;
        self.remote.notice = None;
        cx.notify();
    }

    pub fn cancel_add_machine(&mut self, cx: &mut Context<Self>) {
        self.remote.adding = false;
        cx.notify();
    }

    /// The form's Connect.
    pub fn add_machine(&mut self, cx: &mut Context<Self>) {
        if self.remote.job.is_some() {
            return;
        }
        let target = self.remote.target_input.read(cx).text().trim().to_string();
        let name = self.remote.name_input.read(cx).text().trim().to_string();
        let port_text = self.remote.port_input.read(cx).text().trim().to_string();
        if target.is_empty() {
            return;
        }
        let port = match port_text.as_str() {
            "" => None,
            text => match text.parse::<u16>() {
                Ok(port) if port > 0 => Some(port),
                _ => {
                    self.remote.error = Some("Enter an SSH port from 1 to 65535.".into());
                    cx.notify();
                    return;
                }
            },
        };
        self.begin_machine_setup(Purpose::Add { target, port, name }, JobKind::Add, cx);
    }

    pub fn reconnect_machine(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(machine) = self.machines().iter().find(|m| m.id == id).cloned() else {
            return;
        };
        self.begin_machine_setup(Purpose::Reconnect(machine), JobKind::Reconnect(id.into()), cx);
    }

    pub fn update_machine_host(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(machine) = self.machines().iter().find(|m| m.id == id).cloned() else {
            return;
        };
        self.begin_machine_setup(Purpose::Update(machine), JobKind::Update(id.into()), cx);
    }

    fn begin_machine_setup(&mut self, purpose: Purpose, kind: JobKind, cx: &mut Context<Self>) {
        if self.remote.job.is_some() {
            return;
        }
        self.remote.error = None;
        self.remote.notice = None;
        let job = setup::begin(purpose, change_hook(cx), crate::remote::adopt);
        self.remote.job = Some((job, kind));
        cx.notify();
    }

    /// A finished setup: the machine is saved, the form closes, and
    /// Settings says what to do next.
    fn land_machine_setup(&mut self, cx: &mut Context<Self>) {
        let Some((job, kind)) = self.remote.job.clone() else {
            return;
        };
        let view = job.view();
        if !view.done {
            return;
        }
        self.remote.job = None;
        self.remote.answer_input.update(cx, |input, cx| input.set_text("", cx));
        if let Some(error) = view.error {
            self.remote.error = Some(error);
            return;
        }
        let Some(machine) = view.machine else {
            return;
        };
        let mut machines = self.settings.remote_machines.clone();
        match machines.iter_mut().find(|m| m.id == machine.id) {
            Some(saved) => {
                // Adding a machine again keeps the name it was given.
                let name = if kind == JobKind::Add && !machine.name.is_empty() {
                    machine.name.clone()
                } else {
                    saved.name.clone()
                };
                *saved = Machine { name, ..machine.clone() };
            }
            None => machines.push(machine.clone()),
        }
        self.settings.remote_machines = machines;
        self.save_settings(cx);
        crate::remote::sync_machines(&self.settings.remote_machines);
        self.remote.catalogs.remove(&machine.id);
        let mut notice = match kind {
            JobKind::Update(_) => format!("{} was updated and reconnected.", machine.name),
            JobKind::Reconnect(_) => format!("{} is connected.", machine.name),
            JobKind::Add => {
                self.remote.adding = false;
                for input in [
                    &self.remote.target_input,
                    &self.remote.name_input,
                    &self.remote.port_input,
                ] {
                    input.update(cx, |input, cx| input.set_text("", cx));
                }
                format!(
                    "{} is connected. To work on it, click + next to Projects in the project rail and choose Open folder on a machine.",
                    machine.name
                )
            }
        };
        if let Some(note) = view.note {
            notice = format!("{notice}\n{note}");
        }
        self.remote.notice = Some(notice);
        self.load_machine_catalog(&machine.id, cx);
    }

    /// The SSH prompt's Continue, Trust host and continue, or Reject.
    /// `None` sends what the password field holds.
    pub fn answer_ssh_prompt(&mut self, answer: Option<&str>, cx: &mut Context<Self>) {
        let Some((job, _)) = &self.remote.job else {
            return;
        };
        let Some(prompt) = job.view().prompt else {
            return;
        };
        let answer = match answer {
            Some(answer) => answer.to_string(),
            None if prompt.confirm => return,
            None => self.remote.answer_input.read(cx).text().to_string(),
        };
        if let Err(err) = job.answer(&prompt.id, answer) {
            self.remote.error = Some(err);
        }
        self.remote.answer_input.update(cx, |input, cx| input.set_text("", cx));
        cx.notify();
    }

    pub fn cancel_machine_setup(&mut self, cx: &mut Context<Self>) {
        if let Some((job, _)) = &self.remote.job {
            job.cancel();
        }
        cx.notify();
    }

    pub fn ask_remove_machine(&mut self, id: &str, cx: &mut Context<Self>) {
        self.remote.removing = Some(id.to_string());
        self.remote.error = None;
        cx.notify();
    }

    /// Remove: this BenCode forgets the machine and closes its connection.
    /// The host keeps running there, its agents too; threads in its
    /// projects stay here, and adding it again brings them back.
    pub fn remove_machine(&mut self, id: &str, cx: &mut Context<Self>) {
        self.remote.removing = None;
        let Some(machine) = self.machines().iter().find(|m| m.id == id).cloned() else {
            return;
        };
        self.settings.remote_machines.retain(|m| m.id != id);
        self.save_settings(cx);
        crate::remote::sync_machines(&self.settings.remote_machines);
        self.remote.catalogs.remove(id);
        self.remote.notice = Some(format!(
            "{} was removed from this computer. Its host and any agents running there keep running.",
            machine.name
        ));
        cx.notify();
    }

    // ---- The host's CLIs and models --------------------------------------

    /// The host's CLIs and their models, for the composer's pickers in
    /// its projects. Asked once per connection; Reconnect asks again.
    pub(crate) fn load_machine_catalog(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(conn) = crate::remote::connection(id) else {
            return;
        };
        let entry = self.remote.catalogs.entry(id.to_string()).or_default();
        if entry.loading {
            return;
        }
        entry.loading = true;
        let task = cx.background_executor().spawn(async move {
            let harnesses: Vec<HarnessWire> = conn.call(
                Request::Harnesses,
                crate::remote::connection::DEFAULT_TIMEOUT,
            )?;
            let mut models = HashMap::new();
            for wire in harnesses.iter().filter(|h| h.available) {
                // Probing a CLI starts it; give it the time discovery takes.
                let listed: Result<Vec<ModelOption>, String> = conn.call(
                    Request::Models {
                        harness: wire.harness,
                    },
                    std::time::Duration::from_secs(40),
                );
                match listed {
                    Ok(list) => {
                        models.insert(wire.harness, list);
                    }
                    // The picker keeps this computer's list for it.
                    Err(err) => log::info!("remote: {}'s models: {err}", wire.harness.label()),
                }
            }
            Ok::<_, String>(MachineCatalog {
                harnesses,
                models,
                loading: false,
            })
        });
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let loaded = task.await;
            let landed = this.update(cx, |this, cx| {
                match loaded {
                    Ok(catalog) => {
                        this.remote.catalogs.insert(id, catalog);
                    }
                    Err(err) => {
                        log::info!("remote: the host's CLIs: {err}");
                        this.remote.catalogs.remove(&id);
                    }
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("machine catalog after app drop: {err:#}");
            }
        })
        .detach();
    }

    // ---- Open folder on a machine ------------------------------------------

    pub fn open_remote_folder_dialog(&mut self, cx: &mut Context<Self>) {
        self.remote.folder = Some(FolderDialog {
            machine: self.machines().first().map(|m| m.id.clone()),
            ..FolderDialog::default()
        });
        self.remote.path_input.update(cx, |input, cx| input.set_text("", cx));
        if let Some(id) = self.remote.folder.as_ref().and_then(|f| f.machine.clone()) {
            self.browse_remote_folder(&id, None, cx);
        }
        cx.notify();
    }

    pub fn close_remote_folder_dialog(&mut self, cx: &mut Context<Self>) {
        self.remote.folder = None;
        cx.notify();
    }

    pub fn pick_remote_folder_machine(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(dialog) = self.remote.folder.as_mut() else {
            return;
        };
        dialog.machine = Some(id.to_string());
        dialog.listing = None;
        self.remote.path_input.update(cx, |input, cx| input.set_text("", cx));
        self.browse_remote_folder(id, None, cx);
    }

    /// Lists `path`'s subfolders on the machine; `None` is its home.
    pub fn browse_remote_folder(&mut self, machine: &str, path: Option<String>, cx: &mut Context<Self>) {
        let Some(dialog) = self.remote.folder.as_mut() else {
            return;
        };
        let Some(conn) = crate::remote::connection(machine) else {
            return;
        };
        dialog.generation += 1;
        dialog.loading = true;
        dialog.opening = false;
        dialog.error = None;
        let generation = dialog.generation;
        let task = cx.background_executor().spawn(async move {
            let path = match path {
                Some(path) => path,
                None => conn.connect(None, None)?.home,
            };
            let entries: Vec<EntryWire> = conn.call(
                Request::ReadDir { path: path.clone() },
                crate::remote::connection::DEFAULT_TIMEOUT,
            )?;
            Ok::<_, String>(listing(&path, entries))
        });
        cx.spawn(async move |this, cx| {
            let listed = task.await;
            let landed = this.update(cx, |this, cx| {
                let Some(dialog) = this.remote.folder.as_mut() else {
                    return;
                };
                if dialog.generation != generation {
                    return;
                }
                dialog.loading = false;
                match listed {
                    Ok(listing) => {
                        let path = listing.path.clone();
                        dialog.listing = Some(listing);
                        this.remote
                            .path_input
                            .update(cx, |input, cx| input.set_text(path, cx));
                    }
                    Err(err) => dialog.error = Some(err),
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("folder listing after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// Open: the folder becomes a project of the rail, its threads run on
    /// the machine (MonoCode `projects.open` then `rememberRemoteProject`).
    pub fn open_remote_folder(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.remote.folder.as_mut() else {
            return;
        };
        let Some(machine) = dialog.machine.clone() else {
            return;
        };
        let path = self.remote.path_input.read(cx).text().trim().to_string();
        if path.is_empty() || dialog.opening {
            return;
        }
        if !path.starts_with('/') {
            dialog.error = Some("Enter the folder's absolute path on the machine.".into());
            cx.notify();
            return;
        }
        let Some(conn) = crate::remote::connection(&machine) else {
            return;
        };
        dialog.generation += 1;
        dialog.opening = true;
        dialog.loading = false;
        dialog.error = None;
        let generation = dialog.generation;
        let task = cx.background_executor().spawn({
            let path = path.clone();
            async move {
                let meta: Option<bencode_core::remote::protocol::MetaWire> = conn.call(
                    Request::Stat { path: path.clone() },
                    crate::remote::connection::DEFAULT_TIMEOUT,
                )?;
                match meta {
                    Some(meta) if meta.kind == EntryKind::Dir || meta.target_is_dir => Ok(()),
                    Some(_) => Err(format!("{path} is not a folder.")),
                    None => Err(format!("{path} does not exist on the machine.")),
                }
            }
        });
        cx.spawn(async move |this, cx| {
            let checked = task.await;
            let landed = this.update(cx, |this, cx| {
                let Some(dialog) = this.remote.folder.as_mut() else {
                    return;
                };
                if dialog.generation != generation {
                    return;
                }
                dialog.opening = false;
                match checked {
                    Ok(()) => {
                        this.remote.folder = None;
                        let project = crate::app::normalize_project_path(
                            &crate::remote::remote_path(&machine, &path),
                        );
                        this.update_rail_prefs(|prefs| prefs.with_restored(&project), cx);
                        this.switch_project(project, cx);
                        this.load_machine_catalog(&machine, cx);
                    }
                    Err(err) => dialog.error = Some(err),
                }
                cx.notify();
            });
            if let Err(err) = landed {
                log::debug!("folder open after app drop: {err:#}");
            }
        })
        .detach();
        cx.notify();
    }
}

/// `path`'s subfolders, sorted, as the picker lists them.
fn listing(path: &str, entries: Vec<EntryWire>) -> Listing {
    let mut folders: Vec<String> = entries
        .into_iter()
        .filter(|entry| entry.meta.kind == EntryKind::Dir || entry.meta.target_is_dir)
        .map(|entry| entry.name)
        .collect();
    folders.sort_by_key(|name| name.to_lowercase());
    folders.truncate(MAX_FOLDERS);
    let trimmed = path.trim_end_matches('/');
    let parent = match trimmed.rsplit_once('/') {
        Some(("", _)) if !trimmed.is_empty() => Some("/".to_string()),
        Some((parent, _)) => Some(parent.to_string()),
        None => None,
    };
    Listing {
        path: if trimmed.is_empty() { "/".into() } else { trimmed.into() },
        parent,
        folders,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bencode_core::remote::protocol::MetaWire;

    fn entry(name: &str, kind: EntryKind) -> EntryWire {
        EntryWire {
            name: name.into(),
            meta: MetaWire {
                kind,
                target_is_dir: false,
                size: 0,
                modified_ms: 0,
                readonly: false,
            },
        }
    }

    #[test]
    fn listings_keep_folders_sorted_with_their_parent() {
        let list = listing(
            "/home/me/",
            vec![
                entry("b", EntryKind::Dir),
                entry("a.txt", EntryKind::File),
                entry("A", EntryKind::Dir),
            ],
        );
        assert_eq!(list.path, "/home/me");
        assert_eq!(list.parent.as_deref(), Some("/home"));
        assert_eq!(list.folders, ["A", "b"]);
        assert_eq!(listing("/home", vec![]).parent.as_deref(), Some("/"));
        assert_eq!(listing("/", vec![]).parent, None);
    }
}
