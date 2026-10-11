//! Exact frame and sample stepping for export (EX-002, AU-010).

use crate::time::{FrameRate, SampleRate, TICKS_PER_SECOND, Tick, TimeRange};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExportTimeError {
    #[error("time arithmetic overflowed")]
    Overflow,
}

/// The output grid of one export: output frame `n` shows sequence time
/// `start + n / rate`, floored onto the tick grid. Every position is an
/// integer computed from `n`, so nothing drifts or repeats over hours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportClock {
    range: TimeRange,
    rate: FrameRate,
    frames: u64,
}

impl ExportClock {
    /// `range` must be non-empty (guaranteed by `TimeRange`). A range shorter
    /// than one output frame still exports one frame.
    pub fn new(range: TimeRange, rate: FrameRate) -> Result<ExportClock, ExportTimeError> {
        // N = ceil(duration · num / (TPS · den)), in i128.
        let n = i128::from(range.duration().raw()) * i128::from(rate.num());
        let d = i128::from(TICKS_PER_SECOND) * i128::from(rate.den());
        let frames = u64::try_from((n + d - 1) / d).map_err(|_| ExportTimeError::Overflow)?;
        Ok(ExportClock {
            range,
            rate,
            frames,
        })
    }

    pub const fn range(&self) -> TimeRange {
        self.range
    }

    pub const fn rate(&self) -> FrameRate {
        self.rate
    }

    pub const fn frame_count(&self) -> u64 {
        self.frames
    }

    /// Sequence time shown by output frame `n`.
    pub fn frame_time(&self, n: u64) -> Result<Tick, ExportTimeError> {
        let offset = i128::from(n) * i128::from(TICKS_PER_SECOND) * i128::from(self.rate.den())
            / i128::from(self.rate.num());
        let offset = i64::try_from(offset).map_err(|_| ExportTimeError::Overflow)?;
        self.range
            .start()
            .checked_add(Tick::from_raw(offset))
            .map_err(|_| ExportTimeError::Overflow)
    }

    /// Audio samples covering exactly the range: `floor(t · sr / TPS)` at
    /// both ends, so adjacent ranges tile with no gap or overlap.
    pub fn sample_count(&self, sample_rate: SampleRate) -> Result<u64, ExportTimeError> {
        let at = |t: Tick| {
            i128::from(t.raw()) * i128::from(sample_rate.hz()) / i128::from(TICKS_PER_SECOND)
        };
        let start = at(self.range.start());
        let end = at(self.range.end());
        u64::try_from(end - start).map_err(|_| ExportTimeError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn rate(num: u32, den: u32) -> FrameRate {
        FrameRate::new(num, den).unwrap()
    }

    fn range(start: i64, duration: i64) -> TimeRange {
        TimeRange::new(Tick::from_raw(start), Tick::from_raw(duration)).unwrap()
    }

    #[test]
    fn tiny_ranges_export_one_frame() {
        let r = rate(25, 1);
        assert_eq!(
            ExportClock::new(range(0, r.frame_ticks().raw()), r)
                .unwrap()
                .frame_count(),
            1
        );
        assert_eq!(ExportClock::new(range(0, 1), r).unwrap().frame_count(), 1);
        assert_eq!(
            ExportClock::new(range(5, r.frame_ticks().raw() + 1), r)
                .unwrap()
                .frame_count(),
            2
        );
    }

    #[test]
    fn ntsc_over_a_day_has_no_duplicates_or_drift() {
        let r = rate(30_000, 1001);
        for hours in [1_i64, 24] {
            let dur = hours * 3600 * TICKS_PER_SECOND;
            let clock = ExportClock::new(range(0, dur), r).unwrap();
            let n = clock.frame_count();
            // The project rate is on the tick grid: frames are exactly one
            // frame apart.
            assert_eq!(
                clock.frame_time(n - 1).unwrap(),
                r.frame_start(i64::try_from(n - 1).unwrap()).unwrap()
            );
            assert!(clock.frame_time(n - 1).unwrap() < range(0, dur).end());
            assert!(clock.frame_time(n).unwrap() >= range(0, dur).end());
        }
    }

    #[test]
    fn cadence_from_25_to_30_repeats_every_6_frames() {
        // A 25 fps timeline exported at 30 fps: which 25 fps frame each output shows.
        let src = rate(25, 1);
        let clock = ExportClock::new(range(0, TICKS_PER_SECOND), rate(30, 1)).unwrap();
        let shown: Vec<i64> = (0..12)
            .map(|n| src.frame_index(clock.frame_time(n).unwrap()))
            .collect();
        assert_eq!(shown, vec![0, 0, 1, 2, 3, 4, 5, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn adjacent_ranges_tile_samples_exactly() {
        let r = rate(30_000, 1001);
        let f = r.frame_ticks().raw();
        for sr in [44_100, 48_000] {
            let sr = SampleRate::new(sr).unwrap();
            let whole = ExportClock::new(range(7 * f, 1000 * f), r)
                .unwrap()
                .sample_count(sr)
                .unwrap();
            let a = ExportClock::new(range(7 * f, 333 * f), r)
                .unwrap()
                .sample_count(sr)
                .unwrap();
            let b = ExportClock::new(range(340 * f, 667 * f), r)
                .unwrap()
                .sample_count(sr)
                .unwrap();
            assert_eq!(a + b, whole);
        }
    }

    #[test]
    fn extreme_values_are_errors_not_panics() {
        let clock = ExportClock::new(range(0, i64::MAX / 2), rate(120, 1)).unwrap();
        assert_eq!(clock.frame_time(u64::MAX), Err(ExportTimeError::Overflow));
    }

    proptest! {
        #[test]
        fn frame_times_increase_and_stay_inside_the_range(
            start in 0i64..1_000_000_000_000,
            dur in 1i64..100_000_000_000,
            (num, den) in prop_oneof![Just((24_000u32, 1001u32)), Just((25, 1)), Just((30_000, 1001)), Just((60, 1)), Just((120, 1))],
        ) {
            let r = range(start, dur);
            let clock = ExportClock::new(r, rate(num, den)).unwrap();
            let n = clock.frame_count();
            prop_assert!(n >= 1);
            prop_assert_eq!(clock.frame_time(0).unwrap(), r.start());
            prop_assert!(clock.frame_time(n - 1).unwrap() < r.end());
            for k in 0..n.min(50) {
                prop_assert!(clock.frame_time(k).unwrap() < clock.frame_time(k + 1).unwrap());
            }
        }
    }
}
