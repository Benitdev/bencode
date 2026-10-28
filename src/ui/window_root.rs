//! The window's root: the composer runner's layer, then the app's shell.
//! The runner redraws every frame of a turn; the app's regions are cached
//! (`ui/region.rs`), so those frames re-render only the shell. A region
//! redraws when the app notifies, when asked alone, or when a view or
//! followed model (`Regions::follow`) it draws notifies.

use gpui::{AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div};

use crate::app::BenCodeApp;
use crate::launch::LaunchData;
use crate::ui::composer::runner_view::RunnerLayer;

pub struct WindowRoot {
    app: Entity<BenCodeApp>,
    selection: Entity<bencode_ui::selection::TextSelection>,
    runner: Entity<RunnerLayer>,
    /// For the startup trace: the first render is marked.
    drawn: bool,
}

impl WindowRoot {
    pub fn new(window: &mut Window, launch: LaunchData, cx: &mut Context<Self>) -> Self {
        let app = cx.new(|cx| BenCodeApp::new(window, launch, cx));
        let weak = app.downgrade();
        let runner = cx.new(|_| RunnerLayer::new(weak));
        let selection = app.read(cx).text_selection.clone();
        Self {
            app,
            selection,
            runner,
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
        // The selection's layer goes first, so that its listeners run
        // after every element's; the runner layer reads the composer
        // geometry the app measured last frame before the app lays out
        // again.
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(bencode_ui::selection::SelectionLayer::new(&self.selection))
            .child(self.runner.clone())
            // Not cached: the app's own render is the shell (layout,
            // overlays, dialogs), and its parts are cached regions
            // (`ui/region.rs`). A cached app would re-render every region
            // whenever one of them redraws, being their ancestor.
            .child(self.app.clone())
    }
}
