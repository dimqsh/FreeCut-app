//! Half-open time ranges.

use super::{Tick, TimeError};

/// A half-open range `[start, start + duration)` with `duration > 0`.
///
/// The constructor checks that the end fits in `i64`, so [`TimeRange::end`]
/// can't fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeRange {
    start: Tick,
    duration: Tick,
}

impl TimeRange {
    pub fn new(start: Tick, duration: Tick) -> Result<TimeRange, TimeError> {
        if !duration.is_positive() {
            return Err(TimeError::NonPositiveDuration);
        }
        start.checked_add(duration)?;
        Ok(TimeRange { start, duration })
    }

    pub fn from_start_end(start: Tick, end: Tick) -> Result<TimeRange, TimeError> {
        TimeRange::new(start, end.checked_sub(start)?)
    }

    pub const fn start(self) -> Tick {
        self.start
    }

    pub const fn duration(self) -> Tick {
        self.duration
    }

    pub const fn end(self) -> Tick {
        // Invariant: `new` checked that start + duration fits, and the fields
        // are private.
        Tick::from_raw(self.start.raw() + self.duration.raw())
    }

    pub fn contains(self, t: Tick) -> bool {
        self.start <= t && t < self.end()
    }

    /// True when `t` lies strictly inside, excluding both edges.
    pub fn strictly_contains(self, t: Tick) -> bool {
        self.start < t && t < self.end()
    }

    pub fn overlaps(self, other: TimeRange) -> bool {
        self.start < other.end() && other.start < self.end()
    }

    pub fn intersection(self, other: TimeRange) -> Option<TimeRange> {
        let start = self.start.max(other.start);
        let end = self.end().min(other.end());
        TimeRange::from_start_end(start, end).ok()
    }

    pub fn shifted(self, delta: Tick) -> Result<TimeRange, TimeError> {
        TimeRange::new(self.start.checked_add(delta)?, self.duration)
    }

    pub fn with_start(self, start: Tick) -> Result<TimeRange, TimeError> {
        TimeRange::new(start, self.duration)
    }

    pub fn with_duration(self, duration: Tick) -> Result<TimeRange, TimeError> {
        TimeRange::new(self.start, duration)
    }
}

/// A range `[start, start + duration)` with `duration >= 0`: a marker's span,
/// where 0 means a point (TL-037). Like [`TimeRange`], the end always fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MarkRange {
    start: Tick,
    duration: Tick,
}

impl MarkRange {
    pub fn new(start: Tick, duration: Tick) -> Result<MarkRange, TimeError> {
        if duration.is_negative() {
            return Err(TimeError::Negative);
        }
        start.checked_add(duration)?;
        Ok(MarkRange { start, duration })
    }

    /// A point at `start`.
    pub const fn point(start: Tick) -> MarkRange {
        MarkRange {
            start,
            duration: Tick::ZERO,
        }
    }

    pub const fn start(self) -> Tick {
        self.start
    }

    pub const fn duration(self) -> Tick {
        self.duration
    }

    pub const fn is_point(self) -> bool {
        self.duration.raw() == 0
    }

    pub const fn end(self) -> Tick {
        // Invariant: `new` checked that start + duration fits, and `point`
        // adds nothing; the fields are private.
        Tick::from_raw(self.start.raw() + self.duration.raw())
    }

    pub fn with_start(self, start: Tick) -> Result<MarkRange, TimeError> {
        MarkRange::new(start, self.duration)
    }

    pub fn with_duration(self, duration: Tick) -> Result<MarkRange, TimeError> {
        MarkRange::new(self.start, duration)
    }
}

impl core::fmt::Display for TimeRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "[{}, {})", self.start.raw(), self.end().raw())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: i64, dur: i64) -> TimeRange {
        TimeRange::new(Tick::from_raw(start), Tick::from_raw(dur)).unwrap()
    }

    #[test]
    fn construction_rejects_empty_negative_and_overflowing_ranges() {
        assert_eq!(
            TimeRange::new(Tick::ZERO, Tick::ZERO),
            Err(TimeError::NonPositiveDuration)
        );
        assert_eq!(
            TimeRange::new(Tick::ZERO, Tick::from_raw(-1)),
            Err(TimeError::NonPositiveDuration)
        );
        assert_eq!(
            TimeRange::new(Tick::MAX, Tick::from_raw(1)),
            Err(TimeError::Overflow)
        );
    }

    #[test]
    fn mark_ranges_allow_points_but_not_negative_or_overflowing_spans() {
        let p = MarkRange::point(Tick::from_raw(5));
        assert!(p.is_point());
        assert_eq!(p.end(), Tick::from_raw(5));
        assert_eq!(
            MarkRange::new(Tick::ZERO, Tick::from_raw(-1)),
            Err(TimeError::Negative)
        );
        assert_eq!(
            MarkRange::new(Tick::MAX, Tick::from_raw(1)),
            Err(TimeError::Overflow)
        );
        let r = MarkRange::new(Tick::from_raw(10), Tick::from_raw(4)).unwrap();
        assert_eq!(
            (r.start(), r.end()),
            (Tick::from_raw(10), Tick::from_raw(14))
        );
    }

    #[test]
    fn half_open_semantics() {
        let a = r(10, 10);
        assert!(a.contains(Tick::from_raw(10)));
        assert!(!a.contains(Tick::from_raw(20)));
        assert!(!a.strictly_contains(Tick::from_raw(10)));
        assert!(a.strictly_contains(Tick::from_raw(11)));
        assert!(!a.overlaps(r(20, 5)), "touching ranges don't overlap");
        assert!(a.overlaps(r(19, 5)));
        assert_eq!(a.intersection(r(15, 10)), Some(r(15, 5)));
        assert_eq!(a.intersection(r(20, 10)), None);
    }
}
