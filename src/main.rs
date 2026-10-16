mod app;
mod backlog;
mod browser;
mod db;
pub mod external_editor;
mod git;
mod github;
mod github_accounts;
mod launch;
mod logging;
pub mod mcp;
mod monocode_import;
mod monos;
mod notifications;
mod process_stats;
mod project_search;
mod pty_host;
mod rate_limits;
mod remote;
mod schedule;
mod settings;
mod skills;
mod sounds;
mod startup_trace;
mod storage;
mod terminal_process;
mod ui;
mod updater;
mod work_items;
mod workspace;

use bencode_core::{harness, keychain};

use gpui::{
    App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};

fn main() {
    startup_trace::init();
    // The terminal host and the dock's attach clients are this binary too.
    if let Some(code) = pty_host::run_from_args() {
        std::process::exit(code);
    }
    // So is the MCP server that gives agents the in-app browser.
    if let Some(code) = browser::run_from_args() {
        std::process::exit(code);
    }
    // And the askpass OpenSSH runs while Settings connects a machine.
    if let Some(code) = remote::askpass::run_from_env() {
        std::process::exit(code);
    }
    logging::init();
    startup_trace::mark("main");
    // The data folder is read while GPUI sets the platform up.
    let launch = launch::start();

    let application = gpui_platform::application().with_assets(ui::icons::Assets);
    startup_trace::mark("application created");
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
            startup_trace::mark("application launched");
            // Without Ely's assets and fonts nothing can be drawn.
            if let Err(err) = ely_gpui_component::init(cx) {
                log::error!("ely init failed: {err:#}");
                cx.quit();
                return;
            }
            app::commands::install(cx);
            // Read on its own thread since `main` began; most launches find
            // it done.
            let mut launch = launch.wait();
            startup_trace::mark("data folder read");
            let saved = &launch.settings;
            let system_dark = app::is_dark_appearance(cx.window_appearance());
            let appearance = app::appearance_prefs(saved);
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

            let import_error = launch.import_error.take();
            let window = cx
                .open_window(options, |window, cx| {
                    startup_trace::mark("window opened");
                    cx.new(|cx| ui::window_root::WindowRoot::new(window, launch, cx))
                })
                .expect("Failed to open BenCode window");
            startup_trace::mark("open_window returned");
            // The menu bar after the first frame, or soon anyway if the
            // window does not draw (opened hidden).
            let menus_due = std::rc::Rc::new(std::cell::Cell::new(true));
            let install_menus = move |cx: &mut App| {
                if menus_due.replace(false) {
                    app::commands::install_menus(cx);
                    startup_trace::mark("menus installed");
                }
            };
            let after_frame = install_menus.clone();
            let waiting = window.update(cx, |_, window, _| {
                window.on_next_frame(move |_, cx| after_frame(cx));
            });
            if let Err(err) = waiting {
                log::error!("could not wait for the first frame: {err:#}");
            }
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                cx.update(install_menus);
            })
            .detach();
            // BenCode's state, running agents included, lives in its one
            // window, and macOS keeps the app running when it closes. So the
            // close button hides the app, as ⌘H does, and the Dock icon brings
            // it back; ⌘Q quits. With the floating chat open, only this
            // window goes: hiding the app would take the chat with it.
            let hide_on_close = window.update(cx, |_, window, cx| {
                window.on_window_should_close(cx, |window, cx| {
                    if cx.windows().len() > 1 {
                        ui::native_window::order_out(window);
                    } else {
                        cx.hide();
                    }
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
