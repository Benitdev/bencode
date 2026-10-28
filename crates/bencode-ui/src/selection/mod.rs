//! Selecting text with the pointer, as a web page does: a drag across
//! paragraphs and messages, a double or triple click, shift to extend, ⌘A
//! and ⌘C. Adapted from gpui-kit's window text selection (gpui-base
//! `text_selection`, Apache-2.0, NOTICE): one layer listens for the window
//! (`SelectionLayer`), text registers where it painted each frame
//! (`SelectableText`) and keeps that registration while a cached view
//! replays it, a press finds its text by hitbox, a drag past a list's edge
//! scrolls it, and the selection paints as bands of whole rows.
//!
//! Unlike gpui-kit, a selection is held by place (a document, a segment, a
//! byte; `model.rs`), not by painted point: it outlives its text scrolling
//! out of a virtual list, ⌘A takes a whole document and a copy reaches
//! text never drawn. It stays in the document it was started in.

mod geometry;
mod layer;
mod model;
#[cfg(test)]
mod tests;
mod text;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gpui::{
    App, Bounds, Entity, FocusHandle, Global, Hitbox, ListState, Pixels, SharedString, Task,
    TextLayout,
};

pub use layer::SelectionLayer;
pub use model::{Joint, SegKey};
pub use text::SelectableText;

use model::{Model, Segment};

/// A body of text one selection can span: a thread, a review's lane, an
/// Inbox item. Built where it is drawn; cheap to clone.
#[derive(Clone)]
pub struct Document(Rc<DocumentInner>);

/// Redraws what shows a document.
type Redraw = Rc<dyn Fn(&mut App)>;

#[derive(Clone)]
struct DocumentInner {
    id: SharedString,
    selection: Entity<TextSelection>,
    lines: bool,
    list: Option<ListState>,
    focus: Option<FocusHandle>,
    on_change: Option<Redraw>,
}

impl Document {
    pub fn new(id: impl Into<SharedString>, selection: &Entity<TextSelection>) -> Self {
        Self(Rc::new(DocumentInner {
            id: id.into(),
            selection: selection.clone(),
            lines: false,
            list: None,
            focus: None,
            on_change: None,
        }))
    }

    /// Each block is one line (a diff's rows): a copy joins blocks with a
    /// newline, not a blank line.
    pub fn lines(mut self) -> Self {
        Rc::make_mut(&mut self.0).lines = true;
        self
    }

    /// The list it scrolls in: a drag past its edges selects to the edge
    /// and scrolls it.
    pub fn list(mut self, list: ListState) -> Self {
        Rc::make_mut(&mut self.0).list = Some(list);
        self
    }

    /// What a press on its text focuses, for the keys of its selection.
    pub fn focus(mut self, focus: FocusHandle) -> Self {
        Rc::make_mut(&mut self.0).focus = Some(focus);
        self
    }

    /// Redraws what shows it: the selection changed in it.
    pub fn on_change(mut self, redraw: impl Fn(&mut App) + 'static) -> Self {
        Rc::make_mut(&mut self.0).on_change = Some(Rc::new(redraw));
        self
    }

    pub fn id(&self) -> &SharedString {
        &self.0.id
    }

    /// The window's selection it joins.
    pub fn selection(&self) -> &Entity<TextSelection> {
        &self.0.selection
    }

    /// Deferred: the change may come from inside an update of whatever
    /// the redraw updates.
    fn changed(&self, cx: &mut App) {
        if let Some(redraw) = self.0.on_change.clone() {
            cx.defer(move |cx| redraw(cx));
        }
    }
}

/// A segment as it was painted.
struct Painted {
    key: SegKey,
    layout: TextLayout,
    bounds: Bounds<Pixels>,
    hitbox: Hitbox,
    /// The frame it was painted in.
    generation: u64,
    /// Lives while its element does, through frames a cached view replays.
    alive: Weak<()>,
}

/// What the window painted, swept after each frame (`layer::finish`).
#[derive(Default)]
struct Frame {
    painted: HashMap<(SharedString, SegKey), Painted>,
    docs: HashMap<SharedString, Document>,
    generation: u64,
    finishing: bool,
    /// The text moved under a drag (a scroll): its end is found again
    /// once the frame shows where.
    refresh: bool,
}

/// A drag held past a list's edge scrolls it on a timer.
struct AutoScroll {
    delta: Pixels,
    _task: Task<()>,
}

/// One window's text selection; `SelectionLayer` listens for it.
#[derive(Default)]
pub struct TextSelection {
    model: RefCell<Model>,
    frame: RefCell<Frame>,
    /// The selection's first line as last painted: a menu sits over it.
    first_line: Cell<Option<Bounds<Pixels>>>,
    /// Where presses keep the selection (its menu), as last painted.
    kept: Cell<Option<Bounds<Pixels>>>,
    /// The document whose empty space the press being dispatched hit.
    beside: RefCell<Option<SharedString>>,
    scroll: RefCell<Option<AutoScroll>>,
}

/// A press something else answers (a button, a header): it starts no
/// selection. Set while the press is dispatched, cleared before the next.
#[derive(Default)]
struct Claimed(bool);

impl Global for Claimed {}

/// The press being dispatched is this element's own: no text selection
/// starts from it. Buttons and anything that answers a press call it.
pub fn claim_press(cx: &mut App) {
    cx.set_global(Claimed(true));
}

impl TextSelection {
    /// The document the selection (or the press making it) is in.
    pub fn doc(&self) -> Option<SharedString> {
        self.model.borrow().doc().cloned()
    }

    pub fn has_selection(&self) -> bool {
        self.model.borrow().has_selection()
    }

    pub fn is_dragging_in(&self, doc: &str) -> bool {
        let model = self.model.borrow();
        model.is_dragging() && model.doc().is_some_and(|d| d == doc)
    }

    /// The selected text as shown; `fallback` gives the text of blocks its
    /// document has not drawn.
    pub fn selected_text(&self, fallback: &[(usize, String)]) -> Option<String> {
        self.model.borrow().selected_text(fallback)
    }

    /// What a finished selection inside the quotable text of one block of
    /// `doc` quotes, and its first line as painted, for a menu over it.
    pub fn quote(&self, doc: &str) -> Option<(String, Bounds<Pixels>)> {
        let model = self.model.borrow();
        if !model.released() || model.doc().is_none_or(|d| d != doc) {
            return None;
        }
        Some((model.quote()?, self.first_line.get()?))
    }

    /// A press inside `bounds` (the selection's menu, painted there) keeps
    /// the selection.
    pub fn keep_presses_within(&self, bounds: Bounds<Pixels>) {
        self.kept.set(Some(bounds));
    }

    /// A press on `doc`'s empty space (a margin, the gap between messages)
    /// that nothing else took: it starts at the nearest character. Called
    /// from the document's own mouse-down listener, before the layer's.
    pub fn press_beside(&self, doc: &str) {
        *self.beside.borrow_mut() = Some(SharedString::new(doc));
    }

    /// Drops the selection; `false` when there was none.
    pub fn clear(this: &Entity<Self>, cx: &mut App) -> bool {
        let (cleared, doc) = {
            let state = this.read(cx);
            state.first_line.set(None);
            state.kept.set(None);
            state.scroll.borrow_mut().take();
            let mut model = state.model.borrow_mut();
            (model.clear(), model.doc().cloned())
        };
        if cleared {
            Self::changed(this, doc.as_ref(), cx);
        }
        cleared
    }

    /// ⌘A: the whole of `doc`, which has been drawn.
    pub fn select_all(this: &Entity<Self>, doc: &str, cx: &mut App) {
        let Some(document) = this.read(cx).frame.borrow().docs.get(doc).cloned() else {
            log::debug!("select all in {doc}, never drawn");
            return;
        };
        let previous = {
            let state = this.read(cx);
            state.kept.set(None);
            let mut model = state.model.borrow_mut();
            let previous = model.doc().cloned();
            model.select_all(document.id(), document.0.lines);
            previous
        };
        if previous.as_ref().is_some_and(|p| p != document.id()) {
            Self::changed(this, previous.as_ref(), cx);
        }
        document.changed(cx);
    }

    /// The documents `gone` names changed: what was selected by block no
    /// longer names the same text.
    pub fn forget(this: &Entity<Self>, gone: impl Fn(&str) -> bool, cx: &mut App) {
        let (cleared, doc) = {
            let mut model = this.read(cx).model.borrow_mut();
            let doc = model.doc().cloned();
            (model.forget(gone), doc)
        };
        if cleared {
            this.read(cx).kept.set(None);
            Self::changed(this, doc.as_ref(), cx);
        }
    }

    /// The text moved out from under the selection's menu: it closes and
    /// the selection stays. `true` when it was open.
    pub fn dismiss(this: &Entity<Self>, cx: &mut App) -> bool {
        let (dismissed, doc) = {
            let state = this.read(cx);
            let mut model = state.model.borrow_mut();
            (model.dismiss(), model.doc().cloned())
        };
        if dismissed {
            this.read(cx).kept.set(None);
            Self::changed(this, doc.as_ref(), cx);
        }
        dismissed
    }

    /// Records segment `key` of `doc` as painted this frame.
    fn register(&self, doc: &Document, segment: Segment, painted: Painted) {
        let id = doc.id();
        self.model.borrow_mut().register(id, painted.key, segment);
        let mut frame = self.frame.borrow_mut();
        if !(frame.docs.get(id)).is_some_and(|known| Rc::ptr_eq(&known.0, &doc.0)) {
            frame.docs.insert(id.clone(), doc.clone());
        }
        let generation = frame.generation;
        frame.painted.insert(
            (id.clone(), painted.key),
            Painted {
                generation,
                ..painted
            },
        );
    }

    /// Redraws what shows `doc`.
    fn changed(this: &Entity<Self>, doc: Option<&SharedString>, cx: &mut App) {
        let document = doc.and_then(|doc| this.read(cx).frame.borrow().docs.get(doc).cloned());
        if let Some(document) = document {
            document.changed(cx);
        }
    }
}
