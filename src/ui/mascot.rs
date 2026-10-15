//! MonoCode's project mascots (`projectMascots.ts`): 8×8 pixel sprites,
//! `#` painted in the project colour and `.` left clear, each with a rest
//! and a talk frame swapped while a turn runs.

use std::time::Duration;

use gpui::{AnyElement, App, Hsla, IntoElement, ParentElement, Pixels, Styled, div};

use crate::ui::scale::px;
use crate::ui::spinner;

/// One sprite frame, top row first.
pub type Rows = [&'static str; 8];

pub struct Mascot {
    pub rest: Rows,
    pub talk: Rows,
}

/// MonoCode `PROJECT_MASCOTS`, in its order (the hash indexes it).
pub static MASCOTS: [Mascot; 10] = [
    // invader
    Mascot {
        rest: [
            "..#..#..", ".######.", "##.##.##", "########", ".######.", ".#.##.#.", "#.#..#.#",
            "........",
        ],
        talk: [
            "..#..#..", ".######.", "##.##.##", "########", ".######.", "#.####.#", ".#....#.",
            "#......#",
        ],
    },
    // ghost
    Mascot {
        rest: [
            "..####..", ".######.", "##.##.##", "########", "########", "########", "########",
            "#.##.##.",
        ],
        talk: [
            "..####..", ".######.", "#.##.###", "########", "########", "########", "########",
            ".##.##.#",
        ],
    },
    // robot
    Mascot {
        rest: [
            "...#....", ".######.", ".#.##.#.", ".######.", ".#....#.", ".######.", "..#..#..",
            "........",
        ],
        talk: [
            "....#...", ".######.", ".#.##.#.", ".######.", ".##..##.", ".######.", ".#....#.",
            "........",
        ],
    },
    // cat
    Mascot {
        rest: [
            ".#....#.", ".##..##.", "########", "#.####.#", "########", "###..###", ".######.",
            "..#..#..",
        ],
        talk: [
            ".#....#.", ".##..##.", "########", "#.####.#", "########", "########", ".######.",
            ".#....#.",
        ],
    },
    // skull
    Mascot {
        rest: [
            ".######.", "########", "##.##.##", "########", ".##..##.", ".######.", ".#.##.#.",
            "........",
        ],
        talk: [
            ".######.", "########", "##.##.##", "########", ".##..##.", ".######.", ".#....#.",
            "..####..",
        ],
    },
    // crab
    Mascot {
        rest: [
            "#......#", ".#....#.", ".######.", "##.##.##", "########", "#.####.#", "#......#",
            "........",
        ],
        talk: [
            "#......#", "##....##", ".######.", "##.##.##", "########", ".######.", "#.#..#.#",
            "........",
        ],
    },
    // mushroom
    Mascot {
        rest: [
            "..####..", ".######.", "########", "##.##.##", "########", "...##...", "...##...",
            "..####..",
        ],
        talk: [
            "........", "..####..", ".######.", "########", "##.##.##", "...##...", "...##...",
            "..####..",
        ],
    },
    // rocket
    Mascot {
        rest: [
            "...##...", "..####..", "..#..#..", "..####..", ".######.", ".######.", "##....##",
            "..####..",
        ],
        talk: [
            "...##...", "..####..", "..#..#..", "..####..", ".######.", ".######.", "##....##",
            "...##...",
        ],
    },
    // dino
    Mascot {
        rest: [
            "...#####", "...##.##", "...#####", ".#######", "########", "#####...", ".##.##..",
            "..#..#..",
        ],
        talk: [
            "...#####", "...##.##", "...#####", ".#######", "########", "#####...", "..##.##.",
            "..#...#.",
        ],
    },
    // frog
    Mascot {
        rest: [
            "........", "##....##", "#.####.#", "########", "########", ".######.", "##....##",
            "........",
        ],
        talk: [
            "##....##", "#.####.#", "########", "########", ".######.", "##....##", "#......#",
            "........",
        ],
    },
];

/// MonoCode's mascot names, in `MASCOTS` order (saved picks use them).
pub const MASCOT_NAMES: [&str; 10] = [
    "invader", "ghost", "robot", "cat", "skull", "crab", "mushroom", "rocket", "dino", "frog",
];

/// MonoCode `projectMascot(project, name)`: a saved pick by name, else
/// the one hashed from the project.
pub fn mascot_for(project: &str, name: Option<&str>) -> &'static Mascot {
    name.and_then(|name| MASCOT_NAMES.iter().position(|n| *n == name))
        .map_or_else(|| project_mascot(project), |ix| &MASCOTS[ix])
}

/// The name of the mascot `mascot_for` shows.
pub fn mascot_name_for(project: &str, name: Option<&str>) -> &'static str {
    let shown = mascot_for(project, name);
    MASCOTS
        .iter()
        .position(|m| std::ptr::eq(m, shown))
        .map_or(MASCOT_NAMES[0], |ix| MASCOT_NAMES[ix])
}

/// MonoCode `projectMascot`: a stable pick per project name.
pub fn project_mascot(project: &str) -> &'static Mascot {
    let hash = project.encode_utf16().fold(0u32, |hash, unit| {
        hash.wrapping_mul(131).wrapping_add(u32::from(unit))
    });
    &MASCOTS[hash as usize % MASCOTS.len()]
}

/// The filled runs of each row, as (row, column, length), mirrored when
/// facing left (MonoCode flips the sprite with `scaleX(-1)`).
pub fn runs(rows: &Rows, mirrored: bool) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for (y, row) in rows.iter().enumerate() {
        let cells: Vec<bool> = row.chars().map(|c| c == '#').collect();
        let cell = |x: usize| cells[if mirrored { cells.len() - 1 - x } else { x }];
        let mut x = 0;
        while x < cells.len() {
            if !cell(x) {
                x += 1;
                continue;
            }
            let start = x;
            while x < cells.len() && cell(x) {
                x += 1;
            }
            out.push((y, start, x - start));
        }
    }
    out
}

/// A sprite `size` wide, one square per filled run.
pub fn pixel_sprite(rows: &Rows, size: Pixels, color: Hsla, mirrored: bool) -> AnyElement {
    let unit = size / 8.0;
    div()
        .relative()
        .size(size)
        .flex_none()
        .children(runs(rows, mirrored).into_iter().map(|(y, x, len)| {
            div()
                .absolute()
                .left(unit * x as f32)
                .top(unit * y as f32)
                .w(unit * len as f32)
                .h(unit)
                .bg(color)
        }))
        .into_any_element()
}

/// MonoCode `PixelMascot`'s motion by status (`pixel-bob`, `pixel-hop`,
/// `pixel-frame-a` / `-b`): at rest it bobs unless held still, while it
/// works it hops and talks, and while it waits on the user it stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MascotMotion {
    Still,
    Bob,
    Hop,
}

/// `pixel-bob 3.6s step-end`: up for the second half of the beat.
pub const BOB: Duration = Duration::from_millis(3600);
/// `pixel-hop`: the bob at 1.2s.
const HOP: Duration = Duration::from_millis(1200);
/// `pixel-frame-a` / `-b`: the talk frame for the second half of 600ms.
const TALK: Duration = Duration::from_millis(600);
/// `translateY(-0.75px)` on the sprite's 16-unit grid.
const LIFT: f32 = 0.75 / 16.0;

/// Whether the sprite is up and on its talk frame, at `beat` and `talk`
/// (each 0..1 through its period).
pub fn pose(motion: MascotMotion, beat: f32, talk: f32) -> (bool, bool) {
    match motion {
        MascotMotion::Still => (false, false),
        MascotMotion::Bob => (beat >= 0.5, false),
        MascotMotion::Hop => (beat >= 0.5, talk >= 0.5),
    }
}

/// MonoCode `FIGURES`' eye cells (x, y) for the rest and talk frames, in
/// `MASCOTS` order: the holes the blink closes.
const EYES: [(&[(usize, usize)], &[(usize, usize)]); 10] = [
    (&[(2, 2), (5, 2)], &[(2, 2), (5, 2)]),
    (&[(2, 2), (5, 2)], &[(1, 2), (4, 2)]),
    (&[(2, 2), (5, 2)], &[(2, 2), (5, 2)]),
    (&[(1, 3), (6, 3)], &[(1, 3), (6, 3)]),
    (&[(2, 2), (5, 2)], &[(2, 2), (5, 2)]),
    (&[(2, 3), (5, 3)], &[(2, 3), (5, 3)]),
    (&[(2, 3), (5, 3)], &[(2, 4), (5, 4)]),
    (&[(3, 2), (4, 2)], &[(3, 2), (4, 2)]),
    (&[(5, 1)], &[(5, 1)]),
    (&[(1, 2), (6, 2)], &[(1, 1), (6, 1)]),
];

/// `pixel-eyes-open` / `-closed`: a 5.2s cycle, the eyes shut from 96% to
/// 98.5% of it.
pub const BLINK: Duration = Duration::from_millis(5200);
const BLINK_EDGES: [f32; 2] = [0.96, 0.985];
/// `pixel-flicker 2.4s`: the bulb dims to 70% from 40% to 48%.
const FLICKER: Duration = Duration::from_millis(2400);
const FLICKER_EDGES: [f32; 2] = [0.40, 0.48];

/// MonoCode `BULB`, in units of its 16-unit grid, drawn at `translate(12.5
/// -2.5)` over a mascot that needs you.
const BULB: [(f32, f32, f32, f32, u32); 7] = [
    (1.0, 0.0, 2.0, 1.0, 0xFFF3B0),
    (0.0, 1.0, 1.0, 1.0, 0xFFF3B0),
    (1.0, 1.0, 2.0, 1.0, 0xFFD24A),
    (3.0, 1.0, 1.0, 1.0, 0xFFD24A),
    (0.0, 2.0, 4.0, 1.0, 0xFFD24A),
    (1.0, 3.0, 2.0, 1.0, 0xF29A1F),
    (1.0, 4.0, 2.0, 1.0, 0x8C8680),
];

/// MonoCode `PixelMascot`'s status and `still`: how a mascot moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MascotLook {
    pub motion: MascotMotion,
    /// Its eyes blink (`still` turns it off).
    pub blink: bool,
    /// It needs you: a light bulb over it.
    pub bulb: bool,
    /// MonoCode `blinkDelay`: mascots side by side do not blink in unison.
    pub offset_ms: u64,
}

/// MonoCode `blinkDelay(seed)`: a stable offset into the blink's cycle.
fn blink_offset(seed: &str) -> u64 {
    let hash = seed
        .encode_utf16()
        .fold(0u32, |hash, unit| hash.wrapping_mul(31).wrapping_add(u32::from(unit)));
    u64::from(hash % 5200)
}

impl MascotLook {
    /// `status="idle"`, not still: it bobs and blinks.
    pub fn idle(seed: &str) -> Self {
        Self {
            motion: MascotMotion::Bob,
            blink: true,
            bulb: false,
            offset_ms: blink_offset(seed),
        }
    }

    /// MonoCode `MonoTitle` / `MonoRailMascot` (`still={status ===
    /// "idle"}`): at rest it holds still; working it hops, talks and
    /// blinks; needing you it stands, blinks, and has an idea.
    pub fn for_status(status: crate::monos::MonoStatus, seed: &str) -> Self {
        use crate::monos::MonoStatus;
        let offset_ms = blink_offset(seed);
        match status {
            MonoStatus::Idle => Self {
                motion: MascotMotion::Still,
                blink: false,
                bulb: false,
                offset_ms,
            },
            MonoStatus::Working => Self {
                motion: MascotMotion::Hop,
                blink: true,
                bulb: false,
                offset_ms,
            },
            MonoStatus::NeedsYou => Self {
                motion: MascotMotion::Still,
                blink: true,
                bulb: true,
                offset_ms,
            },
        }
    }

    /// A greeting's mascot (`MonoHeader` with the Mono's status): it bobs
    /// at rest.
    pub fn greeting(status: crate::monos::MonoStatus, seed: &str) -> Self {
        match status {
            crate::monos::MonoStatus::Idle => Self::idle(seed),
            other => Self::for_status(other, seed),
        }
    }

    /// Whether it changes with no agent running: what the app's clock
    /// must wake for.
    pub fn moves_at_rest(self) -> bool {
        self.motion == MascotMotion::Bob || self.blink || self.bulb
    }

    /// How long until it next changes (`start_clock` wakes then).
    pub fn next_change(self) -> Duration {
        let offset = Duration::from_millis(self.offset_ms);
        let mut next = Duration::from_secs(60);
        if self.motion == MascotMotion::Bob {
            next = next.min(spinner::until_step(BOB / 2));
        }
        if self.blink {
            next = next.min(spinner::until_edge(BLINK, offset, &BLINK_EDGES));
        }
        if self.bulb {
            next = next.min(spinner::until_edge(FLICKER, Duration::ZERO, &FLICKER_EDGES));
        }
        next
    }
}

/// Whether the eyes are shut at `blink` (0..1 through the cycle).
pub fn eyes_shut(blink: f32) -> bool {
    (BLINK_EDGES[0]..BLINK_EDGES[1]).contains(&blink)
}

fn rgb(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

/// MonoCode `PixelMascot`: the sprite `size` wide in `look`, stepped by the
/// app's clock; held still (no bob, hop or blink) while the system asks for
/// less motion, the bulb shown steady.
pub fn pixel_mascot(
    mascot: &'static Mascot,
    size: f32,
    color: Hsla,
    look: MascotLook,
    cx: &App,
) -> AnyElement {
    let still = spinner::still(cx);
    let motion = if still { MascotMotion::Still } else { look.motion };
    let beat = match motion {
        MascotMotion::Hop => spinner::cycle(HOP),
        _ => spinner::cycle(BOB),
    };
    let (lifted, talking) = pose(motion, beat, spinner::cycle(TALK));
    let offset = Duration::from_millis(look.offset_ms);
    let shut = look.blink && !still && eyes_shut(spinner::cycle_offset(BLINK, offset));
    let index = MASCOTS
        .iter()
        .position(|m| std::ptr::eq(m, mascot))
        .unwrap_or(0);
    let cell = size / 8.0;
    let eyes = if talking { EYES[index].1 } else { EYES[index].0 };
    // The lid: the face's colour over the eye, the lash line left open.
    let lids = shut.then(|| {
        eyes.iter().flat_map(move |&(x, y)| {
            let (left, top) = (cell * x as f32, cell * y as f32);
            [(top, cell * 0.55), (top + cell * 0.8, cell * 0.2)].map(|(at, h)| {
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(at))
                    .w(px(cell))
                    .h(px(h))
                    .bg(color)
            })
        })
    });
    // The bulb on the 16-unit grid the sprite's 8 cells span 12 of, from 2.
    let unit = size / 12.0;
    let bulb = look.bulb.then(|| {
        let dim = !still && (FLICKER_EDGES[0]..FLICKER_EDGES[1]).contains(&spinner::cycle(FLICKER));
        div()
            .absolute()
            .left(px((12.5 - 2.0) * unit))
            .top(px((-2.5 - 2.0) * unit))
            .opacity(if dim { 0.7 } else { 1.0 })
            .children(BULB.iter().map(|&(x, y, w, h, fill)| {
                div()
                    .absolute()
                    .left(px(x * unit))
                    .top(px(y * unit))
                    .w(px(w * unit))
                    .h(px(h * unit))
                    .bg(rgb(fill))
            }))
    });
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .top(px(if lifted { -size * LIFT } else { 0.0 }))
        .child(pixel_sprite(
            if talking { &mascot.talk } else { &mascot.rest },
            px(size),
            color,
            false,
        ))
        .children(lids.into_iter().flatten())
        .children(bulb)
        .into_any_element()
}

/// MonoCode's 8×8 coin and star frames (`composerRunner.ts`).
pub const COIN_FACE: Rows = [
    "........", "..####..", ".######.", "########", "########", ".######.", "..####..", "........",
];
pub const COIN_EDGE: Rows = [
    "........", "...##...", "...##...", "...##...", "...##...", "...##...", "...##...", "........",
];
pub const STAR_FACE: Rows = [
    "...##...", "...##...", "..####..", "########", "########", "..####..", "...##...", "...##...",
];
pub const STAR_EDGE: Rows = [
    "........", "...##...", "...##...", "..####..", "..####..", "...##...", "...##...", "........",
];

#[cfg(test)]
mod tests {
    #[test]
    fn a_mascot_moves_by_its_status() {
        assert_eq!(pose(MascotMotion::Still, 0.9, 0.9), (false, false));
        assert_eq!(pose(MascotMotion::Bob, 0.2, 0.9), (false, false));
        assert_eq!(pose(MascotMotion::Bob, 0.7, 0.9), (true, false));
        assert_eq!(pose(MascotMotion::Hop, 0.7, 0.2), (true, false));
        assert_eq!(pose(MascotMotion::Hop, 0.2, 0.7), (false, true));
    }

    use super::*;

    #[test]
    fn eyes_are_holes_in_every_frame() {
        for (ix, mascot) in MASCOTS.iter().enumerate() {
            for (rows, eyes) in [(&mascot.rest, EYES[ix].0), (&mascot.talk, EYES[ix].1)] {
                for &(x, y) in eyes {
                    assert_eq!(rows[y].as_bytes()[x], b'.', "{} at {x},{y}", MASCOT_NAMES[ix]);
                }
            }
        }
    }

    #[test]
    fn a_mascot_blinks_briefly_and_at_its_own_time() {
        assert!(!eyes_shut(0.5));
        assert!(eyes_shut(0.97));
        assert!(!eyes_shut(0.99));
        assert_ne!(blink_offset("crab#1"), blink_offset("frog#1"));
        let idle = MascotLook::for_status(crate::monos::MonoStatus::Idle, "a");
        assert!(!idle.moves_at_rest());
        let waiting = MascotLook::for_status(crate::monos::MonoStatus::NeedsYou, "a");
        assert!(waiting.bulb && waiting.blink && waiting.motion == MascotMotion::Still);
        assert!(MascotLook::idle("a").moves_at_rest());
        assert!(waiting.next_change() <= BLINK);
    }

    #[test]
    fn rows_become_runs_and_mirror() {
        let rows: Rows = [
            "##..#...", "........", "........", "........", "........", "........", "........",
            "........",
        ];
        assert_eq!(runs(&rows, false), [(0, 0, 2), (0, 4, 1)]);
        assert_eq!(runs(&rows, true), [(0, 3, 1), (0, 6, 2)]);
    }

    #[test]
    fn every_frame_is_eight_by_eight() {
        for mascot in &MASCOTS {
            for row in mascot.rest.iter().chain(&mascot.talk) {
                assert_eq!(row.len(), 8);
            }
        }
    }

    #[test]
    fn the_pick_follows_monocode_hash() {
        // "a" = 97 → 97 % 10 = 7, the rocket.
        assert!(std::ptr::eq(project_mascot("a"), &MASCOTS[7]));
        // 131 * 97 + 98 = 12805 → 5, the crab.
        assert!(std::ptr::eq(project_mascot("ab"), &MASCOTS[5]));
    }
}
