//! The master safety limiter (AU-010): −1 dBFS sample-peak ceiling with
//! about 5 ms of look-ahead.
//!
//! The signal is delayed by `L` samples. For every incoming frame the gain
//! that would keep it under the ceiling is `need = min(1, ceiling / peak)`
//! (stereo-linked). The gain applied to the frame leaving the delay line is
//! the average, over the last `L + 1` frames, of the minimum `need` over the
//! `L + 1` frames before each of them. Every term of that average has seen
//! the outgoing frame's own `need`, so the average never exceeds it: a peak is
//! caught without clipping, and the gain ramps down linearly over `L` frames
//! instead of stepping. Release is a smooth exponential recovery. A final
//! clamp catches float rounding, so the output never exceeds the ceiling.
//!
//! Below the ceiling `need` is 1 everywhere and the limiter is a pure delay.

use std::collections::VecDeque;

use crate::gain::db_to_amplitude;

/// Ceiling in dBFS (sample peak).
const CEILING_DB: f32 = -1.0;
/// Look-ahead in seconds.
const LOOKAHEAD_SECS: f64 = 0.005;
/// Time constant of the gain recovery in seconds.
const RELEASE_SECS: f64 = 0.08;
/// Gain difference below which the release jumps to its target.
const RELEASE_SNAP: f64 = 1e-4;

#[derive(Debug)]
pub(crate) struct Limiter {
    ceiling: f32,
    lookahead: usize,
    release: f64,
    enabled: bool,
    /// Delayed stereo frames, oldest first; always `lookahead` long.
    delay: VecDeque<[f32; 2]>,
    /// Monotonic deque of `(frame number, need)` for the sliding minimum.
    minima: VecDeque<(u64, f32)>,
    /// The last `lookahead + 1` sliding minima, for the running average.
    window: VecDeque<f32>,
    window_sum: f64,
    /// Kept in `f64` so the slow release doesn't stall on `f32` rounding.
    gain: f64,
    frame: u64,
}

impl Limiter {
    pub(crate) fn new(rate_hz: u32) -> Limiter {
        let lookahead = ((f64::from(rate_hz) * LOOKAHEAD_SECS).round() as usize).max(1);
        let release = 1.0 - (-1.0 / (RELEASE_SECS * f64::from(rate_hz))).exp();
        let mut l = Limiter {
            ceiling: db_to_amplitude(CEILING_DB),
            lookahead,
            release,
            enabled: true,
            delay: VecDeque::with_capacity(lookahead + 1),
            minima: VecDeque::with_capacity(lookahead + 2),
            window: VecDeque::with_capacity(lookahead + 2),
            window_sum: 0.0,
            gain: 1.0,
            frame: 0,
        };
        l.reset();
        l
    }

    /// Latency in frames: output frame `n` is input frame `n − latency`.
    pub(crate) fn latency(&self) -> usize {
        self.lookahead
    }

    #[cfg(test)]
    pub(crate) fn ceiling(&self) -> f32 {
        self.ceiling
    }

    /// Off: frames pass through the delay unchanged (still `latency` late).
    pub(crate) fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    /// Forgets all history: silence in the delay line, unity gain.
    pub(crate) fn reset(&mut self) {
        self.delay.clear();
        self.delay
            .extend(std::iter::repeat_n([0.0; 2], self.lookahead));
        self.minima.clear();
        self.window.clear();
        self.window
            .extend(std::iter::repeat_n(1.0, self.lookahead + 1));
        self.window_sum = (self.lookahead + 1) as f64;
        self.gain = 1.0;
        self.frame = 0;
    }

    /// Pushes one frame in and returns the frame `latency` frames older,
    /// limited.
    pub(crate) fn process(&mut self, frame: [f32; 2]) -> [f32; 2] {
        let peak = frame[0].abs().max(frame[1].abs());
        let need = if peak > self.ceiling && peak.is_finite() {
            self.ceiling / peak
        } else if peak.is_finite() {
            1.0
        } else {
            0.0
        };

        // Sliding minimum of `need` over the last `lookahead + 1` frames.
        let n = self.frame;
        self.frame += 1;
        while self.minima.back().is_some_and(|&(_, v)| v >= need) {
            self.minima.pop_back();
        }
        self.minima.push_back((n, need));
        let oldest = n.saturating_sub(self.lookahead as u64);
        while self.minima.front().is_some_and(|&(i, _)| i < oldest) {
            self.minima.pop_front();
        }
        let min = self.minima.front().map_or(1.0, |&(_, v)| v);

        // Running average of the minima over the same span.
        self.window.push_back(min);
        self.window_sum += f64::from(min);
        if let Some(old) = self.window.pop_front() {
            self.window_sum -= f64::from(old);
        }
        let smooth = (self.window_sum / (self.lookahead + 1) as f64).min(1.0);

        // Attack follows the target down at once; release recovers smoothly
        // and snaps the last 0.001 dB so the limiter becomes exactly
        // transparent again.
        self.gain = if smooth < self.gain || smooth - self.gain < RELEASE_SNAP {
            smooth
        } else {
            self.gain + (smooth - self.gain) * self.release
        };

        self.delay.push_back(frame);
        let out = self.delay.pop_front().unwrap_or([0.0; 2]);
        if !self.enabled {
            return out;
        }
        if self.gain >= 1.0 {
            // Transparent: no rounding at all below the ceiling.
            return out;
        }
        let (c, g) = (self.ceiling, self.gain as f32);
        [(out[0] * g).clamp(-c, c), (out[1] * g).clamp(-c, c)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(l: &mut Limiter, input: &[[f32; 2]]) -> Vec<[f32; 2]> {
        input.iter().map(|&f| l.process(f)).collect()
    }

    fn sine(n: usize, amp: f32, hz: f32, rate: f32) -> Vec<[f32; 2]> {
        (0..n)
            .map(|i| {
                let v = amp * (std::f32::consts::TAU * hz * i as f32 / rate).sin();
                [v, -v]
            })
            .collect()
    }

    #[test]
    fn latency_is_about_five_ms() {
        assert_eq!(Limiter::new(48_000).latency(), 240);
        assert_eq!(Limiter::new(44_100).latency(), 221);
    }

    #[test]
    fn transparent_below_the_ceiling() {
        let mut l = Limiter::new(48_000);
        let input = sine(4800, 0.8, 440.0, 48_000.0);
        let out = run(&mut l, &input);
        let lat = l.latency();
        assert!(out[..lat].iter().all(|f| *f == [0.0, 0.0]));
        assert_eq!(&out[lat..], &input[..input.len() - lat]);
    }

    #[test]
    fn never_exceeds_the_ceiling() {
        let mut l = Limiter::new(48_000);
        let c = l.ceiling();
        assert!((c - 0.891_250_9).abs() < 1e-6);
        let mut input = sine(9600, 3.0, 997.0, 48_000.0);
        // A single-sample spike and a full-scale square burst.
        input[5000] = [40.0, -40.0];
        input.extend(std::iter::repeat_n([1.0, 1.0], 2000));
        input.extend(std::iter::repeat_n([0.0, 0.0], 500));
        let out = run(&mut l, &input);
        let max = out
            .iter()
            .flat_map(|f| f.iter())
            .fold(0f32, |m, v| m.max(v.abs()));
        assert!(max <= c, "peak {max} over ceiling {c}");
        // The loud parts are actually loud, not crushed to silence.
        assert!(max > 0.85 * c);
    }

    #[test]
    fn gain_ramps_instead_of_stepping() {
        let mut l = Limiter::new(48_000);
        let lat = l.latency();
        // Quiet constant signal, then a loud constant one.
        let mut input = vec![[0.5, 0.5]; 2000];
        input.extend(std::iter::repeat_n([2.0, 2.0], 2000));
        let out = run(&mut l, &input);
        // The quiet part starts dipping `lat` frames before the jump arrives
        // and never by more than one ramp step per frame.
        let step = 0.5 / lat as f32 * 1.01;
        for w in out[lat..2000 + lat].windows(2) {
            assert!((w[0][0] - w[1][0]).abs() <= step, "{w:?}");
        }
        // Once settled on the loud part it sits right at the ceiling.
        assert!((out[3500][0] - l.ceiling()).abs() < 1e-5);
    }

    #[test]
    fn recovers_after_a_peak() {
        let mut l = Limiter::new(48_000);
        let mut input = vec![[4.0, 4.0]; 100];
        input.extend(std::iter::repeat_n([0.5, 0.5], 48_000));
        let out = run(&mut l, &input);
        // A second later the gain is back to unity.
        assert_eq!(out.last().copied(), Some([0.5, 0.5]));
    }

    #[test]
    fn disabled_is_a_pure_delay() {
        let mut l = Limiter::new(48_000);
        l.set_enabled(false);
        let input = sine(1000, 3.0, 440.0, 48_000.0);
        let out = run(&mut l, &input);
        assert_eq!(&out[l.latency()..], &input[..1000 - l.latency()]);
    }
}
