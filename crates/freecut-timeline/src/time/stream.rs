//! Conversion between container timestamps and model ticks.
//!
//! `freecut-media` must use exactly these two functions: one per direction,
//! paired so that "the frame displayed at `t`" is consistent even for time
//! bases that aren't exact on the tick grid (such as 1/15360).

use super::{Rounding, TICKS_PER_SECOND, Tick, TimeError, div_round};

/// Seconds per unit = `num/den`, like FFmpeg's `AVRational` time base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamTimeBase {
    num: i32,
    den: i32,
}

impl StreamTimeBase {
    pub fn new(num: i32, den: i32) -> Result<StreamTimeBase, TimeError> {
        if num <= 0 || den <= 0 {
            return Err(TimeError::InvalidTimeBase { num, den });
        }
        Ok(StreamTimeBase { num, den })
    }

    pub const fn num(self) -> i32 {
        self.num
    }

    pub const fn den(self) -> i32 {
        self.den
    }
}

/// The canonical stream timestamp → tick conversion: exact rational
/// multiplication, rounded to the nearest tick with ties to even.
pub fn ticks_from_stream(ts: i64, tb: StreamTimeBase) -> Result<Tick, TimeError> {
    let n = i128::from(ts) * i128::from(tb.num) * i128::from(TICKS_PER_SECOND);
    let v = div_round(n, i128::from(tb.den), Rounding::NearestTiesEven);
    i64::try_from(v)
        .map(Tick::from_raw)
        .map_err(|_| TimeError::Overflow)
}

/// The largest `ts` with `ticks_from_stream(ts) <= t`: the exact inverse used
/// for seek targets and "which source frame is shown at `t`".
pub fn stream_ts_at_or_before(t: Tick, tb: StreamTimeBase) -> Result<i64, TimeError> {
    let per_unit = i128::from(tb.num) * i128::from(TICKS_PER_SECOND);
    let guess = div_round(
        i128::from(t.raw()) * i128::from(tb.den),
        per_unit,
        Rounding::Floor,
    );
    let mut ts = i64::try_from(guess).map_err(|_| TimeError::Overflow)?;
    // `ticks_from_stream` rounds to nearest, so the answer is within one unit
    // of the floor guess; these loops run at most a couple of times. A probe
    // past the end of the tick range is necessarily later than `t`.
    while let Some(next) = ts.checked_add(1) {
        match ticks_from_stream(next, tb) {
            Ok(at) if at <= t => ts = next,
            _ => break,
        }
    }
    while ticks_from_stream(ts, tb)? > t {
        ts = ts.checked_sub(1).ok_or(TimeError::Overflow)?;
    }
    Ok(ts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn exact_time_bases_convert_exactly() {
        let tb = StreamTimeBase::new(1, 90_000).unwrap();
        assert_eq!(
            ticks_from_stream(90_000, tb),
            Ok(Tick::from_raw(TICKS_PER_SECOND))
        );
        assert_eq!(ticks_from_stream(1, tb), Ok(Tick::from_raw(7_840)));
        let tb = StreamTimeBase::new(1001, 30_000).unwrap();
        assert_eq!(ticks_from_stream(1, tb), Ok(Tick::from_raw(23_543_520)));
    }

    #[test]
    fn inexact_time_base_rounds_to_nearest_even() {
        // 1/15360 s = 45_937.5 ticks.
        let tb = StreamTimeBase::new(1, 15_360).unwrap();
        assert_eq!(ticks_from_stream(1, tb), Ok(Tick::from_raw(45_938)));
        assert_eq!(ticks_from_stream(3, tb), Ok(Tick::from_raw(137_812)));
    }

    #[test]
    fn the_inverse_works_at_the_top_of_the_tick_range() {
        let tb = StreamTimeBase::new(1, 90_000).unwrap();
        assert_eq!(stream_ts_at_or_before(Tick::MAX, tb), Ok(i64::MAX / 7_840));
    }

    #[test]
    fn invalid_time_bases_are_refused() {
        assert!(StreamTimeBase::new(0, 1).is_err());
        assert!(StreamTimeBase::new(1, -1).is_err());
    }

    proptest! {
        #[test]
        fn inverse_is_the_largest_timestamp_at_or_before(
            t in -10_000_000_000_000i64..10_000_000_000_000,
            (num, den) in prop_oneof![
                Just((1, 15_360)), Just((1, 90_000)), Just((1, 1_000_000)),
                Just((1001, 30_000)), Just((1, 48_000)), Just((1, 600)),
            ],
        ) {
            let tb = StreamTimeBase::new(num, den).unwrap();
            let t = Tick::from_raw(t);
            let ts = stream_ts_at_or_before(t, tb).unwrap();
            prop_assert!(ticks_from_stream(ts, tb).unwrap() <= t);
            prop_assert!(ticks_from_stream(ts + 1, tb).unwrap() > t);
        }

        #[test]
        fn round_trip_through_ticks_returns_the_same_timestamp(
            ts in -1_000_000_000_000i64..1_000_000_000_000,
            (num, den) in prop_oneof![Just((1, 15_360)), Just((1, 1_000_000)), Just((1, 44_100))],
        ) {
            let tb = StreamTimeBase::new(num, den).unwrap();
            let t = ticks_from_stream(ts, tb).unwrap();
            prop_assert_eq!(stream_ts_at_or_before(t, tb).unwrap(), ts);
        }
    }
}
