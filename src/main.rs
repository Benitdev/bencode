mod app;
mod backlog;
mod db;
pub mod external_editor;
mod git;
mod github;
mod github_accounts;
mod harness;
mod keychain;
mod logging;
pub mod mcp;
mod monocode_import;
mod process_stats;
mod project_search;
mod pty_host;
mod rate_limits;
mod schedule;
mod settings;
mod skills;
mod storage;
mod terminal_process;
mod ui;
mod updater;
mod work_items;
mod workspace;

use gpui::{
    App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};

use monocode_import::ImportOutcome;

fn main() {
    // The terminal host and the dock's attach clients are this binary too.
    if let Some(code) = pty_host::run_from_args() {
        std::process::exit(code);
    }
    logging::init();

    let application = gpui_platform::application().with_assets(ui::icons::Assets);
    // The Dock icon with no window on screen (hidden or minimized): bring
    // BenCode's window back.
    application.on_reopen(|cx| {
        cx.activate(true);
        for window in cx.windows() {
            if let Err(err) = window.update(cx, |_, window, _| window.activate_window()) {
                log::warn!("could not show the window: {err:#}");
            }
        }
    });
    application
        .run(|cx: &mut App| {
            // Without Ely's assets and fonts nothing can be drawn.
            if let Err(err) = ely_gpui_component::init(cx) {
                log::error!("ely init failed: {err:#}");
                cx.quit();
                return;
            }
            app::commands::install(cx);
            // Before the database opens: the first launch on its own data
            // brings along what a MonoCode install held.
            let imported = monocode_import::import_once();
            let mut saved = settings::settings_dir()
                .map(|dir| settings::load_from(&dir))
                .unwrap_or_default();
            let mut import_error = None;
            match imported {
                ImportOutcome::Nothing => {}
                ImportOutcome::Imported(imported) => {
                    if monocode_import::merge_accounts(&mut saved.provider_accounts, imported.accounts)
                        && let Some(dir) = settings::settings_dir()
                        && let Err(err) = settings::save_to(&dir, &saved)
                    {
                        log::error!("saving the imported account names: {err:#}");
                    }
                }
                ImportOutcome::Failed(err) => import_error = Some(err),
            }
            let system_dark = app::is_dark_appearance(cx.window_appearance());
            let appearance = app::appearance_prefs(&saved);
            ui::theme::install(
                app::theme_mode(saved.theme, system_dark),
                &appearance.tint,
                appearance.user_accent(),
                cx,
            );
            ui::appearance::AppearanceTokens::set(appearance.tokens(), cx);

            let bounds = Bounds::centered(None, size(px(1200.0), px(780.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("BenCode".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(12.0), px(14.0))),
                }),
                window_min_size: Some(size(px(880.0), px(560.0))),
                // The title bar's drag regions move the window themselves
                // (`ui/window_drag.rs`), so dragging a tab no longer does.
                app_owns_titlebar_drag: true,
                ..Default::default()
            };

            let import_failed = import_error.is_some();
            let window = cx
                .open_window(options, |window, cx| {
                    cx.new(|cx| ui::window_root::WindowRoot::new(window, saved, import_failed, cx))
                })
                .expect("Failed to open BenCode window");
            // BenCode's state, running agents included, lives in its one
            // window, and macOS keeps the app running when it closes. So the
            // close button hides the app, as ⌘H does, and the Dock icon brings
            // it back; ⌘Q quits.
            let hide_on_close = window.update(cx, |_, window, cx| {
                window.on_window_should_close(cx, |_, cx| {
                    cx.hide();
                    false
                });
            });
            if let Err(err) = hide_on_close {
                log::error!("could not make the close button hide BenCode: {err:#}");
            }
            if let Some(err) = import_error {
                let detail = format!(
                    "{err}\n\nNothing will be saved this launch. BenCode will try the import again next time it starts."
                );
                let shown = window.update(cx, |_, window, cx| {
                    // The answer does not matter; the dialog only informs.
                    drop(window.prompt(
                        gpui::PromptLevel::Critical,
                        "BenCode could not import your MonoCode data",
                        Some(&detail),
                        &["OK"],
                        cx,
                    ));
                });
                if let Err(err) = shown {
                    log::error!("could not show the import failure: {err:#}");
                }
            }

            cx.activate(true);
        });
}
