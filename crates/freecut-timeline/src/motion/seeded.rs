//! Seeded motion (TX-056, FX-030): per-unit random offsets and turns,
//! smooth jitter, flicker dips and sparkle moments. Every number comes from
//! integer hashing of [`crate::model::AnimParams::seed`], the unit's index
//! and what the number is for, so preview, export and every OS draw the
//! same ones, and a unit's motion never depends on the other units.
//!
//! Times here are seconds of the slot (or of the cycle, for loops) as
//! `f64`: this is the evaluation edge, after progress has left rational
//! time.

use glam::DVec2;

/// What a seeded number is for, so channels draw independent numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stream {
    OffsetAngle = 1,
    OffsetRadius = 2,
    Rotate = 3,
    JitterX = 4,
    JitterY = 5,
    JitterRotate = 6,
    Flicker = 7,
    Sparkle = 8,
}

/// The shortest time from one flicker dip to the next (TX-056's Flicker):
/// a dip is two sharp changes, so dips a second apart keep every second
/// under FX-030's three sharp changes, and well under 2 dips per second.
pub(crate) const FLICKER_GAP_SECS: f64 = 1.0;

/// How long a dip lasts without a frame rate: two frames at 30 fps.
pub(crate) const FLICKER_DIP_SECS: f64 = 2.0 / 30.0;

/// How long a sparkle swell lasts (TX-056: 0.2 s).
pub(crate) const SPARKLE_SECS: f64 = 0.2;

/// Jitter changes direction this many times a second (FX-030, TX-056).
const JITTER_TURNS_PER_SECOND: f64 = 12.0;

/// SplitMix64's finalizer: a bijective 64-bit mix.
const fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A uniform number in `[0, 1)` for one (seed, unit, stream, index).
pub(crate) fn uniform(seed: u32, unit: u32, stream: Stream, index: u32) -> f64 {
    let key = (u64::from(seed) << 32) | u64::from(unit);
    let what = ((stream as u64) << 32) | u64::from(index);
    let h = mix(mix(key) ^ mix(what));
    // The top 53 bits: exact in an f64.
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// A uniform number in `[−1, 1)`.
pub(crate) fn signed(seed: u32, unit: u32, stream: Stream) -> f64 {
    2.0 * uniform(seed, unit, stream, 0) - 1.0
}

/// Scatter's start offset of one unit as a share of the amplitude: a
/// seeded direction and a seeded distance of 0.5–1 (so every unit visibly
/// moves).
pub(crate) fn scatter(seed: u32, unit: u32) -> DVec2 {
    let angle = std::f64::consts::TAU * uniform(seed, unit, Stream::OffsetAngle, 0);
    let r = 0.5 + 0.5 * uniform(seed, unit, Stream::OffsetRadius, 0);
    DVec2::new(angle.cos(), angle.sin()) * r
}

/// Smooth seeded noise in `[−1, 1]` at progress `q` of a span of
/// `span_secs` (FX-030's jitter): knots alternate in sign with seeded
/// magnitudes of 0.4–1, 12 a second (an even number per span, at least
/// two) from a seeded offset (vertical half a knot later than horizontal),
/// joined by smoothstep, so it is C1, never leaves `[−1, 1]`,
/// changes direction at every knot and repeats every span (a Loop's
/// cycle is seamless).
pub(crate) fn jitter(seed: u32, unit: u32, stream: Stream, q: f64, span_secs: f64) -> f64 {
    let halves = (span_secs * JITTER_TURNS_PER_SECOND / 2.0).round();
    // At least one pair, at most 5000: a clamped, whole, positive float.
    let n = 2 * (halves.clamp(1.0, 5000.0) as u32);
    // The knots start at a seeded offset; vertical jitter turns half a knot
    // after horizontal (and turns a quarter), so at a horizontal extreme
    // the vertical motion is mid-swing and the path wanders in 2D instead
    // of along a diagonal.
    let lag = match stream {
        Stream::JitterY => 0.5,
        Stream::JitterRotate => 0.25,
        _ => 0.0,
    };
    let shift = uniform(seed, unit, Stream::JitterX, u32::MAX - 1) + lag;
    let x = (q.rem_euclid(1.0) * f64::from(n) + shift).rem_euclid(f64::from(n));
    if !x.is_finite() {
        return 0.0;
    }
    let whole = x.floor();
    let f = x - whole;
    // In 0..n by construction (clamped against rounding at q → 1).
    let i = (whole as u32).min(n - 1);
    let flip = if uniform(seed, unit, stream, u32::MAX) < 0.5 {
        1.0
    } else {
        -1.0
    };
    let knot = |k: u32| {
        let sign = if k.is_multiple_of(2) { flip } else { -flip };
        sign * (0.4 + 0.6 * uniform(seed, unit, stream, k))
    };
    let (a, b) = (knot(i), knot((i + 1) % n));
    let s = f * f * (3.0 - 2.0 * f);
    a + (b - a) * s
}

/// 1 while a flicker dip of `dip_secs` is on at progress `q` of a cycle of
/// `cycle_secs`, else 0 (TX-056's Flicker). A cycle holds
/// `⌊cycle / 1 s⌋` dips (none under a second), each in its own equal slot
/// at a seeded place, the whole pattern shifted by a seeded offset: any
/// two dips, also across the cycle seam, start at least
/// [`FLICKER_GAP_SECS`] apart.
pub(crate) fn flicker_dip(seed: u32, unit: u32, q: f64, cycle_secs: f64, dip_secs: f64) -> f64 {
    // Anything but a finite cycle of at least one gap (NaN included) has
    // no dips.
    if !cycle_secs.is_finite() || cycle_secs < FLICKER_GAP_SECS {
        return 0.0;
    }
    // 1..=64 by the check above; whole after floor.
    let n = (cycle_secs / FLICKER_GAP_SECS).floor().clamp(1.0, 64.0) as u32;
    let slot = cycle_secs / f64::from(n);
    let slack = slot - FLICKER_GAP_SECS;
    let dip = dip_secs.clamp(0.0, FLICKER_GAP_SECS / 2.0);
    let shift = uniform(seed, unit, Stream::Flicker, u32::MAX) * cycle_secs;
    let t = q.rem_euclid(1.0) * cycle_secs;
    let on = (0..n).any(|i| {
        let start = shift + f64::from(i) * slot + uniform(seed, unit, Stream::Flicker, i) * slack;
        (t - start).rem_euclid(cycle_secs) < dip
    });
    if on { 1.0 } else { 0.0 }
}

/// A sparkle swell in `[0, 1]` at progress `q` of a cycle of
/// `cycle_secs` (TX-056's Sparkle): one raised-cosine bump of 0.2 s (the
/// whole cycle when shorter) per cycle, centred on a seeded moment, wrapping
/// across the cycle seam.
pub(crate) fn sparkle(seed: u32, unit: u32, q: f64, cycle_secs: f64) -> f64 {
    if !cycle_secs.is_finite() || cycle_secs <= 0.0 {
        return 0.0;
    }
    let width = SPARKLE_SECS.min(cycle_secs);
    let centre = uniform(seed, unit, Stream::Sparkle, 0) * cycle_secs;
    let t = q.rem_euclid(1.0) * cycle_secs;
    let mut d = (t - centre).rem_euclid(cycle_secs);
    if d > cycle_secs / 2.0 {
        d -= cycle_secs;
    }
    if !d.is_finite() || d.abs() >= width / 2.0 {
        return 0.0;
    }
    let c = (std::f64::consts::PI * d / width).cos();
    c * c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_is_deterministic_in_range_and_independent_per_key() {
        let a = uniform(7, 3, Stream::Rotate, 0);
        assert_eq!(a, uniform(7, 3, Stream::Rotate, 0));
        assert!((0.0..1.0).contains(&a));
        // Golden values pin the hash, so a refactor can't silently move
        // every seeded preset.
        assert_eq!(
            uniform(0, 0, Stream::OffsetAngle, 0).to_bits(),
            4_606_591_826_501_506_445
        );
        assert_ne!(a, uniform(8, 3, Stream::Rotate, 0));
        assert_ne!(a, uniform(7, 4, Stream::Rotate, 0));
        assert_ne!(a, uniform(7, 3, Stream::JitterX, 0));
        assert_ne!(a, uniform(7, 3, Stream::Rotate, 1));
        // Roughly uniform: the mean of many draws is near 1/2.
        let mean: f64 = (0..4000)
            .map(|u| uniform(1, u, Stream::Sparkle, 0))
            .sum::<f64>()
            / 4000.0;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
    }

    #[test]
    fn jitter_is_bounded_periodic_smooth_and_turns_twelve_times_a_second() {
        for seed in 0..5 {
            let at = |q: f64| jitter(seed, 2, Stream::JitterX, q, 1.0);
            assert!((at(0.0) - at(1.0)).abs() < 1e-12, "periodic");
            assert!((at(0.0) - at(1.0 - 1e-9)).abs() < 1e-6, "seamless");
            let samples: Vec<f64> = (0..2400).map(|i| at(f64::from(i) / 2400.0)).collect();
            assert!(samples.iter().all(|v| v.abs() <= 1.0 + 1e-12));
            // Direction changes over one cycle, wrapping at the seam: sign
            // changes of the slope.
            let n = samples.len();
            let turns = (0..n)
                .filter(|&i| {
                    let (a, b, c) = (samples[(i + n - 1) % n], samples[i], samples[(i + 1) % n]);
                    (b - a) * (c - b) < 0.0
                })
                .count();
            assert_eq!(turns, 12, "seed {seed}");
            // Smooth: no step bigger than the steepest smoothstep allows.
            let max_step = samples
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0, f64::max);
            assert!(max_step < 2.0 * 1.5 * 12.0 / 2400.0 + 1e-9, "{max_step}");
        }
        // Two seconds turn 24 times; a tiny span still has a pair of knots.
        let at = |q: f64| jitter(0, 0, Stream::JitterRotate, q, 0.01);
        assert!((at(0.0) - at(1.0)).abs() < 1e-12);
        assert_eq!(jitter(0, 0, Stream::JitterX, f64::NAN, 1.0), 0.0);
    }

    #[test]
    fn flicker_dips_stay_a_second_apart_across_the_seam() {
        for seed in 0..20 {
            for cycle in [1.0, 1.5, 2.0, 3.7, 10.0] {
                let n = 20_000;
                let on: Vec<bool> = (0..n)
                    .map(|i| {
                        let q = f64::from(i) / f64::from(n);
                        flicker_dip(seed, 0, q, cycle, FLICKER_DIP_SECS) > 0.5
                    })
                    .collect();
                // Dip starts over two cycles, so the seam is crossed.
                let mut starts = Vec::new();
                for k in 0..2 * n {
                    let (a, b) = (
                        on[(k as usize) % on.len()],
                        on[((k + 1) as usize) % on.len()],
                    );
                    if !a && b {
                        starts.push(f64::from(k + 1) / f64::from(n) * cycle);
                    }
                }
                let want = 2 * (cycle / FLICKER_GAP_SECS).floor() as usize;
                assert_eq!(starts.len(), want, "seed {seed}, cycle {cycle}");
                for w in starts.windows(2) {
                    assert!(w[1] - w[0] >= FLICKER_GAP_SECS - 1e-3, "{starts:?}");
                }
            }
        }
        // Too short a cycle: no dips at all.
        assert!((0..100).all(|i| flicker_dip(0, 0, f64::from(i) / 100.0, 0.9, 0.05) == 0.0));
        assert_eq!(flicker_dip(0, 0, 0.5, f64::NAN, 0.05), 0.0);
    }

    #[test]
    fn sparkle_swells_once_per_cycle_for_a_fifth_of_a_second() {
        for unit in 0..10 {
            let at = |q: f64| sparkle(3, unit, q, 2.0);
            let samples: Vec<f64> = (0..2000).map(|i| at(f64::from(i) / 2000.0)).collect();
            let on = samples.iter().filter(|v| **v > 0.0).count();
            // 0.2 s of 2 s: a tenth of the samples (the bump's ends are 0).
            assert!((195..=201).contains(&on), "{on}");
            assert!(samples.iter().any(|v| *v > 0.99));
            assert!((at(0.0) - at(1.0)).abs() < 1e-12);
        }
        // A cycle shorter than the swell: the bump spans the cycle.
        assert!((sparkle(0, 0, 0.0, 0.1) - sparkle(0, 0, 1.0 - 1e-12, 0.1)).abs() < 1e-6);
        assert_eq!(sparkle(0, 0, 0.5, 0.0), 0.0);
        assert_eq!(sparkle(0, 0, f64::NAN, 1.0), 0.0);
    }

    #[test]
    fn scatter_offsets_are_between_half_and_full_amplitude() {
        for unit in 0..200 {
            let r = scatter(9, unit).length();
            assert!((0.5..=1.0).contains(&r), "{r}");
        }
        assert_ne!(scatter(9, 0), scatter(9, 1));
        assert_eq!(scatter(9, 0), scatter(9, 0));
    }
}
