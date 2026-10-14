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

/// A sprite `size` wide in `motion`, stepped by the app's clock
/// (`ui/spinner.rs`); still while the system asks for less motion.
pub fn moving_sprite(
    mascot: &'static Mascot,
    size: f32,
    color: Hsla,
    motion: MascotMotion,
    cx: &App,
) -> AnyElement {
    let motion = if spinner::still(cx) {
        MascotMotion::Still
    } else {
        motion
    };
    let beat = match motion {
        MascotMotion::Hop => spinner::cycle(HOP),
        _ => spinner::cycle(BOB),
    };
    let (lifted, talking) = pose(motion, beat, spinner::cycle(TALK));
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
