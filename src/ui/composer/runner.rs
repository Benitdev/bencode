//! MonoCode's composer runner physics (`composerRunner.ts`): the project
//! mascot patrols the composer's top edge while a turn runs, hops coins
//! that appear above it, bonks the "Jump to latest" chevron the first time
//! (recoil, shake, orbiting stars) and hops it after, then leaps off and
//! sinks behind the rim when the turn ends. All pure; times are in ms,
//! heights in px above the rim, x in px along the track.

pub const RUNNER_SIZE: f32 = 16.0;
pub const RUNNER_SPEED_PX: f32 = 160.0;
pub const RUNNER_INSET: f32 = 10.0;
const JUMP_LEAD: f32 = 18.0;
const JUMP_CLEARANCE: f32 = 10.0;
const JUMP_MIN: f32 = 28.0;

pub const COIN_SIZE: f32 = 12.0;
pub const COIN_HOVER: f32 = 42.0;
const COIN_WIDTH: f32 = 8.0;
const COIN_JUMP_LEAD: f32 = 34.0;
const COIN_GAP_MS: (f32, f32) = (7_000.0, 18_000.0);
const COIN_FIRST_MS: (f32, f32) = (3_500.0, 9_000.0);
const COLLECT_X: f32 = 10.0;
pub const COLLECT_POP_MS: f32 = 280.0;
pub const COLLECT_POP_PX: f32 = 16.0;

pub const EXIT_MS: f32 = 560.0;
const EXIT_PEAK: f32 = 44.0;
pub const EXIT_SINK: f32 = 20.0;
const EXIT_APEX: f32 = 0.38;

const CRASH_RECOIL_PX: f32 = 18.0;
const CRASH_RECOIL_MS: f32 = 140.0;
pub const CRASH_STUN_MS: f32 = 560.0;
const CRASH_SHAKE_MS: f32 = 480.0;
pub const STAR_SIZE: f32 = 8.0;
const STAR_COUNT: usize = 3;
const STAR_ORBIT: f32 = 11.0;
const STAR_SPIN_MS: f32 = 520.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    Right,
    Left,
}

impl Facing {
    pub fn sign(self) -> f32 {
        match self {
            Self::Right => 1.0,
            Self::Left => -1.0,
        }
    }
}

/// A hurdle on the rim (the chevron), in track coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obstacle {
    pub left: f32,
    pub right: f32,
    /// Peak of the jump arc above the rim.
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coin {
    /// Centre along the track.
    pub x: f32,
    /// How high the mascot must jump to grab it.
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Sprite centre along the track.
    pub x: f32,
    /// Feet above the rim; negative sinks behind the box.
    pub y: f32,
    pub facing: Facing,
}

/// A rectangle in window coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn width(&self) -> f32 {
        self.right - self.left
    }
}

fn arc(x: f32, left: f32, right: f32, height: f32, lead: f32) -> f32 {
    let (start, end) = (left - lead, right + lead);
    if end <= start || x <= start || x >= end {
        return 0.0;
    }
    let t = (x - start) / (end - start);
    4.0 * t * (1.0 - t) * height
}

/// The body, not the shoes, meets the coin.
fn coin_jump_peak(coin: &Coin) -> f32 {
    (coin.height - RUNNER_SIZE / 2.0).max(0.0)
}

/// MonoCode `jumpHeight`: a parabola over the obstacle and each coin.
pub fn jump_height(x: f32, obstacle: Option<&Obstacle>, coins: &[Coin]) -> f32 {
    let over = obstacle.map_or(0.0, |o| arc(x, o.left, o.right, o.height, JUMP_LEAD));
    coins
        .iter()
        .map(|coin| {
            arc(
                x,
                coin.x - COIN_WIDTH / 2.0,
                coin.x + COIN_WIDTH / 2.0,
                coin_jump_peak(coin),
                COIN_JUMP_LEAD,
            )
        })
        .fold(over, f32::max)
}

/// The inner track a runner paces, inside the insets.
pub fn inner_width(width: f32) -> f32 {
    (width - RUNNER_INSET * 2.0).max(0.0)
}

/// MonoCode `stepAlong`: run at speed, turning at either end.
pub fn step_along(along: f32, facing: Facing, dt_ms: f32, track: f32) -> (f32, Facing) {
    if track <= 0.0 {
        return (0.0, Facing::Right);
    }
    let next = along + facing.sign() * RUNNER_SPEED_PX * dt_ms / 1000.0;
    if next >= track {
        (track, Facing::Left)
    } else if next <= 0.0 {
        (0.0, Facing::Right)
    } else {
        (next, facing)
    }
}

/// MonoCode `poseAt`.
pub fn pose_at(
    along: f32,
    facing: Facing,
    width: f32,
    obstacle: Option<&Obstacle>,
    coins: &[Coin],
) -> Pose {
    let x = RUNNER_INSET + along.clamp(0.0, inner_width(width));
    Pose {
        x,
        y: jump_height(x, obstacle, coins),
        facing,
    }
}

/// MonoCode `scaleTrackX`: same relative spot after a resize.
pub fn scale_track_x(x: f32, from: f32, to: f32) -> f32 {
    if from <= 0.0 { 0.0 } else { x * to / from }
}

/// MonoCode `hitsChevron`: first contact this turn, on the ground.
pub fn hits_chevron(pose: &Pose, obstacle: Option<&Obstacle>, learned: bool) -> bool {
    let Some(o) = obstacle.filter(|_| !learned && pose.y <= 0.5) else {
        return false;
    };
    let half = RUNNER_SIZE / 2.0;
    match pose.facing {
        Facing::Right => pose.x + half >= o.left && pose.x - half < o.right,
        Facing::Left => pose.x - half <= o.right && pose.x + half > o.left,
    }
}

/// MonoCode `recoilAlong`: knocked back, easing out, on the track.
pub fn recoil_along(hit: f32, facing: Facing, elapsed_ms: f32, track: f32) -> f32 {
    let t = (elapsed_ms / CRASH_RECOIL_MS).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t) * (1.0 - t);
    (hit - facing.sign() * CRASH_RECOIL_PX * eased).clamp(0.0, track)
}

/// MonoCode `stunShake`: a decaying wobble.
pub fn stun_shake(elapsed_ms: f32) -> (f32, f32) {
    if elapsed_ms <= 0.0 || elapsed_ms >= CRASH_SHAKE_MS {
        return (0.0, 0.0);
    }
    let decay = 1.0 - elapsed_ms / CRASH_SHAKE_MS;
    (
        ((elapsed_ms / 32.0).sin() * 3.0 * decay).round(),
        ((elapsed_ms / 26.0).cos() * 2.0 * decay).round(),
    )
}

/// MonoCode `stunStars`: offsets from the sprite's top-left and opacity.
pub fn stun_stars(elapsed_ms: f32) -> Vec<(f32, f32, f32)> {
    if !(0.0..CRASH_STUN_MS).contains(&elapsed_ms) {
        return Vec::new();
    }
    let fade_at = CRASH_STUN_MS - 140.0;
    let opacity = if elapsed_ms < fade_at {
        1.0
    } else {
        (1.0 - (elapsed_ms - fade_at) / 140.0).max(0.0)
    };
    let origin = (RUNNER_SIZE - STAR_SIZE) / 2.0;
    let angle = elapsed_ms / STAR_SPIN_MS * std::f32::consts::TAU;
    (0..STAR_COUNT)
        .map(|i| {
            let a = angle + i as f32 * std::f32::consts::TAU / STAR_COUNT as f32;
            (
                (origin + a.cos() * STAR_ORBIT).round(),
                (origin - 5.0 + a.sin() * STAR_ORBIT).round(),
                opacity,
            )
        })
        .collect()
}

/// MonoCode `coinCollected`: the sprite overlaps the coin.
pub fn coin_collected(pose: &Pose, coin: &Coin) -> bool {
    if (pose.x - coin.x).abs() > COLLECT_X {
        return false;
    }
    let (top, bottom) = (pose.y + RUNNER_SIZE, pose.y);
    top >= coin.height - COIN_SIZE / 2.0 && bottom <= coin.height + COIN_SIZE / 2.0
}

/// MonoCode `nextCoinDelay`; `random` is in `0..1`.
pub fn next_coin_delay(first: bool, random: f32) -> f32 {
    let (min, max) = if first { COIN_FIRST_MS } else { COIN_GAP_MS };
    min + random * (max - min)
}

/// MonoCode `pickCoinX`: away from the runner and the chevron when it can.
pub fn pick_coin_x(
    width: f32,
    runner_x: f32,
    obstacle: Option<&Obstacle>,
    mut random: impl FnMut() -> f32,
) -> Option<f32> {
    let min = RUNNER_INSET + COIN_JUMP_LEAD + 8.0;
    let max = width - RUNNER_INSET - COIN_JUMP_LEAD - 8.0;
    if max <= min {
        return None;
    }
    for _ in 0..8 {
        let x = min + random() * (max - min);
        let near_runner = (x - runner_x).abs() < 40.0;
        let on_chevron = obstacle.is_some_and(|o| x >= o.left - 6.0 && x <= o.right + 6.0);
        if !near_runner && !on_chevron {
            return Some(x);
        }
    }
    Some(min + random() * (max - min))
}

/// MonoCode `exitJumpY`: a hop that peaks, then sinks behind the rim.
pub fn exit_jump_y(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return -EXIT_SINK;
    }
    if t < EXIT_APEX {
        let u = t / EXIT_APEX;
        return EXIT_PEAK * (1.0 - (1.0 - u) * (1.0 - u));
    }
    let u = (t - EXIT_APEX) / (1.0 - EXIT_APEX);
    EXIT_PEAK + (-EXIT_SINK - EXIT_PEAK) * u * u
}

/// MonoCode `spriteClipBottom`: what the rim hides of a sinking sprite.
pub fn sprite_clip_bottom(y: f32) -> f32 {
    if y >= 0.0 {
        0.0
    } else {
        (-y).ceil().min(RUNNER_SIZE)
    }
}

/// MonoCode `obstacleFromRects`: a control sitting on the rim, overlapping
/// it, becomes a hurdle.
pub fn obstacle_from_rects(track: &Rect, button: Option<&Rect>) -> Option<Obstacle> {
    let b = button?;
    if b.right <= track.left || b.left >= track.right {
        return None;
    }
    if b.bottom < track.top - 48.0 || b.top > track.top + 12.0 {
        return None;
    }
    Some(Obstacle {
        left: b.left - track.left,
        right: b.right - track.left,
        height: (track.top - b.top + JUMP_CLEARANCE).max(JUMP_MIN),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runner_turns_at_both_ends() {
        assert_eq!(
            step_along(95.0, Facing::Right, 100.0, 100.0),
            (100.0, Facing::Left)
        );
        assert_eq!(
            step_along(5.0, Facing::Left, 100.0, 100.0),
            (0.0, Facing::Right)
        );
        let (along, facing) = step_along(10.0, Facing::Right, 100.0, 100.0);
        assert!((along - 26.0).abs() < 1e-3 && facing == Facing::Right);
    }

    #[test]
    fn jumps_peak_over_the_middle_of_a_hurdle() {
        let hurdle = Obstacle {
            left: 100.0,
            right: 120.0,
            height: 30.0,
        };
        assert_eq!(jump_height(50.0, Some(&hurdle), &[]), 0.0);
        assert!((jump_height(110.0, Some(&hurdle), &[]) - 30.0).abs() < 1e-3);
        let coin = Coin {
            x: 200.0,
            height: COIN_HOVER,
        };
        assert!((jump_height(200.0, None, &[coin]) - (COIN_HOVER - 8.0)).abs() < 1e-3);
    }

    #[test]
    fn coins_are_grabbed_at_the_top_of_the_hop() {
        let coin = Coin {
            x: 200.0,
            height: COIN_HOVER,
        };
        let peak = Pose {
            x: 200.0,
            y: jump_height(200.0, None, &[coin]),
            facing: Facing::Right,
        };
        assert!(coin_collected(&peak, &coin));
        let ground = Pose { y: 0.0, ..peak };
        assert!(!coin_collected(&ground, &coin));
    }

    #[test]
    fn the_first_bonk_only_happens_on_the_ground() {
        let hurdle = Obstacle {
            left: 100.0,
            right: 120.0,
            height: 30.0,
        };
        let at = Pose {
            x: 95.0,
            y: 0.0,
            facing: Facing::Right,
        };
        assert!(hits_chevron(&at, Some(&hurdle), false));
        assert!(!hits_chevron(&at, Some(&hurdle), true));
        assert!(!hits_chevron(&Pose { y: 5.0, ..at }, Some(&hurdle), false));
        assert_eq!(
            recoil_along(50.0, Facing::Right, CRASH_RECOIL_MS, 200.0),
            32.0
        );
        assert_eq!(stun_stars(CRASH_STUN_MS).len(), 0);
        assert_eq!(stun_stars(10.0).len(), 3);
    }

    #[test]
    fn the_exit_hop_rises_then_sinks_behind_the_rim() {
        assert_eq!(exit_jump_y(0.0), 0.0);
        assert!((exit_jump_y(EXIT_APEX) - EXIT_PEAK).abs() < 1e-3);
        assert_eq!(exit_jump_y(1.0), -EXIT_SINK);
        assert_eq!(sprite_clip_bottom(-4.2), 5.0);
        assert_eq!(sprite_clip_bottom(-40.0), RUNNER_SIZE);
    }

    #[test]
    fn only_a_control_on_the_rim_is_a_hurdle() {
        let track = Rect {
            left: 100.0,
            top: 500.0,
            right: 900.0,
            bottom: 508.0,
        };
        let chevron = Rect {
            left: 480.0,
            top: 470.0,
            right: 504.0,
            bottom: 494.0,
        };
        let hurdle = obstacle_from_rects(&track, Some(&chevron)).unwrap();
        assert_eq!(
            (hurdle.left, hurdle.right, hurdle.height),
            (380.0, 404.0, 40.0)
        );
        let far = Rect {
            top: 300.0,
            bottom: 324.0,
            ..chevron
        };
        assert_eq!(obstacle_from_rects(&track, Some(&far)), None);
    }

    #[test]
    fn coins_keep_clear_of_the_runner() {
        let mut values = [0.0, 0.9].into_iter().cycle();
        let x = pick_coin_x(800.0, 60.0, None, || values.next().unwrap()).unwrap();
        assert!((x - 60.0).abs() >= 40.0);
        assert_eq!(pick_coin_x(80.0, 0.0, None, || 0.5), None);
        assert_eq!(next_coin_delay(true, 0.0), 3_500.0);
    }
}
