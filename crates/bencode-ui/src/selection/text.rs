//! A run of text that joins its document's selection (gpui-kit's
//! `SelectableText`): it registers where it painted, and paints the part
//! selected as bands under its glyphs.

use std::rc::Rc;

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId,
    IntoElement, LayoutId, Pixels, SharedString, StyledText, TextLayout, TextRun, Window, fill,
    point, px, size,
};

use super::geometry::bands;
use super::model::{Joint, SegKey, Segment};
use super::{Document, Painted, TextSelection};
use crate::theme::ActiveTheme;

/// `text` in `runs`, segment `key` of `doc`.
pub struct SelectableText {
    id: ElementId,
    doc: Document,
    key: SegKey,
    joint: Joint,
    quotable: bool,
    text: SharedString,
    styled: StyledText,
}

impl SelectableText {
    pub fn new(
        id: impl Into<ElementId>,
        doc: &Document,
        key: SegKey,
        text: impl Into<SharedString>,
        runs: Vec<TextRun>,
    ) -> Self {
        let text: SharedString = text.into();
        Self {
            id: id.into(),
            doc: doc.clone(),
            key,
            joint: Joint::Block,
            quotable: false,
            styled: StyledText::new(text.clone()).with_runs(runs),
            text,
        }
    }

    /// What a copy puts between this segment and the one before it in its
    /// block.
    pub fn joint(mut self, joint: Joint) -> Self {
        self.joint = joint;
        self
    }

    /// A selection inside this block's quotable text can be quoted
    /// (`TextSelection::quote`).
    pub fn quotable(mut self, quotable: bool) -> Self {
        self.quotable = quotable;
        self
    }

    /// Where its characters are once laid out, for the caller's own clicks
    /// (links).
    pub fn layout(&self) -> TextLayout {
        self.styled.layout().clone()
    }
}

impl IntoElement for SelectableText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.styled.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.styled
            .prepaint(id, inspector_id, bounds, &mut (), window, cx);
        let hitbox: Hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        // The element's own state: replayed with a cached view, dropped
        // with the element.
        let alive = window.with_element_state(
            id.expect("a selectable text has an id"),
            |state: Option<Rc<()>>, _| {
                let state = state.unwrap_or_default();
                (Rc::downgrade(&state), state)
            },
        );
        let segment = Segment {
            text: self.text.clone(),
            joint: self.joint,
            quotable: self.quotable,
        };
        let painted = Painted {
            key: self.key,
            layout: self.styled.layout().clone(),
            bounds,
            hitbox,
            generation: 0,
            alive,
        };
        (self.doc.0.selection.read(cx)).register(&self.doc, segment, painted);
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled.layout().clone();
        let color = cx.theme().colors.selection;
        let selection = self.doc.0.selection.read(cx);
        selection.paint_selected(self.doc.id(), self.key, &layout, bounds, window, color);
        self.styled
            .paint(id, inspector_id, bounds, &mut (), &mut (), window, cx);
    }
}

impl TextSelection {
    /// Paints what is selected of segment `key` of `doc`, laid out as
    /// `layout` in `bounds`, and notes the selection's first line when it
    /// starts here.
    fn paint_selected(
        &self,
        doc: &str,
        key: SegKey,
        layout: &TextLayout,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        color: gpui::Hsla,
    ) {
        let model = self.model.borrow();
        let len = layout.len();
        let Some(range) = model.range_in(doc, key, len) else {
            return;
        };
        let (Some(start), Some(end)) = (
            layout.position_for_index(range.start),
            layout.position_for_index(range.end),
        ) else {
            return;
        };
        let line = layout.line_height();
        let continues = range.end == len && model.continues_past(doc, key);
        for band in bands(start, end, bounds, line, continues) {
            window.paint_quad(fill(band, color));
        }
        if model.span().is_some_and(|(first, _)| first.key == key) {
            let to = if end.y == start.y {
                end.x
            } else {
                bounds.right()
            };
            let width = (to - start.x).max(px(1.0));
            (self.first_line).set(Some(Bounds::new(
                point(start.x, start.y),
                size(width, line),
            )));
        }
    }
}
