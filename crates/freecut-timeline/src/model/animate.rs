//! Keyframed values (TR-027, the subset AU-008 needs today): a static value
//! plus optional keyframes, anchored in the clip's source time.
//!
//! Everything here is exact integer data. Interpolation between keys is
//! done by each value type ([`Interpolate`]) from an exact rational
//! progress, and rounding to the type's step happens only when a value is
//! evaluated, never when it is stored.

use std::sync::Arc;

use crate::time::{Rounding, Tick, div_round};

/// How the segment that starts at a key reaches the next key (TR-027,
/// TR-031). Also the easing of clip animations (FX-018).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Interp {
    /// Straight from this key's value to the next key's.
    #[default]
    Linear,
    /// This key's value until the next key, then a jump.
    Hold,
    /// Ease in, out or in-out at one of four strengths (TR-031).
    Ease { curve: EaseCurve, dir: EaseDir },
    /// Passes the target slightly and settles back.
    OvershootOut,
    /// Pulls back slightly before leaving.
    OvershootIn,
    /// A springy wobble into place.
    ElasticOut,
    /// Bounces against the target value.
    BounceOut,
}

/// The strength of an [`Interp::Ease`] (TR-031): gentle, medium, strong and
/// very strong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EaseCurve {
    /// Gentle.
    Sine,
    /// Medium.
    Quad,
    /// Strong.
    Cubic,
    /// Very strong.
    Expo,
}

/// Which end of an [`Interp::Ease`] segment is slow (TR-031).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EaseDir {
    /// Slow start.
    In,
    /// Slow end.
    Out,
    /// Slow at both ends.
    InOut,
}

impl Interp {
    /// Every preset of TR-031's menu, in menu order (Custom curves arrive
    /// with TR-032).
    pub const PRESETS: [Interp; 18] = {
        use EaseCurve::{Cubic, Expo, Quad, Sine};
        use EaseDir::{In, InOut, Out};
        const fn e(curve: EaseCurve, dir: EaseDir) -> Interp {
            Interp::Ease { curve, dir }
        }
        [
            Interp::Linear,
            e(Quad, In),
            e(Quad, Out),
            e(Quad, InOut),
            Interp::Hold,
            e(Sine, In),
            e(Sine, Out),
            e(Sine, InOut),
            e(Cubic, In),
            e(Cubic, Out),
            e(Cubic, InOut),
            e(Expo, In),
            e(Expo, Out),
            e(Expo, InOut),
            Interp::OvershootOut,
            Interp::OvershootIn,
            Interp::ElasticOut,
            Interp::BounceOut,
        ]
    };

    /// The eased progress `e(p)` for segment progress `p` in `[0, 1]`
    /// (TR-031), from the functions' mathematical definitions: `e(0) = 0`,
    /// `e(1) = 1`. Overshoot, elastic and bounce leave `[0, 1]` in between
    /// by design. `Hold` stays at 0 until the end. Inputs outside `[0, 1]`
    /// are clamped. For the render and evaluation edge only: stored values
    /// never pass through floats.
    pub fn ease(self, p: f64) -> f64 {
        use std::f64::consts::PI;
        let p = if p.is_nan() { 0.0 } else { p.clamp(0.0, 1.0) };
        match self {
            Interp::Linear => p,
            Interp::Hold => {
                if p >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Interp::Ease { curve, dir } => {
                let ease_in = |x: f64| match curve {
                    EaseCurve::Sine => 1.0 - (x * PI / 2.0).cos(),
                    EaseCurve::Quad => x * x,
                    EaseCurve::Cubic => x * x * x,
                    EaseCurve::Expo => {
                        if x <= 0.0 {
                            0.0
                        } else {
                            (2f64).powf(10.0 * x - 10.0)
                        }
                    }
                };
                match dir {
                    EaseDir::In => ease_in(p),
                    EaseDir::Out => 1.0 - ease_in(1.0 - p),
                    EaseDir::InOut => {
                        if p < 0.5 {
                            ease_in(2.0 * p) / 2.0
                        } else {
                            1.0 - ease_in(2.0 - 2.0 * p) / 2.0
                        }
                    }
                }
            }
            Interp::OvershootOut => 1.0 - back_in(1.0 - p),
            Interp::OvershootIn => back_in(p),
            Interp::ElasticOut => {
                if p <= 0.0 || p >= 1.0 {
                    p
                } else {
                    let c4 = 2.0 * PI / 3.0;
                    (2f64).powf(-10.0 * p) * ((10.0 * p - 0.75) * c4).sin() + 1.0
                }
            }
            Interp::BounceOut => bounce_out(p),
        }
    }

    /// The same curve played backwards in time: `1 − e(1 − p)`. Ease out
    /// becomes ease in, overshoot out becomes overshoot in (FX-018's exits).
    pub fn ease_mirrored(self, p: f64) -> f64 {
        let p = if p.is_nan() { 0.0 } else { p.clamp(0.0, 1.0) };
        1.0 - self.ease(1.0 - p)
    }
}

/// The "back" curve: overshoot of about 10 % (the common constant
/// 1.70158).
fn back_in(x: f64) -> f64 {
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    c3 * x * x * x - c1 * x * x
}

/// Four parabolic arcs of decreasing height, landing on 1.
fn bounce_out(x: f64) -> f64 {
    let n1 = 7.5625;
    let d1 = 2.75;
    if x < 1.0 / d1 {
        n1 * x * x
    } else if x < 2.0 / d1 {
        let x = x - 1.5 / d1;
        n1 * x * x + 0.75
    } else if x < 2.5 / d1 {
        let x = x - 2.25 / d1;
        n1 * x * x + 0.9375
    } else {
        let x = x - 2.625 / d1;
        n1 * x * x + 0.984375
    }
}

/// One keyframe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Keyframe<T> {
    /// Source time of the clip (TR-027's anchoring): the key sticks to the
    /// footage through trims, splits and moves.
    pub at: Tick,
    pub value: T,
    /// The interpolation of the segment that starts here.
    pub interp: Interp,
}

/// Exact progress `num / den` between two keys. A linear segment gives
/// `0 < num < den`; an eased one (TR-031, [`Progress::from_eased`]) may
/// leave `[0, 1]`, for overshooting, elastic and bouncing curves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Progress {
    num: i64,
    den: i64,
}

impl Progress {
    /// The fixed denominator of eased progress: 2^24 steps.
    const EASED_DEN: i64 = 1 << 24;

    /// `num / den`, when it lies strictly between 0 and 1.
    pub const fn new(num: i64, den: i64) -> Option<Progress> {
        if num > 0 && num < den {
            Some(Progress { num, den })
        } else {
            None
        }
    }

    /// Eased progress `e` (TR-031) as an exact fraction of 2^24 steps,
    /// clamped to −4..=5 (far beyond any preset's overshoot). Not a number
    /// counts as 0.
    pub fn from_eased(e: f64) -> Progress {
        let e = if e.is_nan() { 0.0 } else { e.clamp(-4.0, 5.0) };
        // In range after the clamp, so the cast is exact enough and fits.
        let num = (e * Self::EASED_DEN as f64).round() as i64;
        Progress {
            num,
            den: Self::EASED_DEN,
        }
    }

    /// The fraction as a float, for value types that interpolate in
    /// floating point (colours in linear light).
    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    pub const fn num(self) -> i64 {
        self.num
    }

    pub const fn den(self) -> i64 {
        self.den
    }

    /// True from the midpoint on.
    pub fn is_past_half(self) -> bool {
        2 * i128::from(self.num) >= i128::from(self.den)
    }
}

/// Values that can be keyed (TR-027). Evaluation must be pure.
pub trait Interpolate: Clone + PartialEq {
    /// The value a fraction `p` of the way from `a` to `b`.
    fn interpolate(a: &Self, b: &Self, p: Progress) -> Self;
}

/// `a + (b − a)·p`, rounded to the nearest integer, exact halves to the
/// even one (unbiased, and symmetric under reversal of the segment).
/// Eased progress outside `[0, 1]` extrapolates; the result saturates at
/// the `i64` range.
pub fn lerp_i64(a: i64, b: i64, p: Progress) -> i64 {
    let delta = (i128::from(b) - i128::from(a)) * i128::from(p.num);
    let step = div_round(delta, i128::from(p.den), Rounding::NearestTiesEven);
    let v = i128::from(a) + step;
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

/// Why keyframes from storage were refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyframeError {
    #[error("keyframe times must be strictly increasing")]
    Unordered,
}

/// What an [`Animatable`] does at one instant, for evaluators that work a
/// segment at a time (the audio engine evaluates gain per sample).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment<'a, T> {
    /// No keys: the static value.
    Static(&'a T),
    /// A constant from a key: on it, before the first, after the last, or
    /// inside a [`Interp::Hold`] segment.
    Key(&'a Keyframe<T>),
    /// Strictly inside an interpolated segment; `from.interp` names its
    /// curve (linear or one of TR-031's easings).
    Ramp {
        from: &'a Keyframe<T>,
        to: &'a Keyframe<T>,
    },
}

/// A property that may be animated (TR-027): its static value, and keys
/// sorted by strictly increasing source time. With keys, the keys decide
/// the value and the static value is only kept for when the last key goes.
///
/// Keys outside the clip's trimmed range are kept and still shape the value
/// at the clip edge. A split gives both halves all keys (TR-027's one split
/// rule), which the clone of a clip does. The key list is shared, so clones
/// are cheap.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Animatable<T> {
    value: T,
    keys: Arc<[Keyframe<T>]>,
}

impl<T: Default> Default for Animatable<T> {
    fn default() -> Self {
        Animatable::new(T::default())
    }
}

impl<T> From<T> for Animatable<T> {
    fn from(value: T) -> Self {
        Animatable::new(value)
    }
}

impl<T> Animatable<T> {
    /// A value without keys.
    pub fn new(value: T) -> Animatable<T> {
        Animatable {
            value,
            keys: Arc::from(Vec::new()),
        }
    }

    /// Builds from stored parts (project loading). Refuses keys that are not
    /// in strictly increasing time order.
    pub fn from_parts(value: T, keys: Vec<Keyframe<T>>) -> Result<Animatable<T>, KeyframeError> {
        if keys.windows(2).any(|w| w[0].at >= w[1].at) {
            return Err(KeyframeError::Unordered);
        }
        Ok(Animatable {
            value,
            keys: Arc::from(keys),
        })
    }

    /// A value over an already sorted, shared key list (no copy).
    pub(crate) fn from_shared(value: T, keys: Arc<[Keyframe<T>]>) -> Animatable<T> {
        debug_assert!(keys.windows(2).all(|w| w[0].at < w[1].at));
        Animatable { value, keys }
    }

    /// The shared key list.
    pub(crate) fn shared_keys(&self) -> &Arc<[Keyframe<T>]> {
        &self.keys
    }

    /// The static value: the value when there are no keys.
    pub fn static_value(&self) -> &T {
        &self.value
    }

    /// The keys, by time.
    pub fn keys(&self) -> &[Keyframe<T>] {
        &self.keys
    }

    pub fn is_animated(&self) -> bool {
        !self.keys.is_empty()
    }

    /// The key at exactly `at`, if any.
    pub fn key_at(&self, at: Tick) -> Option<&Keyframe<T>> {
        self.index_of(at).map(|i| &self.keys[i])
    }

    /// The segment in effect at source time `at`.
    pub fn segment_at(&self, at: Tick) -> Segment<'_, T> {
        let keys = &*self.keys;
        let Some(first) = keys.first() else {
            return Segment::Static(&self.value);
        };
        // Keys at or before `at`.
        let i = keys.partition_point(|k| k.at <= at);
        let Some(from) = i.checked_sub(1).map(|i| &keys[i]) else {
            return Segment::Key(first);
        };
        match keys.get(i) {
            Some(to) if from.at != at && from.interp != Interp::Hold => Segment::Ramp { from, to },
            _ => Segment::Key(from),
        }
    }

    fn index_of(&self, at: Tick) -> Option<usize> {
        self.keys.binary_search_by_key(&at, |k| k.at).ok()
    }
}

impl<T: Clone> Animatable<T> {
    /// The same keys with static value `value`.
    pub fn with_static(&self, value: T) -> Animatable<T> {
        Animatable {
            value,
            keys: Arc::clone(&self.keys),
        }
    }

    /// Adds a key at `at`, or sets the value of the key already there (its
    /// interpolation is kept). Two keys never share a time (TR-027).
    pub fn with_key(&self, at: Tick, value: T) -> Animatable<T> {
        let mut keys = self.keys.to_vec();
        match keys.binary_search_by_key(&at, |k| k.at) {
            Ok(i) => keys[i].value = value,
            Err(i) => keys.insert(
                i,
                Keyframe {
                    at,
                    value,
                    interp: Interp::Linear,
                },
            ),
        }
        self.with_keys(keys)
    }

    /// Sets the interpolation of the key at `at`; `None` when there is no
    /// key there.
    pub fn with_interp(&self, at: Tick, interp: Interp) -> Option<Animatable<T>> {
        let i = self.index_of(at)?;
        let mut keys = self.keys.to_vec();
        keys[i].interp = interp;
        Some(self.with_keys(keys))
    }

    /// Moves the key at `from` toward `to`, clamped strictly between its
    /// neighbours so keys never reorder or collide (AU-008, TR-029). `None`
    /// when there is no key at `from`.
    pub fn with_key_moved(&self, from: Tick, to: Tick) -> Option<Animatable<T>> {
        let i = self.index_of(from)?;
        let lo = i
            .checked_sub(1)
            .map_or(i64::MIN, |p| self.keys[p].at.raw().saturating_add(1));
        let hi = self
            .keys
            .get(i + 1)
            .map_or(i64::MAX, |n| n.at.raw().saturating_sub(1));
        // Neighbours are at least one tick apart from the key, so lo <= hi.
        let at = Tick::from_raw(to.raw().clamp(lo, hi.max(lo)));
        let mut keys = self.keys.to_vec();
        keys[i].at = at;
        Some(self.with_keys(keys))
    }

    /// Removes the keys at the listed times (times without a key are
    /// ignored). When no key is left, the static value becomes the value of
    /// the earliest removed key, so the property doesn't jump there
    /// (TR-027). `None` when no listed time has a key.
    pub fn without_keys(&self, times: &[Tick]) -> Option<Animatable<T>> {
        let (gone, kept): (Vec<_>, Vec<_>) = self
            .keys
            .iter()
            .cloned()
            .partition(|k| times.contains(&k.at));
        let first_gone = gone.into_iter().next()?;
        if kept.is_empty() {
            return Some(Animatable::new(first_gone.value));
        }
        Some(self.with_keys(kept))
    }

    /// Removes every key; the static value becomes the first key's value.
    /// Unchanged when there are no keys.
    pub fn cleared(&self) -> Animatable<T> {
        match self.keys.first() {
            Some(first) => Animatable::new(first.value.clone()),
            None => self.clone(),
        }
    }

    fn with_keys(&self, keys: Vec<Keyframe<T>>) -> Animatable<T> {
        debug_assert!(keys.windows(2).all(|w| w[0].at < w[1].at));
        Animatable {
            value: self.value.clone(),
            keys: Arc::from(keys),
        }
    }
}

impl<T: Interpolate> Animatable<T> {
    /// The value at source time `at` (TR-027): the static value without
    /// keys; the first key's value before it and the last key's after it;
    /// a key's own value on it; between two keys, the earlier key's
    /// interpolation.
    pub fn value_at(&self, at: Tick) -> T {
        match self.segment_at(at) {
            Segment::Static(v) => v.clone(),
            Segment::Key(k) => k.value.clone(),
            Segment::Ramp { from, to } => {
                // Inside the segment: 0 < at − from < to − from.
                let num = at.raw().saturating_sub(from.at.raw());
                let den = to.at.raw().saturating_sub(from.at.raw());
                let Some(p) = Progress::new(num, den) else {
                    return from.value.clone();
                };
                let p = match from.interp {
                    Interp::Linear | Interp::Hold => p,
                    // The exact rational becomes a float only to feed the
                    // easing function (TR-027).
                    eased => Progress::from_eased(eased.ease(p.as_f64())),
                };
                T::interpolate(&from.value, &to.value, p)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Interpolate for i64 {
        fn interpolate(a: &i64, b: &i64, p: Progress) -> i64 {
            lerp_i64(*a, *b, p)
        }
    }

    fn t(v: i64) -> Tick {
        Tick::from_raw(v)
    }

    fn key(at: i64, value: i64) -> Keyframe<i64> {
        Keyframe {
            at: t(at),
            value,
            interp: Interp::Linear,
        }
    }

    #[test]
    fn evaluation_follows_tr_027() {
        let a = Animatable::from_parts(7, vec![key(10, 0), key(20, 100)]).unwrap();
        assert_eq!(a.value_at(t(0)), 0, "first key's value before it");
        assert_eq!(a.value_at(t(10)), 0);
        assert_eq!(a.value_at(t(15)), 50);
        assert_eq!(a.value_at(t(13)), 30);
        assert_eq!(a.value_at(t(20)), 100);
        assert_eq!(a.value_at(t(99)), 100, "last key's value after it");
        assert_eq!(Animatable::new(7).value_at(t(5)), 7);
        let hold = a.with_interp(t(10), Interp::Hold).unwrap();
        assert_eq!(hold.value_at(t(19)), 0);
        assert_eq!(hold.value_at(t(20)), 100);
        let one = Animatable::from_parts(7, vec![key(10, 3)]).unwrap();
        assert_eq!((one.value_at(t(0)), one.value_at(t(50))), (3, 3));
    }

    #[test]
    fn lerp_is_exact_and_rounds_halves_to_even() {
        let p = |n, d| Progress::new(n, d).unwrap();
        assert_eq!(lerp_i64(0, 3, p(1, 2)), 2); // 1.5 → 2
        assert_eq!(lerp_i64(0, 5, p(1, 2)), 2); // 2.5 → 2
        assert_eq!(lerp_i64(10, 0, p(1, 4)), 8); // 7.5 → 8
        assert_eq!(lerp_i64(i64::MIN, i64::MAX, p(1, 2)), 0);
        assert_eq!(Progress::new(0, 3), None);
        assert_eq!(Progress::new(3, 3), None);
    }

    #[test]
    fn easing_presets_follow_tr_031() {
        for interp in Interp::PRESETS {
            if interp == Interp::Hold {
                continue;
            }
            assert!(interp.ease(0.0).abs() < 1e-9, "{interp:?} e(0)");
            assert!((interp.ease(1.0) - 1.0).abs() < 1e-9, "{interp:?} e(1)");
            assert!(
                (interp.ease_mirrored(0.3) - (1.0 - interp.ease(0.7))).abs() < 1e-12,
                "{interp:?}"
            );
        }
        // Monotonic where TR-031 expects it.
        for interp in Interp::PRESETS
            .iter()
            .filter(|i| matches!(i, Interp::Ease { .. }))
        {
            let mut prev = 0.0;
            for k in 1..=100 {
                let e = interp.ease(f64::from(k) / 100.0);
                assert!(e >= prev - 1e-12, "{interp:?} at {k}");
                prev = e;
            }
        }
        let quad_out = Interp::Ease {
            curve: EaseCurve::Quad,
            dir: EaseDir::Out,
        };
        assert!((quad_out.ease(0.5) - 0.75).abs() < 1e-12);
        assert!(Interp::OvershootOut.ease(0.7) > 1.0, "overshoots");
        assert!(Interp::OvershootIn.ease(0.2) < 0.0, "pulls back");
        assert!((0..=100).any(|k| Interp::ElasticOut.ease(f64::from(k) / 100.0) > 1.0));
        assert!((0..=100).all(|k| Interp::BounceOut.ease(f64::from(k) / 100.0) <= 1.0 + 1e-9));
        assert_eq!(Interp::Hold.ease(0.99), 0.0);
        assert_eq!(Interp::Linear.ease(f64::NAN), 0.0);
        assert_eq!(Interp::Linear.ease(7.0), 1.0, "clamped input");
    }

    #[test]
    fn eased_segments_round_only_on_evaluation() {
        let a = Animatable::new(0).with_key(t(0), 0).with_key(t(100), 1000);
        let quad_in = Interp::Ease {
            curve: EaseCurve::Quad,
            dir: EaseDir::In,
        };
        let eased = a.with_interp(t(0), quad_in).unwrap();
        assert_eq!(eased.value_at(t(50)), 250);
        assert_eq!(eased.value_at(t(10)), 10);
        assert_eq!(eased.value_at(t(100)), 1000);
        assert_eq!(eased.keys()[0].value, 0, "stored values untouched");
        let over = a.with_interp(t(0), Interp::OvershootOut).unwrap();
        assert!(over.value_at(t(70)) > 1000, "overshoot extrapolates");
        assert!(matches!(eased.segment_at(t(50)), Segment::Ramp { .. }));
        assert_eq!(lerp_i64(0, i64::MAX, Progress::from_eased(2.0)), i64::MAX);
        assert_eq!(Progress::from_eased(f64::NAN), Progress::from_eased(0.0));
        assert!((Progress::from_eased(0.25).as_f64() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn segments_name_the_keys_in_effect() {
        let a = Animatable::from_parts(0, vec![key(10, 1), key(20, 2)]).unwrap();
        assert_eq!(a.segment_at(t(5)), Segment::Key(&a.keys()[0]));
        assert_eq!(a.segment_at(t(10)), Segment::Key(&a.keys()[0]));
        assert_eq!(
            a.segment_at(t(11)),
            Segment::Ramp {
                from: &a.keys()[0],
                to: &a.keys()[1]
            }
        );
        assert_eq!(a.segment_at(t(25)), Segment::Key(&a.keys()[1]));
        assert_eq!(Animatable::new(4).segment_at(t(0)), Segment::Static(&4));
    }

    #[test]
    fn stored_keys_must_increase() {
        assert_eq!(
            Animatable::from_parts(0, vec![key(10, 1), key(10, 2)]),
            Err(KeyframeError::Unordered)
        );
        assert_eq!(
            Animatable::from_parts(0, vec![key(10, 1), key(5, 2)]),
            Err(KeyframeError::Unordered)
        );
    }

    #[test]
    fn adding_sets_in_place_and_keeps_order() {
        let a = Animatable::new(5).with_key(t(20), 1).with_key(t(10), 2);
        assert_eq!(a.keys(), &[key(10, 2), key(20, 1)]);
        assert_eq!(*a.static_value(), 5);
        let held = a.with_interp(t(10), Interp::Hold).unwrap();
        let replaced = held.with_key(t(10), 9);
        assert_eq!(replaced.keys()[0].value, 9);
        assert_eq!(replaced.keys()[0].interp, Interp::Hold, "interp kept");
        assert_eq!(a.with_interp(t(11), Interp::Hold), None);
    }

    #[test]
    fn moving_clamps_between_neighbours() {
        let a = Animatable::new(0)
            .with_key(t(10), 1)
            .with_key(t(20), 2)
            .with_key(t(30), 3);
        let moved = a.with_key_moved(t(20), t(100)).unwrap();
        assert_eq!(moved.keys()[1].at, t(29));
        let moved = a.with_key_moved(t(20), t(-100)).unwrap();
        assert_eq!(moved.keys()[1].at, t(11));
        let moved = a.with_key_moved(t(10), t(-100)).unwrap();
        assert_eq!(
            moved.keys()[0].at,
            t(-100),
            "the first key is free leftward"
        );
        let tight = Animatable::new(0).with_key(t(10), 1).with_key(t(11), 2);
        assert_eq!(tight.with_key_moved(t(10), t(50)).unwrap(), tight);
        assert_eq!(a.with_key_moved(t(15), t(16)), None);
    }

    #[test]
    fn removing_the_last_key_writes_the_static_value() {
        let a = Animatable::new(5).with_key(t(10), 1).with_key(t(20), 2);
        let one = a.without_keys(&[t(10)]).unwrap();
        assert_eq!((one.keys(), *one.static_value()), (&[key(20, 2)][..], 5));
        let none = one.without_keys(&[t(20)]).unwrap();
        assert_eq!(none, Animatable::new(2));
        let both = a.without_keys(&[t(20), t(10)]).unwrap();
        assert_eq!(both, Animatable::new(1), "the earliest removed key");
        assert_eq!(a.without_keys(&[t(15)]), None);
        assert_eq!(a.cleared(), Animatable::new(1));
        assert_eq!(Animatable::new(4).cleared(), Animatable::new(4));
    }
}
