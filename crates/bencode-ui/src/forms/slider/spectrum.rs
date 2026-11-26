//! A stepped luminous rail with momentum-preserving spring physics.
//! The thumb, the illuminated beam, the kinetic wake and the starlight glints share one spring state.
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, ElementId, Hsla, Styled, Window, canvas, fill, linear_color_stop, linear_gradient,
    point, size,
};

use crate::theme::{ActiveTheme, Mix};

/// Second-order damped spring constants: snappy (~220ms settle) with a slight tactile overshoot.
const OMEGA: f32 = 26.0;
const DAMPING: f32 = 0.78;
const MAX_DT: f32 = 0.05;
const POS_EPS: f32 = 0.0005;
const VEL_EPS: f32 = 0.002;
const PULSE_DURATION: Duration = Duration::from_millis(260);

/// Motion snapshot shared by the spectrum rail and its thumb on a single frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct Motion {
    /// Clamped visual position along `0.0..=1.0`.
    pub at: f32,
    /// Signed spring velocity in normalized track units per second.
    pub velocity: f32,
    /// Kinetic intensity (`0.0..=1.0`) from velocity and recent step arrival.
    pub energy: f32,
}

#[derive(Clone, Copy, Debug)]
struct Spring {
    pos: f32,
    vel: f32,
    target: f32,
    last: Instant,
    pulse_start: Instant,
    pulse_peak: f32,
}

impl Spring {
    fn settled(target: f32, now: Instant) -> Self {
        Self {
            pos: target,
            vel: 0.0,
            target,
            last: now,
            pulse_start: now - PULSE_DURATION,
            pulse_peak: 0.0,
        }
    }

    /// Exact analytical advance for an underdamped harmonic oscillator.
    /// Frame-rate independent and preserves momentum across rapid retargets.
    fn advance(&mut self, now: Instant) {
        let dt = now
            .saturating_duration_since(self.last)
            .as_secs_f32()
            .min(MAX_DT);
        self.last = now;
        if dt <= 0.0 {
            return;
        }
        let x0 = self.pos - self.target;
        let v0 = self.vel;
        if x0.abs() < POS_EPS && v0.abs() < VEL_EPS {
            self.pos = self.target;
            self.vel = 0.0;
            return;
        }
        let omega_d = OMEGA * (1.0 - DAMPING * DAMPING).sqrt();
        let decay = (-DAMPING * OMEGA * dt).exp();
        let (sin, cos) = (omega_d * dt).sin_cos();
        let c2 = (v0 + DAMPING * OMEGA * x0) / omega_d;
        let x = decay * (x0 * cos + c2 * sin);
        let v = decay * (v0 * cos - (DAMPING * OMEGA * v0 + OMEGA * OMEGA * x0) / omega_d * sin);
        if x.abs() < POS_EPS && v.abs() < VEL_EPS {
            self.pos = self.target;
            self.vel = 0.0;
        } else {
            self.pos = self.target + x;
            self.vel = v;
        }
    }

    fn retarget(&mut self, target: f32, now: Instant) {
        self.advance(now);
        let jump = (target - self.pos).abs();
        self.target = target;
        self.pulse_start = now;
        self.pulse_peak = (jump * 1.4).clamp(0.32, 1.0);
    }

    fn snap(&mut self, target: f32, now: Instant) {
        self.pos = target;
        self.vel = 0.0;
        self.target = target;
        self.last = now;
        self.pulse_peak = 0.0;
    }

    fn pulse(&self, now: Instant) -> f32 {
        if self.pulse_peak <= 0.0 {
            return 0.0;
        }
        let t = now.saturating_duration_since(self.pulse_start).as_secs_f32()
            / PULSE_DURATION.as_secs_f32();
        if t >= 1.0 {
            0.0
        } else {
            self.pulse_peak * (1.0 - t) * (1.0 - t)
        }
    }

    fn is_moving(&self, now: Instant) -> bool {
        (self.pos - self.target).abs() >= POS_EPS
            || self.vel.abs() >= VEL_EPS
            || self.pulse(now) > 0.005
    }

    fn sample(&self, now: Instant) -> Motion {
        let speed = (self.vel.abs() / 3.6).clamp(0.0, 1.0);
        let pulse = self.pulse(now);
        Motion {
            at: self.pos.clamp(0.0, 1.0),
            velocity: self.vel,
            energy: speed.max(pulse).clamp(0.0, 1.0),
        }
    }
}

/// Retargets from the on-screen position and velocity when dragging or stepping across stops.
pub(super) fn motion(id: ElementId, target: f32, window: &mut Window, cx: &mut App) -> Motion {
    let now = Instant::now();
    let state = window.use_keyed_state((id, "spectrum-spring"), cx, |_, _| {
        Spring::settled(target, now)
    });
    let reduced = cx.reduce_motion() || cx.theme().reduced_motion;
    let needs_update = {
        let spring = state.read(cx);
        spring.target != target || spring.is_moving(now)
    };
    if needs_update {
        state.update(cx, |spring, _| {
            if reduced {
                spring.snap(target, now);
            } else if spring.target != target {
                spring.retarget(target, now);
            } else {
                spring.advance(now);
            }
        });
    }
    let spring = state.read(cx);
    if !reduced && spring.is_moving(now) {
        window.request_animation_frame();
        spring.sample(now)
    } else {
        Motion {
            at: target.clamp(0.0, 1.0),
            velocity: 0.0,
            energy: 0.0,
        }
    }
}

pub(super) fn rail(
    motion: Motion,
    intervals: usize,
    hue: Hsla,
    light: Hsla,
    ink: Hsla,
) -> impl gpui::IntoElement {
    let at = motion.at;
    let energy = motion.energy;
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            let h = b.size.height;
            let radius = h * 0.5;
            let travel = (b.size.width - h).max(gpui::Pixels::ZERO);
            let center = b.left() + radius + travel * at;
            let gradient =
                |a, z| linear_gradient(90.0, linear_color_stop(a, 0.0), linear_color_stop(z, 1.0));

            // Deep velvety base rail so unselected stops stay crisp in both dark and light themes.
            let night = hue.mix(&ink, 0.68);
            let dusk = hue.mix(&ink, 0.42);
            window.paint_quad(fill(b, gradient(night, dusk)).corner_radii(radius));

            // Illuminated beam from the left end to the thumb, intensifying with effort & motion.
            let fill_w = (center - b.left() + radius).clamp(h, b.size.width);
            let illuminated = Bounds::new(b.origin, size(fill_w, h));
            let deep = hue.mix(&ink, 0.40);
            let bright = Hsla {
                s: (hue.s.max(0.68) + at * 0.08 + energy * 0.06).min(0.92),
                l: (0.62 + at * 0.07 + energy * 0.05).min(0.76),
                ..hue
            };
            window.paint_quad(fill(illuminated, gradient(deep, bright)).corner_radii(radius));

            // Soft kinetic bloom trailing immediately behind the thumb.
            let wake_w = (h * (1.35 + energy * 1.15)).min(fill_w);
            let wake_left = (b.left() + fill_w - wake_w).max(b.left());
            let wake = Bounds::new(point(wake_left, b.top()), size(b.left() + fill_w - wake_left, h));
            window.paint_quad(
                fill(
                    wake,
                    gradient(bright.opacity(0.0), light.opacity(0.16 + energy * 0.20)),
                )
                .corner_radii(radius),
            );

            // Subtle upper glass specular ribbon along the charged bar.
            let sheen_left = b.left() + radius * 0.65;
            let sheen_right = (center + radius * 0.35).min(b.right() - radius * 0.45);
            let sheen_w = sheen_right - sheen_left;
            if sheen_w > h * 0.35 {
                let sheen_h = (h * 0.09).max(gpui::px(1.2));
                let sheen = Bounds::new(point(sheen_left, b.top() + h * 0.13), size(sheen_w, sheen_h));
                window.paint_quad(
                    fill(
                        sheen,
                        gradient(light.opacity(0.04), light.opacity(0.22 + energy * 0.16)),
                    )
                    .corner_radii(sheen_h * 0.5),
                );
            }

            let bead = |x, y, w, height, color, window: &mut Window| {
                window.paint_quad(
                    fill(
                        Bounds::new(point(x - w * 0.5, y - height * 0.5), size(w, height)),
                        color,
                    )
                    .corner_radii(height * 0.5),
                );
            };

            // Starlight constellation: stars wake and shimmer as the thumb sweeps over them.
            for (i, (x, y)) in [
                (0.06, 0.72),
                (0.125, 0.28),
                (0.20, 0.77),
                (0.255, 0.23),
                (0.33, 0.72),
                (0.385, 0.27),
                (0.47, 0.76),
                (0.53, 0.24),
                (0.615, 0.73),
                (0.685, 0.26),
                (0.75, 0.76),
                (0.835, 0.30),
                (0.935, 0.71),
            ]
            .into_iter()
            .enumerate()
            {
                let distance = (at - x).abs();
                let proximity = (1.0 - distance * 5.2).clamp(0.0, 1.0);
                let glow = proximity * proximity * (0.65 + energy * 0.65);
                let base_alpha = if x <= at { 0.38 } else { 0.14 };
                let alpha = (base_alpha + glow * 0.48).min(0.92);
                let tone = light.opacity(alpha);
                let (sx, sy) = (b.left() + radius + travel * x, b.top() + h * y);
                let dot = h * (0.044 + glow * 0.036);
                bead(sx, sy, dot, dot, tone, window);
                if i % 4 == 1 {
                    let arm = dot * (2.5 + glow * 1.15);
                    let core = dot * 0.60;
                    bead(sx, sy, arm, core, tone, window);
                    bead(sx, sy, core, arm, tone, window);
                }
            }

            // Stepped stop detents; dots smoothly tuck away under the thumb so edges stay clean.
            if (1..=12).contains(&intervals) {
                for i in 0..=intervals {
                    let stop_f = i as f32 / intervals as f32;
                    let clearance = ((stop_f - at).abs() / 0.075).clamp(0.0, 1.0);
                    if clearance <= 0.05 {
                        continue;
                    }
                    let sx = b.left() + radius + travel * stop_f;
                    let alpha = if stop_f < at { 0.68 } else { 0.78 } * clearance;
                    let size = h * (0.105 + 0.015 * clearance);
                    bead(sx, b.top() + radius, size, size, light.opacity(alpha), window);
                }
            }

            // Soft luminous halo behind the jewel thumb; swells gently while gliding.
            let mid_y = b.top() + radius;
            let outer_halo = h * (0.94 + energy * 0.06);
            bead(
                center,
                mid_y,
                outer_halo,
                outer_halo,
                bright.mix(&light, 0.45).opacity(0.32 + energy * 0.28),
                window,
            );
            let inner_halo = h * 0.84;
            bead(
                center,
                mid_y,
                inner_halo,
                inner_halo,
                light.opacity(0.24 + energy * 0.24),
                window,
            );
        },
    )
    .absolute()
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_converges_smoothly_and_settles() {
        let start = Instant::now();
        let mut spring = Spring::settled(0.0, start);
        spring.retarget(1.0, start);
        let mut now = start;
        let mut peak = 0.0_f32;
        for _ in 0..40 {
            now += Duration::from_millis(16);
            spring.advance(now);
            peak = peak.max(spring.pos);
        }
        assert!(peak > 1.005 && peak < 1.04, "unexpected overshoot peak {peak}");
        assert!((spring.pos - 1.0).abs() < 1e-4);
        assert!(spring.vel.abs() < 1e-4);
        assert!(!spring.is_moving(now));
    }

    #[test]
    fn retargeting_mid_flight_preserves_momentum() {
        let start = Instant::now();
        let mut spring = Spring::settled(0.0, start);
        spring.retarget(0.4, start);
        let mid = start + Duration::from_millis(48);
        spring.advance(mid);
        let vel_before = spring.vel;
        assert!(vel_before > 1.0, "expected forward momentum, got {vel_before}");

        spring.retarget(0.8, mid);
        assert!(
            (spring.vel - vel_before).abs() < 1e-5,
            "retarget should preserve velocity instead of stalling"
        );
        let next = mid + Duration::from_millis(16);
        spring.advance(next);
        assert!(spring.pos > 0.2, "spring should continue gliding forward");
    }

    #[test]
    fn snap_settles_immediately_for_reduced_motion() {
        let start = Instant::now();
        let mut spring = Spring::settled(0.0, start);
        spring.retarget(0.5, start);
        let later = start + Duration::from_millis(20);
        spring.snap(1.0, later);
        assert_eq!(spring.pos, 1.0);
        assert_eq!(spring.vel, 0.0);
        assert!(!spring.is_moving(later));
    }
}
