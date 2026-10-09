//! Frame timings for perf work (BenCode's own). `BENCODE_FRAME_BENCH=1`
//! opens the longest thread, draws the window by hand a fixed number of
//! times per case and prints how long a draw took: with nothing changed,
//! with the whole app notified, and with parts of the window taken away to
//! see what each costs.
//!
//! Run it on a copy of the data folder (a scratch `HOME`, see AGENTS.md):
//! it changes the layout as it goes and saves it. The launch jobs that
//! reach the network, the Keychain or an agent stay off while it runs
//! (`BenCodeApp::new`); the terminal case starts a shell, which ends with
//! the app.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::Result;
use gpui::{AsyncWindowContext, Context, WeakEntity, Window, px, size};

use crate::app::BenCodeApp;
use crate::app::file_pane::PaneTab;

/// Draws per case, unless `BENCODE_FRAME_BENCH` gives a number (a long run
/// leaves time to sample the process).
const FRAMES: usize = 120;
const ENV: &str = "BENCODE_FRAME_BENCH";
/// The launch's loads (sessions, the git snapshot) land in this time.
const SETTLE: Duration = Duration::from_secs(3);
/// Between the draws that let a changed layout settle.
const SETTLE_STEP: Duration = Duration::from_millis(40);
const SETTLE_DRAWS: usize = 25;
/// A window of the size the app is used at, not the one it opens with.
const WINDOW: (f32, f32) = (1600.0, 1000.0);

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os(ENV).is_some())
}

fn frames() -> usize {
    std::env::var(ENV)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|frames| *frames > 1)
        .unwrap_or(FRAMES)
}

type Step = Box<dyn Fn(&mut BenCodeApp, &mut Context<BenCodeApp>)>;

struct Case {
    name: &'static str,
    /// Changes the window before the case is measured.
    set_up: Step,
    /// Marks the window dirty before each draw.
    dirty: fn(&mut BenCodeApp, &mut Context<BenCodeApp>),
}

fn nothing(_: &mut BenCodeApp, _: &mut Context<BenCodeApp>) {}

fn notify_app(_: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    cx.notify();
}

/// A file every checkout has, and a long one.
const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/app.rs");

/// The cases in the order they run; each starts from the window the one
/// before it left. `thread` is the long thread the bench opened.
fn cases(thread: Option<String>) -> Vec<Case> {
    let case = |name, set_up: Step, dirty| Case {
        name,
        set_up,
        dirty,
    };
    vec![
        case("nothing notified", Box::new(nothing), nothing),
        case("app notified", Box::new(nothing), notify_app),
        case(
            "every row selected, selection notified",
            Box::new(nothing),
            |app, cx| app.select_all_transcript(cx),
        ),
        case(
            "app notified, sidebar closed",
            Box::new(|app, cx| {
                app.clear_transcript_selection(cx);
                app.is_sidebar_open = false;
                app.sidebar_drawer_open = false;
            }),
            notify_app,
        ),
        case(
            "app notified, sidebar and rail closed",
            Box::new(|app, _| app.is_rail_open = false),
            notify_app,
        ),
        case(
            "app notified, empty thread (sidebar, rail back)",
            Box::new(|app, cx| {
                app.is_sidebar_open = true;
                app.is_rail_open = true;
                app.create_new_session(cx);
            }),
            notify_app,
        ),
        case(
            "app notified, long thread, a file beside it",
            Box::new(move |app, cx| {
                if let Some(id) = &thread {
                    app.open_session(id.clone(), cx);
                }
                // The next render opens it: that needs the window.
                app.file_tree.pending_open = Some(FILE.to_string());
            }),
            notify_app,
        ),
        case(
            "app notified, long thread, Changes beside it",
            Box::new(|app, cx| {
                let tab = PaneTab::Changes {
                    cwd: app.current_cwd.clone(),
                    side: None,
                    focus: None,
                };
                app.open_pane_tab(tab, true, cx);
            }),
            notify_app,
        ),
        case(
            "app notified, long thread, terminal under it",
            Box::new(|app, cx| {
                while app.close_active_pane_tab(cx) {}
                app.set_terminal_open(true, cx);
            }),
            notify_app,
        ),
    ]
}

/// One draw of the window, timed: layout, prepaint and paint of whatever is
/// dirty. The scene is not presented, so the GPU's share is left out.
fn draw(cx: &mut AsyncWindowContext) -> Result<Duration> {
    cx.update(|window, cx| {
        let started = Instant::now();
        window.draw(cx).clear(cx);
        started.elapsed()
    })
}

async fn settle(cx: &mut AsyncWindowContext) -> Result<()> {
    for _ in 0..SETTLE_DRAWS {
        cx.update(|window, _| window.refresh())?;
        draw(cx)?;
        cx.background_executor().timer(SETTLE_STEP).await;
    }
    Ok(())
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

async fn run(this: WeakEntity<BenCodeApp>, cx: &mut AsyncWindowContext) -> Result<()> {
    cx.update(|window, _| window.resize(size(px(WINDOW.0), px(WINDOW.1))))?;
    cx.background_executor().timer(SETTLE).await;
    let thread = this.update(cx, |app, cx| {
        let longest = app
            .sessions
            .iter()
            .max_by_key(|s| s.blocks.len())
            .map(|s| (s.id.clone(), s.blocks.len()));
        if let Some((id, _)) = &longest {
            app.open_session(id.clone(), cx);
        }
        // The saved layout may have the dock open; its case opens it.
        app.set_terminal_open(false, cx);
        longest
    })?;
    settle(cx).await?;
    let viewport = cx.update(|window, _| window.viewport_size())?;
    let frames = frames();
    println!(
        "frame bench: {}x{} window, {frames} draws per case",
        f32::from(viewport.width),
        f32::from(viewport.height)
    );
    match &thread {
        Some((id, blocks)) => println!("thread {id}: {blocks} blocks"),
        None => println!("no thread to open"),
    }
    println!("{:<52} {:>8} {:>8} {:>8}", "case", "median", "p95", "max");
    // `BENCODE_FRAME_BENCH_CASE`: only the cases whose name holds it are
    // measured (the others still set the window up for the next).
    let only = std::env::var("BENCODE_FRAME_BENCH_CASE").ok();
    for case in cases(thread.map(|(id, _)| id)) {
        this.update(cx, |app, cx| {
            (case.set_up)(app, cx);
            cx.notify();
        })?;
        settle(cx).await?;
        if only.as_ref().is_some_and(|only| !case.name.contains(only.as_str())) {
            continue;
        }
        let mut took = Vec::with_capacity(frames);
        for _ in 0..frames {
            // Its own update: the observers a notify reaches run as it ends.
            this.update(cx, |app, cx| (case.dirty)(app, cx))?;
            took.push(draw(cx)?);
        }
        took.sort();
        println!(
            "{:<52} {:>6.2}ms {:>6.2}ms {:>6.2}ms",
            case.name,
            millis(took[frames / 2]),
            millis(took[frames * 95 / 100]),
            millis(took[frames - 1]),
        );
    }
    Ok(())
}

impl BenCodeApp {
    /// Runs the bench once the window is up, then quits.
    pub(crate) fn start_frame_bench(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            if let Err(err) = run(this, cx).await {
                log::error!("frame bench stopped: {err:#}");
            }
            if let Err(err) = cx.update(|_, cx| cx.quit()) {
                log::error!("frame bench could not quit: {err:#}");
            }
        })
        .detach();
    }
}
