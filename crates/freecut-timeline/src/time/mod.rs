//! Exact model time (TL-001).
//!
//! Every instant and span in the edit model is an integer number of ticks of
//! 1/705,600,000 s. That rate divides evenly into every common video frame
//! duration (including the 1001-denominator NTSC rates) and every common audio
//! sample rate, so frame and sample positions are exact integers. Floats are
//! only produced at the UI edge ([`Tick::as_secs_f64`]).

mod range;
mod rate;
mod rational;
mod stream;
mod timecode;

pub use range::{MarkRange, TimeRange};
pub use rate::{FrameRate, SampleRate};
pub use rational::Rational;
pub use stream::{StreamTimeBase, stream_ts_at_or_before, ticks_from_stream};
pub use timecode::Timecode;

/// Ticks per second of the model timebase.
///
/// `705_600_000 = 2⁹·3²·5⁵·7²`: divisible by every common frame and sample rate.
pub const TICKS_PER_SECOND: i64 = 705_600_000;

/// Errors from time arithmetic and time-type construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TimeError {
    #[error("time arithmetic overflowed")]
    Overflow,
    #[error("duration must be positive")]
    NonPositiveDuration,
    #[error("time must not be negative")]
    Negative,
    #[error("invalid rate {num}/{den}")]
    InvalidRate { num: u32, den: u32 },
    #[error("rate {num}/{den} has no whole-tick frame duration")]
    OffGridRate { num: u32, den: u32 },
    #[error("invalid stream time base {num}/{den}")]
    InvalidTimeBase { num: i32, den: i32 },
    #[error("invalid rational {num}/{den}")]
    InvalidRational { num: i64, den: i64 },
}

/// How a division that doesn't come out even is rounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    /// Toward negative infinity.
    Floor,
    /// Toward positive infinity.
    Ceil,
    /// Nearest; exact halves go to the smaller value (the frame-grid rule).
    NearestTiesDown,
    /// Nearest; exact halves go to the even value (unbiased).
    NearestTiesEven,
}

/// `n / d` rounded as requested. `d` must be positive.
///
/// Uses Euclidean division so negative values round consistently rather than
/// truncating toward zero.
pub(crate) fn div_round(n: i128, d: i128, mode: Rounding) -> i128 {
    debug_assert!(d > 0, "callers pass validated positive denominators");
    let (q, r) = (n.div_euclid(d), n.rem_euclid(d));
    match mode {
        Rounding::Floor => q,
        Rounding::Ceil => q + i128::from(r != 0),
        Rounding::NearestTiesDown => q + i128::from(2 * r > d),
        Rounding::NearestTiesEven => match (2 * r).cmp(&d) {
            core::cmp::Ordering::Less => q,
            core::cmp::Ordering::Greater => q + 1,
            core::cmp::Ordering::Equal => q + (q & 1),
        },
    }
}

/// An instant or a span in model ticks.
///
/// There are deliberately no `Add`/`Sub` operator impls: every arithmetic step
/// is checked, because a silent wrap would corrupt a project.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(i64);

impl Tick {
    pub const ZERO: Tick = Tick(0);
    pub const MAX: Tick = Tick(i64::MAX);

    pub const fn from_raw(ticks: i64) -> Tick {
        Tick(ticks)
    }

    pub const fn raw(self) -> i64 {
        self.0
    }

    /// Whole seconds; exact.
    pub fn from_secs(secs: i64) -> Result<Tick, TimeError> {
        secs.checked_mul(TICKS_PER_SECOND)
            .map(Tick)
            .ok_or(TimeError::Overflow)
    }

    pub fn checked_add(self, other: Tick) -> Result<Tick, TimeError> {
        self.0
            .checked_add(other.0)
            .map(Tick)
            .ok_or(TimeError::Overflow)
    }

    pub fn checked_sub(self, other: Tick) -> Result<Tick, TimeError> {
        self.0
            .checked_sub(other.0)
            .map(Tick)
            .ok_or(TimeError::Overflow)
    }

    pub fn checked_neg(self) -> Result<Tick, TimeError> {
        self.0.checked_neg().map(Tick).ok_or(TimeError::Overflow)
    }

    pub fn checked_mul_int(self, factor: i64) -> Result<Tick, TimeError> {
        self.0
            .checked_mul(factor)
            .map(Tick)
            .ok_or(TimeError::Overflow)
    }

    /// `self * r`, computed exactly in `i128` and rounded once.
    pub fn mul_rational(self, r: Rational, mode: Rounding) -> Result<Tick, TimeError> {
        let v = div_round(
            i128::from(self.0) * i128::from(r.num()),
            i128::from(r.den()),
            mode,
        );
        i64::try_from(v).map(Tick).map_err(|_| TimeError::Overflow)
    }

    /// `self / r`, computed exactly in `i128` and rounded once.
    pub fn div_rational(self, r: Rational, mode: Rounding) -> Result<Tick, TimeError> {
        self.mul_rational(r.recip()?, mode)
    }

    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// Absolute distance between two instants, saturating at `u64::MAX`.
    pub const fn abs_diff(self, other: Tick) -> u64 {
        self.0.abs_diff(other.0)
    }

    /// Seconds as a float, for display and layout only. Never feed the result
    /// back into the model.
    #[allow(clippy::cast_precision_loss)] // display-only conversion
    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }
}

impl core::fmt::Display for Tick {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}t", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn div_round_modes_on_positive_and_negative_values() {
        // 7 / 2 = 3.5, -7 / 2 = -3.5
        assert_eq!(div_round(7, 2, Rounding::Floor), 3);
        assert_eq!(div_round(7, 2, Rounding::Ceil), 4);
        assert_eq!(div_round(7, 2, Rounding::NearestTiesDown), 3);
        assert_eq!(div_round(7, 2, Rounding::NearestTiesEven), 4);
        assert_eq!(div_round(5, 2, Rounding::NearestTiesEven), 2);
        assert_eq!(div_round(-7, 2, Rounding::Floor), -4);
        assert_eq!(div_round(-7, 2, Rounding::Ceil), -3);
        assert_eq!(div_round(-7, 2, Rounding::NearestTiesDown), -4);
        assert_eq!(div_round(-7, 2, Rounding::NearestTiesEven), -4);
        // Non-halves round to nearest in every nearest mode.
        assert_eq!(div_round(8, 3, Rounding::NearestTiesDown), 3);
        assert_eq!(div_round(7, 3, Rounding::NearestTiesDown), 2);
    }

    #[test]
    fn checked_arithmetic_reports_overflow_instead_of_wrapping() {
        assert_eq!(
            Tick::MAX.checked_add(Tick::from_raw(1)),
            Err(TimeError::Overflow)
        );
        assert_eq!(
            Tick::from_raw(i64::MIN).checked_neg(),
            Err(TimeError::Overflow)
        );
        assert_eq!(
            Tick::from_raw(i64::MIN).checked_sub(Tick::from_raw(1)),
            Err(TimeError::Overflow)
        );
    }

    #[test]
    fn mul_rational_is_exact_and_rounds_once() {
        let half = Rational::new(1, 2).unwrap();
        assert_eq!(
            Tick::from_raw(3).mul_rational(half, Rounding::Floor),
            Ok(Tick::from_raw(1))
        );
        assert_eq!(
            Tick::from_raw(3).mul_rational(half, Rounding::Ceil),
            Ok(Tick::from_raw(2))
        );
        // A product that overflows i64 but whose quotient fits is still exact.
        let big = Rational::new(3, 4).unwrap();
        let t = Tick::from_raw(i64::MAX / 2);
        assert_eq!(
            t.mul_rational(big, Rounding::Floor),
            Ok(Tick::from_raw(((i128::from(i64::MAX / 2) * 3) / 4) as i64))
        );
    }

    #[test]
    fn from_secs_is_exact() {
        assert_eq!(Tick::from_secs(2), Ok(Tick::from_raw(1_411_200_000)));
        assert_eq!(Tick::from_secs(i64::MAX), Err(TimeError::Overflow));
    }
}
