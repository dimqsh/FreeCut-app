//! Export progress (EX-022): a lock-free cell the job writes and the UI
//! reads on repaint, the ETA window, and the strings the progress and
//! result views show.
//!
//! - Progress is `0.99 × frames muxed / N` while rendering; finishing fills
//!   the last 1 % (export-pipeline §8).
//! - The ETA uses the average frame rate of the last 10 s and is shown after
//!   3 s.
//! - Speed is that rate over the output frame rate ("1.8× real time").

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::Duration;

/// Where an export job is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Snapshot, settings, claiming the file, opening the encoder.
    Preparing,
    /// The frame loop.
    Rendering,
    /// Encoder drain, trailer and fast-start rewrite, verification, fsync,
    /// rename.
    Finishing,
    /// The job has ended (done, failed or cancelled).
    Ended,
}

impl Phase {
    const fn to_u8(self) -> u8 {
        match self {
            Phase::Preparing => 0,
            Phase::Rendering => 1,
            Phase::Finishing => 2,
            Phase::Ended => 3,
        }
    }

    const fn from_u8(v: u8) -> Phase {
        match v {
            0 => Phase::Preparing,
            1 => Phase::Rendering,
            2 => Phase::Finishing,
            _ => Phase::Ended,
        }
    }

    /// The label of the progress view.
    pub const fn label(self) -> &'static str {
        match self {
            Phase::Preparing => "Preparing",
            Phase::Rendering => "Rendering",
            Phase::Finishing => "Finishing",
            Phase::Ended => "Done",
        }
    }
}

/// Shared between the job (writer) and the UI (reader). No locks, no
/// channel backlog: the UI reads the latest values whenever it repaints.
#[derive(Debug, Default)]
pub struct ProgressCell {
    phase: AtomicU8,
    frames_total: AtomicU64,
    /// Frames handed to the encoder.
    frames_rendered: AtomicU64,
    /// Video packets written to the file: the progress measure. It trails
    /// `frames_rendered` by the encoder's lookahead.
    frames_muxed: AtomicU64,
}

/// One reading of a [`ProgressCell`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub frames_total: u64,
    pub frames_rendered: u64,
    pub frames_muxed: u64,
}

impl ProgressCell {
    pub fn set_phase(&self, phase: Phase) {
        self.phase.store(phase.to_u8(), Ordering::Release);
    }

    pub fn set_total(&self, frames: u64) {
        self.frames_total.store(frames, Ordering::Release);
    }

    pub fn set_frames(&self, rendered: u64, muxed: u64) {
        self.frames_rendered.store(rendered, Ordering::Release);
        self.frames_muxed.store(muxed, Ordering::Release);
    }

    pub fn read(&self) -> Progress {
        Progress {
            phase: Phase::from_u8(self.phase.load(Ordering::Acquire)),
            frames_total: self.frames_total.load(Ordering::Acquire),
            frames_rendered: self.frames_rendered.load(Ordering::Acquire),
            frames_muxed: self.frames_muxed.load(Ordering::Acquire),
        }
    }
}

impl Progress {
    /// 0..=1 for the progress bar: 99 % for the frames, the last 1 % for
    /// finishing.
    pub fn fraction(&self) -> f32 {
        match self.phase {
            Phase::Preparing => 0.0,
            Phase::Rendering => {
                if self.frames_total == 0 {
                    0.0
                } else {
                    let done = self.frames_muxed.min(self.frames_total);
                    (0.99 * done as f64 / self.frames_total as f64) as f32
                }
            }
            Phase::Finishing => 0.99,
            Phase::Ended => 1.0,
        }
    }
}

/// How far back the ETA's frame rate looks.
pub const ETA_WINDOW: Duration = Duration::from_secs(10);
/// The ETA is hidden before the job has run this long.
pub const ETA_DELAY: Duration = Duration::from_secs(3);

/// `(elapsed, frames muxed)` samples of the last [`ETA_WINDOW`], for the
/// ETA and the speed.
#[derive(Debug, Clone, Default)]
pub struct EtaWindow {
    samples: VecDeque<(Duration, u64)>,
}

impl EtaWindow {
    /// Records that `frames` were done `elapsed` after the start. Samples
    /// older than the window are dropped, except the newest of them, so the
    /// rate always spans the whole window.
    pub fn push(&mut self, elapsed: Duration, frames: u64) {
        if let Some(last) = self.samples.back_mut()
            && last.0 >= elapsed
        {
            // The same instant (or a clock that went back): update it.
            last.1 = last.1.max(frames);
            return;
        }
        self.samples.push_back((elapsed, frames));
        while self.samples.len() > 2
            && self
                .samples
                .get(1)
                .is_some_and(|&(t, _)| t + ETA_WINDOW <= elapsed)
        {
            self.samples.pop_front();
        }
    }

    /// Frames per second over the window; `None` until two samples at
    /// different times exist.
    pub fn rate(&self) -> Option<f64> {
        let (&(t0, f0), &(t1, f1)) = (self.samples.front()?, self.samples.back()?);
        let secs = t1.checked_sub(t0)?.as_secs_f64();
        (secs > 0.0 && f1 >= f0).then(|| (f1 - f0) as f64 / secs)
    }

    /// Time left for `total − done` frames at the window's rate; `None`
    /// before [`ETA_DELAY`] or while nothing has moved.
    pub fn eta(&self, elapsed: Duration, done: u64, total: u64) -> Option<Duration> {
        if elapsed < ETA_DELAY {
            return None;
        }
        let rate = self.rate().filter(|r| *r > 0.0)?;
        let left = total.saturating_sub(done) as f64 / rate;
        Duration::try_from_secs_f64(left).ok()
    }

    /// Output frames per second over real seconds, as a multiple of the
    /// output frame rate `fps` ("1.8× real time").
    pub fn speed(&self, fps: f64) -> Option<f64> {
        (fps > 0.0).then_some(())?;
        self.rate().map(|r| r / fps)
    }
}

/// `m:ss`, or `h:mm:ss` from one hour (elapsed time, ETA, durations).
pub fn format_clock(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A byte count in decimal units, as file managers show it: "840 KB",
/// "12.4 MB", "1.2 GB".
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// "1.8× real time".
pub fn format_speed(speed: f64) -> String {
    if speed >= 10.0 {
        format!("{speed:.0}× real time")
    } else {
        format!("{speed:.1}× real time")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use super::*;

    fn s(secs: f64) -> Duration {
        Duration::from_secs_f64(secs)
    }

    #[test]
    fn the_bar_keeps_the_last_percent_for_finishing() {
        let p = |phase, muxed| Progress {
            phase,
            frames_total: 200,
            frames_rendered: muxed,
            frames_muxed: muxed,
        };
        assert_eq!(p(Phase::Preparing, 0).fraction(), 0.0);
        assert_eq!(p(Phase::Rendering, 0).fraction(), 0.0);
        assert!((p(Phase::Rendering, 100).fraction() - 0.495).abs() < 1e-6);
        assert!((p(Phase::Rendering, 200).fraction() - 0.99).abs() < 1e-6);
        // Never past 99 % while frames are still being written.
        assert!((p(Phase::Rendering, 500).fraction() - 0.99).abs() < 1e-6);
        assert!((p(Phase::Finishing, 200).fraction() - 0.99).abs() < 1e-6);
        assert_eq!(p(Phase::Ended, 200).fraction(), 1.0);
        let empty = Progress {
            frames_total: 0,
            ..p(Phase::Rendering, 0)
        };
        assert_eq!(empty.fraction(), 0.0);
    }

    #[test]
    fn the_cell_round_trips() {
        let cell = ProgressCell::default();
        assert_eq!(cell.read().phase, Phase::Preparing);
        cell.set_total(90);
        cell.set_phase(Phase::Rendering);
        cell.set_frames(50, 12);
        assert_eq!(
            cell.read(),
            Progress {
                phase: Phase::Rendering,
                frames_total: 90,
                frames_rendered: 50,
                frames_muxed: 12,
            }
        );
        for phase in [
            Phase::Preparing,
            Phase::Rendering,
            Phase::Finishing,
            Phase::Ended,
        ] {
            cell.set_phase(phase);
            assert_eq!(cell.read().phase, phase);
        }
    }

    #[test]
    fn eta_waits_three_seconds_then_uses_the_recent_rate() {
        let mut w = EtaWindow::default();
        // 30 frames per second for 2 s: no ETA yet.
        for i in 0..=20 {
            w.push(s(f64::from(i) * 0.1), u64::from(i as u32) * 3);
        }
        assert_eq!(w.eta(s(2.0), 60, 600), None);
        assert!((w.rate().unwrap() - 30.0).abs() < 1e-9);
        // At 4 s, 120 done of 600: 480 left at 30 fps = 16 s.
        for i in 21..=40 {
            w.push(s(f64::from(i) * 0.1), u64::from(i as u32) * 3);
        }
        let eta = w.eta(s(4.0), 120, 600).unwrap();
        assert!((eta.as_secs_f64() - 16.0).abs() < 1e-6, "{eta:?}");
        // Speed at a 30 fps output: real time.
        assert!((w.speed(30.0).unwrap() - 1.0).abs() < 1e-9);
        assert!((w.speed(15.0).unwrap() - 2.0).abs() < 1e-9);
        assert_eq!(w.speed(0.0), None);
    }

    #[test]
    fn eta_forgets_samples_older_than_ten_seconds() {
        let mut w = EtaWindow::default();
        // A slow start (1 fps for 20 s), then fast (10 fps for 10 s).
        for t in 0..=20 {
            w.push(s(f64::from(t)), u64::from(t as u32));
        }
        for t in 21..=30 {
            w.push(s(f64::from(t)), 20 + 10 * u64::from((t - 20) as u32));
        }
        let rate = w.rate().unwrap();
        assert!((rate - 10.0).abs() < 1e-9, "only the last 10 s: {rate}");
    }

    #[test]
    fn a_stalled_job_has_no_eta() {
        let mut w = EtaWindow::default();
        w.push(s(0.0), 0);
        w.push(s(5.0), 0);
        assert_eq!(w.rate(), Some(0.0));
        assert_eq!(w.eta(s(5.0), 0, 100), None);
        // One sample: no rate.
        let mut one = EtaWindow::default();
        one.push(s(4.0), 10);
        assert_eq!(one.rate(), None);
    }

    #[test]
    fn clocks_and_sizes_read_naturally() {
        assert_eq!(format_clock(s(0.0)), "0:00");
        assert_eq!(format_clock(s(5.4)), "0:05");
        assert_eq!(format_clock(s(65.0)), "1:05");
        assert_eq!(format_clock(s(3600.0 + 62.0)), "1:01:02");
        assert_eq!(format_bytes(0), "0 bytes");
        assert_eq!(format_bytes(999), "999 bytes");
        assert_eq!(format_bytes(1000), "1 KB");
        assert_eq!(format_bytes(840_000), "840 KB");
        assert_eq!(format_bytes(12_400_000), "12.4 MB");
        assert_eq!(format_bytes(1_200_000_000), "1.2 GB");
        assert_eq!(format_bytes(999_960), "1.0 MB");
        assert_eq!(format_speed(1.84), "1.8× real time");
        assert_eq!(format_speed(12.3), "12× real time");
    }
}
