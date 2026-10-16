//! A Mono's artifacts on the app (MonoCode `App.tsx`'s agent-app host:
//! `saveArtifact`, `postArtifact`, `openArtifact`; `ArtifactReader`'s
//! state): the credential each Mono's turn carries for `bencode app`, the
//! calls it makes (`app_cli/artifacts.rs`), the card a write leaves on its
//! turn, and the artifact the Overview shows over its Documents list, with
//! the native view an `html` one is drawn in (`browser/artifact.rs`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{Context, Subscription, Window};
use serde_json::json;
use tokio::sync::mpsc;

use crate::app::BenCodeApp;
use crate::app::mono_overview::OverviewTab;
use crate::app_cli::artifacts::{self as calls, ArtifactCard, Call};
use crate::app_cli::server::{self, AppCall};
use crate::browser::artifact::{ArtifactEvent, ArtifactPage};
use crate::db::Artifact;
use bencode_core::harness::AppAccess;

/// MonoCode `APP_TURN_INACTIVE`.
const TURN_INACTIVE: &str = "BenCode app access is inactive: it works only during a Mono's \
    running turn. Retrying this request now will not enable access.";

/// The artifact open over the Documents list (MonoCode `useArtifact`).
pub struct Reader {
    pub id: String,
    /// `None` until read, and when it is gone.
    pub artifact: Option<Artifact>,
    pub loaded: bool,
    pub error: Option<String>,
    /// Copy was pressed: its button says Copied.
    pub copied: bool,
    generation: u64,
}

pub struct ArtifactsState {
    /// Where `bencode app` reaches this app; None when the socket could not
    /// be opened.
    socket: Option<PathBuf>,
    /// Each Mono conversation's credential, made at its first turn.
    tokens: HashMap<String, String>,
    pub reader: Option<Reader>,
    /// The view an `html` artifact is drawn in, made the first time one is
    /// shown and kept for the next.
    pub page: Option<Rc<ArtifactPage>>,
    /// The reader drew the page this frame; it hides otherwise.
    pub page_drawn: std::cell::Cell<bool>,
    page_events: mpsc::UnboundedSender<ArtifactEvent>,
    /// The artifact awaiting Delete's confirmation: (id, title).
    pub pending_delete: Option<(String, String)>,
    next_generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl ArtifactsState {
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> Self {
        let (calls_tx, mut calls_rx) = mpsc::unbounded_channel::<AppCall>();
        let socket = match server::listen(calls_tx) {
            Ok(path) => Some(path),
            Err(err) => {
                log::error!("app access is off: {err:#}");
                None
            }
        };
        cx.spawn(async move |this, cx| {
            while let Some(call) = calls_rx.recv().await {
                if this.update(cx, |this, cx| this.handle_app_call(call, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let (page_events, mut events_rx) = mpsc::unbounded_channel::<ArtifactEvent>();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = events_rx.recv().await {
                let landed = this.update(cx, |this, cx| match event {
                    ArtifactEvent::OpenLink(url) => {
                        // Only a link the user just pressed: the page has
                        // the keyboard then.
                        if this.artifacts.page.as_ref().is_some_and(|page| page.has_keys()) {
                            cx.open_url(&url);
                        }
                    }
                });
                if landed.is_err() {
                    break;
                }
            }
        })
        .detach();
        let mut subscriptions = Vec::new();
        if let Some(path) = socket.clone() {
            subscriptions.push(cx.on_app_quit(move |_, _| {
                server::remove(&path);
                async {}
            }));
        }
        Self {
            socket,
            tokens: HashMap::new(),
            reader: None,
            page: None,
            page_drawn: std::cell::Cell::new(false),
            page_events,
            pending_delete: None,
            next_generation: 0,
            _subscriptions: subscriptions,
        }
    }

    /// Hides the page when the reader did not draw it this frame.
    pub fn hide_undrawn(&self) {
        if !self.page_drawn.get()
            && let Some(page) = &self.page
        {
            page.hide();
        }
    }
}

impl BenCodeApp {
    /// What a turn of `session_id` carries for `bencode app`: a Mono's own
    /// conversation only (MonoCode: "App access is always enabled in this
    /// thread"), on this computer.
    pub(crate) fn app_access_for(&mut self, session_id: &str, cwd: &str) -> Option<AppAccess> {
        if !self.has_app_access(session_id) || crate::remote::is_remote(cwd) {
            return None;
        }
        let socket = self.artifacts.socket.as_ref()?.to_string_lossy().into_owned();
        let token = self
            .artifacts
            .tokens
            .entry(session_id.to_string())
            .or_insert_with(|| crate::app_cli::random_hex(32))
            .clone();
        Some(AppAccess { socket, token })
    }

    /// Whether `session_id`'s turns get `bencode app`: a Mono's, while the
    /// socket is open.
    pub(crate) fn has_app_access(&self, session_id: &str) -> bool {
        self.artifacts.socket.is_some() && self.is_mono_session(session_id)
    }

    /// One `bencode app` call: whose turn sent it, then the action.
    fn handle_app_call(&mut self, call: AppCall, cx: &mut Context<Self>) {
        let session = self
            .artifacts
            .tokens
            .iter()
            .find(|(_, token)| **token == call.request.token)
            .map(|(session, _)| session.clone());
        let Some(session) = session else {
            call.fail("Connection revoked or unauthorized", false);
            return;
        };
        if !self.is_mono_session(&session) {
            call.fail("Only a Mono can create or read artifacts", false);
            return;
        }
        if !self.is_agent_running_in(&session) {
            call.fail(TURN_INACTIVE, false);
            return;
        }
        let parsed = calls::parse(&call.request.action, &call.request.input);
        let parsed = match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                call.fail(error, true);
                return;
            }
        };
        match parsed {
            Call::List {
                kind,
                limit,
                offset,
            } => self.db_then(
                cx,
                |db| db.list_artifacts(),
                move |_, listed, _| match listed {
                    Ok(all) => call.answer(calls::listing(all, kind, limit, offset)),
                    Err(err) => call.fail(format!("{err:#}"), true),
                },
            ),
            Call::Read { id } => self.db_then(
                cx,
                move |db| db.get_artifact(&id),
                move |_, read, _| match read {
                    Ok(Some(artifact)) => match serde_json::to_value(artifact) {
                        Ok(value) => call.answer(value),
                        Err(err) => call.fail(err.to_string(), true),
                    },
                    Ok(None) => call.fail("Artifact was not found", true),
                    Err(err) => call.fail(format!("{err:#}"), true),
                },
            ),
            Call::Write(write) => {
                let request_id = call.request.request_id.clone();
                let summary = write.summary.clone();
                let source = session.clone();
                self.db_then(
                    cx,
                    move |db| calls::save(db, &source, &request_id, &write),
                    move |this, saved, cx| match saved {
                        Ok(artifact) => {
                            let card = ArtifactCard {
                                id: artifact.id.clone(),
                                kind: artifact.kind,
                                title: artifact.title.clone(),
                                summary,
                            };
                            this.artifact_saved(artifact);
                            if this.post_artifact(&session, &card) {
                                let mut reply = serde_json::to_value(&card).unwrap_or(json!({}));
                                reply["saved"] = json!(true);
                                reply["attached"] = json!(true);
                                call.answer(reply);
                            } else {
                                call.fail(
                                    "The artifact's conversation turn is no longer available",
                                    true,
                                );
                            }
                            cx.notify();
                        }
                        Err(err) => call.fail(format!("{err:#}"), true),
                    },
                );
            }
        }
    }

    /// MonoCode `postArtifact`: the card on the turn's message, saved with
    /// the conversation.
    fn post_artifact(&mut self, session_id: &str, card: &ArtifactCard) -> bool {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else {
            return false;
        };
        if !calls::record_card(&mut session.blocks, card) {
            return false;
        }
        self.persist_session(session_id);
        true
    }

    /// A save the reader may be showing.
    fn artifact_saved(&mut self, artifact: Artifact) {
        if let Some(reader) = self
            .artifacts
            .reader
            .as_mut()
            .filter(|reader| reader.id == artifact.id)
        {
            reader.artifact = Some(artifact);
            reader.loaded = true;
            reader.error = None;
        }
    }

    /// MonoCode `onOpenMonoArtifact`: the artifact over the Overview's
    /// Documents list, the Overview opening if it was hidden.
    pub fn open_artifact(&mut self, id: &str, cx: &mut Context<Self>) {
        self.monos.overview_open = true;
        self.monos.overview.select(OverviewTab::Documents);
        self.monos.overview.document = Some(id.to_string());
        self.load_artifact(id, cx);
        cx.notify();
    }

    fn load_artifact(&mut self, id: &str, cx: &mut Context<Self>) {
        self.artifacts.next_generation += 1;
        let generation = self.artifacts.next_generation;
        self.artifacts.reader = Some(Reader {
            id: id.to_string(),
            artifact: None,
            loaded: false,
            error: None,
            copied: false,
            generation,
        });
        let id = id.to_string();
        self.db_then(
            cx,
            move |db| db.get_artifact(&id),
            move |this, read, cx| {
                let Some(reader) = this
                    .artifacts
                    .reader
                    .as_mut()
                    .filter(|reader| reader.generation == generation)
                else {
                    return;
                };
                reader.loaded = true;
                match read {
                    Ok(artifact) => reader.artifact = artifact,
                    Err(err) => {
                        log::warn!("could not read artifact {}: {err:#}", reader.id);
                        reader.error = Some("Could not load this document.".into());
                    }
                }
                cx.notify();
            },
        );
    }

    /// Back from the reader to the Documents list.
    pub fn close_artifact(&mut self, cx: &mut Context<Self>) {
        self.monos.overview.document = None;
        self.artifacts.reader = None;
        cx.notify();
    }

    pub fn copy_artifact(&mut self, cx: &mut Context<Self>) {
        let Some(reader) = self.artifacts.reader.as_mut() else {
            return;
        };
        let Some(artifact) = &reader.artifact else {
            return;
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(artifact.body.clone()));
        reader.copied = true;
        self.play_cue(crate::sounds::Cue::Copy);
        cx.notify();
    }

    /// Delete asks first (MonoCode's `window.confirm`).
    pub fn request_delete_artifact(&mut self, cx: &mut Context<Self>) {
        let Some(artifact) = self
            .artifacts
            .reader
            .as_ref()
            .and_then(|reader| reader.artifact.as_ref())
        else {
            return;
        };
        self.artifacts.pending_delete = Some((artifact.id.clone(), artifact.title.clone()));
        cx.notify();
    }

    pub fn cancel_delete_artifact(&mut self, cx: &mut Context<Self>) {
        self.artifacts.pending_delete = None;
        cx.notify();
    }

    /// MonoCode `deleteArtifact` and `ARTIFACT_DELETED_EVENT`: gone from the
    /// database and from every chat's cards, then the reader goes back.
    pub fn confirm_delete_artifact(&mut self, cx: &mut Context<Self>) {
        let Some((id, _)) = self.artifacts.pending_delete.take() else {
            return;
        };
        let mut touched = Vec::new();
        for session in &mut self.sessions {
            if calls::remove_card(&mut session.blocks, &id) {
                touched.push(session.id.clone());
            }
        }
        for session_id in &touched {
            self.persist_session(session_id);
        }
        let deleting = id.clone();
        self.db_then(
            cx,
            move |db| db.delete_artifact(&deleting),
            move |this, deleted, cx| {
                match deleted {
                    Ok(()) => {
                        if this.monos.overview.document.as_deref() == Some(id.as_str()) {
                            this.close_artifact(cx);
                        }
                    }
                    Err(err) => {
                        log::error!("could not delete artifact {id}: {err:#}");
                        if let Some(reader) =
                            this.artifacts.reader.as_mut().filter(|r| r.id == id)
                        {
                            reader.error = Some("Could not delete this document.".into());
                        }
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    /// The page view for the reader's `html` artifact, made once a window
    /// is at hand (`render`), and handed the page it shows.
    pub(crate) fn sync_artifact_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.artifacts.page_drawn.set(false);
        let Some(artifact) = self
            .artifacts
            .reader
            .as_ref()
            .and_then(|reader| reader.artifact.as_ref())
            .filter(|artifact| artifact.kind == crate::db::ArtifactKind::Html)
        else {
            return;
        };
        if self.artifacts.page.is_none() {
            match ArtifactPage::build(window, self.artifacts.page_events.clone()) {
                Ok(page) => self.artifacts.page = Some(Rc::new(page)),
                Err(err) => {
                    log::error!("{err:#}");
                    return;
                }
            }
        }
        let Some(page) = &self.artifacts.page else {
            return;
        };
        page.show(
            &artifact.id,
            artifact.updated_at,
            crate::ui::artifacts::theme_key(cx),
            || crate::browser::artifact::page_html(&artifact.body, &crate::ui::artifacts::theme_css(cx)),
        );
    }
}
