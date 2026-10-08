//! Notes: the surface's state, the open note's fields, and loading and
//! saving without blocking the UI thread. MonoCode
//! `features/notes/notes.ts` (titles, previews, tags) and the logic of
//! `features/notes/ui/NotesView.tsx`. The views are in `ui/notes/`.

use std::collections::HashMap;
use std::time::Duration;

use ely_gpui_component::forms::{InputEvent, TextInput};
use gpui::{App, AppContext, Context, Entity, Window};

use super::{BenCodeApp, NEW_SESSION_TITLE, Surface, now_ms, text_input, unique_id};
use crate::db::{Note, NoteUpsert};
use crate::ui::page_parts::tint;

pub const UNTITLED_NOTE: &str = "Untitled";
/// MonoCode `MAX_TITLE`, in characters.
const MAX_TITLE: usize = 200;
pub const MAX_NOTE_TAGS: usize = 20;
const MAX_NOTE_TAG_LENGTH: usize = 48;
/// A preview stops growing at this many characters.
const PREVIEW_CHARS: usize = 120;
/// MonoCode saves this long after the last keystroke (`NotesView.tsx`).
const AUTOSAVE_DELAY: Duration = Duration::from_millis(400);
/// MonoCode `min-h-[448px]` at `leading-5`: the Source field's least rows.
const SOURCE_MIN_ROWS: usize = 22;

/// How a note's body is shown (MonoCode `MarkdownViewMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NoteMode {
    #[default]
    Preview,
    Source,
}

/// The Notes surface's data, selection, fields and pending dialogs.
pub struct NotesState {
    pub items: Vec<Note>,
    pub selected_id: Option<String>,
    pub filter_input: Entity<TextInput>,
    pub title_input: Entity<TextInput>,
    pub body_input: Entity<TextInput>,
    pub tag_input: Entity<TextInput>,
    /// The open note's tags as edited; saved with its title and body.
    pub tags: Vec<String>,
    /// Preview or Source, remembered per note while the app runs.
    modes: HashMap<String, NoteMode>,
    /// The list's and the note's share of the width.
    pub shares: [f32; 2],
    /// A new note is on its way to the database.
    pub creating: bool,
    /// The Source field takes the keyboard on the next frame (a new note).
    pub focus_source: bool,
    /// Note id awaiting delete confirmation.
    pub pending_delete: Option<String>,
    /// Last failed save of the open note, shown with a Retry action.
    pub save_error: Option<String>,
    /// Last failed load, shown in place of an empty list.
    pub load_error: Option<String>,
    /// Bumped on every edit; a pending autosave only runs if it still matches.
    autosave_generation: u64,
    /// Counts list loads; `listed` is the newest one shown.
    list_requests: u64,
    listed: u64,
}

impl NotesState {
    /// The fields, with no notes loaded yet.
    pub fn new(window: &mut Window, cx: &mut Context<BenCodeApp>) -> Self {
        let body_input = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line(SOURCE_MIN_ROWS, usize::MAX)
                .placeholder("Write markdown…")
                .highlighter(source_highlights)
        });
        Self {
            items: Vec::new(),
            selected_id: None,
            filter_input: text_input(window, cx, "Filter notes"),
            title_input: text_input(window, cx, UNTITLED_NOTE),
            body_input,
            tag_input: text_input(window, cx, "Add tag…"),
            tags: Vec::new(),
            modes: HashMap::new(),
            shares: [0.24, 0.76],
            creating: false,
            focus_source: false,
            pending_delete: None,
            save_error: None,
            load_error: None,
            autosave_generation: 0,
            list_requests: 0,
            listed: 0,
        }
    }

    pub fn selected(&self) -> Option<&Note> {
        let id = self.selected_id.as_deref()?;
        self.items.iter().find(|note| note.id == id)
    }

    /// The open note's mode: what was last chosen for it, else Preview.
    pub fn mode(&self) -> NoteMode {
        self.selected_id
            .as_ref()
            .and_then(|id| self.modes.get(id))
            .copied()
            .unwrap_or_default()
    }
}

/// MonoCode `isAtxHeadingLine`: up to three spaces, one to six `#`, then a
/// space or the line's end. Returns what follows the marker.
fn atx_heading(line: &str) -> Option<&str> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = line.get(indent..).filter(|_| indent <= 3)?;
    let marks = rest.len() - rest.trim_start_matches('#').len();
    let text = rest.get(marks..).filter(|_| (1..=6).contains(&marks))?;
    (text.is_empty() || text.starts_with(char::is_whitespace)).then_some(text)
}

/// MonoCode `MarkdownSourceHighlight`: heading lines in full ink, the rest
/// a step quieter.
fn source_highlights(text: &str, cx: &App) -> Vec<(std::ops::Range<usize>, ely_gpui_component::forms::Highlight)> {
    use ely_gpui_component::forms::Highlight;
    use ely_gpui_component::theme::ActiveTheme;
    let fg = cx.theme().colors.fg;
    let mut spans = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let end = start + line.trim_end_matches('\n').len();
        if end > start {
            let ink = if atx_heading(&text[start..end]).is_some() {
                fg
            } else {
                fg.opacity(tint::SOURCE)
            };
            spans.push((start..end, Highlight::new(ink)));
        }
        start += line.len();
    }
    spans
}

/// MonoCode `unwrapMarkdown`: images dropped, links reduced to their text,
/// emphasis and code marks removed.
fn unwrap_markdown(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(open) = rest.find('[') {
        let image = rest[..open].ends_with('!');
        // `[text](url)`: the text may not hold a `]`, the url not a `)`.
        let link = rest[open + 1..].find(']').and_then(|close| {
            let close = open + 1 + close;
            let url = rest[close + 1..].strip_prefix('(')?;
            let end = url.find(')')?;
            Some((close, close + 2 + end + 1))
        });
        match link {
            Some((close, after)) if image || close > open + 1 => {
                out.push_str(&rest[..open - usize::from(image)]);
                if !image {
                    out.push_str(&rest[open + 1..close]);
                }
                rest = &rest[after..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = &rest[open + 1..];
            }
        }
    }
    out.push_str(rest);
    out.retain(|c| !matches!(c, '*' | '_' | '`'));
    out.trim().to_string()
}

fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

/// The lines outside fenced code blocks.
fn prose_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut in_fence = false;
    text.lines().filter(move |line| {
        if is_fence(line) {
            in_fence = !in_fence;
            return false;
        }
        !in_fence
    })
}

fn clipped(text: String, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// MonoCode `noteTitle`: the first markdown heading, else the first line of
/// prose, else "Untitled".
pub fn note_title(text: &str) -> String {
    let heading = text
        .lines()
        .filter_map(atx_heading)
        .map(str::trim)
        .find(|heading| !heading.is_empty());
    let candidates = heading.into_iter().chain(
        prose_lines(text)
            .map(str::trim)
            .filter(|line| !line.is_empty() && *line != "---"),
    );
    candidates
        .map(|line| clipped(unwrap_markdown(line), MAX_TITLE))
        .find(|title| !title.is_empty())
        .unwrap_or_else(|| UNTITLED_NOTE.to_string())
}

/// MonoCode `notePreview`: the note's first prose, without headings, list
/// marks or its own title; at most a line's worth.
pub fn note_preview(text: &str, title: &str) -> String {
    let mut preview = String::new();
    for line in prose_lines(text) {
        let line = line.trim();
        if line.is_empty() || line == "---" || atx_heading(line).is_some() {
            continue;
        }
        let item = ['-', '*', '+']
            .into_iter()
            .find_map(|mark| line.strip_prefix(mark))
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .map_or(line, str::trim_start);
        let next = unwrap_markdown(item);
        if next.is_empty() || next == title {
            continue;
        }
        if !preview.is_empty() {
            preview.push(' ');
        }
        preview.push_str(&next);
        if preview.chars().count() >= PREVIEW_CHARS {
            break;
        }
    }
    if preview.chars().count() > PREVIEW_CHARS {
        preview = clipped(preview, PREVIEW_CHARS - 1);
        preview.push('…');
    }
    preview
}

/// MonoCode `normalizeNoteTags`: lower-case, `#` and spaces removed, no
/// repeats, at most `MAX_NOTE_TAGS`.
pub fn normalize_note_tags<S: AsRef<str>>(tags: &[S]) -> Vec<String> {
    let mut normalized: Vec<String> = Vec::new();
    for input in tags {
        let words: Vec<&str> = input.as_ref().trim().trim_start_matches('#').split_whitespace().collect();
        let tag = clipped(words.join("-").to_lowercase(), MAX_NOTE_TAG_LENGTH);
        let tag = tag.trim_end_matches('-');
        if tag.is_empty() || normalized.iter().any(|seen| seen == tag) {
            continue;
        }
        normalized.push(tag.to_string());
        if normalized.len() == MAX_NOTE_TAGS {
            break;
        }
    }
    normalized
}

/// MonoCode `looksLikeProject`: a folder a note can belong to, so not the
/// root, the home folder or the inside of an app bundle.
fn looks_like_project(path: &str, home: Option<&str>) -> bool {
    let trimmed = path.trim_end_matches('/');
    !(trimmed.is_empty() || trimmed == "~" || home == Some(trimmed) || path.contains(".app/"))
}

/// MonoCode `noteSourceProject`: the folder name of the note's project.
pub fn note_project(note: &Note) -> Option<&str> {
    let cwd = note.source_cwd.as_deref()?;
    let home = std::env::var("HOME").ok();
    let name = cwd.trim_end_matches('/').rsplit('/').next()?;
    (looks_like_project(cwd, home.as_deref()) && !name.is_empty()).then_some(name)
}

/// MonoCode's filter: title, body, slug, tags (with or without `#`) and
/// project. `query` must already be lower-case.
pub fn note_matches(note: &Note, query: &str) -> bool {
    let tag = query.trim_start_matches('#');
    query.is_empty()
        || [&note.title, &note.body, &note.slug]
            .into_iter()
            .any(|text| text.to_lowercase().contains(query))
        || note.tags.iter().any(|t| t.contains(tag))
        || note_project(note).is_some_and(|p| p.to_lowercase().contains(query))
}

/// The title a note is saved under: what was typed, else `note_title`.
fn saved_title(typed: &str, body: &str) -> String {
    match typed.trim() {
        "" => note_title(body),
        typed => typed.to_string(),
    }
}

/// MonoCode `onSaved`'s order: newest edit first.
fn sort_notes(notes: &mut [Note]) {
    notes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.id.cmp(&b.id)));
}

impl BenCodeApp {
    pub fn open_notes(&mut self, cx: &mut Context<Self>) {
        self.show_surface(Surface::Notes, cx);
        self.refresh_notes(cx);
        cx.notify();
    }

    pub fn close_notes(&mut self, cx: &mut Context<Self>) {
        if self.surface_open(Surface::Notes) {
            self.close_surface(cx);
        }
    }

    fn next_note_list(&mut self) -> u64 {
        self.notes.list_requests += 1;
        self.notes.list_requests
    }

    fn refresh_notes(&mut self, cx: &mut Context<Self>) {
        let request = self.next_note_list();
        self.db_then(
            cx,
            |db| db.list_notes(),
            move |this, listed, cx| match listed {
                Ok(items) => this.show_notes(request, items, cx),
                Err(err) => {
                    log::error!("list_notes failed: {err:#}");
                    this.notes.load_error = Some(format!("Could not load notes: {err}"));
                    cx.notify();
                }
            },
        );
    }

    /// Shows the list `request` loaded, unless a later load is already
    /// shown. The open note stays open; a deleted one gives way to the first.
    fn show_notes(&mut self, request: u64, items: Vec<Note>, cx: &mut Context<Self>) {
        if request < self.notes.listed {
            return;
        }
        self.notes.listed = request;
        self.notes.load_error = None;
        self.notes.items = items;
        if self.notes.selected().is_none() {
            self.notes.selected_id = None;
            match self.notes.items.first().map(|n| n.id.clone()) {
                Some(id) => self.select_note(id, cx),
                None => self.load_note_fields(None, cx),
            }
        }
        cx.notify();
    }

    /// Puts `note` (or nothing) in the title, tags and Source fields.
    fn load_note_fields(&mut self, note: Option<&Note>, cx: &mut Context<Self>) {
        let (title, body, tags) = note.map_or_else(Default::default, |note| {
            (note.title.clone(), note.body.clone(), note.tags.clone())
        });
        self.notes.title_input.update(cx, |input, cx| input.set_text(title, cx));
        self.notes.body_input.update(cx, |input, cx| input.set_text(body, cx));
        self.notes.tag_input.update(cx, |input, cx| input.set_text("", cx));
        self.notes.tags = tags;
        self.notes.save_error = None;
    }

    pub(crate) fn select_note(&mut self, id: String, cx: &mut Context<Self>) {
        if self.notes.selected_id.as_ref() != Some(&id) {
            self.save_note_if_dirty(cx);
        }
        let note = self.notes.items.iter().find(|n| n.id == id).cloned();
        self.load_note_fields(note.as_ref(), cx);
        self.notes.selected_id = Some(id);
        cx.notify();
    }

    pub(crate) fn set_note_mode(&mut self, mode: NoteMode, cx: &mut Context<Self>) {
        if let Some(id) = self.notes.selected_id.clone() {
            self.notes.modes.insert(id, mode);
            cx.notify();
        }
    }

    /// MonoCode `onCreate`: an Untitled note in the open project, opened in
    /// Source so typing is not behind the preview.
    pub(crate) fn create_new_note(&mut self, cx: &mut Context<Self>) {
        if self.notes.creating {
            return;
        }
        self.save_note_if_dirty(cx);
        self.notes.creating = true;
        let home = std::env::var("HOME").ok();
        let upsert = NoteUpsert {
            id: unique_id("note"),
            title: UNTITLED_NOTE.to_string(),
            body: String::new(),
            tags: Vec::new(),
            source_session_id: None,
            source_cwd: Some(self.current_cwd.clone())
                .filter(|cwd| looks_like_project(cwd, home.as_deref())),
        };
        self.store_new_note(upsert, true, cx);
        cx.notify();
    }

    /// Saves a turn as a note titled after its thread (or the text's first
    /// line), linked to that thread and project, then opens it (MonoCode
    /// `SessionPane` "Save as note").
    pub fn save_turn_to_note(&mut self, text: &str, cx: &mut Context<Self>) {
        let session = self.selected_session();
        let title = session
            .map(|s| s.title.clone())
            .filter(|t| !t.is_empty() && t != NEW_SESSION_TITLE)
            .unwrap_or_else(|| note_title(text));
        let upsert = NoteUpsert {
            id: unique_id("note"),
            title: clipped(title, MAX_TITLE),
            body: text.to_string(),
            tags: Vec::new(),
            source_session_id: session.map(|s| s.id.clone()),
            source_cwd: session.map(|s| s.cwd.clone()).filter(|cwd| !cwd.is_empty()),
        };
        self.store_new_note(upsert, false, cx);
    }

    /// Stores a new note, then shows it in the Notes view; `blank` notes
    /// open in Source with the keyboard.
    fn store_new_note(&mut self, upsert: NoteUpsert, blank: bool, cx: &mut Context<Self>) {
        let request = self.next_note_list();
        self.db_then(
            cx,
            move |db| {
                let note = db.upsert_note(&upsert)?;
                Ok((note.id, db.list_notes()?))
            },
            move |this, stored, cx| {
                this.notes.creating = false;
                match stored {
                    Ok((id, items)) => {
                        this.open_notes(cx);
                        this.show_notes(request, items, cx);
                        this.notes.filter_input.update(cx, |input, cx| input.set_text("", cx));
                        if blank {
                            this.notes.modes.insert(id.clone(), NoteMode::Source);
                            this.notes.focus_source = true;
                        }
                        this.select_note(id, cx);
                    }
                    Err(err) => {
                        log::error!("could not create the note: {err:#}");
                        this.notes.load_error = Some(format!("Could not create the note: {err}"));
                        cx.notify();
                    }
                }
            },
        );
    }

    /// Title and body edits save themselves `AUTOSAVE_DELAY` after typing
    /// stops, and right away when a field loses focus. A title left empty
    /// becomes the note's first line.
    pub(crate) fn on_note_input_event(
        &mut self,
        input: Entity<TextInput>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => {
                self.schedule_note_autosave(cx);
                cx.notify();
            }
            InputEvent::Blur | InputEvent::Submit => {
                if input == self.notes.title_input && input.read(cx).text().trim().is_empty() {
                    let title = note_title(self.notes.body_input.read(cx).text());
                    input.update(cx, |input, cx| input.set_text(title, cx));
                }
                self.save_note_if_dirty(cx);
            }
            InputEvent::Focus => {}
        }
    }

    /// MonoCode `NoteTagsEditor`: Enter, a comma or leaving the field adds
    /// what was typed.
    pub(crate) fn on_note_tag_input_event(
        &mut self,
        input: Entity<TextInput>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        let typed = input.read(cx).text().to_string();
        let add = match event {
            InputEvent::Changed => typed.ends_with(','),
            InputEvent::Submit | InputEvent::Blur => !typed.trim().is_empty(),
            InputEvent::Focus => false,
        };
        if add {
            input.update(cx, |input, cx| input.set_text("", cx));
            self.add_note_tag(typed.trim_end_matches(','), cx);
        }
    }

    fn add_note_tag(&mut self, tag: &str, cx: &mut Context<Self>) {
        let mut tags = self.notes.tags.clone();
        tags.push(tag.to_string());
        self.set_note_tags(normalize_note_tags(&tags), cx);
    }

    pub(crate) fn remove_note_tag(&mut self, tag: &str, cx: &mut Context<Self>) {
        let tags = self.notes.tags.iter().filter(|t| *t != tag).cloned().collect();
        self.set_note_tags(tags, cx);
    }

    fn set_note_tags(&mut self, tags: Vec<String>, cx: &mut Context<Self>) {
        if tags != self.notes.tags {
            self.notes.tags = tags;
            self.save_note_if_dirty(cx);
            cx.notify();
        }
    }

    fn schedule_note_autosave(&mut self, cx: &mut Context<Self>) {
        self.notes.autosave_generation += 1;
        let generation = self.notes.autosave_generation;
        let timer = cx.background_executor().timer(AUTOSAVE_DELAY);
        cx.spawn(async move |this, cx| {
            timer.await;
            let saved = this.update(cx, |this, cx| {
                if this.notes.autosave_generation == generation {
                    this.save_note_if_dirty(cx);
                }
            });
            if let Err(err) = saved {
                log::debug!("note autosave after app drop: {err:#}");
            }
        })
        .detach();
    }

    /// The open note as its fields have it now: MonoCode's `draft`.
    pub(crate) fn note_draft(&self, cx: &App) -> Option<Note> {
        let note = self.notes.selected()?;
        let body = self.notes.body_input.read(cx).text().to_string();
        Some(Note {
            title: saved_title(self.notes.title_input.read(cx).text(), &body),
            body,
            tags: self.notes.tags.clone(),
            ..note.clone()
        })
    }

    /// Saves the open note when its fields differ from what is stored, so
    /// switching, creating or closing never drops edits.
    pub(crate) fn save_note_if_dirty(&mut self, cx: &mut Context<Self>) {
        let (Some(draft), Some(stored)) = (self.note_draft(cx), self.notes.selected()) else {
            return;
        };
        if draft.title != stored.title || draft.body != stored.body || draft.tags != stored.tags {
            self.save_note(draft, None, cx);
        }
    }

    /// Retry after a failed save.
    pub(crate) fn retry_note_save(&mut self, cx: &mut Context<Self>) {
        self.notes.save_error = None;
        if let Some(draft) = self.note_draft(cx) {
            self.save_note(draft, None, cx);
        }
        cx.notify();
    }

    /// MonoCode's project picker in `move` mode: the note now belongs to
    /// `project`.
    pub(crate) fn move_note_to_project(&mut self, project: &str, cx: &mut Context<Self>) {
        if let Some(draft) = self.note_draft(cx) {
            self.save_note(draft, Some(project.to_string()), cx);
        }
    }

    /// Stores `draft`; `project` moves it (`None` keeps where it is). Saves
    /// run on the database thread in the order they were asked for.
    fn save_note(&mut self, draft: Note, project: Option<String>, cx: &mut Context<Self>) {
        let upsert = NoteUpsert {
            id: draft.id,
            title: draft.title,
            body: draft.body,
            tags: draft.tags,
            source_session_id: draft.source_session_id,
            source_cwd: project,
        };
        self.db_then(
            cx,
            move |db| db.upsert_note(&upsert),
            |this, saved, cx| {
                match saved {
                    Ok(saved) => {
                        if let Some(note) = this.notes.items.iter_mut().find(|n| n.id == saved.id) {
                            let open = this.notes.selected_id.as_deref() == Some(saved.id.as_str());
                            *note = saved;
                            sort_notes(&mut this.notes.items);
                            if open {
                                this.notes.save_error = None;
                            }
                        }
                    }
                    Err(err) => {
                        log::error!("upsert_note failed: {err:#}");
                        this.notes.save_error = Some(format!("Could not save note: {err}"));
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn delete_note(&mut self, id: &str, cx: &mut Context<Self>) {
        self.notes.pending_delete = None;
        // The note is going: its unsaved edits go with it.
        if self.notes.selected_id.as_deref() == Some(id) {
            self.notes.autosave_generation += 1;
            self.notes.selected_id = None;
        }
        let (request, id) = (self.next_note_list(), id.to_string());
        self.db_then(
            cx,
            move |db| {
                db.delete_note(&id)?;
                db.list_notes()
            },
            move |this, listed, cx| match listed {
                Ok(items) => this.show_notes(request, items, cx),
                Err(err) => {
                    log::error!("delete_note failed: {err:#}");
                    this.notes.load_error = Some(format!("Could not delete the note: {err}"));
                    this.refresh_notes(cx);
                }
            },
        );
        cx.notify();
    }

    /// MonoCode "Add to chat": a new thread carrying the note as a card.
    pub(crate) fn add_selected_note_to_chat(&mut self, cx: &mut Context<Self>) {
        if let Some(note) = self.note_draft(cx) {
            self.save_note_if_dirty(cx);
            self.add_note_to_chat(&note, cx);
        }
    }

    /// When the open note was last saved, for "Updated …".
    pub(crate) fn note_updated(&self) -> Option<String> {
        let note = self.notes.selected()?;
        Some(crate::ui::relative_time::since(note.updated_at, now_ms()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(title: &str, body: &str, tags: &[&str]) -> Note {
        Note {
            id: "n".into(),
            slug: "release-plan".into(),
            title: title.into(),
            body: body.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            source_cwd: Some("/work/storefront".into()),
            ..Default::default()
        }
    }

    #[test]
    fn titles_come_from_the_first_heading_or_line() {
        assert_eq!(note_title("intro\n\n## The **Plan**\nmore"), "The Plan");
        assert_eq!(note_title("\n---\n[Ship](https://x.dev) on `Friday`"), "Ship on Friday");
        assert_eq!(note_title("```\ncode\n```\nafter"), "after");
        assert_eq!(note_title("![shot](a.png)\n\nreal"), "real");
        assert_eq!(note_title("  \n"), "Untitled");
        // `#hashtag` is not a heading.
        assert_eq!(note_title("#tag first"), "#tag first");
        assert_eq!(note_title(&"x".repeat(300)).chars().count(), 200);
    }

    #[test]
    fn previews_skip_headings_fences_and_the_title() {
        let body = "# Plan\nPlan\n- ship *it*\n```\ncode\n```\n+ then rest";
        assert_eq!(note_preview(body, "Plan"), "ship it then rest");
        assert_eq!(note_preview("# Only a heading", "Only a heading"), "");
        let long = note_preview(&"word ".repeat(60), "t");
        assert_eq!(long.chars().count(), 120);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn tags_are_normalized_like_monocode() {
        assert_eq!(
            normalize_note_tags(&["  #Release Plan ", "release-plan", "", "##x", "a b  c-"]),
            ["release-plan", "x", "a-b-c"]
        );
        let many: Vec<String> = (0..30).map(|n| format!("t{n}")).collect();
        assert_eq!(normalize_note_tags(&many).len(), MAX_NOTE_TAGS);
        assert_eq!(normalize_note_tags(&["x".repeat(80)])[0].len(), 48);
    }

    #[test]
    fn the_filter_reads_title_body_slug_tags_and_project() {
        let n = note("Plan", "Ship the Parser", &["rust"]);
        for query in ["", "parser", "rust", "#rust", "release-plan", "storefront"] {
            assert!(note_matches(&n, query), "{query}");
        }
        assert!(!note_matches(&n, "golang"));
    }

    #[test]
    fn projects_exclude_home_root_and_app_bundles() {
        let home = Some("/Users/me");
        assert!(looks_like_project("/Users/me/code/app", home));
        for path in ["", "/", "~", "/Users/me", "/Users/me/", "/Applications/X.app/Contents"] {
            assert!(!looks_like_project(path, home), "{path}");
        }
        assert_eq!(note_project(&note("t", "", &[])), Some("storefront"));
    }

    #[test]
    fn an_empty_title_is_taken_from_the_body() {
        assert_eq!(saved_title("  ", "# From body"), "From body");
        assert_eq!(saved_title(" Typed ", "# From body"), "Typed");
        assert_eq!(saved_title("", ""), "Untitled");
    }

    #[test]
    fn headings_need_a_space_after_the_marks() {
        assert_eq!(atx_heading("## Two"), Some(" Two"));
        assert_eq!(atx_heading("   # indented"), Some(" indented"));
        assert_eq!(atx_heading("#"), Some(""));
        for line in ["#tag", "    # code", "####### seven", "plain"] {
            assert_eq!(atx_heading(line), None, "{line}");
        }
    }
}
