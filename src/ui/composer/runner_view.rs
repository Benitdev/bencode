//! MonoCode `ComposerRunner`: while the focused thread's turn runs, its
//! project mascot runs along the composer's top edge (or the card stacked on
//! it), grabs coins and bonks then hops "Jump to latest"; when the turn ends
//! it leaps off and sinks behind the rim. `RunnerLayer` steps it once per
//! frame and draws it in window coordinates above everything else.

use std::cell::Cell;
use std::hash::{BuildHasher, RandomState};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, Context, Hsla, IntoElement, ParentElement, Pixels, Render, Styled,
    WeakEntity, Window, anchored, canvas, deferred, div, point,
};

use crate::ui::scale::px;

use super::runner::{
    COIN_HOVER, COIN_SIZE, COLLECT_POP_MS, COLLECT_POP_PX, CRASH_STUN_MS, Coin, EXIT_MS, Facing,
    Obstacle, RUNNER_SIZE, Rect, STAR_SIZE, coin_collected, exit_jump_y, hits_chevron, inner_width,
    jump_height, next_coin_delay, obstacle_from_rects, pick_coin_x, pose_at, recoil_along,
    scale_track_x, sprite_clip_bottom, step_along, stun_shake, stun_stars,
};
use crate::app::BenCodeApp;
use crate::ui::mascot::{
    COIN_EDGE, COIN_FACE, Mascot, STAR_EDGE, STAR_FACE, pixel_sprite, project_mascot,
};

/// MonoCode `--mascot-beat`: rest and talk frames alternate each half.
const MASCOT_HALF_BEAT_MS: u128 = 230;
/// Coins and stars spin at MonoCode's 320ms.
const SPIN_HALF_MS: u128 = 160;
/// Frames longer than this (a stall, an inactive window) count as this.
const MAX_STEP_MS: f32 = 48.0;
const COIN_GOLD: u32 = 0xe8b923;
const STAR_GOLD: u32 = 0xf4e27a;

/// Where the runner runs, measured as the composer is laid out. Each frame
/// takes what the previous layout measured, so a composer that is no longer
/// drawn leaves nothing behind.
#[derive(Clone, Default)]
pub struct RunnerGeometry {
    /// The composer box.
    pub track: Rc<Cell<Option<Rect>>>,
    /// The card stacked on it (question, usage limit, queue), if any.
    pub ledge: Rc<Cell<Option<Rect>>>,
    /// The focused pane's "Jump to latest" chevron, while shown.
    pub chevron: Rc<Cell<Option<Rect>>>,
}

fn rect(bounds: Bounds<Pixels>) -> Rect {
    Rect {
        left: crate::ui::scale::logical(bounds.origin.x),
        top: crate::ui::scale::logical(bounds.origin.y),
        right: crate::ui::scale::logical(bounds.origin.x + bounds.size.width),
        bottom: crate::ui::scale::logical(bounds.origin.y + bounds.size.height),
    }
}

/// A zero-cost element writing its bounds into `cell` as it is laid out.
pub fn measure(cell: &Rc<Cell<Option<Rect>>>) -> impl IntoElement {
    let cell = cell.clone();
    canvas(
        move |bounds, _, _| cell.set(Some(rect(bounds))),
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

struct LiveCoin {
    coin: Coin,
    collected_at: Option<Instant>,
}

struct Stun {
    at: Instant,
    hit_along: f32,
    facing: Facing,
}

struct Exit {
    at: Instant,
    x: f32,
    facing: Facing,
}

/// One run, from a turn's start until the mascot has left.
pub struct Runner {
    session_id: String,
    mascot: &'static Mascot,
    color: Hsla,
    started: Instant,
    last: Instant,
    along: f32,
    facing: Facing,
    width: f32,
    coins: Vec<LiveCoin>,
    next_coin_at: Instant,
    /// Bonked the chevron once this turn; hops it from now on.
    learned: bool,
    stun: Option<Stun>,
    exit: Option<Exit>,
    seed: u64,
}

/// What to draw this frame, in window coordinates.
struct Frame {
    sprite: (f32, f32, f32),
    facing: Facing,
    talking: bool,
    coins: Vec<(f32, f32, f32)>,
    stars: Vec<(f32, f32, f32)>,
}

impl Runner {
    fn new(session_id: String, mascot: &'static Mascot, color: Hsla, now: Instant) -> Self {
        let seed = RandomState::new().hash_one(now) | 1;
        let mut runner = Self {
            session_id,
            mascot,
            color,
            started: now,
            last: now,
            along: 0.0,
            facing: Facing::Right,
            width: 0.0,
            coins: Vec::new(),
            next_coin_at: now,
            learned: false,
            stun: None,
            exit: None,
            seed,
        };
        let first = next_coin_delay(true, runner.random());
        runner.next_coin_at = now + ms(first);
        runner
    }

    /// xorshift64, in `0..1`.
    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Advances one frame. `busy` is whether the turn still runs; returns
    /// false once the exit hop has finished.
    fn step(&mut self, now: Instant, busy: bool, track: Rect, obstacle: Option<Obstacle>) -> bool {
        let dt = (now - self.last).as_secs_f32() * 1000.0;
        let dt = dt.min(MAX_STEP_MS);
        self.last = now;
        let width = track.width();
        if self.width > 0.0 && self.width != width {
            let (from, to) = (inner_width(self.width), inner_width(width));
            self.along = scale_track_x(self.along, from, to);
            if let Some(stun) = &mut self.stun {
                stun.hit_along = scale_track_x(stun.hit_along, from, to);
            }
            if let Some(exit) = &mut self.exit {
                exit.x = scale_track_x(exit.x, self.width, width);
            }
            for live in &mut self.coins {
                live.coin.x = scale_track_x(live.coin.x, self.width, width);
            }
        }
        self.width = width;
        let track_len = inner_width(width);

        if busy {
            if self.exit.take().is_some() {
                self.learned = false;
                self.stun = None;
            }
            if self.stun.is_none() {
                (self.along, self.facing) = step_along(self.along, self.facing, dt, track_len);
            }
        } else if self.exit.is_none() {
            let pose = pose_at(self.along, self.facing, width, None, &[]);
            self.exit = Some(Exit {
                at: now,
                x: pose.x,
                facing: pose.facing,
            });
            self.stun = None;
            for live in &mut self.coins {
                live.collected_at.get_or_insert(now);
            }
        }
        if let Some(exit) = &self.exit {
            let done = since(exit.at, now) >= EXIT_MS;
            self.coins.retain(|live| {
                live.collected_at
                    .is_none_or(|at| since(at, now) < COLLECT_POP_MS)
            });
            return !done;
        }

        if let Some(stun) = &self.stun {
            let elapsed = since(stun.at, now);
            self.along = recoil_along(stun.hit_along, stun.facing, elapsed, track_len);
            self.facing = stun.facing;
            if elapsed >= CRASH_STUN_MS {
                self.learned = true;
                self.stun = None;
            }
        }
        for live in &mut self.coins {
            let off_track = live.coin.x < super::runner::RUNNER_INSET
                || live.coin.x > width - super::runner::RUNNER_INSET;
            if off_track {
                live.collected_at.get_or_insert(now);
            }
        }
        let hurdle = obstacle.filter(|_| self.learned);
        let coins: Vec<Coin> = if self.stun.is_some() {
            Vec::new()
        } else {
            self.coins.iter().map(|live| live.coin).collect()
        };
        let pose = pose_at(self.along, self.facing, width, hurdle.as_ref(), &coins);
        if self.stun.is_none() && hits_chevron(&pose, obstacle.as_ref(), self.learned) {
            self.stun = Some(Stun {
                at: now,
                hit_along: self.along,
                facing: self.facing,
            });
        }
        let has_live = self.coins.iter().any(|live| live.collected_at.is_none());
        if self.stun.is_none() && !has_live && now >= self.next_coin_at {
            let mut draws = [self.random(), self.random(), self.random(), self.random()]
                .into_iter()
                .chain(std::iter::repeat(0.5));
            match pick_coin_x(width, pose.x, obstacle.as_ref(), || {
                draws.next().unwrap_or(0.5)
            }) {
                Some(x) => self.coins.push(LiveCoin {
                    coin: Coin {
                        x,
                        height: COIN_HOVER,
                    },
                    collected_at: None,
                }),
                None => self.next_coin_at = now + Duration::from_secs(2),
            }
        }
        if self.stun.is_none() {
            for i in 0..self.coins.len() {
                if self.coins[i].collected_at.is_none()
                    && coin_collected(&pose, &self.coins[i].coin)
                {
                    self.coins[i].collected_at = Some(now);
                    let gap = next_coin_delay(false, self.random());
                    self.next_coin_at = now + ms(gap);
                }
            }
        }
        // A grabbed coin stays in the pose until the hop lands.
        self.coins.retain(|live| {
            live.collected_at.is_none_or(|at| {
                since(at, now) < COLLECT_POP_MS || jump_height(pose.x, None, &[live.coin]) > 0.5
            })
        });
        true
    }

    fn frame(&self, now: Instant, track: Rect, obstacle: Option<Obstacle>) -> Frame {
        let width = track.width();
        let elapsed = now.duration_since(self.started).as_millis();
        let talking = self.stun.is_none() && (elapsed / MASCOT_HALF_BEAT_MS) % 2 == 1;
        let (x, y, facing) = match &self.exit {
            Some(exit) => {
                let t = (since(exit.at, now) / EXIT_MS).min(1.0);
                (exit.x, exit_jump_y(t), exit.facing)
            }
            None => {
                let hurdle = obstacle.filter(|_| self.learned);
                let coins: Vec<Coin> = if self.stun.is_some() {
                    Vec::new()
                } else {
                    self.coins.iter().map(|live| live.coin).collect()
                };
                let pose = pose_at(self.along, self.facing, width, hurdle.as_ref(), &coins);
                (pose.x, pose.y, pose.facing)
            }
        };
        let (shake_x, shake_y) = self
            .stun
            .as_ref()
            .map_or((0.0, 0.0), |stun| stun_shake(since(stun.at, now)));
        let left = (track.left + x - RUNNER_SIZE / 2.0 + shake_x).round();
        let top = (track.top - RUNNER_SIZE - y + 1.0 + shake_y).round();
        let stars = self.stun.as_ref().map_or_else(Vec::new, |stun| {
            stun_stars(since(stun.at, now))
                .into_iter()
                .map(|(dx, dy, opacity)| (left + dx, top + dy, opacity))
                .collect()
        });
        let bob = (elapsed as f32 / 180.0).sin() * 2.0;
        let coins = self
            .coins
            .iter()
            .map(|live| {
                let pop = live
                    .collected_at
                    .map_or(0.0, |at| (since(at, now) / COLLECT_POP_MS).min(1.0));
                let lift = if live.collected_at.is_none() {
                    bob
                } else {
                    0.0
                };
                (
                    (track.left + live.coin.x - COIN_SIZE / 2.0).round(),
                    (track.top - live.coin.height - COIN_SIZE / 2.0 - lift - COLLECT_POP_PX * pop)
                        .round(),
                    1.0 - pop,
                )
            })
            .collect();
        Frame {
            sprite: (left, top, sprite_clip_bottom(y)),
            facing,
            talking,
            coins,
            stars,
        }
    }
}

fn ms(value: f32) -> Duration {
    Duration::from_secs_f32(value / 1000.0)
}

fn since(at: Instant, now: Instant) -> f32 {
    now.saturating_duration_since(at).as_secs_f32() * 1000.0
}

impl RunnerGeometry {
    /// Forgets the last measurements; called as the app lays out again, so
    /// a composer it no longer draws leaves nothing behind.
    pub fn clear(&self) {
        self.track.set(None);
        self.ledge.set(None);
        self.chevron.set(None);
    }

    /// The track (the stacked card when there is one, else the box) and the
    /// chevron as a hurdle, as the app was last laid out.
    fn current(&self) -> Option<(Rect, Option<Obstacle>)> {
        let (track, ledge, chevron) = (self.track.get(), self.ledge.get(), self.chevron.get());
        let track = ledge
            .filter(|ledge| ledge.width() > 0.0 && ledge.bottom - ledge.top > 1.0)
            .or(track)
            .filter(|track| track.width() > 0.0)?;
        let rim = Rect {
            bottom: track.top + 8.0,
            ..track
        };
        Some((track, obstacle_from_rects(&rim, chevron.as_ref())))
    }
}

/// The runner's own view, beside the (cached) app in the window root. Its
/// per-frame redraws re-render only this layer; the app's last frame is
/// reused until the app itself changes.
pub struct RunnerLayer {
    app: WeakEntity<BenCodeApp>,
    runner: Option<Runner>,
    frame: Option<(Rect, Option<Obstacle>)>,
}

impl RunnerLayer {
    pub fn new(app: WeakEntity<BenCodeApp>) -> Self {
        Self {
            app,
            runner: None,
            frame: None,
        }
    }

    /// Steps the runner once per frame.
    fn step(&mut self, window: &mut Window, cx: &App) {
        let Some(app) = self.app.upgrade() else {
            self.runner = None;
            return;
        };
        let app = app.read(cx);
        self.frame = app.runner_geometry.current();
        if app.composer_mascot_off {
            self.runner = None;
            return;
        }
        let session = app.selected_session();
        let busy_id = session
            .filter(|s| app.is_agent_running_in(&s.id))
            .map(|s| (s.id.clone(), s.cwd.clone()));
        let now = Instant::now();
        // A run belongs to one thread; another thread starts its own.
        if self
            .runner
            .as_ref()
            .is_some_and(|r| Some(&r.session_id) != app.selected_session_id.as_ref())
        {
            self.runner = None;
        }
        if self.runner.is_none() {
            let Some((id, cwd)) = busy_id.clone() else {
                return;
            };
            let project = std::path::Path::new(&cwd)
                .file_name()
                .map_or_else(|| cwd.clone(), |n| n.to_string_lossy().into_owned());
            let color = app.project_color(&cwd);
            self.runner = Some(Runner::new(id, project_mascot(&project), color, now));
        }
        let Some((track, obstacle)) = self.frame else {
            window.request_animation_frame();
            return;
        };
        let busy = busy_id.is_some();
        let alive = self
            .runner
            .as_mut()
            .is_some_and(|runner| runner.step(now, busy, track, obstacle));
        if !alive {
            self.runner = None;
            return;
        }
        if window.is_window_active() {
            window.request_animation_frame();
        }
    }

    /// The runner, its coins and stars, over everything in the window.
    fn render_runner(&self) -> Option<AnyElement> {
        let runner = self.runner.as_ref()?;
        let (track, obstacle) = self.frame?;
        let frame = runner.frame(Instant::now(), track, obstacle);
        let elapsed = runner.started.elapsed().as_millis();
        let face = (elapsed / SPIN_HALF_MS).is_multiple_of(2);
        let at = |x: f32, y: f32| anchored().position(point(px(x), px(y)));
        let (left, top, clip) = frame.sprite;
        let rows = if frame.talking {
            &runner.mascot.talk
        } else {
            &runner.mascot.rest
        };
        // The talk frame hops a pixel (MonoCode `mascot-hop`).
        let hop = if frame.talking { 1.0 } else { 0.0 };
        let visible = (RUNNER_SIZE - clip).max(0.0);
        let sprite = at(left, top - hop).child(
            div()
                .w(px(RUNNER_SIZE))
                .h(px(visible))
                .overflow_hidden()
                .child(pixel_sprite(
                    rows,
                    px(RUNNER_SIZE),
                    runner.color,
                    frame.facing == Facing::Left,
                )),
        );
        let coin_rows = if face { &COIN_FACE } else { &COIN_EDGE };
        let coins = frame.coins.into_iter().map(|(x, y, opacity)| {
            at(x, y).child(div().opacity(opacity).child(pixel_sprite(
                coin_rows,
                px(COIN_SIZE),
                gpui::rgb(COIN_GOLD).into(),
                false,
            )))
        });
        let star_rows = if face { &STAR_FACE } else { &STAR_EDGE };
        let stars = frame.stars.into_iter().map(|(x, y, opacity)| {
            at(x, y).child(div().opacity(opacity).child(pixel_sprite(
                star_rows,
                px(STAR_SIZE),
                gpui::rgb(STAR_GOLD).into(),
                false,
            )))
        });
        Some(
            deferred(div().children(coins).child(sprite).children(stars))
                .with_priority(2)
                .into_any_element(),
        )
    }
}

impl Render for RunnerLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.step(window, cx);
        div().absolute().children(self.render_runner())
    }
}
