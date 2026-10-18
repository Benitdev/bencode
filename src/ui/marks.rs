//! Busy marks drawn over the regions, so the clock that steps them redraws
//! a mark, not the region it sits in (`docs/regions.md`, stage 3).
//!
//! A mark is built from a function of the clock (`mark(build, cx)`). Laid
//! out where it stands, it records its box, clip and text style instead of
//! painting when it is drawn inside a region; `MarkLayer`, drawn in the
//! shell between the regions and the menus and dialogs, builds it again
//! there on every frame. The clock notifies only the layer, so the regions
//! are reused as painted. Drawn anywhere else (the shell's dialogs, the
//! floating chat), a mark paints itself as any element does.
//!
//! The layer knows a mark's box and clip, not its ancestors' opacity or
//! visibility (GPUI keeps those to itself): a mark under a parent that is
//! faded or `invisible()` is drawn at full strength. Leave a mark out of a
//! parent that hides itself (the dragged title tab shows its harness
//! instead); a fade of a few hundred ms (a card sliding in) is accepted.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, Context, Element, ElementId, EntityId,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels, Render, Styled, TextStyle,
    TextStyleRefinement, Window, canvas,
};

/// What draws a mark at the clock's current step.
type Build = Rc<dyn Fn(&App) -> AnyElement>;

/// A mark a region laid out and left for the layer.
struct Slot {
    bounds: Bounds<Pixels>,
    mask: ContentMask<Pixels>,
    text: TextStyle,
    build: Build,
}

#[derive(Default)]
struct Registry {
    /// The regions' views: a mark drawn inside one is left to the layer.
    regions: HashSet<EntityId>,
    /// Each region's marks as it last drew them.
    slots: HashMap<EntityId, Vec<Slot>>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::default();
}

/// A region's view exists: marks drawn in it go to the layer.
pub fn register_region(view: EntityId) {
    REGISTRY.with(|registry| registry.borrow_mut().regions.insert(view));
}

/// A region's view is gone: nothing of it is kept.
pub fn forget_region(view: EntityId) {
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.regions.remove(&view);
        registry.slots.remove(&view);
    });
}

/// A region is drawn again (or not at all): its marks are recorded anew.
pub fn clear_region(view: EntityId) {
    REGISTRY.with(|registry| registry.borrow_mut().slots.remove(&view));
}

/// A busy mark: `build` draws it at the clock's current step.
pub fn mark(build: impl Fn(&App) -> AnyElement + 'static, cx: &App) -> AnyElement {
    let build: Build = Rc::new(build);
    BusyMark {
        inner: build(cx),
        build,
    }
    .into_any_element()
}

struct BusyMark {
    inner: AnyElement,
    build: Build,
}

impl IntoElement for BusyMark {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for BusyMark {
    type RequestLayoutState = ();
    /// Whether the mark paints itself (drawn outside the regions).
    type PrepaintState = bool;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.inner.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let owner = window.current_view();
        let left = REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            if !registry.regions.contains(&owner) {
                return false;
            }
            registry.slots.entry(owner).or_default().push(Slot {
                bounds,
                mask: window.content_mask(),
                text: window.text_style(),
                build: self.build.clone(),
            });
            true
        });
        if !left {
            self.inner.prepaint(window, cx);
        }
        !left
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        paints: &mut bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if *paints {
            self.inner.paint(window, cx);
        }
    }
}

/// The marks the regions left, drawn at the clock's step. Not cached: it
/// draws whenever the shell does, so a region that moved its marks has
/// them drawn where they now are in the same frame.
pub struct MarkLayer;

impl Render for MarkLayer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        canvas(
            |_, window, cx| {
                let slots: Vec<(
                    Bounds<Pixels>,
                    ContentMask<Pixels>,
                    TextStyleRefinement,
                    Build,
                )> = REGISTRY.with(|registry| {
                    registry
                        .borrow()
                        .slots
                        .values()
                        .flatten()
                        .map(|slot| {
                            (
                                slot.bounds,
                                slot.mask,
                                refinement(&slot.text),
                                slot.build.clone(),
                            )
                        })
                        .collect()
                });
                slots
                    .into_iter()
                    .map(|(bounds, mask, text, build)| {
                        let mut element = build(cx);
                        window.with_text_style(Some(text.clone()), |window| {
                            window.with_content_mask(Some(mask), |window| {
                                element.prepaint_as_root(
                                    bounds.origin,
                                    bounds.size.map(AvailableSpace::Definite),
                                    window,
                                    cx,
                                );
                            })
                        });
                        (mask, text, element)
                    })
                    .collect::<Vec<_>>()
            },
            |_, marks, window, cx| {
                for (mask, text, mut element) in marks {
                    window.with_text_style(Some(text), |window| {
                        window.with_content_mask(Some(mask), |window| element.paint(window, cx))
                    });
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

/// The text style a mark inherited where it stands, to lay it out the same
/// over the layer.
fn refinement(style: &TextStyle) -> TextStyleRefinement {
    TextStyleRefinement {
        color: Some(style.color),
        font_family: Some(style.font_family.clone()),
        font_features: Some(style.font_features.clone()),
        font_fallbacks: style.font_fallbacks.clone(),
        font_size: Some(style.font_size),
        line_height: Some(style.line_height),
        font_weight: Some(style.font_weight),
        font_style: Some(style.font_style),
        background_color: style.background_color,
        underline: style.underline,
        strikethrough: style.strikethrough,
        white_space: Some(style.white_space),
        text_overflow: style.text_overflow.clone(),
        text_align: Some(style.text_align),
        line_clamp: style.line_clamp,
    }
}
