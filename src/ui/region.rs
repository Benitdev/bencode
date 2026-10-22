//! The window's parts as views of their own, so a change redraws only the
//! part it touched. GPUI re-renders a view that notified, every view above
//! it, and everything inside a cached view that re-renders; a cached view
//! whose ancestors re-render without it changing is reused as painted.
//! So the regions are flat siblings, each cached, under the app's own
//! render (the shell: layout, overlays and dialogs), which is not cached.
//!
//! A region draws through the app's render functions (`render_region`),
//! with the app's context: listeners and state are the app's as before.
//! Each chat pane of the active tab is a region of its own
//! (`RegionKind::Pane`), keyed by its thread; the splits between them are
//! the shell's.
//! Each region follows the app: a `cx.notify()` on the app redraws them
//! all, as it always did. A change that only one part shows asks for that
//! part alone with `redraw_regions`, without notifying the app.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, IntoElement, ParentElement, Render,
    StyleRefinement, Styled, Subscription, WeakEntity, Window, div,
};

use crate::app::BenCodeApp;

/// One part of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Region {
    Titlebar,
    Rail,
    CompactRail,
    Sidebar,
    /// Every chat pane (`RegionKind::Pane`): a group, never a view of its
    /// own. Redrawing it redraws each pane.
    Chat,
    /// The tabs beside the chat: files, reviews, commits, browser pages.
    FilePane,
    Terminal,
    Footer,
    /// Inbox, Notes, Automations or Settings.
    Surface,
    /// A Mono in the main area, its Overview beside it.
    Mono,
}

impl Region {
    /// Where the busy marks the app's clock steps are drawn
    /// (`ui/spinner.rs`): the rail's mascots, the sidebar's cards, the
    /// title bar's tabs, the transcript's fold line, a Mono's mascot.
    pub const BUSY_MARKS: [Region; 6] = [
        Region::Titlebar,
        Region::Rail,
        Region::CompactRail,
        Region::Sidebar,
        Region::Chat,
        Region::Mono,
    ];
}

/// What a region's view draws.
#[derive(Clone, Debug, PartialEq, Eq)]
enum RegionKind {
    Fixed(Region),
    /// One chat pane: its thread's transcript and composer.
    Pane(String),
}

/// A region's view: it renders its part of the app and redraws when the
/// app notifies or when asked alone.
pub struct RegionView {
    app: WeakEntity<BenCodeApp>,
    kind: RegionKind,
    _follow: Subscription,
    /// The models this region shows (`Regions::follow`).
    _models: Vec<Subscription>,
}

/// Subscribes a region's view to one model it follows.
type Follow = Rc<dyn Fn(&mut Context<RegionView>) -> Subscription>;

impl Render for RegionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Its marks are recorded again as it lays them out.
        crate::ui::marks::clear_region(cx.entity_id());
        let kind = self.kind.clone();
        let drawn = self.app.update(cx, |app, cx| match &kind {
            RegionKind::Fixed(region) => app.render_region(*region, window, cx),
            RegionKind::Pane(session_id) => app.render_pane_region(session_id, cx),
        });
        let content = match drawn {
            Ok(element) => element,
            Err(err) => {
                log::debug!("{kind:?} drawn after app drop: {err:#}");
                div().into_any_element()
            }
        };
        // A cached view's content is laid out as a root, which a size of
        // its own would shrink to its content: this box fills the region,
        // as the flex column the content stood in before did.
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(content)
            .into_any_element()
    }
}

/// The regions' views, made as the shell first draws each.
#[derive(Default)]
pub struct Regions {
    /// Models and the regions that show them.
    follows: Vec<(Vec<Region>, Follow)>,
    views: HashMap<Region, Entity<RegionView>>,
    /// The chat panes' views, by thread.
    panes: HashMap<String, Entity<RegionView>>,
    /// The panes the shell laid out this frame.
    panes_drawn: Vec<String>,
}

fn new_view(
    kind: RegionKind,
    follows: &[(Vec<Region>, Follow)],
    cx: &mut Context<BenCodeApp>,
) -> Entity<RegionView> {
    let app = cx.entity();
    let region = match &kind {
        RegionKind::Fixed(region) => *region,
        RegionKind::Pane(_) => Region::Chat,
    };
    let view = cx.new(|cx| RegionView {
        app: app.downgrade(),
        _follow: cx.observe(&app, |_, _, cx| cx.notify()),
        _models: follows
            .iter()
            .filter(|(regions, _)| regions.contains(&region))
            .map(|(_, follow)| follow(cx))
            .collect(),
        kind,
    });
    crate::ui::marks::register_region(view.entity_id());
    view
}

impl Regions {
    /// `regions` redraw whenever `model` notifies. Set before the regions
    /// are first drawn (`BenCodeApp::new`).
    pub fn follow<T: 'static>(&mut self, model: &Entity<T>, regions: &[Region]) {
        let model = model.clone();
        let follow: Follow = Rc::new(move |cx| cx.observe(&model, |_, _, cx| cx.notify()));
        self.follows.push((regions.to_vec(), follow));
    }

    /// `region`'s view, laid out at `style`: a cached view is laid out from
    /// its style, never measured from its content, so the style gives it a
    /// definite size.
    pub fn element(
        &mut self,
        region: Region,
        style: StyleRefinement,
        cx: &mut Context<BenCodeApp>,
    ) -> AnyElement {
        let view = self
            .views
            .entry(region)
            .or_insert_with(|| new_view(RegionKind::Fixed(region), &self.follows, cx));
        view.clone().cached(style).into_any_element()
    }

    /// The chat pane of `session_id`, filling the box the shell's splits
    /// give it.
    pub fn pane_element(&mut self, session_id: &str, cx: &mut Context<BenCodeApp>) -> AnyElement {
        self.panes_drawn.push(session_id.to_string());
        let view = self.panes.entry(session_id.to_string()).or_insert_with(|| {
            new_view(RegionKind::Pane(session_id.to_string()), &self.follows, cx)
        });
        view.clone()
            .cached(StyleRefinement::default().size_full())
            .into_any_element()
    }

    /// The shell starts laying the window out: no pane drawn yet.
    pub fn begin_frame(&mut self) {
        self.panes_drawn.clear();
    }

    /// Forgets the marks of the regions not drawn this frame, and the
    /// panes no longer shown.
    fn settle_undrawn(&mut self, drawn: &[Region]) {
        for (region, view) in &self.views {
            if !drawn.contains(region) {
                crate::ui::marks::clear_region(view.entity_id());
            }
        }
        let shown = &self.panes_drawn;
        self.panes.retain(|session_id, view| {
            let keep = shown.contains(session_id);
            if !keep {
                crate::ui::marks::forget_region(view.entity_id());
            }
            keep
        });
    }

    /// The views of `regions` that exist (`Chat` is every pane).
    pub fn views(&self, regions: &[Region]) -> Vec<Entity<RegionView>> {
        let mut views = Vec::new();
        for region in regions {
            if *region == Region::Chat {
                views.extend(self.panes.values().cloned());
            } else if let Some(view) = self.views.get(region) {
                views.push(view.clone());
            }
        }
        views
    }

    /// Redraws `regions` alone; the others are reused as painted.
    pub fn redraw(&self, regions: &[Region], cx: &mut App) {
        for view in self.views(regions) {
            view.update(cx, |_, cx| cx.notify());
        }
    }

    /// Redraws the chat pane of `session_id`, when one is shown.
    pub fn redraw_pane(&self, session_id: &str, cx: &mut App) {
        if let Some(view) = self.panes.get(session_id) {
            view.update(cx, |_, cx| cx.notify());
        }
    }
}

impl BenCodeApp {
    /// Redraws `regions` without notifying the app: for a change that only
    /// they show. Anything that may show elsewhere notifies the app.
    pub fn redraw_regions(&self, regions: &[Region], cx: &mut App) {
        self.regions.redraw(regions, cx);
    }

    /// Steps the busy marks (`ui/marks.rs`): the layer redraws, the
    /// regions are reused as painted.
    pub fn step_marks(&self, cx: &mut App) {
        if let Some(layer) = &self.mark_layer {
            layer.update(cx, |_, cx| cx.notify());
        }
    }

    /// Redraws what draws thread `session_id`'s transcript now: its pane,
    /// or the Mono in view. Its card, tab and mascot follow on the clock.
    /// What shows a run's time ("Working for 12s") once a second: the
    /// regions outside the chat, and the panes of the threads that run.
    /// Every chat pane would re-render its rows for nothing.
    pub(crate) fn redraw_run_clocks(&self, cx: &mut App) {
        let outside: Vec<Region> = (Region::BUSY_MARKS.into_iter())
            .filter(|region| *region != Region::Chat)
            .collect();
        self.redraw_regions(&outside, cx);
        for id in self.runs.keys() {
            self.redraw_thread(id, cx);
        }
    }

    pub(crate) fn redraw_thread(&self, session_id: &str, cx: &mut App) {
        // The session a Mono started is drawn inside the Mono's region
        // while its Overview shows it.
        let in_mono = self.mono_view_session() == Some(session_id)
            || self.mono_shown_session() == Some(session_id);
        if self.mono_covers() && in_mono {
            self.redraw_regions(&[Region::Mono], cx);
        } else {
            self.regions.redraw_pane(session_id, cx);
        }
    }

    /// Redraws the focused composer (a keystroke, its pickers): the
    /// selected thread's pane, or the Mono in view.
    pub(crate) fn redraw_composer(&self, cx: &mut App) {
        match self.selected_session_id.as_deref() {
            // Over a Mono the prompt is its chat's or the shown session's,
            // both in its region.
            Some(id) if !self.is_mono_session(id) && !self.mono_covers() => {
                self.regions.redraw_pane(id, cx)
            }
            _ => self.redraw_regions(&[Region::Mono], cx),
        }
    }

    /// Draws the chat pane of `session_id` as its region.
    fn render_pane_region(&mut self, session_id: &str, cx: &mut Context<Self>) -> AnyElement {
        // The runner runs on the focused composer and its pane's chevron.
        if self.selected_session_id.as_deref() == Some(session_id) {
            self.runner_geometry.clear();
        }
        let in_split = self
            .active_layout()
            .is_some_and(|layout| crate::ui::layout::leaf_count(layout) > 1);
        self.render_session_pane(session_id, in_split, cx)
    }

    /// Draws one region. What a region measures or places as it is drawn
    /// (the runner's geometry, the footer's slot, a page's box) is reset
    /// here, by the region that sets it, so a region reused as painted
    /// keeps what it set; the shell resets it while the region is not
    /// drawn at all (`settle_undrawn_regions`).
    pub(crate) fn render_region(
        &mut self,
        region: Region,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match region {
            Region::Titlebar => self.render_titlebar(window, cx).into_any_element(),
            Region::Rail => self.render_project_rail(cx).into_any_element(),
            Region::CompactRail => self.render_compact_rail(cx).into_any_element(),
            Region::Sidebar => self.render_sidebar(cx).into_any_element(),
            // A group: each pane is a region of its own.
            Region::Chat => div().into_any_element(),
            Region::FilePane => {
                self.browser.drawn.set(None);
                let pane = self.render_file_pane(cx).into_any_element();
                self.browser.hide_undrawn();
                pane
            }
            Region::Terminal => self.render_terminal_region(cx),
            Region::Footer => {
                self.process_slot.set(None);
                self.render_usage_footer(cx).into_any_element()
            }
            Region::Surface => self
                .render_surface(cx)
                .unwrap_or_else(|| div().into_any_element()),
            Region::Mono => {
                // Its chat, or the session it shows, may be the focused
                // composer the runner runs on.
                if self.mono_holds_selected() {
                    self.runner_geometry.clear();
                }
                self.artifacts.page_drawn.set(false);
                let mono = self
                    .render_mono_view(cx)
                    .unwrap_or_else(|| div().into_any_element());
                self.artifacts.hide_undrawn();
                mono
            }
        }
    }

    /// Whether the selected thread is drawn in the Mono's region: its
    /// chat, or the session it shows over the Overview.
    fn mono_holds_selected(&self) -> bool {
        let selected = self.selected_session_id.as_deref();
        selected.is_some()
            && (selected == self.mono_view_session() || selected == self.mono_shown_session())
    }

    /// Resets what the regions not drawn this frame measured or placed:
    /// the shell calls this with the regions it laid out.
    pub(crate) fn settle_undrawn_regions(&mut self, drawn: &[Region]) {
        self.regions.settle_undrawn(drawn);
        let focused_drawn = self
            .selected_session_id
            .as_ref()
            .is_some_and(|id| self.regions.panes_drawn.contains(id))
            || (drawn.contains(&Region::Mono) && self.mono_holds_selected());
        if !focused_drawn {
            self.runner_geometry.clear();
        }
        if !drawn.contains(&Region::FilePane) {
            self.browser.drawn.set(None);
            self.browser.hide_undrawn();
        }
        if !drawn.contains(&Region::Footer) {
            self.process_slot.set(None);
        }
        if !drawn.contains(&Region::Mono) {
            self.artifacts.page_drawn.set(false);
            self.artifacts.hide_undrawn();
        }
    }
}
