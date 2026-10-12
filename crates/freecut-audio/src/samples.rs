//! Exact conversion between model time and sample positions (AU-010).
//!
//! Sample `k` at rate `r` covers `[k·TPS/r, (k+1)·TPS/r)` ticks. The tick
//! that maps to a sample is `floor(t·r / TPS)`, computed in `i128` so no
//! timeline length can overflow. Going back, sample `k` starts at the
//! smallest tick that maps to it, `ceil(k·TPS / r)`; every [`SampleRate`]
//! divides [`TICKS_PER_SECOND`], so that is exactly `k · (TPS / r)`.

use freecut_timeline::time::{SampleRate, TICKS_PER_SECOND, Tick, TimeError};

/// Index of the sample at `rate` that covers tick `t`:
/// `floor(t · rate / TICKS_PER_SECOND)`.
pub fn sample_at(t: Tick, rate: SampleRate) -> i64 {
    let n = i128::from(t.raw()) * i128::from(rate.hz());
    let k = n.div_euclid(i128::from(TICKS_PER_SECOND));
    // |k| <= |t| because rate < TICKS_PER_SECOND, so it fits in i64.
    i64::try_from(k).unwrap_or(if k < 0 { i64::MIN } else { i64::MAX })
}

/// First tick of sample `k` at `rate`: the smallest `t` with
/// `sample_at(t, rate) == k`, i.e. `ceil(k · TICKS_PER_SECOND / rate)`.
///
/// Fails with [`TimeError::Overflow`] when that tick doesn't fit in `i64`.
pub fn sample_start(k: i64, rate: SampleRate) -> Result<Tick, TimeError> {
    let n = i128::from(k) * i128::from(TICKS_PER_SECOND);
    let d = i128::from(rate.hz());
    let t = n.div_euclid(d) + i128::from(n.rem_euclid(d) != 0);
    i64::try_from(t)
        .map(Tick::from_raw)
        .map_err(|_| TimeError::Overflow)
}

/// Ticks per sample at `rate`. Exact: every [`SampleRate`] divides the tick
/// rate.
pub(crate) fn sample_ticks(rate: SampleRate) -> i64 {
    rate.sample_ticks().raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(hz: u32) -> SampleRate {
        SampleRate::new(hz).unwrap()
    }

    #[test]
    fn round_trips_are_exact_at_common_rates() {
        for hz in [44_100, 48_000, 96_000, 32_000, 22_050, 88_200] {
            let r = rate(hz);
            for k in [-5_000_000_i64, -1, 0, 1, 2, 47_999, 48_000, 1_234_567] {
                let t = sample_start(k, r).unwrap();
                assert_eq!(sample_at(t, r), k, "{hz} Hz, sample {k}");
                // One tick earlier belongs to the previous sample.
                let before = t.checked_sub(Tick::from_raw(1)).unwrap();
                assert_eq!(sample_at(before, r), k - 1, "{hz} Hz, sample {k}");
                assert_eq!(t.raw(), k * sample_ticks(r));
            }
        }
    }

    #[test]
    fn whole_seconds_land_on_whole_samples() {
        let r = rate(44_100);
        assert_eq!(sample_at(Tick::from_secs(3).unwrap(), r), 132_300);
        assert_eq!(
            sample_start(44_100, r).unwrap(),
            Tick::from_secs(1).unwrap()
        );
        let r = rate(48_000);
        assert_eq!(sample_at(Tick::from_raw(14_699), r), 0);
        assert_eq!(sample_at(Tick::from_raw(14_700), r), 1);
        assert_eq!(sample_at(Tick::from_raw(-1), r), -1);
    }

    #[test]
    fn long_timelines_do_not_overflow() {
        let r = rate(48_000);
        // i64::MAX ticks is about 413 years.
        let k = sample_at(Tick::MAX, r);
        assert_eq!(k, (i128::from(i64::MAX) * 48_000 / 705_600_000) as i64);
        let t = sample_start(k, r).unwrap();
        assert!(t <= Tick::MAX);
        assert_eq!(sample_at(t, r), k);
        assert_eq!(sample_start(k + 1, r), Err(TimeError::Overflow));
        assert_eq!(sample_at(Tick::from_raw(i64::MIN), r), {
            let n = i128::from(i64::MIN) * 48_000;
            n.div_euclid(705_600_000) as i64
        });
        // A 24-hour timeline at 48 kHz.
        let day = Tick::from_secs(86_400).unwrap();
        assert_eq!(sample_at(day, r), 86_400 * 48_000);
    }
}
