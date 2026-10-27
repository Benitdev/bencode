//! Pictures of the window for UI work (BenCode's own, built with
//! `--features snapshot`). `BENCODE_SNAPSHOT=<dir>` opens the window at a
//! fixed size, sets it up scene by scene and saves each scene's frame as
//! `<dir>/<nn>-<scene>.png`, rendered by GPUI off screen: no screen
//! recording grant, and it works with the screen locked. Then it quits.
//!
//! `BENCODE_SNAPSHOT_SCENES=home,changes` picks scenes (all by default);
//! `BENCODE_SNAPSHOT_SIZE=1440x900` the window's size in points;
//! `BENCODE_SNAPSHOT_POINTER=120,560` where the pointer rests. Like the
//! frame bench it runs on a copy of the data folder (a scratch `HOME`, see
//! AGENTS.md) and starts no agent, fetch or sign-in.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui::{AsyncWindowContext, Context, WeakEntity, Window, px, size};

use crate::app::surfaces::Surface;
use crate::app::{AgentRun, BenCodeApp, InspectorMode};
use crate::harness::{HarnessProcessHandle, PermissionRequest};
use crate::ui::layout::SplitDir;

const ENV: &str = "BENCODE_SNAPSHOT";
const SCENES_ENV: &str = "BENCODE_SNAPSHOT_SCENES";
const SIZE_ENV: &str = "BENCODE_SNAPSHOT_SIZE";
const BACKDROP_ENV: &str = "BENCODE_SNAPSHOT_BACKDROP";
const POINTER_ENV: &str = "BENCODE_SNAPSHOT_POINTER";
const DEFAULT_SIZE: (f32, f32) = (1440.0, 900.0);
/// The launch's loads (sessions, the git snapshot) land in this time.
const SETTLE: Duration = Duration::from_secs(3);
const SETTLE_STEP: Duration = Duration::from_millis(40);
const SETTLE_DRAWS: usize = 25;
/// A file every checkout has.
const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/app.rs");
/// A picture every checkout has.
const PICTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/packaging/macos/icon-1024.png");

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os(ENV).is_some())
}

fn out_dir() -> Option<PathBuf> {
    std::env::var_os(ENV).map(PathBuf::from)
}

fn window_size() -> (f32, f32) {
    std::env::var(SIZE_ENV)
        .ok()
        .and_then(|value| {
            let (w, h) = value.split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .unwrap_or(DEFAULT_SIZE)
}

/// `BENCODE_SNAPSHOT_POINTER=x,y`: where the pointer rests in every scene,
/// in points, to see what hovering there shows.
fn pointer() -> Option<(f32, f32)> {
    let value = std::env::var(POINTER_ENV).ok()?;
    let (x, y) = value.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

type SetUp = fn(&mut BenCodeApp, &mut Window, &mut Context<BenCodeApp>);

/// The scenes in order; each starts from the one before.
const SCENES: &[(&str, SetUp)] = &[
    ("home", |_, _, _| {}),
    ("threads", |app, _, cx| {
        app.set_terminal_open(false, cx);
        app.set_inspector_open(false, cx);
    }),
    ("agents", |app, _, cx| {
        stage_agents(app);
        stage_usage(app, cx);
        cx.notify();
    }),
    ("changes", |app, _, cx| {
        app.show_inspector(InspectorMode::Changes, cx)
    }),
    ("explorer", |app, _, cx| {
        app.show_inspector(InspectorMode::Files, cx)
    }),
    ("file", |app, window, cx| {
        app.open_file_in_editor(FILE, window, cx)
    }),
    ("file-alone", |app, _, cx| app.set_inspector_open(false, cx)),
    ("picture", |app, window, cx| {
        app.open_file_in_editor(PICTURE, window, cx)
    }),
    ("split", |app, _, cx| {
        app.split_active_pane(SplitDir::Right, cx)
    }),
    ("terminal", |app, _, cx| app.set_terminal_open(true, cx)),
    ("navigator-closed", |app, _, cx| {
        app.set_terminal_open(false, cx);
        app.is_rail_open = false;
        cx.notify();
    }),
    ("inbox", |app, _, cx| {
        app.is_rail_open = true;
        app.show_surface(Surface::Inbox, cx);
    }),
    ("notes", |app, _, cx| app.show_surface(Surface::Notes, cx)),
    ("automations", |app, _, cx| {
        app.show_surface(Surface::Automations, cx)
    }),
    ("settings", |app, _, cx| {
        app.show_surface(Surface::Settings, cx)
    }),
];

/// Turns as the Agents section shows them, without an agent: a thread of
/// another project waiting on a command, one of this project and one of
/// another at work, one more finished unseen. Their runs end with the
/// snapshots.
fn stage_agents(app: &mut BenCodeApp) {
    let here = app.current_cwd.clone();
    let mut theirs = app
        .sessions
        .iter()
        .filter(|s| s.has_user_message() && !crate::app::same_project_path(&s.cwd, &here))
        .map(|s| s.id.clone());
    let mut ours = app
        .sessions
        .iter()
        .filter(|s| s.has_user_message() && crate::app::same_project_path(&s.cwd, &here))
        .skip(1)
        .map(|s| s.id.clone());
    let waiting = PermissionRequest {
        request_id: "snapshot".into(),
        tool: "Bash".into(),
        description: "npm run test -- --watch=false".into(),
        input: serde_json::Value::Null,
    };
    let picks = [
        (theirs.next(), Some(waiting)),
        (ours.next(), None),
        (theirs.next(), None),
    ];
    for (n, (id, pending)) in picks.into_iter().enumerate() {
        if let Some(id) = id {
            app.runs.insert(id, staged_run(n as u64, pending));
        }
    }
    if let Some(id) = ours.next() {
        app.title_strip.unseen_finished.insert(id);
    }
}

/// The focused thread's account as the usage chip reads one: a 5-hour
/// window at 71% and a week at 68%.
fn stage_usage(app: &mut BenCodeApp, cx: &mut Context<BenCodeApp>) {
    use crate::rate_limits::{ProviderRateLimits, RateLimitStatus, RateLimitWindow};
    let Some(target) = app.usage_target() else {
        return;
    };
    let now = crate::app::now_ms();
    let window = |used_percent, window_minutes, resets_in_minutes: i64| RateLimitWindow {
        used_percent,
        window_minutes,
        resets_at: Some(now + resets_in_minutes * 60_000),
    };
    let limits = ProviderRateLimits {
        session: Some(window(71.0, 300, 4 * 60 + 19)),
        weekly: Some(window(68.0, 10_080, 36 * 60)),
        monthly: None,
        updated_at: now,
        error: None,
        status: RateLimitStatus::Ok,
    };
    app.usage_data.update(cx, |data, cx| {
        data.stage(target.provider, &target.account_id, limits);
        cx.notify();
    });
}

fn staged_run(id: u64, pending: Option<PermissionRequest>) -> AgentRun {
    let (stdin_tx, _) = tokio::sync::mpsc::unbounded_channel();
    let (cancel_tx, _) = tokio::sync::oneshot::channel();
    AgentRun {
        id: u64::MAX - id,
        handle: HarnessProcessHandle::new(stdin_tx, cancel_tx, None, None),
        pending_permission: pending,
        outcome: None,
        automation_run_id: None,
        auto_approve: false,
        can_steer: false,
        purpose: Default::default(),
        compacted: None,
        edit_paths: Default::default(),
        last_saved: std::time::Instant::now(),
        background: Vec::new(),
        answered: true,
        remote: None,
    }
}

/// What shows through the window's glass: `BENCODE_SNAPSHOT_BACKDROP` as
/// `#rrggbb`, else a dark desktop.
fn backdrop() -> [u8; 3] {
    std::env::var(BACKDROP_ENV)
        .ok()
        .and_then(|hex| {
            let hex = hex.trim_start_matches('#');
            let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
            Some([byte(0)?, byte(2)?, byte(4)?])
        })
        .unwrap_or([0x1b, 0x1b, 0x1f])
}

/// The frame is premultiplied over a clear target (GPUI blends colour by
/// source alpha onto black): laid over the backdrop, it is opaque as the
/// window server would show it.
fn flatten(image: &mut image::RgbaImage, backdrop: [u8; 3]) {
    for pixel in image.pixels_mut() {
        let rest = 255 - u16::from(pixel[3]);
        for (channel, under) in pixel.0.iter_mut().take(3).zip(backdrop) {
            let over = u16::from(*channel) + (u16::from(under) * rest + 127) / 255;
            *channel = over.min(255) as u8;
        }
        pixel[3] = 255;
    }
}

fn draw(cx: &mut AsyncWindowContext) -> Result<()> {
    cx.update(|window, cx| window.draw(cx).clear(cx))
}

async fn settle(cx: &mut AsyncWindowContext) -> Result<()> {
    for _ in 0..SETTLE_DRAWS {
        cx.update(|window, _| window.refresh())?;
        draw(cx)?;
        cx.background_executor().timer(SETTLE_STEP).await;
    }
    Ok(())
}

async fn run(this: WeakEntity<BenCodeApp>, cx: &mut AsyncWindowContext) -> Result<()> {
    let dir = out_dir().context("BENCODE_SNAPSHOT names no folder")?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let (width, height) = window_size();
    cx.update(|window, _| window.resize(size(px(width), px(height))))?;
    cx.background_executor().timer(SETTLE).await;
    let picked: Option<Vec<String>> = std::env::var(SCENES_ENV)
        .ok()
        .map(|list| list.split(',').map(|s| s.trim().to_string()).collect());
    for (ix, (name, set_up)) in SCENES.iter().enumerate() {
        if picked
            .as_ref()
            .is_some_and(|picked| !picked.iter().any(|p| p == name))
        {
            continue;
        }
        this.update_in(cx, set_up)?;
        if let Some(at) = pointer() {
            // The pointer rests there, as a hover would leave it.
            cx.update(|window, cx| {
                let event = gpui::MouseMoveEvent {
                    position: gpui::point(px(at.0), px(at.1)),
                    pressed_button: None,
                    modifiers: Default::default(),
                };
                window.dispatch_event(gpui::PlatformInput::MouseMove(event), cx);
            })?;
        }
        settle(cx).await?;
        let mut image = cx.update(|window, _| window.render_to_image())??;
        flatten(&mut image, backdrop());
        let path = dir.join(format!("{ix:02}-{name}.png"));
        image
            .save(&path)
            .with_context(|| format!("saving {}", path.display()))?;
        println!("snapshot: {}", path.display());
    }
    Ok(())
}

impl BenCodeApp {
    /// Takes the pictures once the window is up, then quits.
    pub(crate) fn start_snapshots(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            if let Err(err) = run(this, cx).await {
                log::error!("snapshots stopped: {err:#}");
            }
            if let Err(err) = cx.update(|_, cx| cx.quit()) {
                log::error!("snapshots could not quit: {err:#}");
            }
        })
        .detach();
    }
}
