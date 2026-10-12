//! Clip volume over time: a static gain or volume keyframes (AU-005,
//! AU-008, TR-027).
//!
//! Keyframes are anchored in source time. Between two keyframes the gain
//! follows the earlier keyframe's interpolation: `Linear` moves in the dB
//! domain, except on a segment that touches −∞, which moves in linear
//! amplitude so it doesn't jump; `Hold` keeps the earlier value up to the
//! next keyframe. Before the first keyframe and after the last the gain is
//! that keyframe's.
//!
//! The mixer evaluates the envelope at every sample and smooths it with a
//! centred 10 ms moving average ([`Smoother`]). Being centred, it doesn't
//! lag: a linear-amplitude segment longer than the window comes out
//! unchanged, a dB-linear one within a hair of it, and only corners are
//! rounded. Steps (Hold) become 10 ms ramps and keyframes denser than one per
//! 10 ms are averaged, so neither clicks (AU-008). The average is summed in
//! fixed point, so a sample's value doesn't depend on how the output was cut
//! into blocks.

use std::sync::Arc;

use freecut_timeline::model::Interp;
use freecut_timeline::time::{SampleRate, Tick};

/// How the gain moves from one keyframe to the next (TR-027).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum KeyInterp {
    /// Straight in dB; in linear amplitude when either end is −∞.
    #[default]
    Linear,
    /// The keyframe's value until the next keyframe, then a step.
    Hold,
    /// Like `Linear`, but the segment's progress first passes through one
    /// of TR-031's easing curves (`Interp::ease`). `Linear` and `Hold`
    /// are never wrapped here.
    Eased(Interp),
}

/// One volume keyframe (AU-008).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainKey {
    /// Source time of the keyframe.
    pub at: Tick,
    /// Gain in decibels; `-∞` is silence. NaN and `+∞` count as silence.
    pub db: f32,
    /// Interpolation towards the next keyframe.
    pub interp: KeyInterp,
}

impl GainKey {
    pub fn new(at: Tick, db: f32, interp: KeyInterp) -> GainKey {
        GainKey { at, db, interp }
    }

    /// Linear amplitude of the keyframe's gain (0 for silence).
    fn amplitude(&self) -> f64 {
        if self.db.is_finite() {
            10f64.powf(f64::from(self.db) / 20.0)
        } else {
            0.0
        }
    }
}

/// A non-empty list of volume keyframes, sorted by time. Cloning is cheap.
#[derive(Debug, Clone)]
pub struct GainKeys(Arc<[GainKey]>);

impl PartialEq for GainKeys {
    fn eq(&self, other: &GainKeys) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl GainKeys {
    /// The keyframes sorted by time (keyframes at the same time keep their
    /// order: the gain steps from the first to the last of them); `None`
    /// when there are none.
    pub fn new(keys: impl Into<Vec<GainKey>>) -> Option<GainKeys> {
        let mut keys = keys.into();
        if keys.is_empty() {
            return None;
        }
        keys.sort_by_key(|k| k.at);
        Some(GainKeys(keys.into()))
    }

    pub fn keys(&self) -> &[GainKey] {
        &self.0
    }

    /// The linear amplitude at source time `at`, exactly as keyed (without
    /// the mixer's smoothing).
    pub fn amplitude_at(&self, at: Tick) -> f64 {
        let mut cursor = Cursor::default();
        cursor.amplitude(&self.0, i128::from(at.raw()))
    }

    /// Whether every keyframe is silent.
    pub fn is_silent(&self) -> bool {
        self.0.iter().all(|k| k.amplitude() == 0.0)
    }
}

/// A position in a key list for evaluation at increasing times.
#[derive(Debug, Clone, Copy, Default)]
struct Cursor {
    /// How many keyframes are at or before the last time evaluated.
    next: usize,
}

impl Cursor {
    fn amplitude(&mut self, keys: &[GainKey], t: i128) -> f64 {
        // Times usually increase; anything else starts over.
        if self.next > 0
            && keys
                .get(self.next - 1)
                .is_some_and(|k| i128::from(k.at.raw()) > t)
        {
            self.next = 0;
        }
        while keys
            .get(self.next)
            .is_some_and(|k| i128::from(k.at.raw()) <= t)
        {
            self.next += 1;
        }
        let (Some(a), Some(b)) = (
            self.next.checked_sub(1).and_then(|i| keys.get(i)),
            keys.get(self.next),
        ) else {
            // Before the first keyframe or after the last.
            return keys
                .get(self.next.min(keys.len().saturating_sub(1)))
                .map_or(1.0, GainKey::amplitude);
        };
        if a.interp == KeyInterp::Hold {
            return a.amplitude();
        }
        // `a.at <= t < b.at`, so the span is positive.
        let span = i128::from(b.at.raw()) - i128::from(a.at.raw());
        let p = (t - i128::from(a.at.raw())) as f64 / span as f64;
        let p = match a.interp {
            KeyInterp::Eased(interp) => interp.ease(p),
            KeyInterp::Linear | KeyInterp::Hold => p,
        };
        if a.db.is_finite() && b.db.is_finite() {
            let db = f64::from(a.db) + (f64::from(b.db) - f64::from(a.db)) * p;
            10f64.powf(db / 20.0)
        } else {
            let (x, y) = (a.amplitude(), b.amplitude());
            // An overshooting curve may pass below silence; clamp there.
            (x + (y - x) * p).max(0.0)
        }
    }
}

/// A clip's volume (AU-005, AU-008).
#[derive(Debug, Clone, PartialEq)]
pub enum GainEnvelope {
    /// One linear amplitude for the whole clip; 0 is silent. Negative and
    /// non-finite values count as 0.
    Static(f32),
    /// Volume keyframes, which replace the static volume.
    Keyframes(GainKeys),
}

impl Default for GainEnvelope {
    fn default() -> GainEnvelope {
        GainEnvelope::Static(1.0)
    }
}

/// A static gain of that linear amplitude.
impl From<f32> for GainEnvelope {
    fn from(linear: f32) -> GainEnvelope {
        GainEnvelope::Static(linear)
    }
}

/// A static envelope of exactly that linear amplitude.
impl PartialEq<f32> for GainEnvelope {
    fn eq(&self, linear: &f32) -> bool {
        matches!(self, GainEnvelope::Static(g) if g == linear)
    }
}

impl GainEnvelope {
    /// The linear amplitude at source time `at` (without the mixer's
    /// smoothing).
    pub fn amplitude_at(&self, at: Tick) -> f64 {
        match self {
            GainEnvelope::Static(g) => f64::from(crate::gain::sanitize(*g)),
            GainEnvelope::Keyframes(keys) => keys.amplitude_at(at),
        }
    }

    /// Whether the clip is silent all the way through.
    pub fn is_silent(&self) -> bool {
        match self {
            GainEnvelope::Static(g) => crate::gain::sanitize(*g) == 0.0,
            GainEnvelope::Keyframes(keys) => keys.is_silent(),
        }
    }

    pub(crate) fn keys(&self) -> Option<&GainKeys> {
        match self {
            GainEnvelope::Static(_) => None,
            GainEnvelope::Keyframes(keys) => Some(keys),
        }
    }
}

/// Total width of the smoothing window.
const SMOOTH_SECS: f64 = 0.01;
/// Upper bound on the half-window, for absurd sample rates.
const MAX_HALF: usize = 8_192;
/// Fixed-point scale of the smoothing sums.
const ONE: f64 = (1u64 << 32) as f64;
/// Amplitudes are capped here (+96 dB) before summing, so sums can't
/// overflow.
const MAX_AMPLITUDE: f64 = 65_536.0;

/// Evaluates keyframe envelopes per sample with the centred moving average.
#[derive(Debug, Default)]
pub(crate) struct Smoother {
    /// Samples on each side of the centre.
    half: usize,
    /// Ticks per sample.
    st: i64,
    sums: Vec<i128>,
}

impl Smoother {
    pub(crate) fn new(rate: SampleRate) -> Smoother {
        let half = (f64::from(rate.hz()) * SMOOTH_SECS / 2.0).round() as usize;
        Smoother {
            half: half.clamp(1, MAX_HALF),
            st: crate::samples::sample_ticks(rate),
            sums: Vec::new(),
        }
    }

    /// Fills `out` with the smoothed amplitude of `keys` for `out.len()`
    /// samples, the first starting at source time `at` and each `st` ticks
    /// after the one before.
    pub(crate) fn fill(&mut self, keys: &GainKeys, at: Tick, out: &mut [f64]) {
        let n = out.len();
        if n == 0 {
            return;
        }
        let width = 2 * self.half + 1;
        let first = i128::from(at.raw()) - (self.half as i128) * i128::from(self.st);
        self.sums.clear();
        self.sums.reserve(n + width);
        self.sums.push(0);
        let mut cursor = Cursor::default();
        let mut sum = 0i128;
        for j in 0..n + width - 1 {
            let t = first + j as i128 * i128::from(self.st);
            sum += fixed(cursor.amplitude(&keys.0, t));
            self.sums.push(sum);
        }
        let scale = width as f64 * ONE;
        for (i, o) in out.iter_mut().enumerate() {
            *o = (self.sums[i + width] - self.sums[i]) as f64 / scale;
        }
    }

    /// The smoothed amplitude of the one sample starting at source time
    /// `at`.
    pub(crate) fn at(&mut self, keys: &GainKeys, at: Tick) -> f64 {
        let mut one = [0.0];
        self.fill(keys, at, &mut one);
        one[0]
    }
}

/// An amplitude in fixed point.
fn fixed(amplitude: f64) -> i128 {
    // In range by the clamp, so the cast is exact up to rounding.
    (amplitude.clamp(0.0, MAX_AMPLITUDE) * ONE).round() as i128
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(raw: i64) -> Tick {
        Tick::from_raw(raw)
    }

    fn keys(list: &[(i64, f32, KeyInterp)]) -> GainKeys {
        GainKeys::new(
            list.iter()
                .map(|&(at, db, interp)| GainKey::new(t(at), db, interp))
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    const L: KeyInterp = KeyInterp::Linear;
    const H: KeyInterp = KeyInterp::Hold;
    const SILENT: f32 = f32::NEG_INFINITY;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn db(a: f64) -> f64 {
        20.0 * a.log10()
    }

    #[test]
    fn no_keys_is_no_envelope() {
        assert!(GainKeys::new(Vec::new()).is_none());
    }

    #[test]
    fn one_key_is_a_constant() {
        let k = keys(&[(500, -6.0, L)]);
        let a = 10f64.powf(-6.0 / 20.0);
        for at in [-1_000_000, 0, 500, 501, i64::MAX] {
            assert!(close(k.amplitude_at(t(at)), a));
        }
    }

    #[test]
    fn linear_segments_move_in_decibels() {
        let k = keys(&[(0, 0.0, L), (1000, -20.0, L), (2000, -20.0, L)]);
        assert_eq!(k.amplitude_at(t(-5)), 1.0);
        assert_eq!(k.amplitude_at(t(0)), 1.0);
        // Half-way in time is half-way in dB, not in amplitude.
        assert!(close(db(k.amplitude_at(t(500))), -10.0));
        assert!(close(db(k.amplitude_at(t(250))), -5.0));
        assert!(close(k.amplitude_at(t(1000)), 0.1));
        assert!(close(k.amplitude_at(t(1500)), 0.1));
        assert!(close(k.amplitude_at(t(99_999)), 0.1));
    }

    #[test]
    fn eased_segments_follow_their_curve_in_decibels() {
        use freecut_timeline::model::{EaseCurve, EaseDir};
        let quad_in = KeyInterp::Eased(Interp::Ease {
            curve: EaseCurve::Quad,
            dir: EaseDir::In,
        });
        let k = keys(&[(0, 0.0, quad_in), (1000, -20.0, L)]);
        // Quad in at half-way is a quarter of the way in dB.
        assert!(close(db(k.amplitude_at(t(500))), -5.0));
        assert!(close(k.amplitude_at(t(0)), 1.0));
        assert!(close(k.amplitude_at(t(1000)), 0.1));
        // An overshoot into silence never goes below silence.
        let k = keys(&[
            (0, 0.0, KeyInterp::Eased(Interp::OvershootOut)),
            (1000, SILENT, L),
        ]);
        for at in (0..=1000).step_by(50) {
            assert!(k.amplitude_at(t(at)) >= 0.0);
        }
    }

    #[test]
    fn segments_touching_silence_move_in_amplitude() {
        let k = keys(&[(0, SILENT, L), (1000, 0.0, L), (2000, SILENT, L)]);
        assert_eq!(k.amplitude_at(t(0)), 0.0);
        assert!(close(k.amplitude_at(t(250)), 0.25));
        assert!(close(k.amplitude_at(t(500)), 0.5));
        assert_eq!(k.amplitude_at(t(1000)), 1.0);
        assert!(close(k.amplitude_at(t(1750)), 0.25));
        assert_eq!(k.amplitude_at(t(2000)), 0.0);
        assert_eq!(k.amplitude_at(t(5000)), 0.0);
        // −6 dB down to silence: a straight line in amplitude.
        let k = keys(&[(0, -6.0, L), (100, SILENT, L)]);
        let a = 10f64.powf(-6.0 / 20.0);
        assert!(close(k.amplitude_at(t(50)), a / 2.0));
        // NaN and +∞ are silence too.
        let k = keys(&[(0, f32::NAN, L), (100, f32::INFINITY, L)]);
        assert!(k.is_silent());
        assert_eq!(k.amplitude_at(t(50)), 0.0);
    }

    #[test]
    fn hold_steps_at_the_next_key() {
        let k = keys(&[
            (0, 0.0, H),
            (1000, -20.0, L),
            (2000, SILENT, H),
            (3000, 0.0, L),
        ]);
        assert_eq!(k.amplitude_at(t(0)), 1.0);
        assert_eq!(k.amplitude_at(t(999)), 1.0);
        assert!(close(k.amplitude_at(t(1000)), 0.1));
        assert!(close(k.amplitude_at(t(1500)), 0.05));
        assert_eq!(k.amplitude_at(t(2000)), 0.0);
        assert_eq!(k.amplitude_at(t(2999)), 0.0);
        assert_eq!(k.amplitude_at(t(3000)), 1.0);
    }

    #[test]
    fn keys_are_sorted_and_coincident_keys_step() {
        let k = keys(&[(1000, -20.0, L), (0, 0.0, L), (1000, 0.0, L)]);
        assert_eq!(
            k.keys().iter().map(|k| k.at.raw()).collect::<Vec<_>>(),
            [0, 1000, 1000]
        );
        // Up to the first of the coincident keys, then the last one's value.
        assert!(close(db(k.amplitude_at(t(500))), -10.0));
        assert_eq!(k.amplitude_at(t(1000)), 1.0);
    }

    #[test]
    fn evaluation_out_of_order_matches_in_order() {
        let k = keys(&[
            (0, 0.0, L),
            (100, -12.0, H),
            (200, SILENT, L),
            (300, 3.0, L),
        ]);
        let mut cursor = Cursor::default();
        for at in [250, 50, 299, 0, 150, 1000, -3, 210] {
            assert_eq!(
                cursor.amplitude(k.keys(), at),
                k.amplitude_at(t(at as i64)),
                "at {at}"
            );
        }
    }

    #[test]
    fn static_envelopes() {
        assert_eq!(GainEnvelope::from(0.5).amplitude_at(t(9)), 0.5);
        assert!(GainEnvelope::Static(0.0).is_silent());
        assert!(GainEnvelope::Static(f32::NAN).is_silent());
        assert_eq!(GainEnvelope::Static(-2.0).amplitude_at(t(0)), 0.0);
        assert!(!GainEnvelope::default().is_silent());
        assert_eq!(GainEnvelope::default(), 1.0);
        assert_ne!(GainEnvelope::from(0.5), 1.0);
        let k = GainEnvelope::Keyframes(keys(&[(0, SILENT, L), (10, 0.0, L)]));
        assert!(!k.is_silent());
        assert_eq!(k.amplitude_at(t(5)), 0.5);
    }

    fn smoother() -> Smoother {
        Smoother::new(SampleRate::new(48_000).unwrap())
    }

    const ST: i64 = 14_700;

    #[test]
    fn smoothing_keeps_constants_and_long_linear_ramps_exact() {
        let mut s = smoother();
        let k = keys(&[(0, -6.0, L)]);
        let mut out = vec![0.0; 1000];
        s.fill(&k, t(-500 * ST), &mut out);
        let a = k.amplitude_at(t(0));
        assert!(out.iter().all(|&v| (v - a).abs() < 1e-9));
        // A 1 s ramp in amplitude: the centred average is the ramp itself
        // away from its corners.
        let k = keys(&[(0, SILENT, L), (48_000 * ST, 0.0, L)]);
        s.fill(&k, t(1000 * ST), &mut out);
        for (i, v) in out.iter().enumerate() {
            let exact = k.amplitude_at(t((1000 + i as i64) * ST));
            assert!((v - exact).abs() < 1e-9, "sample {i}: {v} vs {exact}");
        }
    }

    #[test]
    fn smoothing_does_not_lag() {
        // A 0 → −40 dB ramp over 100 ms: the smoothed curve stays within a
        // tenth of a dB of the exact one in the middle (the average of an
        // exponential runs a hair high, but never behind).
        let mut s = smoother();
        let k = keys(&[(0, 0.0, L), (4_800 * ST, -40.0, L)]);
        let mut out = vec![0.0; 3_000];
        s.fill(&k, t(1_000 * ST), &mut out);
        for (i, v) in out.iter().enumerate() {
            let exact = db(k.amplitude_at(t((1_000 + i as i64) * ST)));
            assert!((db(*v) - exact).abs() < 0.1, "sample {i}");
        }
    }

    #[test]
    fn a_hold_step_becomes_a_centred_ramp() {
        let mut s = smoother();
        // From 1 to 0.1 at sample 1000.
        let k = keys(&[(0, 0.0, H), (1_000 * ST, -20.0, H)]);
        let mut out = vec![0.0; 2_000];
        s.fill(&k, t(0), &mut out);
        // Untouched until 240 samples before the step and from 240 after.
        assert!(out[..760].iter().all(|&v| v == 1.0));
        assert!(out[1_241..].iter().all(|&v| close(v, 0.1)));
        // Half-way exactly at the step, so it doesn't lag.
        assert!(close(out[1_000], (241.0 * 0.1 + 240.0) / 481.0));
        // Monotonic, with steps no larger than 0.9 / 481.
        assert!(out.windows(2).all(|w| w[1] <= w[0]));
        let step = out.windows(2).map(|w| w[0] - w[1]).fold(0.0, f64::max);
        assert!(step <= 0.9 / 481.0 + 1e-12, "{step}");
    }

    #[test]
    fn dense_keys_are_averaged() {
        // Alternating 0 dB / −∞ every 2 ms (96 samples): the smoothed gain
        // hovers around the mean instead of buzzing.
        let mut s = smoother();
        let list: Vec<GainKey> = (0..200)
            .map(|i| GainKey::new(t(i * 96 * ST), if i % 2 == 0 { 0.0 } else { SILENT }, H))
            .collect();
        let k = GainKeys::new(list).unwrap();
        let mut out = vec![0.0; 9_000];
        s.fill(&k, t(500 * ST), &mut out);
        let (lo, hi) = out
            .iter()
            .fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        assert!(lo > 0.4 && hi < 0.6, "{lo} .. {hi}");
        let step = out
            .windows(2)
            .map(|w| (w[0] - w[1]).abs())
            .fold(0.0, f64::max);
        assert!(step <= 1.0 / 481.0 + 1e-12);
    }

    #[test]
    fn smoothing_is_independent_of_blocks() {
        let mut s = smoother();
        let k = keys(&[
            (0, 0.0, L),
            (700 * ST, -30.0, H),
            (900 * ST, SILENT, L),
            (2_000 * ST, 6.0, L),
        ]);
        let mut whole = vec![0.0; 3_000];
        s.fill(&k, t(0), &mut whole);
        let mut pieces = Vec::new();
        let mut at = 0;
        for n in [1, 7, 480, 13, 999, 1_500] {
            let mut out = vec![0.0; n];
            s.fill(&k, t(at * ST), &mut out);
            pieces.extend(out);
            at += n as i64;
        }
        assert_eq!(&pieces[..], &whole[..pieces.len()]);
        assert_eq!(s.at(&k, t(1234 * ST)), whole[1234]);
    }
}
