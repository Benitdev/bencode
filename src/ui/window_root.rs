//! The window's root: the app, cached, between the composer runner's layer
//! and the process readout's.
//! The runner redraws every frame of a turn; caching keeps those frames
//! from re-rendering the whole app, which redraws only when it notifies
//! (or a view it draws does). An entity it only reads is not enough: the
//! app observes it and notifies (`transcript_selection`).

use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, StyleRefinement, Styled,
    Window, div,
};

use crate::app::BenCodeApp;
use crate::launch::LaunchData;
use crate::ui::composer::runner_view::RunnerLayer;
use crate::ui::footer::ProcessLayer;

pub struct WindowRoot {
    app: Entity<BenCodeApp>,
    runner: Entity<RunnerLayer>,
    process: Entity<ProcessLayer>,
    /// For the startup trace: the first render is marked.
    drawn: bool,
}

impl WindowRoot {
    pub fn new(window: &mut Window, launch: LaunchData, cx: &mut Context<Self>) -> Self {
        let app = cx.new(|cx| BenCodeApp::new(window, launch, cx));
        let weak = app.downgrade();
        let runner = cx.new(|_| RunnerLayer::new(weak));
        let slot = app.read(cx).process_slot.clone();
        let process = cx.new(|cx| ProcessLayer::new(slot, window, cx));
        Self {
            app,
            runner,
            process,
            drawn: false,
        }
    }
}

impl Render for WindowRoot {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if crate::startup_trace::enabled() && !self.drawn {
            self.drawn = true;
            crate::startup_trace::mark("first render");
            window.on_next_frame(|_, cx| {
                crate::startup_trace::landed(
                    crate::startup_trace::FIRST_FRAME,
                    "first frame presented",
                    cx,
                );
            });
        }
        // The runner layer goes first: it reads the composer geometry the
        // app measured last frame before the app lays out again.
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(self.runner.clone())
            .child(
                self.app
                    .clone()
                    .cached(StyleRefinement::default().size_full()),
            )
            // After the app: it lays its readout out over the footer slot
            // the app has just measured, and paints above it.
            .child(self.process.clone())
    }
}
