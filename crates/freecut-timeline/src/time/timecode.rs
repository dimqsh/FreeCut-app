//! `HH:MM:SS:FF` timecode (TL-002), integer-only.

use super::{FrameRate, Tick, TimeError};

/// A frame-counted timecode. Hours are unbounded (no 24 h wrap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timecode {
    pub hours: u64,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u32,
    /// SMPTE drop-frame counting (shown with `;` before the frame field).
    pub drop_frame: bool,
}

/// `(frames dropped per minute, frames per 10 minutes, frames per minute)` for
/// the rates that have a drop-frame variant.
fn drop_frame_params(rate: FrameRate) -> Option<(i64, i64, i64)> {
    match (rate.num(), rate.den()) {
        (30_000, 1001) => Some((2, 17_982, 1_798)),
        (60_000, 1001) => Some((4, 35_964, 3_596)),
        _ => None,
    }
}

impl Timecode {
    /// True for 29.97 and 59.94, the rates where drop-frame counting exists.
    pub fn supports_drop_frame(rate: FrameRate) -> bool {
        drop_frame_params(rate).is_some()
    }

    /// The timecode of the frame displayed at `t`. `drop_frame` is ignored for
    /// rates without a drop-frame variant.
    pub fn from_tick(t: Tick, rate: FrameRate, drop_frame: bool) -> Result<Timecode, TimeError> {
        if t.is_negative() {
            return Err(TimeError::Negative);
        }
        let mut n = rate.frame_index(t);
        let params = if drop_frame {
            drop_frame_params(rate)
        } else {
            None
        };
        if let Some((drop, per10, per_min)) = params {
            let tens = n / per10;
            let rem = n % per10;
            let mut skipped = 9 * drop * tens;
            if rem >= drop {
                skipped += drop * ((rem - drop) / per_min);
            }
            n = n.checked_add(skipped).ok_or(TimeError::Overflow)?;
        }
        let fps = i64::from(rate.nominal_fps());
        let frames = n % fps;
        let total_secs = n / fps;
        // Casts below are bounded by the moduli (frames < fps <= u32::MAX,
        // seconds and minutes < 60) and by n >= 0.
        Ok(Timecode {
            hours: u64::try_from(total_secs / 3600).map_err(|_| TimeError::Overflow)?,
            minutes: u8::try_from((total_secs / 60) % 60).map_err(|_| TimeError::Overflow)?,
            seconds: u8::try_from(total_secs % 60).map_err(|_| TimeError::Overflow)?,
            frames: u32::try_from(frames).map_err(|_| TimeError::Overflow)?,
            drop_frame: params.is_some(),
        })
    }
}

impl core::fmt::Display for Timecode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let sep = if self.drop_frame { ';' } else { ':' };
        write!(
            f,
            "{:02}:{:02}:{:02}{sep}{:02}",
            self.hours, self.minutes, self.seconds, self.frames
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_frame(rate: FrameRate, n: i64) -> Tick {
        rate.frame_start(n).unwrap()
    }

    #[test]
    fn non_drop_formatting() {
        let r = FrameRate::new(25, 1).unwrap();
        let tc = Timecode::from_tick(at_frame(r, 25 * 3661 + 7), r, false).unwrap();
        assert_eq!(tc.to_string(), "01:01:01:07");
        let r = FrameRate::new(30_000, 1001).unwrap();
        assert_eq!(
            Timecode::from_tick(at_frame(r, 1800), r, false)
                .unwrap()
                .to_string(),
            "00:01:00:00"
        );
    }

    #[test]
    fn drop_frame_known_vectors() {
        let r = FrameRate::new(30_000, 1001).unwrap();
        let tc = |n| {
            Timecode::from_tick(at_frame(r, n), r, true)
                .unwrap()
                .to_string()
        };
        assert_eq!(tc(1799), "00:00:59;29");
        assert_eq!(tc(1800), "00:01:00;02");
        assert_eq!(tc(17_982), "00:10:00;00");
        assert_eq!(tc(17_981), "00:09:59;29");
    }

    #[test]
    fn hours_do_not_wrap_and_negative_is_refused() {
        let r = FrameRate::new(24, 1).unwrap();
        let tc = Timecode::from_tick(at_frame(r, 24 * 3600 * 120), r, false).unwrap();
        assert_eq!(tc.to_string(), "120:00:00:00");
        assert_eq!(
            Timecode::from_tick(Tick::from_raw(-1), r, false),
            Err(TimeError::Negative)
        );
    }

    #[test]
    fn drop_frame_is_ignored_for_integer_rates() {
        let r = FrameRate::new(30, 1).unwrap();
        assert!(!Timecode::from_tick(Tick::ZERO, r, true).unwrap().drop_frame);
    }
}
