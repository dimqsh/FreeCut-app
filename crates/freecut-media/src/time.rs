//! Time inside media (I1, I2): stream timestamps ↔ media time, sample index
//! math, nominal frame rates and the VFR heuristic. Pure functions.
//!
//! *Media time* is in ticks with 0 at the first presentable frame of the
//! default video stream (or the first decoded sample of the default audio
//! stream when there is no video). Every stream keeps its offset relative to
//! that origin, so audio that starts before the picture has negative times.
//!
//! Stream timestamps become ticks only through
//! [`freecut_timeline::time::ticks_from_stream`], and frame choice compares
//! ticks, never stream timestamps: time bases such as 1/15360 don't divide
//! the tick rate (L2).

use freecut_timeline::time::{
    Rational, StreamTimeBase, TICKS_PER_SECOND, Tick, TimeError, stream_ts_at_or_before,
    ticks_from_stream,
};

/// Maps one stream's native timestamps to media time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamClock {
    tb: StreamTimeBase,
    /// The media origin as an absolute tick time (same rounding function).
    origin: Tick,
}

impl StreamClock {
    pub(crate) fn new(num: i32, den: i32, origin: Tick) -> Result<StreamClock, TimeError> {
        Ok(StreamClock {
            tb: StreamTimeBase::new(num, den)?,
            origin,
        })
    }

    pub(crate) fn time_base(&self) -> StreamTimeBase {
        self.tb
    }

    /// Media time of stream timestamp `ts`.
    pub(crate) fn media_time(&self, ts: i64) -> Result<Tick, TimeError> {
        ticks_from_stream(ts, self.tb)?.checked_sub(self.origin)
    }

    /// Media time of an absolute tick time of the file (for times that come
    /// in another time base, such as the container's µs).
    pub(crate) fn absolute_to_media(&self, abs: Tick) -> Result<Tick, TimeError> {
        abs.checked_sub(self.origin)
    }

    /// The largest timestamp whose media time is `<= t`. Only for seek
    /// targets, never for equality tests.
    pub(crate) fn ts_at_or_before(&self, t: Tick) -> Result<i64, TimeError> {
        stream_ts_at_or_before(t.checked_add(self.origin)?, self.tb)
    }
}

/// `floor(a * b / c)` in `i128`, saturated to `i64`. `c` must be positive.
fn mul_div_floor(a: i64, b: i64, c: i64) -> i64 {
    let v = (i128::from(a) * i128::from(b)).div_euclid(i128::from(c));
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

/// `ceil(a * b / c)` in `i128`, saturated to `i64`. `c` must be positive.
fn mul_div_ceil(a: i64, b: i64, c: i64) -> i64 {
    let n = i128::from(a) * i128::from(b);
    let c = i128::from(c);
    let v = n.div_euclid(c) + i128::from(n.rem_euclid(c) != 0);
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

/// Index of the sample covering media time `t` at `rate` Hz:
/// `floor(t * rate / TPS)`, also for negative times.
pub(crate) fn sample_floor(t: Tick, rate: u32) -> i64 {
    mul_div_floor(t.raw(), i64::from(rate.max(1)), TICKS_PER_SECOND)
}

/// The sample nearest to `t` (ties up): where a frame starting at `t` puts
/// its first sample.
pub(crate) fn sample_nearest(t: Tick, rate: u32) -> i64 {
    let half = Tick::from_raw(TICKS_PER_SECOND / i64::from(rate.max(1)) / 2);
    sample_floor(t.checked_add(half).unwrap_or(t), rate)
}

/// Start time of sample `m` at `rate` Hz (exact when `rate` divides the tick
/// rate, otherwise floored).
pub(crate) fn sample_time(m: i64, rate: u32) -> Tick {
    Tick::from_raw(mul_div_floor(m, TICKS_PER_SECOND, i64::from(rate.max(1))))
}

/// Converts a sample index between rates, rounding up: the first `to_rate`
/// sample at or after input sample `m`.
pub(crate) fn rescale_ceil(m: i64, from_rate: u32, to_rate: u32) -> i64 {
    mul_div_ceil(m, i64::from(to_rate), i64::from(from_rate.max(1)))
}

/// Converts a sample index between rates, rounding down.
pub(crate) fn rescale_floor(m: i64, from_rate: u32, to_rate: u32) -> i64 {
    mul_div_floor(m, i64::from(to_rate), i64::from(from_rate.max(1)))
}

/// Rates a measured rate snaps to when within 0.1 % (media-io probe step 8).
const STANDARD_RATES: [(i64, i64); 14] = [
    (24_000, 1001),
    (24, 1),
    (25, 1),
    (30_000, 1001),
    (30, 1),
    (48_000, 1001),
    (48, 1),
    (50, 1),
    (60_000, 1001),
    (60, 1),
    (100, 1),
    (120_000, 1001),
    (120, 1),
    (240, 1),
];

/// A usable frame rate: positive and below 1000 fps (MLT ignores
/// `r_frame_rate >= 1000`, which some demuxers report for "unknown").
fn usable_rate(num: i32, den: i32) -> Option<(i64, i64)> {
    (num > 0 && den > 0 && i64::from(num) < 1000 * i64::from(den))
        .then_some((i64::from(num), i64::from(den)))
}

/// `(num, den)` snapped to the nearest standard rate within 0.1 %, else
/// reduced. (24 and 23.976 are exactly 0.1 % apart, so "nearest" matters.)
fn snap((num, den): (i64, i64)) -> Option<Rational> {
    // Relative difference to sn/sd is diff / (sn * den), with
    // diff = |num * sd - sn * den|; candidates are compared cross-multiplied.
    let mut best: Option<(i128, i128, (i64, i64))> = None;
    for (sn, sd) in STANDARD_RATES {
        let diff = (i128::from(num) * i128::from(sd) - i128::from(sn) * i128::from(den)).abs();
        let scale = i128::from(sn) * i128::from(den);
        if diff * 1000 > scale {
            continue;
        }
        let better = match best {
            None => true,
            Some((bd, bs, _)) => diff * bs < bd * scale,
        };
        if better {
            best = Some((diff, scale, (sn, sd)));
        }
    }
    match best {
        Some((_, _, (sn, sd))) => Rational::new(sn, sd).ok(),
        None => Rational::new(num, den).ok(),
    }
}

/// The nominal frame rate: FFmpeg's guess (`av_guess_frame_rate`, which
/// takes the stream's real base rate `r_frame_rate` and falls back to the
/// average only when the base rate is a time-base artifact; media-io probe
/// step 8) when usable, else the average, snapped to a standard rate within
/// 0.1 %. The average comes second because demuxers measure it from
/// timestamps, which Matroska rounds to milliseconds (29.97 fps measures as
/// 1000/33, L10).
pub(crate) fn nominal_rate(guess: (i32, i32), avg: (i32, i32)) -> Option<Rational> {
    usable_rate(guess.0, guess.1)
        .or_else(|| usable_rate(avg.0, avg.1))
        .and_then(snap)
}

/// Whether two snapped rates differ by more than the time base explains:
/// more than 0.1 %, and more than one time-base unit per frame at rate `b`
/// (an average measured from 1 ms timestamps can be off by that much, L10).
fn rates_disagree(a: Rational, b: Rational, tb: (i32, i32)) -> bool {
    let (an, ad) = (i128::from(a.num()), i128::from(a.den()));
    let (bn, bd) = (i128::from(b.num()), i128::from(b.den()));
    // |a − b| / b = diff / scale.
    let diff = (an * bd - bn * ad).abs();
    let scale = bn * ad;
    if diff * 1000 <= scale {
        return false;
    }
    // One unit (tn / td s) relative to one frame (bd / bn s) is
    // tn * bn / (td * bd); a time base that isn't positive allows nothing.
    let (tn, td) = if tb.0 > 0 && tb.1 > 0 {
        (i128::from(tb.0), i128::from(tb.1))
    } else {
        (0, 1)
    };
    diff * td * bd > scale * tn * bn
}

/// VFR heuristic (PM-035, media-io probe step 8).
///
/// `pts` are packet timestamps in units of the time base `tb` (any order;
/// B-frames reorder them). The stream is variable-rate when the spread of
/// the deltas between sorted timestamps exceeds 1 % of their median *and*
/// two time-base units (which absorbs millisecond-rounded containers, L10),
/// or when the average and base rates disagree after snapping by more than
/// 0.1 % and more than the time base's rounding explains (integer multiples
/// excepted: field-rate base rates of interlaced streams).
pub(crate) fn is_variable_rate(
    pts: &[i64],
    avg: (i32, i32),
    base: (i32, i32),
    tb: (i32, i32),
) -> bool {
    let mut sorted: Vec<i64> = pts.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut deltas: Vec<i128> = sorted
        .windows(2)
        .map(|w| i128::from(w[1]) - i128::from(w[0]))
        .collect();
    if deltas.len() >= 2 {
        deltas.sort_unstable();
        let min = deltas[0];
        let max = deltas[deltas.len() - 1];
        let median = deltas[deltas.len() / 2];
        if median > 0 && (max - min) * 100 > median && max - min > 2 {
            return true;
        }
    }
    match (
        usable_rate(avg.0, avg.1).and_then(snap),
        usable_rate(base.0, base.1).and_then(snap),
    ) {
        (Some(a), Some(b)) if a != b => {
            // base / avg being a whole number means a field-rate base.
            let field_rate = b.checked_div(a).is_ok_and(|ratio| ratio.den() == 1);
            !field_rate && rates_disagree(a, b, tb)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn media_time_subtracts_the_origin() {
        let c = StreamClock::new(1, 90_000, Tick::from_raw(7_840 * 9_000)).unwrap();
        assert_eq!(c.media_time(9_000), Ok(Tick::ZERO));
        assert_eq!(c.media_time(18_000), Ok(Tick::from_raw(70_560_000)));
        assert_eq!(c.media_time(0), Ok(Tick::from_raw(-70_560_000)));
    }

    #[test]
    fn sample_math_is_exact_for_common_rates() {
        let t = Tick::from_raw(TICKS_PER_SECOND);
        assert_eq!(sample_floor(t, 48_000), 48_000);
        assert_eq!(sample_floor(Tick::from_raw(-1), 48_000), -1);
        assert_eq!(sample_time(48_000, 48_000), t);
        assert_eq!(sample_time(-1, 48_000), Tick::from_raw(-14_700));
        assert_eq!(sample_nearest(Tick::from_raw(14_699), 48_000), 1);
        assert_eq!(sample_nearest(Tick::from_raw(7_349), 48_000), 0);
        assert_eq!(rescale_ceil(1470, 44_100, 48_000), 1600);
        assert_eq!(rescale_ceil(1, 44_100, 48_000), 2);
        assert_eq!(rescale_floor(1, 44_100, 48_000), 1);
        assert_eq!(rescale_floor(-1, 44_100, 48_000), -2);
    }

    #[test]
    fn rates_snap_to_standard_values() {
        assert_eq!(nominal_rate((25, 1), (25, 1)), Rational::new(25, 1).ok());
        // An average measured over a 1/15360 time base.
        assert_eq!(
            nominal_rate((30_000, 1001), (1_920_000, 64_063)),
            Rational::new(30_000, 1001).ok()
        );
        // Without a usable guess the average is used.
        assert_eq!(nominal_rate((0, 0), (24, 1)), Rational::new(24, 1).ok());
        assert_eq!(
            nominal_rate((24_000, 1001), (0, 1)),
            Rational::new(24_000, 1001).ok()
        );
        assert_eq!(nominal_rate((0, 1), (2399, 100)), Rational::new(24, 1).ok());
        assert_eq!(nominal_rate((60, 1), (60, 1)), Rational::new(60, 1).ok());
        // "Unknown" base rates of 1000 fps and more are ignored (MLT).
        assert_eq!(nominal_rate((90_000, 1), (0, 1)), None);
        assert_eq!(
            nominal_rate((90_000, 1), (25, 1)),
            Rational::new(25, 1).ok()
        );
        // Off-standard rates stay exact.
        assert_eq!(nominal_rate((15, 1), (15, 1)), Rational::new(15, 1).ok());
    }

    #[test]
    fn matroska_millisecond_average_does_not_change_the_rate() {
        // FFmpeg measures 29.97 fps in 1 ms timestamps as 1000/33 (L10).
        assert_eq!(
            nominal_rate((30_000, 1001), (1000, 33)),
            Rational::new(30_000, 1001).ok()
        );
        let pts: Vec<i64> = (0..90).map(|k| (k * 1001 + 15) / 30).collect();
        assert!(!is_variable_rate(
            &pts,
            (1000, 33),
            (30_000, 1001),
            (1, 1000)
        ));
        // In a fine time base the same disagreement is real.
        let pts: Vec<i64> = (0..90).map(|k| k * 1001).collect();
        assert!(is_variable_rate(
            &pts,
            (1000, 33),
            (30_000, 1001),
            (1, 30_000)
        ));
    }

    #[test]
    fn cfr_with_coarse_rounding_is_not_vfr() {
        // 29.97 fps in a 1 ms time base: deltas of 33 and 34.
        let pts: Vec<i64> = (0..300).map(|k| (k * 1001 + 15) / 30).collect();
        assert!(!is_variable_rate(
            &pts,
            (30_000, 1001),
            (30_000, 1001),
            (1, 1000)
        ));
        // 29.97 fps in 1/15360: deltas of 512 and 513.
        let pts: Vec<i64> = (0..300)
            .map(|k| (k * 15_360 * 1001 + 15_000) / 30_000)
            .collect();
        assert!(!is_variable_rate(
            &pts,
            (30_000, 1001),
            (30_000, 1001),
            (1, 15_360)
        ));
    }

    #[test]
    fn reordered_cfr_is_not_vfr_and_jitter_is() {
        let mut pts: Vec<i64> = (0..60).map(|k| k * 512).collect();
        pts.swap(3, 5);
        pts.swap(10, 11);
        assert!(!is_variable_rate(&pts, (30, 1), (30, 1), (1, 15_360)));
        let jitter: Vec<i64> = (0..60).map(|k| k * 3000 + (k % 3) * 150).collect();
        assert!(is_variable_rate(&jitter, (30, 1), (30, 1), (1, 90_000)));
    }

    #[test]
    fn rate_disagreement_is_vfr_except_field_rates() {
        let pts: Vec<i64> = (0..10).map(|k| k * 1000).collect();
        assert!(is_variable_rate(&pts, (24, 1), (30, 1), (1, 30_000)));
        assert!(!is_variable_rate(&pts, (25, 1), (50, 1), (1, 50_000)));
        // Even a 1 ms time base doesn't excuse 24 vs 30 fps.
        assert!(is_variable_rate(&pts, (24, 1), (30, 1), (1, 1000)));
    }

    proptest! {
        #[test]
        fn seek_target_never_follows_a_wanted_timestamp(
            t in -1_000_000_000_000i64..1_000_000_000_000,
            origin in -1_000_000_000i64..1_000_000_000,
            (num, den) in prop_oneof![Just((1, 15_360)), Just((1, 90_000)), Just((1001, 30_000)), Just((1, 1000))],
        ) {
            let c = StreamClock::new(num, den, Tick::from_raw(origin)).unwrap();
            let t = Tick::from_raw(t);
            let ts = c.ts_at_or_before(t).unwrap();
            prop_assert!(c.media_time(ts).unwrap() <= t);
            prop_assert!(c.media_time(ts + 1).unwrap() > t);
        }

        #[test]
        fn sample_round_trip(m in -10_000_000_000i64..10_000_000_000, rate in prop_oneof![Just(8_000u32), Just(44_100), Just(48_000), Just(96_000)]) {
            prop_assert_eq!(sample_floor(sample_time(m, rate), rate), m);
            prop_assert_eq!(sample_nearest(sample_time(m, rate), rate), m);
        }
    }
}
