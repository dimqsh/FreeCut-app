//! Frame and sample rates on the tick grid.

use super::{Rounding, TICKS_PER_SECOND, Tick, TimeError, div_round};

fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// A sequence frame rate `num/den` frames per second whose frame duration is a
/// whole number of ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameRate {
    num: u32,
    den: u32,
    /// Ticks per frame.
    frame: i64,
}

impl FrameRate {
    /// The rates offered in project settings (PM-005), as exact rationals.
    pub const PRESETS: [(u32, u32); 10] = [
        (24_000, 1001),
        (24, 1),
        (25, 1),
        (30_000, 1001),
        (30, 1),
        (48, 1),
        (50, 1),
        (60_000, 1001),
        (60, 1),
        (120, 1),
    ];

    pub fn new(num: u32, den: u32) -> Result<FrameRate, TimeError> {
        if num == 0 || den == 0 {
            return Err(TimeError::InvalidRate { num, den });
        }
        let g = gcd_u32(num, den);
        let (num, den) = (num / g, den / g);
        let total = i128::from(TICKS_PER_SECOND) * i128::from(den);
        if total % i128::from(num) != 0 {
            return Err(TimeError::OffGridRate { num, den });
        }
        let frame = i64::try_from(total / i128::from(num)).map_err(|_| TimeError::Overflow)?;
        Ok(FrameRate { num, den, frame })
    }

    pub const fn num(self) -> u32 {
        self.num
    }

    pub const fn den(self) -> u32 {
        self.den
    }

    /// Duration of one frame.
    pub const fn frame_ticks(self) -> Tick {
        Tick::from_raw(self.frame)
    }

    /// Nearest frame boundary; exact halves go to the earlier frame (TL-001).
    pub fn quantize(self, t: Tick) -> Result<Tick, TimeError> {
        self.round_to_grid(t, Rounding::NearestTiesDown)
    }

    /// Frame boundary chosen by `mode`.
    pub fn round_to_grid(self, t: Tick, mode: Rounding) -> Result<Tick, TimeError> {
        let k = div_round(i128::from(t.raw()), i128::from(self.frame), mode);
        i64::try_from(k * i128::from(self.frame))
            .map(Tick::from_raw)
            .map_err(|_| TimeError::Overflow)
    }

    /// Index of the frame displayed at `t` (frame `k` covers `[k·f, (k+1)·f)`).
    pub const fn frame_index(self, t: Tick) -> i64 {
        t.raw().div_euclid(self.frame)
    }

    /// Start of frame `k`.
    pub fn frame_start(self, k: i64) -> Result<Tick, TimeError> {
        k.checked_mul(self.frame)
            .map(Tick::from_raw)
            .ok_or(TimeError::Overflow)
    }

    /// `n` frames as a span.
    pub fn frames(self, n: i64) -> Result<Tick, TimeError> {
        self.frame_start(n)
    }

    pub const fn is_on_grid(self, t: Tick) -> bool {
        t.raw().rem_euclid(self.frame) == 0
    }

    /// Frames per second rounded up, as used for timecode frame fields
    /// (30 for 29.97).
    pub const fn nominal_fps(self) -> u32 {
        self.num.div_ceil(self.den)
    }
}

impl core::fmt::Display for FrameRate {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.den == 1 {
            write!(f, "{} fps", self.num)
        } else {
            write!(f, "{}/{} fps", self.num, self.den)
        }
    }
}

/// An audio sample rate on the tick grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SampleRate(u32);

impl SampleRate {
    pub fn new(hz: u32) -> Result<SampleRate, TimeError> {
        if hz == 0 || TICKS_PER_SECOND % i64::from(hz) != 0 {
            return Err(TimeError::OffGridRate { num: hz, den: 1 });
        }
        Ok(SampleRate(hz))
    }

    pub const fn hz(self) -> u32 {
        self.0
    }

    pub const fn sample_ticks(self) -> Tick {
        Tick::from_raw(TICKS_PER_SECOND / self.0 as i64)
    }

    /// Index of the sample covering `t`.
    pub const fn sample_index(self, t: Tick) -> i64 {
        t.raw().div_euclid(TICKS_PER_SECOND / self.0 as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_rate_is_on_the_tick_grid() {
        let expected = [
            29_429_400, 29_400_000, 28_224_000, 23_543_520, 23_520_000, 14_700_000, 14_112_000,
            11_771_760, 11_760_000, 5_880_000,
        ];
        for ((num, den), ticks) in FrameRate::PRESETS.into_iter().zip(expected) {
            let rate = FrameRate::new(num, den).unwrap();
            assert_eq!(rate.frame_ticks().raw(), ticks, "{num}/{den}");
        }
    }

    #[test]
    fn rates_are_reduced_and_off_grid_rates_refused() {
        let r = FrameRate::new(60, 2).unwrap();
        assert_eq!((r.num(), r.den()), (30, 1));
        assert_eq!(
            FrameRate::new(11, 1).unwrap_err(),
            TimeError::OffGridRate { num: 11, den: 1 }
        );
        assert!(FrameRate::new(0, 1).is_err());
        assert!(FrameRate::new(1, 0).is_err());
    }

    #[test]
    fn quantize_rounds_half_down() {
        let r = FrameRate::new(25, 1).unwrap();
        let f = r.frame_ticks().raw();
        assert_eq!(r.quantize(Tick::from_raw(f / 2)).unwrap(), Tick::ZERO);
        assert_eq!(
            r.quantize(Tick::from_raw(f / 2 + 1)).unwrap(),
            Tick::from_raw(f)
        );
        assert_eq!(
            r.quantize(Tick::from_raw(3 * f - 1)).unwrap(),
            Tick::from_raw(3 * f)
        );
        assert_eq!(
            r.quantize(Tick::from_raw(-f / 2)).unwrap(),
            Tick::from_raw(-f)
        );
    }

    #[test]
    fn frame_index_floors() {
        let r = FrameRate::new(30_000, 1001).unwrap();
        let f = r.frame_ticks().raw();
        assert_eq!(r.frame_index(Tick::from_raw(0)), 0);
        assert_eq!(r.frame_index(Tick::from_raw(f - 1)), 0);
        assert_eq!(r.frame_index(Tick::from_raw(f)), 1);
        assert_eq!(r.frame_index(Tick::from_raw(-1)), -1);
        assert_eq!(r.nominal_fps(), 30);
    }

    #[test]
    fn sample_rates() {
        for hz in [
            8_000, 11_025, 16_000, 22_050, 32_000, 44_100, 48_000, 88_200, 96_000, 192_000,
        ] {
            assert!(SampleRate::new(hz).is_ok(), "{hz}");
        }
        assert!(SampleRate::new(384_000).is_err());
        assert_eq!(
            SampleRate::new(48_000).unwrap().sample_ticks().raw(),
            14_700
        );
    }
}
