//! CSS timing curves for animations ported from MonoCode.

/// CSS `cubic-bezier(x1, y1, x2, y2)` as a GPUI easing: progress at time
/// `t` (0..1), found by bisecting the curve's x.
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 {
    move |t: f32| {
        let t = t.clamp(0.0, 1.0);
        let curve = |s: f32, a: f32, b: f32| {
            let inv = 1.0 - s;
            3.0 * inv * inv * s * a + 3.0 * inv * s * s * b + s * s * s
        };
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if curve(mid, x1, x2) < t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        curve((lo + hi) / 2.0, y1, y2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_start_end_and_lead_like_css() {
        let ease = cubic_bezier(0.32, 0.72, 0.0, 1.0);
        assert!(ease(0.0).abs() < 1e-3 && (ease(1.0) - 1.0).abs() < 1e-3);
        assert!(
            ease(0.5) > 0.8,
            "MonoCode's push is front-loaded: {}",
            ease(0.5)
        );
        let linear = cubic_bezier(0.0, 0.0, 1.0, 1.0);
        assert!((linear(0.3) - 0.3).abs() < 1e-2);
    }
}
