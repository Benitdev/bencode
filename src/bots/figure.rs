//! BenCode's own figures for bots, where MonoCode's Monos wear one of its
//! ten 8×8 project mascots: a body, eyes and gear on a 12×12 grid, shaded
//! in three tones of the bot's colour, cheeks glowing like the embers in
//! BenCode's icon, and a mood that moves it. Pure: [`compose`] gives the
//! cells of a frame; `ui/figure.rs` paints them and the menu bar rasterizes
//! them ([`rasterize`]).
//!
//! The roster keeps a figure as a code (`"flame.big.antenna"`, [`Figure::code`]);
//! a part this build does not know reads as that part's first, so a pick
//! made by a newer BenCode never breaks the roster.

use std::collections::BTreeMap;

use gpui::Hsla;

/// Cells on a side.
pub const GRID: i8 = 12;

macro_rules! parts {
    ($(#[$doc:meta])* $name:ident { $($variant:ident => $code:literal, $label:literal;)+ }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &[$name] = &[$($name::$variant),+];

            /// How the roster writes it.
            pub fn code(self) -> &'static str {
                match self {
                    $($name::$variant => $code),+
                }
            }

            /// How the pickers name it.
            pub fn label(self) -> &'static str {
                match self {
                    $($name::$variant => $label),+
                }
            }

            fn parse(code: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|part| part.code() == code)
            }
        }
    };
}

parts! {
    /// The silhouette.
    Body {
        Blob => "blob", "Blob";
        Bean => "bean", "Bean";
        Flame => "flame", "Flame";
        Block => "block", "Block";
        Bun => "bun", "Bun";
    }
}

parts! {
    /// How it looks out.
    Eyes {
        Dot => "dot", "Dots";
        Tall => "tall", "Tall";
        Big => "big", "Big";
        Visor => "visor", "Visor";
        Cyclops => "cyclops", "One eye";
    }
}

parts! {
    /// What it wears.
    Gear {
        None => "none", "Nothing";
        Antenna => "antenna", "Antenna";
        Sprout => "sprout", "Sprout";
        Hardhat => "hardhat", "Hard hat";
        Headphones => "headphones", "Headphones";
        Shades => "shades", "Shades";
        Scarf => "scarf", "Scarf";
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Figure {
    pub body: Body,
    pub eyes: Eyes,
    pub gear: Gear,
}

/// The looks new bots take, in turn, before any repeats.
pub const STARTERS: [Figure; 8] = [
    Figure::new(Body::Flame, Eyes::Big, Gear::None),
    Figure::new(Body::Blob, Eyes::Dot, Gear::Antenna),
    Figure::new(Body::Block, Eyes::Visor, Gear::Headphones),
    Figure::new(Body::Bun, Eyes::Tall, Gear::Scarf),
    Figure::new(Body::Bean, Eyes::Dot, Gear::Sprout),
    Figure::new(Body::Blob, Eyes::Cyclops, Gear::Hardhat),
    Figure::new(Body::Flame, Eyes::Dot, Gear::Shades),
    Figure::new(Body::Bean, Eyes::Big, Gear::Antenna),
];

impl Figure {
    pub const fn new(body: Body, eyes: Eyes, gear: Gear) -> Self {
        Self { body, eyes, gear }
    }

    /// `body.eyes.gear`, as the roster keeps it.
    pub fn code(self) -> String {
        format!(
            "{}.{}.{}",
            self.body.code(),
            self.eyes.code(),
            self.gear.code()
        )
    }

    /// A code read back; a part it does not know is that part's first.
    pub fn parse(code: &str) -> Option<Self> {
        let mut parts = code.split('.');
        let body = parts.next().filter(|part| !part.is_empty())?;
        Some(Self {
            body: Body::parse(body).unwrap_or(Body::ALL[0]),
            eyes: parts.next().and_then(Eyes::parse).unwrap_or(Eyes::ALL[0]),
            gear: parts.next().and_then(Gear::parse).unwrap_or(Gear::ALL[0]),
        })
    }

    /// The look a Mono made before bots had figures takes, from the
    /// MonoCode mascot it wore; any other picks one from `seed`.
    pub fn from_mascot(mascot: &str, seed: &str) -> Self {
        use {Body as B, Eyes as E, Gear as G};
        let (body, eyes, gear) = match mascot {
            "invader" => (B::Bun, E::Dot, G::Antenna),
            "ghost" => (B::Flame, E::Big, G::None),
            "robot" => (B::Block, E::Visor, G::Antenna),
            "cat" => (B::Bun, E::Tall, G::None),
            "skull" => (B::Block, E::Cyclops, G::None),
            "crab" => (B::Blob, E::Dot, G::Headphones),
            "mushroom" => (B::Blob, E::Dot, G::Hardhat),
            "rocket" => (B::Flame, E::Dot, G::Antenna),
            "dino" => (B::Bean, E::Tall, G::Scarf),
            "frog" => (B::Blob, E::Big, G::None),
            _ => return Self::seeded(seed),
        };
        Self::new(body, eyes, gear)
    }

    /// A look picked from `seed`, stable for it.
    pub fn seeded(seed: &str) -> Self {
        let hash = seed.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        }) as usize;
        Self {
            body: Body::ALL[hash % Body::ALL.len()],
            eyes: Eyes::ALL[(hash / 7) % Eyes::ALL.len()],
            gear: Gear::ALL[(hash / 61) % Gear::ALL.len()],
        }
    }

    /// The next starter none of `taken` wears, else one from `seed`.
    pub fn next(taken: &[Figure], seed: &str) -> Self {
        STARTERS
            .iter()
            .copied()
            .find(|figure| !taken.contains(figure))
            .unwrap_or_else(|| Self::seeded(seed))
    }
}

/// What the figure is up to; each has its own motion and marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    /// Nothing to do: it stands, and breathes where it is shown off.
    Idle,
    /// A turn runs: it hops and talks, eyes on the work, cheeks aglow.
    Working,
    /// Waiting on the user: its cheeks flare and a "!" goes up.
    NeedsYou,
    /// A turn just finished: eyes closed in a smile and a few sparks, for
    /// a moment.
    Done,
    /// The last turn failed: it squints under a puff of smoke.
    Failed,
    /// Idle with a habit coming up: asleep until then, its cheeks low.
    Asleep,
}

/// One frame of a figure: where it is in its motion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub mood: Option<Mood>,
    /// Up on its hop.
    pub up: bool,
    /// Its mouth open.
    pub talk: bool,
    /// Eyes shut for a blink.
    pub blink: bool,
    /// The flame's tip on its other side.
    pub flicker: bool,
    /// Where its eyes look: -1 left, 0 ahead, 1 right.
    pub look: i8,
    /// The glow in its cheeks, 0 (low) to 3 (flaring).
    pub glow: u8,
    /// The second place of its sparks, smoke and z.
    pub alt: bool,
    /// The marks around it (the "!", sparks, smoke and z), left out where
    /// the figure is too small to carry them.
    pub marks: bool,
}

/// How far each of a figure's cycles has come, 0..1: what [`Frame::at`]
/// steps it by. The views read them from the app's clock.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Phases {
    /// The hop while it works, and the flame's flicker.
    pub beat: f32,
    /// The mouth while it talks.
    pub talk: f32,
    /// The blink's cycle, offset per figure.
    pub blink: f32,
    /// A slow breath for the cheeks, and the marks' two places.
    pub breath: f32,
    /// The eyes' glance while it works.
    pub glance: f32,
}

/// The blink: eyes shut from 96% to 98.5% of its cycle (MonoCode's
/// `pixel-eyes-closed`).
pub const BLINK_EDGES: [f32; 2] = [0.96, 0.985];

impl Frame {
    /// A figure standing still in `mood`: no hop, blink or breath.
    pub fn still(mood: Mood, marks: bool) -> Self {
        Self {
            mood: Some(mood),
            glow: rest_glow(mood),
            marks,
            ..Self::default()
        }
    }

    /// The frame for `mood` at `phases`. `lively` figures (the greeting,
    /// the settings face) breathe and blink at rest; the others hold still
    /// while idle, as MonoCode's do.
    pub fn at(mood: Mood, lively: bool, phases: Phases, marks: bool) -> Self {
        let blink = (BLINK_EDGES[0]..BLINK_EDGES[1]).contains(&phases.blink);
        let breath_up = phases.breath >= 0.5;
        let still = Self::still(mood, marks);
        match mood {
            Mood::Working => Self {
                up: phases.beat >= 0.5,
                talk: phases.talk >= 0.5,
                blink,
                flicker: phases.beat >= 0.5,
                look: [0, 1, 0, -1][(phases.glance * 4.0) as usize % 4],
                glow: if phases.talk >= 0.5 { 3 } else { 2 },
                ..still
            },
            Mood::NeedsYou => Self {
                blink,
                flicker: breath_up,
                glow: if breath_up { 3 } else { 2 },
                alt: breath_up,
                ..still
            },
            Mood::Done => Self {
                up: lively && breath_up,
                blink: false,
                alt: phases.beat >= 0.5,
                ..still
            },
            Mood::Idle if lively => Self {
                up: breath_up,
                blink,
                flicker: breath_up,
                glow: if breath_up { 2 } else { 1 },
                ..still
            },
            Mood::Asleep if lively => Self {
                alt: breath_up,
                glow: if breath_up { 1 } else { 0 },
                ..still
            },
            Mood::Idle | Mood::Asleep | Mood::Failed => still,
        }
    }
}

/// The cheeks' glow at rest in `mood`.
fn rest_glow(mood: Mood) -> u8 {
    match mood {
        Mood::Failed | Mood::Asleep => 0,
        Mood::Idle => 1,
        Mood::Working => 2,
        Mood::NeedsYou | Mood::Done => 3,
    }
}

/// How a bot's last turn ended, for as long as its figure shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Done,
    Failed,
}

/// How long a finished turn's sparks last.
pub const DONE_FOR: std::time::Duration = std::time::Duration::from_secs(5);

/// The mood of an idle bot, and when it changes by itself: a failed last
/// turn shows until the next one, a finished one for [`DONE_FOR`] after it
/// ended (`ago`); otherwise it sleeps while one of its habits is coming up.
pub fn idle_mood(
    ended: Option<(Ended, std::time::Duration)>,
    habit_ahead: bool,
) -> (Mood, Option<(std::time::Duration, Mood)>) {
    let rest = if habit_ahead {
        Mood::Asleep
    } else {
        Mood::Idle
    };
    match ended {
        Some((Ended::Failed, _)) => (Mood::Failed, None),
        Some((Ended::Done, ago)) if ago < DONE_FOR => (Mood::Done, Some((DONE_FOR - ago, rest))),
        _ => (rest, None),
    }
}

/// What colours a cell. The body's three come from the bot's colour; the
/// rest are the figure's own ([`tone_color`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tone {
    Shade,
    Body,
    Light,
    Eye,
    Glint,
    Glow(u8),
    Graphite,
    Steel,
    Hat,
    HatShade,
    Leaf,
    LeafDark,
    Scarf,
    ScarfShade,
    Alert,
    Spark,
    Smoke,
    Haze,
    Sleep,
}

/// A painted cell; `x` and `y` may fall outside the grid (the marks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub x: i8,
    pub y: i8,
    pub tone: Tone,
}

/// A body's silhouette and where its face and gear sit.
struct Art {
    rows: [&'static str; 12],
    /// The rows that change while it flickers (the flame's tip), from the top.
    flicker: &'static [&'static str],
    /// Left and right eye columns, and their row. The cheeks glow on the
    /// row below, outside them, and the mouth sits between.
    eyes: (i8, i8, i8),
    /// The column an antenna or a sprout grows from.
    crown: i8,
    /// The row a hat's brim sits on: the hat covers the body above it.
    brim: i8,
    /// The row a scarf wraps.
    neck: i8,
}

static BLOB: Art = Art {
    rows: [
        "............",
        "............",
        "............",
        "....####....",
        "..########..",
        ".##########.",
        ".##########.",
        "############",
        "############",
        "############",
        ".##########.",
        "..########..",
    ],
    flicker: &[],
    eyes: (4, 7, 7),
    crown: 6,
    brim: 5,
    neck: 10,
};

static BEAN: Art = Art {
    rows: [
        "............",
        "............",
        "....####....",
        "...######...",
        "..########..",
        "..########..",
        "..########..",
        "..########..",
        "..########..",
        "...######...",
        "...##..##...",
        "...##..##...",
    ],
    flicker: &[],
    eyes: (4, 7, 5),
    crown: 6,
    brim: 4,
    neck: 8,
};

static FLAME: Art = Art {
    rows: [
        "............",
        "......#.....",
        ".....##.....",
        ".....###....",
        "....####....",
        "...######...",
        "..########..",
        "..########..",
        ".##########.",
        ".##########.",
        "..########..",
        "...######...",
    ],
    flicker: &[
        "............",
        ".....#......",
        ".....##.....",
        "....###.....",
    ],
    eyes: (4, 7, 7),
    crown: 6,
    brim: 5,
    neck: 10,
};

static BLOCK: Art = Art {
    rows: [
        "............",
        "............",
        "..########..",
        ".##########.",
        ".##########.",
        ".##########.",
        ".##########.",
        ".##########.",
        ".##########.",
        ".##########.",
        "..########..",
        "..##....##..",
    ],
    flicker: &[],
    eyes: (3, 8, 6),
    crown: 6,
    brim: 3,
    neck: 9,
};

static BUN: Art = Art {
    rows: [
        "............",
        "............",
        "..#......#..",
        ".###....###.",
        ".##########.",
        ".##########.",
        "############",
        "############",
        "############",
        "############",
        ".##########.",
        "..########..",
    ],
    flicker: &[],
    eyes: (4, 7, 7),
    crown: 6,
    brim: 4,
    neck: 10,
};

impl Body {
    fn art(self) -> &'static Art {
        match self {
            Body::Blob => &BLOB,
            Body::Bean => &BEAN,
            Body::Flame => &FLAME,
            Body::Block => &BLOCK,
            Body::Bun => &BUN,
        }
    }
}

/// The cells being painted, later layers over earlier ones.
#[derive(Default)]
struct Layers(BTreeMap<(i8, i8), Tone>);

impl Layers {
    fn put(&mut self, x: i8, y: i8, tone: Tone) {
        self.0.insert((y, x), tone);
    }

    fn get(&self, x: i8, y: i8) -> Option<Tone> {
        self.0.get(&(y, x)).copied()
    }

    /// Paints over the body only.
    fn over_body(&mut self, x: i8, y: i8, tone: Tone) {
        if self.get(x, y).is_some() {
            self.put(x, y, tone);
        }
    }
}

/// A body's filled cells, the flicker's rows in place of its own.
fn silhouette(art: &Art, flicker: bool) -> [[bool; 12]; 12] {
    let mut mask = [[false; 12]; 12];
    for (y, row) in art.rows.iter().enumerate() {
        let row = if flicker {
            art.flicker.get(y).copied().unwrap_or(row)
        } else {
            row
        };
        for (x, cell) in row.bytes().enumerate().take(12) {
            mask[y][x] = cell == b'#';
        }
    }
    mask
}

/// The cells of `figure` in `frame`, a row at a time.
pub fn compose(figure: Figure, frame: Frame) -> Vec<Cell> {
    let art = figure.body.art();
    let mood = frame.mood.unwrap_or(Mood::Idle);
    let mask = silhouette(art, frame.flicker && !art.flicker.is_empty());
    let filled = |x: i8, y: i8| {
        (0..GRID).contains(&x) && (0..GRID).contains(&y) && mask[y as usize][x as usize]
    };
    let mut layers = Layers::default();
    let (left, right, eye_row) = art.eyes;

    // The body, lit from the top left: its lower and right rims in shade,
    // the top of its left half in light.
    for y in 0..GRID {
        for x in 0..GRID {
            if !filled(x, y) {
                continue;
            }
            let tone = if !filled(x, y + 1) || (!filled(x + 1, y) && y > eye_row) {
                Tone::Shade
            } else if (!filled(x, y - 1) && x < GRID / 2) || (!filled(x - 1, y) && y < eye_row) {
                Tone::Light
            } else {
                Tone::Body
            };
            layers.put(x, y, tone);
        }
    }
    let top = |x: i8| (0..GRID).find(|&y| filled(x, y));

    // Gear worn under the face.
    match figure.gear {
        Gear::Hardhat => {
            let dome: Vec<(i8, i8)> = (0..art.brim)
                .flat_map(|y| (0..GRID).map(move |x| (x, y)))
                .filter(|&(x, y)| filled(x, y))
                .collect();
            for &(x, y) in &dome {
                let lit = dome.iter().all(|&(ox, oy)| oy > y || (oy == y && ox >= x));
                layers.put(x, y, if lit { Tone::Light } else { Tone::Hat });
            }
            // The dome's top-left cell catches the light.
            if let Some(&(x, y)) = dome.first() {
                layers.put(x, y, Tone::Hat);
            }
            let brim: Vec<i8> = (0..GRID).filter(|&x| filled(x, art.brim)).collect();
            if let (Some(&first), Some(&last)) = (brim.first(), brim.last()) {
                for x in first - 1..=last + 1 {
                    layers.put(x, art.brim, Tone::HatShade);
                }
            }
            for (x, y) in dome {
                if layers.get(x, y) == Some(Tone::Light) {
                    layers.put(x, y, Tone::Hat);
                }
            }
        }
        Gear::Scarf => {
            let wrap: Vec<i8> = (0..GRID).filter(|&x| filled(x, art.neck)).collect();
            for &x in &wrap {
                layers.put(x, art.neck, Tone::Scarf);
            }
            if let Some(&last) = wrap.last() {
                // Its end hangs down the front.
                layers.over_body(last - 2, art.neck + 1, Tone::ScarfShade);
            }
        }
        _ => {}
    }

    // The eyes.
    let look = if figure.eyes == Eyes::Visor {
        0
    } else {
        frame.look
    };
    let (l, r) = (left + look, right + look);
    let eye = Tone::Eye;
    match mood {
        Mood::Done => match figure.eyes {
            Eyes::Visor => {
                for x in l - 1..=r + 1 {
                    layers.put(x, eye_row, eye);
                }
                for x in [l, r] {
                    layers.put(x, eye_row - 1, eye);
                    layers.put(x, eye_row, Tone::Body);
                }
            }
            Eyes::Cyclops => {
                let c = (l + r) / 2;
                for (x, y) in [(c - 1, 0), (c, -1), (c + 1, -1), (c + 2, 0)] {
                    layers.put(x, eye_row + y, eye);
                }
            }
            // Eyes closed in a smile, each a slant up from its outer edge
            // (two whole carets would run together).
            _ => {
                for (x, out) in [(l, l - 1), (r, r + 1)] {
                    layers.put(out, eye_row, eye);
                    layers.put(x, eye_row - 1, eye);
                }
            }
        },
        Mood::Asleep => match figure.eyes {
            Eyes::Visor => {
                for x in l - 1..=r + 1 {
                    layers.put(x, eye_row, Tone::Shade);
                }
            }
            _ => {
                for x in [l - 1, l, r, r + 1] {
                    layers.put(x, eye_row, eye);
                }
            }
        },
        _ => {
            // Failed squints: its lids half down.
            let shut = frame.blink;
            let squint = mood == Mood::Failed;
            match figure.eyes {
                Eyes::Dot => {
                    for x in [l, r] {
                        layers.put(x, eye_row, if shut { Tone::Shade } else { eye });
                        if squint {
                            layers.put(x, eye_row - 1, Tone::Shade);
                        }
                    }
                }
                Eyes::Tall => {
                    for x in [l, r] {
                        layers.put(x, eye_row, eye);
                        if !shut && !squint {
                            layers.put(x, eye_row - 1, eye);
                        }
                    }
                }
                Eyes::Big => {
                    for (x0, glint) in [(l - 1, l - 1), (r, r)] {
                        for x in x0..=x0 + 1 {
                            layers.put(x, eye_row, eye);
                            if !shut {
                                let lid = if squint { Tone::Shade } else { eye };
                                layers.put(x, eye_row - 1, lid);
                            }
                        }
                        if !shut && !squint {
                            layers.put(glint, eye_row - 1, Tone::Glint);
                        }
                    }
                }
                Eyes::Visor => {
                    let tone = if shut || squint { Tone::Shade } else { eye };
                    for x in l - 1..=r + 1 {
                        layers.put(x, eye_row, tone);
                    }
                    // A light runs along it while it works.
                    if mood == Mood::Working && !shut {
                        let span = (r + 1) - (l - 1) + 1;
                        let at = l - 1 + ((frame.look + 1) * (span - 1) / 2);
                        layers.put(at, eye_row, Tone::Glint);
                    }
                }
                Eyes::Cyclops => {
                    let c = (l + r) / 2;
                    for x in c..=c + 1 {
                        layers.put(x, eye_row, eye);
                        if !shut {
                            let lid = if squint { Tone::Shade } else { eye };
                            layers.put(x, eye_row - 1, lid);
                        }
                    }
                    if !shut && !squint {
                        layers.put(c, eye_row - 1, Tone::Glint);
                    }
                }
            }
        }
    }

    // The cheeks, where its glow shows, and the mouth between them: open
    // while it talks, a smile once done.
    let cheeks = eye_row + 1;
    let glow = Tone::Glow(frame.glow.min(3));
    layers.over_body(left - 1, cheeks, glow);
    layers.over_body(right + 1, cheeks, glow);
    if mood == Mood::Done || frame.talk {
        layers.over_body(5, cheeks, Tone::Eye);
        layers.over_body(6, cheeks, Tone::Eye);
    }

    // Gear worn over the face.
    let crown = art.crown;
    let crown_top = top(crown).unwrap_or(0);
    match figure.gear {
        Gear::Antenna => {
            layers.put(crown, crown_top - 1, Tone::Steel);
            layers.put(crown, crown_top - 2, Tone::Steel);
            layers.put(crown, crown_top - 3, Tone::Glow(frame.glow.max(1)));
        }
        Gear::Sprout => {
            layers.put(crown, crown_top - 1, Tone::LeafDark);
            layers.put(crown - 1, crown_top - 2, Tone::Leaf);
            layers.put(crown - 2, crown_top - 2, Tone::LeafDark);
            layers.put(crown + 1, crown_top - 2, Tone::Leaf);
            layers.put(crown + 1, crown_top - 3, Tone::Leaf);
            layers.put(crown + 2, crown_top - 3, Tone::LeafDark);
        }
        Gear::Headphones => {
            let row: Vec<i8> = (0..GRID).filter(|&x| filled(x, eye_row)).collect();
            if let (Some(&first), Some(&last)) = (row.first(), row.last()) {
                // The band over the head, following it, then the cups.
                for x in first..=last {
                    if let Some(y) = top(x) {
                        layers.put(x, y - 1, Tone::Steel);
                    }
                }
                for x0 in [first - 1, last] {
                    for x in x0..=x0 + 1 {
                        for y in eye_row - 1..=eye_row + 1 {
                            layers.put(x, y, Tone::Graphite);
                        }
                    }
                }
            }
        }
        Gear::Shades => {
            let lenses: &[i8] = match figure.eyes {
                Eyes::Visor => &[],
                Eyes::Cyclops => &[(left + right) / 2],
                Eyes::Dot | Eyes::Tall | Eyes::Big => &[left - 1, right],
            };
            if lenses.is_empty() {
                for x in left - 1..=right + 1 {
                    layers.put(x, eye_row - 1, Tone::Graphite);
                    layers.put(x, eye_row, Tone::Graphite);
                }
                layers.put(left - 1, eye_row - 1, Tone::Steel);
            }
            for &x0 in lenses {
                for x in x0..=x0 + 1 {
                    layers.put(x, eye_row - 1, Tone::Graphite);
                    layers.put(x, eye_row, Tone::Graphite);
                }
                layers.put(x0, eye_row - 1, Tone::Steel);
            }
            if lenses.len() == 2 {
                for x in left + 1..right {
                    layers.put(x, eye_row - 1, Tone::Graphite);
                }
            }
        }
        _ => {}
    }

    // The marks around it.
    if frame.marks {
        let alt = i8::from(frame.alt);
        match mood {
            Mood::NeedsYou => {
                for (x, y) in [
                    (10, -4),
                    (11, -4),
                    (10, -3),
                    (11, -3),
                    (10, -2),
                    (11, -2),
                    (10, 0),
                    (11, 0),
                ] {
                    layers.put(x, y - alt, Tone::Alert);
                }
            }
            Mood::Done => {
                let sparks: [(i8, i8); 3] = if frame.alt {
                    [(0, 1), (11, 3), (9, -1)]
                } else {
                    [(1, -1), (10, 0), (12, 5)]
                };
                for (x, y) in sparks {
                    layers.put(x, y, Tone::Spark);
                }
            }
            Mood::Failed => {
                // A puff of smoke off the top of its head.
                let (x, y) = (crown + 1, crown_top - 2);
                for (dx, dy, tone) in [
                    (1, 0, Tone::Smoke),
                    (0, -1, Tone::Smoke),
                    (1, -1, Tone::Smoke),
                    (2, -1, Tone::Smoke),
                    (1, -2, Tone::Haze),
                    (2, -2, Tone::Haze),
                ] {
                    layers.put(x + dx, y + dy, tone);
                }
            }
            Mood::Asleep => {
                let (x, y) = (8, -4 - alt);
                for (dx, dy) in [
                    (0, 0),
                    (1, 0),
                    (2, 0),
                    (3, 0),
                    (2, 1),
                    (1, 2),
                    (0, 3),
                    (1, 3),
                    (2, 3),
                    (3, 3),
                ] {
                    layers.put(x + dx, y + dy, Tone::Sleep);
                }
            }
            Mood::Idle | Mood::Working => {}
        }
    }

    let lift = -i8::from(frame.up);
    layers
        .0
        .into_iter()
        .map(|((y, x), tone)| Cell {
            x,
            y: y + lift,
            tone,
        })
        .collect()
}

/// Neighbouring cells of a row in one tone, as (row, first column, length,
/// tone): what a painter draws as one rectangle.
pub fn runs(cells: &[Cell]) -> Vec<(i8, i8, i8, Tone)> {
    let mut out: Vec<(i8, i8, i8, Tone)> = Vec::new();
    for cell in cells {
        match out.last_mut() {
            Some((y, x, len, tone))
                if *y == cell.y && *tone == cell.tone && *x + *len == cell.x =>
            {
                *len += 1
            }
            _ => out.push((cell.y, cell.x, 1, cell.tone)),
        }
    }
    out
}

fn hex(rgb: u32) -> Hsla {
    gpui::rgb(rgb).into()
}

/// The colour of `tone` on a figure whose body is `body`.
pub fn tone_color(tone: Tone, body: Hsla) -> Hsla {
    let tint = |s: f32, l: f32| Hsla {
        s: s.clamp(0.0, 1.0),
        l: l.clamp(0.0, 1.0),
        ..body
    };
    match tone {
        Tone::Body => body,
        Tone::Shade => tint(body.s * 1.05, body.l * 0.7),
        Tone::Light => tint(body.s * 0.9, body.l + (1.0 - body.l) * 0.4),
        Tone::Eye => tint(body.s.min(0.4), 0.13),
        Tone::Glint => hex(0xFFFFFF),
        Tone::Glow(0) => hex(0xA0705C),
        Tone::Glow(1) => hex(0xFF8F6B),
        Tone::Glow(2) => hex(0xFFB054),
        Tone::Glow(_) => hex(0xFFE39A),
        Tone::Graphite => hex(0x343A46),
        Tone::Steel => hex(0x6B7385),
        Tone::Hat => hex(0xF5C342),
        Tone::HatShade => hex(0xC48A17),
        Tone::Leaf => hex(0x6CCB5F),
        Tone::LeafDark => hex(0x2F8A3E),
        Tone::Scarf => hex(0xE5484D),
        Tone::ScarfShade => hex(0xA8252B),
        Tone::Alert => hex(0xFF7A1A),
        Tone::Spark => hex(0xFFD84D),
        Tone::Smoke => hex(0x8E8882),
        Tone::Haze => hex(0xC4BEB8),
        Tone::Sleep => hex(0x8FA8D6),
    }
}

/// `cells` painted into a `side`-pixel square of RGBA, `cell` pixels a
/// cell, the grid's corner at `origin` (pixels; marks outside the square
/// are cut).
#[cfg(test)]
pub fn rasterize(
    cells: &[Cell],
    body: Hsla,
    cell: usize,
    side: usize,
    origin: (i32, i32),
) -> Vec<u8> {
    let mut rgba = vec![0u8; side * side * 4];
    paint_cells(&mut rgba, side, cells, body, cell, origin);
    rgba
}

/// `cells` painted over what a `side`-pixel square of RGBA holds, `cell`
/// pixels a cell, the grid's corner at `origin` (pixels; marks outside the
/// square are cut): the menu bar's portraits.
pub fn paint_cells(
    rgba: &mut [u8],
    side: usize,
    cells: &[Cell],
    body: Hsla,
    cell: usize,
    origin: (i32, i32),
) {
    let cell = cell as i32;
    for item in cells {
        let color = gpui::Rgba::from(tone_color(item.tone, body));
        let fill = [color.r, color.g, color.b, color.a].map(|c| (c * 255.0).round() as u8);
        let (x0, y0) = (
            origin.0 + i32::from(item.x) * cell,
            origin.1 + i32::from(item.y) * cell,
        );
        for y in y0.max(0)..(y0 + cell).min(side as i32) {
            for x in x0.max(0)..(x0 + cell).min(side as i32) {
                let at = (y as usize * side + x as usize) * 4;
                rgba[at..at + 4].copy_from_slice(&fill);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_figure() -> impl Iterator<Item = Figure> {
        Body::ALL.iter().flat_map(|&body| {
            Eyes::ALL.iter().flat_map(move |&eyes| {
                Gear::ALL
                    .iter()
                    .map(move |&gear| Figure::new(body, eyes, gear))
            })
        })
    }

    const MOODS: [Mood; 6] = [
        Mood::Idle,
        Mood::Working,
        Mood::NeedsYou,
        Mood::Done,
        Mood::Failed,
        Mood::Asleep,
    ];

    #[test]
    fn a_code_reads_back_and_unknown_parts_fall_back() {
        for figure in every_figure() {
            assert_eq!(Figure::parse(&figure.code()), Some(figure));
        }
        assert_eq!(
            Figure::parse("bean.laser.cape"),
            Some(Figure::new(Body::Bean, Eyes::Dot, Gear::None))
        );
        assert_eq!(
            Figure::parse("comet"),
            Some(Figure::new(Body::Blob, Eyes::Dot, Gear::None))
        );
        assert_eq!(Figure::parse(""), None);
    }

    #[test]
    fn every_body_is_twelve_square_with_its_face_on_it() {
        for &body in Body::ALL {
            let art = body.art();
            for row in art.rows.iter().chain(art.flicker) {
                assert_eq!(row.len(), 12, "{body:?}");
            }
            let mask = silhouette(art, false);
            let on = |x: i8, y: i8| mask[y as usize][x as usize];
            let (left, right, row) = art.eyes;
            for x in [left - 1, left, right, right + 1] {
                assert!(on(x, row) && on(x, row - 1), "{body:?} eyes at {x}");
            }
            // Cheeks, mouth and smile.
            for (x, y) in [(left - 1, row + 1), (right + 1, row + 1), (5, row + 1)] {
                assert!(on(x, y), "{body:?} cheeks and mouth at {x},{y}");
            }
            assert!(art.neck > row + 1, "{body:?} scarf under the mouth");
            assert!(
                on(art.crown, art.brim) && on(art.crown, art.neck),
                "{body:?}"
            );
        }
    }

    #[test]
    fn every_figure_stays_near_its_box_in_every_mood() {
        for figure in every_figure() {
            for mood in MOODS {
                for frame in [
                    Frame::still(mood, true),
                    Frame::at(
                        mood,
                        true,
                        Phases {
                            beat: 0.7,
                            talk: 0.7,
                            breath: 0.7,
                            glance: 0.3,
                            ..Phases::default()
                        },
                        true,
                    ),
                ] {
                    for cell in compose(figure, frame) {
                        assert!(
                            (-6..GRID + 2).contains(&cell.y) && (-2..GRID + 2).contains(&cell.x),
                            "{figure:?} {mood:?} {cell:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn small_figures_carry_no_marks() {
        let figure = Figure::new(Body::Blob, Eyes::Dot, Gear::None);
        for mood in MOODS {
            assert!(
                compose(figure, Frame::still(mood, false))
                    .iter()
                    .all(|cell| (0..GRID).contains(&cell.x) && (0..GRID).contains(&cell.y)),
                "{mood:?}"
            );
        }
    }

    #[test]
    fn a_working_figure_hops_and_talks_on_its_beat() {
        let rest = Phases::default();
        let up = Phases {
            beat: 0.6,
            talk: 0.6,
            ..rest
        };
        let down = Frame::at(Mood::Working, false, rest, true);
        let hop = Frame::at(Mood::Working, false, up, true);
        assert!(!down.up && hop.up && hop.talk && hop.glow == 3);
        let idle = Frame::at(Mood::Idle, false, up, true);
        assert_eq!(idle, Frame::still(Mood::Idle, true));
        let lively = Frame::at(
            Mood::Idle,
            true,
            Phases {
                breath: 0.6,
                ..rest
            },
            true,
        );
        assert!(lively.up && lively.glow == 2);
        let figure = Figure::new(Body::Bean, Eyes::Dot, Gear::None);
        let low = compose(figure, down).iter().map(|c| c.y).min();
        let high = compose(figure, hop).iter().map(|c| c.y).min();
        assert_eq!(high, low.map(|y| y - 1));
    }

    #[test]
    fn an_idle_bot_shows_how_its_turn_ended_then_rests() {
        use std::time::Duration;
        let second = Duration::from_secs(1);
        assert_eq!(idle_mood(None, false), (Mood::Idle, None));
        assert_eq!(idle_mood(None, true), (Mood::Asleep, None));
        assert_eq!(
            idle_mood(Some((Ended::Done, second)), true),
            (Mood::Done, Some((DONE_FOR - second, Mood::Asleep)))
        );
        assert_eq!(
            idle_mood(Some((Ended::Done, DONE_FOR)), false),
            (Mood::Idle, None)
        );
        assert_eq!(
            idle_mood(Some((Ended::Failed, Duration::from_secs(600))), true),
            (Mood::Failed, None)
        );
    }

    #[test]
    fn runs_join_a_row_of_one_tone() {
        let cell = |x, y, tone| Cell { x, y, tone };
        let cells = [
            cell(0, 0, Tone::Body),
            cell(1, 0, Tone::Body),
            cell(2, 0, Tone::Eye),
            cell(4, 0, Tone::Eye),
            cell(0, 1, Tone::Body),
        ];
        assert_eq!(
            runs(&cells),
            [
                (0, 0, 2, Tone::Body),
                (0, 2, 1, Tone::Eye),
                (0, 4, 1, Tone::Eye),
                (1, 0, 1, Tone::Body),
            ]
        );
    }

    #[test]
    fn new_bots_take_the_starters_in_turn() {
        assert_eq!(Figure::next(&[], "a"), STARTERS[0]);
        assert_eq!(Figure::next(&STARTERS[..2], "a"), STARTERS[2]);
        assert_eq!(Figure::next(&STARTERS, "a"), Figure::seeded("a"));
        assert_eq!(
            Figure::from_mascot("robot", "x"),
            Figure::new(Body::Block, Eyes::Visor, Gear::Antenna)
        );
        assert_eq!(Figure::from_mascot("", "x"), Figure::seeded("x"));
    }

    #[test]
    fn the_tones_follow_the_body() {
        let body = gpui::hsla(0.6, 0.7, 0.5, 1.0);
        assert!(tone_color(Tone::Shade, body).l < body.l);
        assert!(tone_color(Tone::Light, body).l > body.l);
        assert_eq!(tone_color(Tone::Body, body), body);
        let image = rasterize(
            &[Cell {
                x: 1,
                y: 0,
                tone: Tone::Glint,
            }],
            body,
            2,
            4,
            (0, 0),
        );
        assert_eq!(&image[8..12], &[255, 255, 255, 255]);
        assert_eq!(&image[0..4], &[0, 0, 0, 0]);
    }

    /// Writes every body, eyes, gear and mood to `target/figures.png` to
    /// look at: `cargo test figure_sheet -- --ignored`.
    #[test]
    #[ignore]
    fn figure_sheet() {
        const CELL: usize = 8;
        const PITCH: usize = 18 * CELL;
        let colors = [
            gpui::hsla(25.0 / 360.0, 0.85, 0.58, 1.0),
            gpui::hsla(210.0 / 360.0, 0.7, 0.6, 1.0),
            gpui::hsla(140.0 / 360.0, 0.5, 0.5, 1.0),
            gpui::hsla(330.0 / 360.0, 0.65, 0.65, 1.0),
            gpui::hsla(260.0 / 360.0, 0.6, 0.66, 1.0),
        ];
        let mut sheet: Vec<Vec<(Figure, Frame, Hsla)>> = Vec::new();
        for (ix, &body) in Body::ALL.iter().enumerate() {
            sheet.push(
                Eyes::ALL
                    .iter()
                    .map(|&eyes| {
                        (
                            Figure::new(body, eyes, Gear::None),
                            Frame::still(Mood::Idle, true),
                            colors[ix],
                        )
                    })
                    .collect(),
            );
        }
        for (ix, &body) in Body::ALL.iter().enumerate() {
            sheet.push(
                Gear::ALL
                    .iter()
                    .map(|&gear| {
                        (
                            Figure::new(body, Eyes::ALL[ix], gear),
                            Frame::still(Mood::Idle, true),
                            colors[(ix + 1) % colors.len()],
                        )
                    })
                    .collect(),
            );
        }
        for (ix, &body) in Body::ALL.iter().enumerate() {
            let figure = STARTERS
                .iter()
                .copied()
                .find(|f| f.body == body)
                .unwrap_or(Figure::new(body, Eyes::Dot, Gear::None));
            sheet.push(
                MOODS
                    .iter()
                    .map(|&mood| (figure, Frame::still(mood, true), colors[ix]))
                    .chain([(
                        figure,
                        Frame::at(
                            Mood::Working,
                            true,
                            Phases {
                                beat: 0.7,
                                talk: 0.7,
                                glance: 0.3,
                                ..Phases::default()
                            },
                            true,
                        ),
                        colors[ix],
                    )])
                    .collect(),
            );
        }
        let width = sheet.iter().map(Vec::len).max().unwrap_or(1) * PITCH;
        let height = sheet.len() * PITCH;
        let mut image = image::RgbaImage::from_pixel(
            width as u32,
            height as u32,
            image::Rgba([30, 30, 34, 255]),
        );
        for (row, items) in sheet.iter().enumerate() {
            for (col, (figure, frame, color)) in items.iter().enumerate() {
                let tile = rasterize(
                    &compose(*figure, *frame),
                    *color,
                    CELL,
                    PITCH,
                    (3 * CELL as i32, 5 * CELL as i32),
                );
                for y in 0..PITCH {
                    for x in 0..PITCH {
                        let px = &tile[(y * PITCH + x) * 4..][..4];
                        if px[3] > 0 {
                            image.put_pixel(
                                (col * PITCH + x) as u32,
                                (row * PITCH + y) as u32,
                                image::Rgba([px[0], px[1], px[2], px[3]]),
                            );
                        }
                    }
                }
            }
        }
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/figures.png");
        image.save(&out).unwrap();
        println!("{}", out.display());
    }
}
