//! The input level meter (AU-027): peak, RMS, held peak and the clip
//! latch, measured after the input gain by the capture callback and read by
//! the UI through atomics.
//!
//! Ballistics (FreeCut defaults): the peak falls at 20 dB/s, the RMS is an
//! exponential average over 300 ms, the held peak stays 1.5 s before it
//! falls with the peak, and the clip light latches at −0.1 dBFS until it is
//! cleared or a new take starts.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// What the meter reads (all after the input gain). Levels are in dBFS,
/// floored at [`METER_FLOOR_DB`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeterReading {
    pub peak_db: f32,
    pub rms_db: f32,
    pub held_peak_db: f32,
    /// Latched: some sample reached [`CLIP_DB`] since the last reset.
    pub clipped: bool,
}

/// The lowest level a reading reports (silence).
pub const METER_FLOOR_DB: f32 = -120.0;
/// A sample at or above this level lights the clip latch.
pub const CLIP_DB: f32 = -0.1;
/// How long the held peak stays before it falls.
pub const HOLD_SECS: f32 = 1.5;
/// How fast the peak falls.
pub const FALL_DB_PER_SEC: f32 = 20.0;
/// The RMS averaging time constant.
pub const RMS_SECS: f32 = 0.3;

/// Linear amplitude to dBFS, floored.
pub(crate) fn to_db(linear: f32) -> f32 {
    if linear > 0.0 {
        (20.0 * linear.log10()).max(METER_FLOOR_DB)
    } else {
        METER_FLOOR_DB
    }
}

/// The published values (linear, as `f32` bits).
#[derive(Debug, Default)]
pub(crate) struct MeterCells {
    peak: AtomicU32,
    rms: AtomicU32,
    held: AtomicU32,
    clipped: AtomicBool,
}

impl MeterCells {
    pub(crate) fn read(&self) -> MeterReading {
        let get = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        MeterReading {
            peak_db: to_db(get(&self.peak)),
            rms_db: to_db(get(&self.rms)),
            held_peak_db: to_db(get(&self.held)),
            clipped: self.clipped.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn reset_clip(&self) {
        self.clipped.store(false, Ordering::Relaxed);
    }
}

/// The callback's side: running state, published once per block.
/// Real-time safe (no allocation, no locks).
#[derive(Debug)]
pub(crate) struct MeterState {
    rate: f32,
    clip: f32,
    rms_coeff: f32,
    block_peak: f32,
    clipped: bool,
    frames: u32,
    peak: f32,
    mean_square: f32,
    held: f32,
    hold_left: f32,
}

impl MeterState {
    pub(crate) fn new(rate: u32) -> MeterState {
        let rate = rate.max(1) as f32;
        MeterState {
            rate,
            clip: 10f32.powf(CLIP_DB / 20.0),
            rms_coeff: 1.0 - (-1.0 / (RMS_SECS * rate)).exp(),
            block_peak: 0.0,
            clipped: false,
            frames: 0,
            peak: 0.0,
            mean_square: 0.0,
            held: 0.0,
            hold_left: 0.0,
        }
    }

    /// One frame of the take's channels.
    #[inline]
    pub(crate) fn frame(&mut self, samples: &[f32]) {
        let mut square = 0.0f32;
        for &s in samples {
            let a = s.abs();
            if a > self.block_peak {
                self.block_peak = a;
            }
            if a >= self.clip {
                self.clipped = true;
            }
            square += s * s;
        }
        let square = square / samples.len().max(1) as f32;
        self.mean_square += (square - self.mean_square) * self.rms_coeff;
        self.frames += 1;
    }

    /// Applies the block's ballistics and publishes the values.
    pub(crate) fn publish(&mut self, cells: &MeterCells) {
        let secs = self.frames as f32 / self.rate;
        let fall = 10f32.powf(-FALL_DB_PER_SEC * secs / 20.0);
        self.peak = self.block_peak.max(self.peak * fall);
        if self.block_peak >= self.held {
            self.held = self.block_peak;
            self.hold_left = HOLD_SECS;
        } else if self.hold_left > secs {
            self.hold_left -= secs;
        } else {
            self.hold_left = 0.0;
            self.held = self.peak;
        }
        cells.peak.store(self.peak.to_bits(), Ordering::Relaxed);
        cells.rms.store(
            self.mean_square.max(0.0).sqrt().to_bits(),
            Ordering::Relaxed,
        );
        cells.held.store(self.held.to_bits(), Ordering::Relaxed);
        if self.clipped {
            cells.clipped.store(true, Ordering::Relaxed);
        }
        self.block_peak = 0.0;
        self.clipped = false;
        self.frames = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(state: &mut MeterState, cells: &MeterCells, value: f32, frames: usize) {
        for _ in 0..frames / 100 {
            for _ in 0..100 {
                state.frame(&[value, -value]);
            }
            state.publish(cells);
        }
    }

    #[test]
    fn a_steady_level_reads_the_same_peak_rms_and_hold() {
        let cells = MeterCells::default();
        let mut m = MeterState::new(8_000);
        run(&mut m, &cells, 0.5, 40_000);
        let r = cells.read();
        let half = 20.0 * 0.5f32.log10();
        assert!((r.peak_db - half).abs() < 1e-3, "{r:?}");
        assert!((r.rms_db - half).abs() < 1e-3, "{r:?}");
        assert!((r.held_peak_db - half).abs() < 1e-3, "{r:?}");
        assert!(!r.clipped);
    }

    #[test]
    fn the_peak_falls_the_hold_waits_and_the_clip_latches() {
        let cells = MeterCells::default();
        let mut m = MeterState::new(8_000);
        run(&mut m, &cells, 1.0, 800);
        assert!(cells.read().clipped);
        // 1 s of silence: the peak fell 20 dB, the held peak still holds.
        run(&mut m, &cells, 0.0, 8_000);
        let r = cells.read();
        assert!((r.peak_db + 20.0).abs() < 0.01, "{r:?}");
        assert!(r.held_peak_db.abs() < 1e-3, "{r:?}");
        assert!(r.clipped, "the latch stays");
        // After 1.5 s the held peak falls with the peak.
        run(&mut m, &cells, 0.0, 4_800);
        let r = cells.read();
        assert!((r.held_peak_db - r.peak_db).abs() < 1e-3, "{r:?}");
        assert!(r.peak_db < -31.0, "{r:?}");
        cells.reset_clip();
        assert!(!cells.read().clipped);
        // Just under −0.1 dBFS doesn't clip.
        run(&mut m, &cells, 0.988, 100);
        assert!(!cells.read().clipped);
        run(&mut m, &cells, 0.989, 100);
        assert!(cells.read().clipped);
    }

    #[test]
    fn silence_reads_the_floor() {
        let cells = MeterCells::default();
        let r = cells.read();
        assert_eq!(
            (r.peak_db, r.rms_db, r.held_peak_db),
            (METER_FLOOR_DB, METER_FLOOR_DB, METER_FLOOR_DB)
        );
        assert_eq!(to_db(1.0), 0.0);
    }
}
