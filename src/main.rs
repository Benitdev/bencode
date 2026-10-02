mod app;
mod db;
pub mod external_editor;
mod git;
mod harness;
pub mod mcp;
mod schedule;
mod settings;
mod skills;
mod ui;
mod workspace;

use app::BenCodeApp;
use ely_gpui_component::Assets;
use gpui::{
    App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};

fn main() {
    env_logger::init();

    gpui_platform::application()
        .with_assets(Assets)
        .run(|cx: &mut App| {
            ely_gpui_component::init(cx);
            app::commands::install(cx);
            let saved = settings::settings_dir()
                .map(|dir| settings::load_from(&dir))
                .unwrap_or_default();
            ui::theme::install(app::theme_mode(saved.theme), cx);

            let bounds = Bounds::centered(None, size(px(1200.0), px(780.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("BenCode".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.0), px(18.0))),
                }),
                window_min_size: Some(size(px(880.0), px(560.0))),
                ..Default::default()
            };

            cx.open_window(options, |window, cx| {
                cx.new(|cx| BenCodeApp::new(window, saved, cx))
            })
            .expect("Failed to open BenCode window");

            cx.activate(true);
        });
}
