//! Frame timings for perf work (BenCode's own). `BENCODE_FRAME_BENCH=1`
//! opens the longest thread, draws the window by hand a fixed number of
//! times per case and prints how long a draw took: with nothing changed,
//! with the whole app notified, and with parts of the window taken away to
//! see what each costs. Then what opening a view costs (the change and its
//! first draw), and every frame of a fling from the end of the longest
//! thread to its start and back.
//!
//! Run it on a copy of the data folder (a scratch `HOME`, see AGENTS.md):
//! it changes the layout as it goes and saves it. The launch jobs that
//! reach the network, the Keychain or an agent stay off while it runs
//! (`BenCodeApp::new`); the terminal case starts a shell, which ends with
//! the app.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::Result;
use gpui::{AsyncWindowContext, Context, ListOffset, WeakEntity, Window, px, size};

use crate::app::file_pane::PaneTab;
use crate::app::surfaces::Surface;
use crate::app::{BenCodeApp, subagents};
use crate::ui::region::Region;
use crate::ui::transcript::turns::{self, Item, Row};

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

/// A commit this repository's history has: one Rust file, a screen of
/// changed lines.
const COMMIT: &str = "4ddb50b09cbcf9f872d26cc07f4a9ade047ed27c";

/// The subagent run with the longest trail: its thread and block.
fn longest_run(app: &BenCodeApp) -> Option<(String, usize)> {
    app.sessions
        .iter()
        .flat_map(|s| s.blocks.iter().enumerate().map(move |(ix, b)| (s, ix, b)))
        .map(|(s, ix, b)| (subagents::steps(b).len(), s.id.clone(), ix))
        .max_by_key(|(steps, ..)| *steps)
        .filter(|(steps, ..)| *steps > 0)
        .map(|(_, id, ix)| (id, ix))
}

/// The thread of the longest run, every turn's fold open.
fn open_longest_run_thread(app: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    app.set_terminal_open(false, cx);
    let Some((id, _)) = longest_run(app) else {
        return;
    };
    app.open_session(id.clone(), cx);
    if let Some(session) = app.sessions.iter().find(|s| s.id == id) {
        let users = session.blocks.iter().filter(|b| b.role == "user");
        app.transcript_ui
            .open_folds
            .extend(users.map(|b| b.id.clone()));
    }
}

/// The longest run opened onto its trail and scrolled to.
fn open_longest_run(app: &mut BenCodeApp, _: &mut Context<BenCodeApp>) {
    let Some((id, ix)) = longest_run(app) else {
        return;
    };
    let Some(session) = app.sessions.iter().find(|s| s.id == id) else {
        return;
    };
    let Some(view) = app.transcripts.get(&id) else {
        return;
    };
    let group = |row: &Row| match *row {
        Row::Item { turn, item } | Row::FoldItem { turn, item, .. } => {
            match &view.turns[turn].items[item] {
                Item::Subagents(group) | Item::Activity(group) => {
                    Some(group).filter(|group| group.contains(&ix))
                }
                Item::Block(_) => None,
            }
        }
        _ => None,
    };
    let Some((row, group)) = view
        .rows
        .iter()
        .enumerate()
        .find_map(|(row, item)| Some((row, group(item)?)))
    else {
        return;
    };
    // A run inside a phase shows once the phase is open; its own trail's
    // phases open too, as the live one is while the run works.
    let mut phases = turns::build_phases(&session.blocks, group);
    let steps: Vec<_> = subagents::steps(&session.blocks[ix])
        .iter()
        .map(subagents::step_block)
        .collect();
    let all: Vec<usize> = (0..steps.len()).collect();
    phases.extend(turns::build_phases(&steps, &all));
    let ui = &mut app.transcript_ui;
    ui.phase_open
        .extend(phases.into_iter().map(|phase| (phase.id, true)));
    ui.subagent_open.insert(session.blocks[ix].id.clone(), true);
    ui.reflow += 1;
    BENCH_RUN.with(|cell| cell.set(Some(row)));
    {
        view.list.scroll_to(ListOffset {
            item_ix: row,
            offset_in_item: px(0.0),
        });
    }
}

thread_local! {
    /// The list row of the open run, for the case that streams into it.
    static BENCH_RUN: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// A step lands on the open run, as one does while it works: its row is
/// measured again, as the live tail's rows are.
fn stream_into_run(app: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let Some((id, ix)) = longest_run(app) else {
        return;
    };
    let Some(session) = app.sessions.iter_mut().find(|s| s.id == id) else {
        return;
    };
    let call = session.blocks[ix]
        .tool
        .as_ref()
        .and_then(|t| t.get("callId"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let step = crate::harness::AgentStep {
        id: format!("bench-{n}"),
        kind: crate::harness::StepKind::Tool,
        text: format!("grep -rn \"bench {n}\" src | head"),
        tool_name: Some("Bash".into()),
        status: Some("completed".into()),
        detail: None,
    };
    subagents::record_step(session, &call, &step);
    if let (Some(view), Some(row)) = (app.transcripts.get(&id), BENCH_RUN.with(|c| c.get())) {
        view.list.remeasure_items(row..row + 1);
    }
    cx.notify();
}

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
        // What the regions save (`ui/region.rs`): the same window, with
        // only some of it asked to redraw.
        case(
            "long thread and a file: clock step (mark layer)",
            Box::new(nothing),
            |app, cx| app.step_marks(cx),
        ),
        case(
            "long thread and a file: busy-mark regions (once a second)",
            Box::new(nothing),
            |app, cx| app.redraw_regions(&Region::BUSY_MARKS, cx),
        ),
        case(
            "long thread and a file: every chat pane redrawn",
            Box::new(nothing),
            |app, cx| app.redraw_regions(&[Region::Chat], cx),
        ),
        case(
            "long thread and a file: a token (its pane)",
            Box::new(nothing),
            |app, cx| {
                if let Some(id) = app.selected_session_id.clone() {
                    app.redraw_thread(&id, cx);
                }
            },
        ),
        case(
            "long thread and a file: a keystroke (the composer's pane)",
            Box::new(nothing),
            |app, cx| app.redraw_composer(cx),
        ),
        case(
            "long thread and a file: file pane redrawn (a keystroke)",
            Box::new(nothing),
            |app, cx| app.redraw_regions(&[Region::FilePane], cx),
        ),
        case(
            "long thread and a file: footer redrawn",
            Box::new(nothing),
            |app, cx| app.redraw_regions(&[Region::Footer], cx),
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
            "app notified, long thread, a commit beside it",
            Box::new(|app, cx| {
                app.set_diff_split(false, cx);
                let tab = PaneTab::Commit {
                    cwd: env!("CARGO_MANIFEST_DIR").to_string(),
                    sha: COMMIT.to_string(),
                    short_sha: COMMIT[..7].to_string(),
                    subject: String::new(),
                };
                app.open_pane_tab(tab, true, cx);
            }),
            notify_app,
        ),
        case(
            "app notified, long thread, the commit side by side",
            Box::new(|app, cx| app.set_diff_split(true, cx)),
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
        case(
            "app notified, a long subagent run, closed",
            Box::new(open_longest_run_thread),
            notify_app,
        ),
        case(
            "app notified, a long subagent run, open",
            Box::new(open_longest_run),
            notify_app,
        ),
        case(
            "a long subagent run open, a step streamed in",
            Box::new(nothing),
            stream_into_run,
        ),
    ]
}

/// Opening a view: `open` is timed with the draw that shows it; `close`
/// puts the window back before the next try.
struct Opening {
    name: &'static str,
    open: Step,
    close: Step,
}

/// Tries per opening: each is a cold first draw, so fewer than a case's.
const OPENINGS: usize = 15;

/// Drops a thread's transcript view, so opening it lays it out and
/// measures its rows again, as the first time it opens after a launch.
fn forget_view(app: &mut BenCodeApp, id: &str) {
    app.transcripts.remove(id);
}

fn openings(long: Option<String>, other: Option<String>) -> Vec<Opening> {
    let surface = |name, surface: Surface| Opening {
        name,
        open: Box::new(move |app, cx| app.show_surface(surface, cx)),
        close: Box::new(|app, cx| app.close_surface(cx)),
    };
    let thread = |name, cold: bool, id: Option<String>, back: Option<String>| Opening {
        name,
        open: Box::new(move |app, cx| {
            if let Some(id) = &id {
                if cold {
                    forget_view(app, id);
                }
                app.open_session(id.clone(), cx);
            }
        }),
        close: Box::new(move |app, cx| {
            if let Some(back) = &back {
                app.open_session(back.clone(), cx);
            }
        }),
    };
    vec![
        thread(
            "open the longest thread, first time",
            true,
            long.clone(),
            other.clone(),
        ),
        thread(
            "open the longest thread again",
            false,
            long.clone(),
            other.clone(),
        ),
        thread(
            "open another thread, first time",
            true,
            other.clone(),
            long.clone(),
        ),
        surface("open Inbox", Surface::Inbox),
        surface("open Notes", Surface::Notes),
        surface("open Automations", Surface::Automations),
        surface("open Settings", Surface::Settings),
    ]
}

/// Pixels a frame of the fling moves: a quick trackpad flick at 120Hz.
const FLING_STEP: f32 = 60.0;
/// A frame at 120Hz.
const FRAME_BUDGET: Duration = Duration::from_micros(8333);

/// Flings the open thread `step` pixels a frame (up when negative) until
/// it stops: each frame is a trackpad's scroll event over the transcript,
/// dispatched as the platform's would be, and the draw it causes.
async fn fling(
    this: &WeakEntity<BenCodeApp>,
    id: &str,
    step: f32,
    cx: &mut AsyncWindowContext,
) -> Result<Vec<Duration>> {
    let mut took = Vec::new();
    let mut last = None;
    let mut still = 0;
    // A long thread is a few hundred screens.
    for _ in 0..5_000 {
        let Some((center, top)) = this.update(cx, |app, _| {
            let list = &app.transcripts.get(id)?.list;
            let top = list.logical_scroll_top();
            Some((
                list.viewport_bounds().center(),
                (top.item_ix, top.offset_in_item),
            ))
        })?
        else {
            break;
        };
        // Stopped: the position held for a few frames.
        if last == Some(top) {
            still += 1;
            if still == 3 {
                break;
            }
        } else {
            still = 0;
        }
        last = Some(top);
        let started = Instant::now();
        cx.update(|window, cx| {
            let event = gpui::ScrollWheelEvent {
                position: center,
                // A positive delta scrolls the content down: toward the start.
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-step))),
                modifiers: Default::default(),
                touch_phase: gpui::TouchPhase::Moved,
            };
            window.dispatch_event(gpui::PlatformInput::ScrollWheel(event), cx);
        })?;
        draw(cx)?;
        took.push(started.elapsed());
    }
    Ok(took)
}

fn print_frames(name: &str, mut took: Vec<Duration>) {
    if took.is_empty() {
        println!("{name:<52} (nothing to measure)");
        return;
    }
    let over = took.iter().filter(|t| **t > FRAME_BUDGET).count();
    took.sort();
    let n = took.len();
    println!(
        "{:<52} {:>6.2}ms {:>6.2}ms {:>6.2}ms   {n} frames, {over} over 8.3ms",
        name,
        millis(took[n / 2]),
        millis(took[n * 95 / 100]),
        millis(took[n - 1]),
    );
}

async fn run_openings(
    this: &WeakEntity<BenCodeApp>,
    long: Option<String>,
    other: Option<String>,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let only = std::env::var("BENCODE_FRAME_BENCH_CASE").ok();
    for opening in openings(long, other) {
        if only
            .as_ref()
            .is_some_and(|only| !opening.name.contains(only.as_str()))
        {
            continue;
        }
        let mut took = Vec::with_capacity(OPENINGS);
        for _ in 0..OPENINGS {
            this.update(cx, |app, cx| {
                (opening.close)(app, cx);
                cx.notify();
            })?;
            settle(cx).await?;
            let started = Instant::now();
            this.update(cx, |app, cx| {
                (opening.open)(app, cx);
                cx.notify();
            })?;
            draw(cx)?;
            took.push(started.elapsed());
        }
        this.update(cx, |app, cx| (opening.close)(app, cx))?;
        took.sort();
        println!(
            "{:<52} {:>6.2}ms {:>6.2}ms {:>6.2}ms",
            opening.name,
            millis(took[OPENINGS / 2]),
            millis(took[OPENINGS * 95 / 100]),
            millis(took[OPENINGS - 1]),
        );
    }
    Ok(())
}

async fn run_flings(
    this: &WeakEntity<BenCodeApp>,
    long: &str,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    // Opened fresh, at its end, every turn's work unfolded: the longest
    // read there is, and no row above the screen drawn yet.
    this.update(cx, |app, cx| {
        forget_view(app, long);
        app.open_session(long.to_string(), cx);
        if let Some(session) = app.sessions.iter().find(|s| s.id == long) {
            let users = session.blocks.iter().filter(|b| b.role == "user");
            app.transcript_ui
                .open_folds
                .extend(users.map(|b| b.id.clone()));
        }
        cx.notify();
    })?;
    settle(cx).await?;
    let rows = this.update(cx, |app, _| app.transcripts.get(long).map(|v| v.rows.len()))?;
    println!("{} rows, {FLING_STEP}px a frame", rows.unwrap_or(0));
    print_frames(
        "fling to the start, rows drawn the first time",
        fling(this, long, -FLING_STEP, cx).await?,
    );
    print_frames(
        "fling back to the end, rows drawn before",
        fling(this, long, FLING_STEP, cx).await?,
    );
    print_frames(
        "fling to the start again",
        fling(this, long, -FLING_STEP, cx).await?,
    );
    Ok(())
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
    let long = thread.map(|(id, _)| id);
    for case in cases(long.clone()) {
        this.update(cx, |app, cx| {
            (case.set_up)(app, cx);
            cx.notify();
        })?;
        settle(cx).await?;
        if only
            .as_ref()
            .is_some_and(|only| !case.name.contains(only.as_str()))
        {
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
    // The views back to a plain window: the long thread, nothing beside it.
    let other = this.update(cx, |app, cx| {
        app.set_terminal_open(false, cx);
        while app.close_active_pane_tab(cx) {}
        let long = long.as_deref();
        app.sessions
            .iter()
            .filter(|s| Some(s.id.as_str()) != long && !app.is_mono_session(&s.id))
            .max_by_key(|s| s.blocks.len())
            .map(|s| s.id.clone())
    })?;
    println!(
        "\n{:<52} {:>8} {:>8} {:>8}",
        "opening (change + first draw)", "median", "p95", "max"
    );
    run_openings(&this, long.clone(), other, cx).await?;
    if let Some(long) = &long {
        println!(
            "\n{:<52} {:>8} {:>8} {:>8}",
            "fling, every frame", "median", "p95", "max"
        );
        // `BENCODE_FRAME_BENCH_FLINGS`: the flings again, from a cold
        // view each time, long enough to `sample` them.
        let times = std::env::var("BENCODE_FRAME_BENCH_FLINGS")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(1);
        for _ in 0..times {
            run_flings(&this, long, cx).await?;
        }
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
