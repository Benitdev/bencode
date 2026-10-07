//! The window's root: the app, cached, under the composer runner's layer.
//! The runner redraws every frame of a turn; caching keeps those frames
//! from re-rendering the whole app, which redraws only when it notifies
//! (or an entity it read does).

use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, StyleRefinement, Styled,
    Window, div,
};

use crate::app::BenCodeApp;
use crate::settings::AppSettings;
use crate::ui::composer::runner_view::RunnerLayer;

pub struct WindowRoot {
    app: Entity<BenCodeApp>,
    runner: Entity<RunnerLayer>,
}

impl WindowRoot {
    /// `import_failed`: the first-launch import did not finish (see
    /// `BenCodeApp::new`).
    pub fn new(
        window: &mut Window,
        saved: AppSettings,
        import_failed: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let app = cx.new(|cx| BenCodeApp::new(window, saved, import_failed, cx));
        let weak = app.downgrade();
        let runner = cx.new(|_| RunnerLayer::new(weak));
        Self { app, runner }
    }
}

impl Render for WindowRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
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
    }
}
