//! Clip gain, fades, pan and the stream-mix factor (AU-005, AU-006,
//! AU-013, AU-014, AU-038).

use freecut_timeline::model::GainDb;
use freecut_timeline::time::Tick;

use crate::source::MixInput;

/// Linear amplitude for a gain in decibels; `-∞` (and NaN) is silence.
pub fn db_to_amplitude(db: f32) -> f32 {
    if db.is_nan() || db == f32::NEG_INFINITY {
        return 0.0;
    }
    10f32.powf(db / 20.0)
}

/// Linear amplitude of a stored clip gain (AU-005): `10^(dB / 20)`, and 0
/// for −∞. Computed in `f64` from the exact hundredths of a decibel.
pub fn gain_amplitude(gain: GainDb) -> f32 {
    if gain.is_silent() {
        return 0.0;
    }
    // hundredths / 100 dB, then / 20 for amplitude.
    10f64.powf(f64::from(gain.hundredths()) / 2_000.0) as f32
}

/// The fade lengths actually rendered for a clip of `duration` (AU-006):
/// `in = min(fade_in, duration)`, `out = min(fade_out, duration − in)`.
/// Negative stored lengths count as 0.
pub fn effective_fades(duration: Tick, fade_in: Tick, fade_out: Tick) -> (Tick, Tick) {
    let duration = duration.max(Tick::ZERO);
    let fade_in = fade_in.clamp(Tick::ZERO, duration);
    // Both are within [0, duration], so the difference can't overflow.
    let rest = Tick::from_raw(duration.raw() - fade_in.raw());
    (fade_in, fade_out.clamp(Tick::ZERO, rest))
}

/// The shape of a fade (AU-006). `p` runs from 0 at the silent end of the
/// fade to 1 at full level; a fade-out mirrors the fade-in shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FadeCurve {
    /// Linear amplitude: `p`.
    #[default]
    Linear,
    /// S-curve: `½ − ½·cos(π·p)`.
    Smooth,
    /// Equal-power quarter sine, quick at first: `sin(π·p / 2)`.
    Fast,
    /// Gradual at first: `1 − cos(π·p / 2)`.
    Slow,
}

impl FadeCurve {
    /// The gain at progress `p` through the fade; `p` is clamped to
    /// `[0, 1]`, and both ends are exact (0 and 1).
    pub fn gain(self, p: f64) -> f64 {
        if p.is_nan() || p <= 0.0 {
            return 0.0;
        }
        if p >= 1.0 {
            return 1.0;
        }
        use std::f64::consts::{FRAC_PI_2, PI};
        match self {
            FadeCurve::Linear => p,
            FadeCurve::Smooth => 0.5 - 0.5 * (PI * p).cos(),
            FadeCurve::Fast => (FRAC_PI_2 * p).sin(),
            FadeCurve::Slow => 1.0 - (FRAC_PI_2 * p).cos(),
        }
    }
}

/// One edge's fade as stored (AU-006): its length on the timeline and its
/// curve. The mixer clamps lengths to the clip ([`effective_fades`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Fade {
    pub len: Tick,
    pub curve: FadeCurve,
}

impl Fade {
    /// No fade.
    pub const NONE: Fade = Fade {
        len: Tick::ZERO,
        curve: FadeCurve::Linear,
    };

    pub fn new(len: Tick, curve: FadeCurve) -> Fade {
        Fade { len, curve }
    }

    /// A linear-amplitude fade of `len`.
    pub fn linear(len: Tick) -> Fade {
        Fade::new(len, FadeCurve::Linear)
    }
}

/// A linear fade of that length.
impl From<Tick> for Fade {
    fn from(len: Tick) -> Fade {
        Fade::linear(len)
    }
}

/// Left and right gains of the balance law (AU-013, AU-014) for `pan` in
/// `[-1, 1]` (−1 fully left, +1 fully right): at centre both channels are at
/// unity, and moving towards one side lowers the other channel linearly, to
/// silence at ±1. Values outside the range are clamped; NaN is centre.
pub fn balance_gains(pan: f32) -> [f32; 2] {
    let p = if pan.is_nan() {
        0.0
    } else {
        pan.clamp(-1.0, 1.0)
    };
    if p > 0.0 {
        [1.0 - p, 1.0]
    } else {
        [1.0, 1.0 + p]
    }
}

/// The gain each stream gets when `streams` streams of one clip are summed
/// ("All streams mixed", AU-038): `1/√n`, so the sum stays close to one
/// stream's level. One stream (or none) plays at unity.
pub fn stream_mix_factor(streams: usize) -> f32 {
    if streams <= 1 {
        return 1.0;
    }
    (1.0 / (streams as f64).sqrt()) as f32
}

/// A linear gain as given to the mixer: negative, NaN and infinite values
/// are silence.
pub(crate) fn sanitize(linear: f32) -> f32 {
    if linear.is_finite() && linear > 0.0 {
        linear
    } else {
        0.0
    }
}

/// The fades of one input, evaluated per sample.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Fades {
    start: i64,
    end: i64,
    fade_in: i64,
    fade_out: i64,
    in_curve: FadeCurve,
    out_curve: FadeCurve,
}

impl Fades {
    pub(crate) fn new(input: &MixInput) -> Fades {
        let (fade_in, fade_out) = effective_fades(
            input.range.duration(),
            input.fade_in.len,
            input.fade_out.len,
        );
        Fades {
            start: input.range.start().raw(),
            end: input.range.end().raw(),
            fade_in: fade_in.raw(),
            fade_out: fade_out.raw(),
            in_curve: input.fade_in.curve,
            out_curve: input.fade_out.curve,
        }
    }

    /// Whether the fades leave every sample at full level.
    pub(crate) fn is_flat(&self) -> bool {
        self.fade_in == 0 && self.fade_out == 0
    }

    /// `volume` times the fade gains for the sample covering `[t, t + len)`.
    ///
    /// The fade-in is evaluated at the sample's start and the fade-out at its
    /// end, so a fade-out is the exact mirror of a fade-in: the clip's first
    /// sample of a fade-in and its last sample of a fade-out are both 0.
    pub(crate) fn apply(&self, volume: f64, t: i64, len: i64) -> f32 {
        let mut g = volume;
        if self.fade_in > 0 {
            let p = (t.saturating_sub(self.start)) as f64 / self.fade_in as f64;
            g *= self.in_curve.gain(p);
        }
        if self.fade_out > 0 {
            let p = (self.end.saturating_sub(t.saturating_add(len))) as f64 / self.fade_out as f64;
            g *= self.out_curve.gain(p);
        }
        g as f32
    }
}

#[cfg(test)]
mod tests {
    use freecut_timeline::time::TimeRange;

    use super::*;
    use crate::source::SourceKey;

    fn t(raw: i64) -> Tick {
        Tick::from_raw(raw)
    }

    fn input(start: i64, dur: i64, fade_in: Fade, fade_out: Fade) -> MixInput {
        MixInput {
            fade_in,
            fade_out,
            ..MixInput::new(
                1,
                SourceKey {
                    asset: 1,
                    stream: 0,
                },
                TimeRange::new(t(start), t(dur)).unwrap(),
                Tick::ZERO,
            )
        }
    }

    fn linear(len: i64) -> Fade {
        Fade::linear(t(len))
    }

    #[test]
    fn decibels_convert_to_amplitude() {
        assert_eq!(db_to_amplitude(0.0), 1.0);
        assert!((db_to_amplitude(-6.0206) - 0.5).abs() < 1e-4);
        assert!((db_to_amplitude(20.0) - 10.0).abs() < 1e-4);
        assert_eq!(db_to_amplitude(f32::NEG_INFINITY), 0.0);
        assert_eq!(db_to_amplitude(f32::NAN), 0.0);
    }

    #[test]
    fn stored_gains_convert_to_amplitude() {
        let amp = |h| gain_amplitude(GainDb::from_hundredths(h));
        assert_eq!(gain_amplitude(GainDb::ZERO), 1.0);
        assert_eq!(gain_amplitude(GainDb::SILENT), 0.0);
        assert!((gain_amplitude(GainDb::MAX) - 10.0).abs() < 1e-6);
        assert!((gain_amplitude(GainDb::MIN) - 0.001).abs() < 1e-9);
        assert!((amp(-2_000) - 0.1).abs() < 1e-7);
        assert!((amp(-602) - 0.5).abs() < 1e-3);
        // Agrees with the float conversion the UI edge uses.
        assert!((amp(-350) - db_to_amplitude(-3.5)).abs() < 1e-6);
    }

    #[test]
    fn fades_clamp_to_the_clip() {
        let e = |d, i, o| effective_fades(t(d), t(i), t(o));
        assert_eq!(e(100, 30, 40), (t(30), t(40)));
        // Trimmed shorter than the fades: fade-in wins, fade-out gets the rest.
        assert_eq!(e(100, 80, 40), (t(80), t(20)));
        assert_eq!(e(100, 150, 40), (t(100), t(0)));
        assert_eq!(e(100, -5, 500), (t(0), t(100)));
    }

    #[test]
    fn linear_fade_shapes() {
        // A clip of 10 samples of 10 ticks each with 4-sample fades.
        let f = Fades::new(&input(1000, 100, linear(40), linear(40)));
        let gains: Vec<f32> = (0..10).map(|k| f.apply(0.5, 1000 + k * 10, 10)).collect();
        assert_eq!(
            gains,
            [0.0, 0.125, 0.25, 0.375, 0.5, 0.5, 0.375, 0.25, 0.125, 0.0]
        );
        // The fade-out mirrors the fade-in.
        for k in 0..10 {
            assert_eq!(gains[k], gains[9 - k]);
        }
    }

    #[test]
    fn curve_shapes() {
        use std::f64::consts::FRAC_1_SQRT_2;
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        for curve in [
            FadeCurve::Linear,
            FadeCurve::Smooth,
            FadeCurve::Fast,
            FadeCurve::Slow,
        ] {
            // Exact ends, clamped outside, monotonic in between.
            assert_eq!(curve.gain(0.0), 0.0, "{curve:?}");
            assert_eq!(curve.gain(1.0), 1.0, "{curve:?}");
            assert_eq!(curve.gain(-0.5), 0.0, "{curve:?}");
            assert_eq!(curve.gain(1.5), 1.0, "{curve:?}");
            assert_eq!(curve.gain(f64::NAN), 0.0, "{curve:?}");
            let samples: Vec<f64> = (0..=100)
                .map(|k| curve.gain(f64::from(k) / 100.0))
                .collect();
            assert!(samples.windows(2).all(|w| w[1] > w[0]), "{curve:?}");
        }
        assert!(close(FadeCurve::Linear.gain(0.25), 0.25));
        // Smooth: an S-curve, symmetric about the middle.
        assert!(close(FadeCurve::Smooth.gain(0.5), 0.5));
        assert!(close(
            FadeCurve::Smooth.gain(0.25),
            0.5 - 0.5 * FRAC_1_SQRT_2
        ));
        assert!(close(
            FadeCurve::Smooth.gain(0.2) + FadeCurve::Smooth.gain(0.8),
            1.0
        ));
        // Fast: equal power, so a fade-in and the mirrored fade-out keep
        // constant power where they cross.
        assert!(close(FadeCurve::Fast.gain(0.5), FRAC_1_SQRT_2));
        let (a, b) = (FadeCurve::Fast.gain(0.3), FadeCurve::Fast.gain(0.7));
        assert!(close(a * a + b * b, 1.0));
        // Fast is quick at first, Slow gradual at first.
        assert!(FadeCurve::Fast.gain(0.1) > 0.1);
        assert!(FadeCurve::Slow.gain(0.1) < 0.1);
        assert!(close(FadeCurve::Slow.gain(0.5), 1.0 - FRAC_1_SQRT_2));
    }

    #[test]
    fn fade_outs_mirror_their_curve() {
        for curve in [FadeCurve::Smooth, FadeCurve::Fast, FadeCurve::Slow] {
            let fade = Fade::new(t(100), curve);
            let f = Fades::new(&input(0, 1000, fade, fade));
            for k in 0..10 {
                let rise = f.apply(1.0, k * 10, 10);
                let fall = f.apply(1.0, 1000 - (k + 1) * 10, 10);
                assert_eq!(rise, fall, "{curve:?} sample {k}");
                assert_eq!(rise, curve.gain(k as f64 / 10.0) as f32);
            }
            // Full level between the fades.
            assert_eq!(f.apply(1.0, 500, 10), 1.0);
        }
        // Each edge keeps its own curve.
        let f = Fades::new(&input(
            0,
            1000,
            Fade::new(t(100), FadeCurve::Fast),
            Fade::new(t(100), FadeCurve::Slow),
        ));
        assert_eq!(f.apply(1.0, 50, 10), FadeCurve::Fast.gain(0.5) as f32);
        assert_eq!(f.apply(1.0, 940, 10), FadeCurve::Slow.gain(0.5) as f32);
    }

    #[test]
    fn overlong_fades_are_clamped_when_evaluated() {
        // Fade-in longer than the clip: one ramp over the whole clip, no fade-out.
        let f = Fades::new(&input(0, 100, linear(1000), linear(1000)));
        assert_eq!(f.apply(1.0, 0, 10), 0.0);
        assert_eq!(f.apply(1.0, 50, 10), 0.5);
        assert_eq!(f.apply(1.0, 90, 10), 0.9);
    }

    #[test]
    fn balance_law() {
        assert_eq!(balance_gains(0.0), [1.0, 1.0]);
        assert_eq!(balance_gains(1.0), [0.0, 1.0]);
        assert_eq!(balance_gains(-1.0), [1.0, 0.0]);
        assert_eq!(balance_gains(0.25), [0.75, 1.0]);
        assert_eq!(balance_gains(-0.5), [1.0, 0.5]);
        assert_eq!(balance_gains(3.0), [0.0, 1.0]);
        assert_eq!(balance_gains(-3.0), [1.0, 0.0]);
        assert_eq!(balance_gains(f32::NAN), [1.0, 1.0]);
    }

    #[test]
    fn stream_mix_keeps_the_level_of_one_stream() {
        assert_eq!(stream_mix_factor(0), 1.0);
        assert_eq!(stream_mix_factor(1), 1.0);
        assert!((stream_mix_factor(2) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-7);
        // About −3 dB for two streams.
        let db = 20.0 * stream_mix_factor(2).log10();
        assert!((db + 3.0103).abs() < 1e-3, "{db}");
        assert!((stream_mix_factor(4) - 0.5).abs() < 1e-7);
        // n streams of equal power sum to the power of one.
        for n in 2..8 {
            let f = f64::from(stream_mix_factor(n));
            assert!((f * f * n as f64 - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn invalid_gains_are_silence() {
        assert_eq!(sanitize(0.5), 0.5);
        assert_eq!(sanitize(-1.0), 0.0);
        assert_eq!(sanitize(f32::NAN), 0.0);
        assert_eq!(sanitize(f32::INFINITY), 0.0);
    }
}
