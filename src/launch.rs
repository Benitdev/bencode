//! What a launch reads from the data folder before its window can draw:
//! MonoCode's data on a first launch, the settings, the database and the
//! threads, notes and automations the window opens on (BenCode's own).
//!
//! None of it needs GPUI, so `main` starts it on a thread of its own before
//! GPUI sets the platform up (about 70ms of the main thread), and takes it
//! once that is done. Parsing the recent threads' transcripts is most of it,
//! and grows with the history.

use std::thread::JoinHandle;

use crate::db::{AppDb, AutomationRow, Note, SessionRow};
use crate::monocode_import::{self, ImportOutcome};
use crate::settings::{self, AppSettings};

/// Threads loaded at launch; a project's others load when it opens.
pub const RECENT_SESSION_LIMIT: usize = 50;

pub struct LaunchData {
    pub settings: AppSettings,
    /// The first-launch import did not finish: nothing is saved this launch.
    pub import_error: Option<String>,
    pub db: AppDb,
    pub sessions: Vec<SessionRow>,
    pub notes: Vec<Note>,
    pub automations: Vec<AutomationRow>,
}

/// The read under way.
pub struct Pending(Option<JoinHandle<LaunchData>>);

/// Starts reading; with no thread to spare, `wait` reads in place.
pub fn start() -> Pending {
    let thread = std::thread::Builder::new()
        .name("bencode-launch".into())
        .spawn(read);
    match thread {
        Ok(handle) => Pending(Some(handle)),
        Err(err) => {
            log::warn!("reading the data folder on the main thread: {err:#}");
            Pending(None)
        }
    }
}

impl Pending {
    pub fn wait(self) -> LaunchData {
        match self.0 {
            // A panic there is one the main thread would have had.
            Some(handle) => handle
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            None => read(),
        }
    }
}

fn read() -> LaunchData {
    // Before the database opens: the first launch on its own data brings
    // along what a MonoCode install held.
    let imported = monocode_import::import_once();
    crate::startup_trace::mark("import checked");
    let mut settings = settings::settings_dir()
        .map(|dir| settings::load_from(&dir))
        .unwrap_or_default();
    let mut import_error = None;
    match imported {
        ImportOutcome::Nothing => {}
        ImportOutcome::Imported(imported) => {
            if monocode_import::merge_accounts(&mut settings.provider_accounts, imported.accounts)
                && let Some(dir) = settings::settings_dir()
                && let Err(err) = settings::save_to(&dir, &settings)
            {
                log::error!("saving the imported account names: {err:#}");
            }
        }
        ImportOutcome::Failed(err) => import_error = Some(err),
    }
    crate::startup_trace::mark("settings loaded");
    let db = if import_error.is_some() {
        // Opening the default path would create an empty database, and
        // the import would never run again.
        log::error!("the import failed; nothing will be saved this launch");
        AppDb::open_fallback()
    } else {
        AppDb::open_default().unwrap_or_else(|err| {
            log::error!("database unavailable ({err:#}); nothing will be saved this launch");
            AppDb::open_fallback()
        })
    };
    crate::startup_trace::mark("database opened");
    let sessions = db
        .list_recent_sessions(RECENT_SESSION_LIMIT)
        .unwrap_or_else(|err| {
            log::error!("failed to load sessions: {err:#}");
            Vec::new()
        });
    crate::startup_trace::mark("recent sessions loaded");
    let notes = db.list_notes().unwrap_or_else(|err| {
        log::error!("failed to load notes: {err:#}");
        Vec::new()
    });
    let automations = db.list_automations().unwrap_or_else(|err| {
        log::error!("failed to load automations: {err:#}");
        Vec::new()
    });
    crate::startup_trace::mark("notes and automations loaded");
    LaunchData {
        settings,
        import_error,
        db,
        sessions,
        notes,
        automations,
    }
}
