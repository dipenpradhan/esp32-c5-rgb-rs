//! RGB color math for the blend effect.

/// An RGB color as `[R, G, B]` (the order used by the config JSON and the
/// web UI; the WS2812 wire order GRB is applied in [`crate::ws2812`]).
pub type Rgb = [u8; 3];

/// Linearly interpolate between two RGB colors.
///
/// `step` is in the range `0..=total`. With `step == 0` the result is exactly
/// `from`; with `step == total` it is exactly `to` (guaranteed by the integer
/// formula below, no drift at either endpoint).
pub fn interpolate(from: &Rgb, to: &Rgb, step: u32, total: u32) -> Rgb {
    [
        lerp_u8(from[0], to[0], step, total),
        lerp_u8(from[1], to[1], step, total),
        lerp_u8(from[2], to[2], step, total),
    ]
}

/// Linear interpolation for a single u8 channel.
///
/// Computed with a single integer division (`from + (to - from) * step / total`)
/// so that:
/// - `step == 0` is exactly `from` and `step == total` is exactly `to`,
/// - `from == to` is exactly `from` at every step (no integer-drift),
/// - the result is monotonic between the endpoints.
///
/// The arithmetic is in `i64`, not `i32`: `step` is a `u32` that can exceed
/// `i32::MAX`, and `delta * step` (|delta| ≤ 255, step up to ~4.3e9) can
/// exceed `i32::MAX` for any step past ~8.4e6. In `i64` the product is always
/// ≤ ~1.1e12, far below `i64::MAX`, so this is correct for *any* `step`
/// without relying on the caller having validated it (defence in depth).
///
/// `total == 0` returns `from` (avoids division by zero when a blend has
/// zero steps).
pub fn lerp_u8(from: u8, to: u8, step: u32, total: u32) -> u8 {
    if total == 0 {
        return from;
    }
    let from_i = from as i64;
    let delta = to as i64 - from_i;
    let v = from_i + delta * step as i64 / total as i64;
    v.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_endpoints_exact() {
        // Both endpoints must land exactly on the target colors.
        assert_eq!(lerp_u8(0, 255, 0, 10), 0);
        assert_eq!(lerp_u8(0, 255, 10, 10), 255);
        assert_eq!(lerp_u8(255, 0, 0, 20), 255);
        assert_eq!(lerp_u8(255, 0, 20, 20), 0);
        assert_eq!(lerp_u8(100, 200, 0, 5), 100);
        assert_eq!(lerp_u8(100, 200, 5, 5), 200);
    }

    #[test]
    fn lerp_midpoint_close() {
        // Halfway should be (from + to) / 2 within 1 step.
        let mid = lerp_u8(0, 255, 1, 2);
        assert!((126..=129).contains(&mid), "midpoint was {mid}");

        let mid2 = lerp_u8(50, 200, 1, 2);
        assert!((124..=127).contains(&mid2), "midpoint was {mid2}");
    }

    #[test]
    fn lerp_total_zero_returns_from() {
        assert_eq!(lerp_u8(1, 2, 0, 0), 1);
        // Even with a non-zero step, a zero total must not panic or divide.
        assert_eq!(lerp_u8(7, 9, 3, 0), 7);
    }

    #[test]
    fn lerp_same_color_is_constant() {
        for step in 0..=8 {
            assert_eq!(lerp_u8(42, 42, step, 8), 42);
        }
    }

    #[test]
    fn lerp_is_monotonic_for_increasing_target() {
        let prev = &mut 0u8;
        for step in 0..=10 {
            let v = lerp_u8(0, 255, step, 10);
            assert!(v >= *prev, "not monotonic at step {step}: {v} < {prev}");
            *prev = v;
        }
        assert_eq!(*prev, 255);
    }

    #[test]
    fn lerp_is_monotonic_for_decreasing_target() {
        let prev = &mut 255u8;
        for step in 0..=10 {
            let v = lerp_u8(255, 0, step, 10);
            assert!(v <= *prev, "not monotonic at step {step}: {v} > {prev}");
            *prev = v;
        }
        assert_eq!(*prev, 0);
    }

    #[test]
    fn interpolate_full_range() {
        let from: Rgb = [255, 0, 0];
        let to: Rgb = [0, 255, 255];
        assert_eq!(interpolate(&from, &to, 0, 20), [255, 0, 0]);
        assert_eq!(interpolate(&from, &to, 20, 20), [0, 255, 255]);

        let mid = interpolate(&from, &to, 10, 20);
        // Red 255→0, green 0→255, blue 0→255: each ~128 halfway (±1).
        assert!((126..=130).contains(&mid[0]), "red was {}", mid[0]);
        assert!((126..=130).contains(&mid[1]), "green was {}", mid[1]);
        assert!((126..=130).contains(&mid[2]), "blue was {}", mid[2]);
    }

    #[test]
    fn interpolate_identity() {
        let c: Rgb = [10, 20, 30];
        assert_eq!(interpolate(&c, &c, 3, 3), c);
        assert_eq!(interpolate(&c, &c, 0, 3), c);
    }

    #[test]
    fn interpolate_zero_total_returns_from() {
        let from: Rgb = [1, 2, 3];
        let to: Rgb = [4, 5, 6];
        assert_eq!(interpolate(&from, &to, 0, 0), from);
    }

    #[test]
    fn lerp_u8_overflow_proof() {
        // `step` is a u32 taken straight from the (network-supplied) config, so
        // a huge value is a realistic input, not a corner case. With the old
        // i32 arithmetic (`delta * step as i32`), the product 255 * 9_000_000
        // = 2_295_000_000 exceeds i32::MAX (2_147_483_647): it panics in debug
        // builds and silently wraps (garbage colours) in release. The correct
        // result is simply clamped to the far endpoint, `to`.
        assert_eq!(lerp_u8(0, 255, 9_000_000, 10), 255);
        // Descending case: the product is negative and underflows i32::MIN the
        // same way; the result clamps to `to` = 0.
        assert_eq!(lerp_u8(255, 0, 9_000_000, 10), 0);
    }
}
