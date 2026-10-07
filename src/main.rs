mod app;
mod db;
pub mod external_editor;
mod git;
mod github;
mod harness;
pub mod mcp;
mod schedule;
mod settings;
mod skills;
mod ui;
mod workspace;

use gpui::{
    App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};

fn main() {
    env_logger::init();

    gpui_platform::application()
        .with_assets(ui::icons::Assets)
        .run(|cx: &mut App| {
            // Without Ely's assets and fonts nothing can be drawn.
            if let Err(err) = ely_gpui_component::init(cx) {
                log::error!("ely init failed: {err:#}");
                cx.quit();
                return;
            }
            app::commands::install(cx);
            let saved = settings::settings_dir()
                .map(|dir| settings::load_from(&dir))
                .unwrap_or_default();
            let system_dark = app::is_dark_appearance(cx.window_appearance());
            ui::theme::install(app::theme_mode(saved.theme, system_dark), cx);

            let bounds = Bounds::centered(None, size(px(1200.0), px(780.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("BenCode".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.0), px(18.0))),
                }),
                window_min_size: Some(size(px(880.0), px(560.0))),
                // The title bar's drag regions move the window themselves
                // (`ui/window_drag.rs`), so dragging a tab no longer does.
                app_owns_titlebar_drag: true,
                ..Default::default()
            };

            cx.open_window(options, |window, cx| {
                cx.new(|cx| ui::window_root::WindowRoot::new(window, saved, cx))
            })
            .expect("Failed to open BenCode window");

            cx.activate(true);
        });
}
