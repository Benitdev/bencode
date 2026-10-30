//! A bot's or a project's figure on screen (`bots/figure.rs` composes it): one canvas, a
//! quad for each run of a tone, its edges on whole device pixels. A figure
//! that moves is a busy mark (`ui/marks.rs`), stepped by the app's clock;
//! one that holds still is drawn once.

use std::time::{Duration, Instant};

use gpui::{AnyElement, App, Bounds, Hsla, IntoElement, Styled, canvas, fill, point};

use crate::bots::figure::{self, BLINK_EDGES, Figure, Frame, GRID, Mood, Phases};
use crate::ui::scale::px;
use crate::ui::spinner;

/// The hop while it works (up for the second half), the flame's flicker
/// and the sparks' change.
const BEAT: Duration = Duration::from_millis(1200);
/// The mouth while it talks.
const TALK: Duration = Duration::from_millis(600);
/// The blink's cycle (MonoCode's `pixel-eyes-open`).
const BLINK: Duration = Duration::from_millis(5200);
/// A breath at rest, the cheeks' flare and the marks' two places.
const BREATH: Duration = Duration::from_millis(3600);
/// The eyes' glance while it works: four places.
const GLANCE: Duration = Duration::from_millis(4800);
/// Smaller than this, the marks around a figure would crowd what is beside
/// it; its mood shows in its face alone.
const MARKS_FROM: f32 = 16.0;

/// How a figure looks: its mood, and how it moves in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub mood: Mood,
    /// It breathes and blinks at rest: the greeting, the settings face.
    pub lively: bool,
    /// Its blink's place in the cycle, so figures side by side do not
    /// blink together (MonoCode `blinkDelay`).
    pub offset_ms: u64,
    /// Its mood lasts until then and becomes the other (a finished turn's
    /// sparks).
    pub until: Option<(Instant, Mood)>,
}

/// A stable offset into the blink's cycle for `seed`.
fn blink_offset(seed: &str) -> u64 {
    let hash = seed.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    u64::from(hash) % BLINK.as_millis() as u64
}

impl Look {
    /// `mood`, held still at rest.
    pub fn new(mood: Mood, seed: &str) -> Self {
        Self {
            mood,
            lively: false,
            offset_ms: blink_offset(seed),
            until: None,
        }
    }

    /// Breathing and blinking at rest.
    pub fn lively(self) -> Self {
        Self {
            lively: true,
            ..self
        }
    }

    /// Its mood now: the one it changes to once `until` has passed.
    fn mood_at(self, now: Instant) -> Mood {
        match self.until {
            Some((until, after)) if now >= until => after,
            _ => self.mood,
        }
    }

    /// Whether it changes with no agent running: what the app's clock wakes
    /// for (a working figure moves while its agent runs, and the clock
    /// runs then anyway).
    pub fn moves_at_rest(self) -> bool {
        self.lively
            || self.until.is_some()
            || matches!(self.mood_at(Instant::now()), Mood::NeedsYou | Mood::Done)
    }

    /// How long until it next changes (`start_clock` wakes then).
    pub fn next_change(self) -> Duration {
        let now = Instant::now();
        let mood = self.mood_at(now);
        let mut next = Duration::from_secs(60);
        if let Some((until, _)) = self.until.filter(|(until, _)| *until > now) {
            next = next.min(until - now + Duration::from_millis(1));
        }
        if self.lively || matches!(mood, Mood::NeedsYou | Mood::Working) {
            let offset = Duration::from_millis(self.offset_ms);
            next = next.min(spinner::until_edge(BLINK, offset, &BLINK_EDGES));
        }
        if self.lively || mood == Mood::NeedsYou {
            next = next.min(spinner::until_step(BREATH / 2));
        }
        if mood == Mood::Done {
            next = next.min(spinner::until_step(BEAT / 2));
        }
        next
    }

    /// The frame it is on now, by the app's clock; held still while the
    /// system asks for less motion.
    fn frame(self, marks: bool, cx: &App) -> Frame {
        let mood = self.mood_at(Instant::now());
        if spinner::still(cx) {
            return Frame::still(mood, marks);
        }
        let phases = Phases {
            beat: spinner::cycle(BEAT),
            talk: spinner::cycle(TALK),
            blink: spinner::cycle_offset(BLINK, Duration::from_millis(self.offset_ms)),
            breath: spinner::cycle(BREATH),
            glance: spinner::cycle(GLANCE),
        };
        Frame::at(mood, self.lively, phases, marks)
    }

    /// Whether it ever moves: one that does not is drawn once, not as a
    /// mark the clock steps.
    fn moves(self) -> bool {
        self.mood == Mood::Working || self.moves_at_rest()
    }
}

/// `figure` in `frame`, `size` wide: its marks reach outside that box.
fn painted(figure: Figure, frame: Frame, size: f32, color: Hsla) -> AnyElement {
    figure_frame(figure, frame, size, color, false)
}

/// `figure` in a frame its caller steps (the composer's runner), `size`
/// wide, facing left when `mirrored`.
pub fn figure_frame(
    figure: Figure,
    frame: Frame,
    size: f32,
    color: Hsla,
    mirrored: bool,
) -> AnyElement {
    let mut runs = figure::runs(&figure::compose(figure, frame));
    if mirrored {
        for (_, x, len, _) in &mut runs {
            *x = GRID - *x - *len;
        }
    }
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let unit = bounds.size.width / f32::from(GRID);
            let scale = window.scale_factor();
            // Each edge on a device pixel, so neighbouring runs meet
            // without a seam or an overlap.
            let snap = |length: gpui::Pixels| (length * scale).round() / scale;
            for (y, x, len, tone) in &runs {
                let left = snap(bounds.origin.x + unit * f32::from(*x));
                let right = snap(bounds.origin.x + unit * f32::from(*x + *len));
                let top = snap(bounds.origin.y + unit * f32::from(*y));
                let bottom = snap(bounds.origin.y + unit * f32::from(*y + 1));
                window.paint_quad(fill(
                    Bounds::from_corners(point(left, top), point(right, bottom)),
                    figure::tone_color(*tone, color),
                ));
            }
        },
    )
    .flex_none()
    .size(px(size))
    .into_any_element()
}

/// `figure` standing still and idle, `size` wide: where a bot signs a
/// turn, lists, pickers.
pub fn figure_still(figure: Figure, size: f32, color: Hsla) -> AnyElement {
    painted(figure, Frame::still(Mood::Idle, false), size, color)
}

/// `figure` in `look`, `size` wide, moving by the app's clock while its
/// look moves.
pub fn figure_view(figure: Figure, size: f32, color: Hsla, look: Look, cx: &App) -> AnyElement {
    let marks = size >= MARKS_FROM;
    if !look.moves() {
        return painted(figure, look.frame(marks, cx), size, color);
    }
    crate::ui::marks::mark(
        move |cx| painted(figure, look.frame(marks, cx), size, color),
        cx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_figure_moves_at_rest_only_when_it_has_something_to_show() {
        assert!(!Look::new(Mood::Idle, "a").moves_at_rest());
        assert!(!Look::new(Mood::Asleep, "a").moves_at_rest());
        assert!(!Look::new(Mood::Failed, "a").moves());
        assert!(Look::new(Mood::Working, "a").moves());
        assert!(Look::new(Mood::NeedsYou, "a").moves_at_rest());
        assert!(Look::new(Mood::Idle, "a").lively().moves_at_rest());
        let done = Look {
            until: Some((Instant::now() + Duration::from_secs(2), Mood::Idle)),
            ..Look::new(Mood::Done, "a")
        };
        assert!(done.moves_at_rest());
        assert!(done.next_change() <= BEAT / 2 + Duration::from_millis(1));
    }

    #[test]
    fn a_passing_mood_gives_way_to_the_next() {
        let now = Instant::now();
        let done = Look {
            until: Some((now, Mood::Asleep)),
            ..Look::new(Mood::Done, "a")
        };
        assert_eq!(done.mood_at(now), Mood::Asleep);
        assert_eq!(done.mood_at(now - Duration::from_millis(1)), Mood::Done);
    }

    #[test]
    fn figures_side_by_side_blink_at_their_own_time() {
        assert_ne!(blink_offset("bot-1"), blink_offset("bot-2"));
        assert!(blink_offset("bot-1") < BLINK.as_millis() as u64);
    }
}
