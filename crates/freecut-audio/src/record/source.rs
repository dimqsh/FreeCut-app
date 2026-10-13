//! Where captured audio comes from ([`CaptureSource`]) and where a source
//! puts it ([`CaptureSink`]).
//!
//! The sink runs inside the source's callback. It converts to `f32`, maps
//! the device's channels to the take's, applies the input gain through a
//! 10 ms smoother, updates the meter, feeds monitoring, and pushes the
//! take's frames into the writer's ring. It never blocks, allocates or
//! locks: a full ring ends the take ([`EndReason::Overrun`]) instead of
//! waiting.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use rtrb::Producer;

use super::RecordError;
use super::meter::MeterState;
use super::monitor::MonitorTap;
use super::shared::{
    COUNTING, END_DEVICE_LOST, END_OVERRUN, END_RANGE, END_RATE_CHANGED, PREROLL, RECORDING, Shared,
};

#[cfg(doc)]
use super::EndReason;

/// A capture device's format: what the sink receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFormat {
    pub rate: u32,
    /// Interleaved channels per frame.
    pub channels: u16,
}

/// Something that captures audio: a microphone ([`CpalInput`](super::CpalInput),
/// feature `record`) or a test signal ([`SyntheticInput`](super::SyntheticInput)).
pub trait CaptureSource: Send {
    /// The format the source delivers, fixed once it is open.
    fn format(&self) -> SourceFormat;

    /// Starts delivering frames to `sink` (from the source's own thread or
    /// callback) until [`CaptureSource::stop`].
    fn start(&mut self, sink: CaptureSink) -> Result<(), RecordError>;

    /// Stops delivering; once it returns, the sink is no longer called.
    fn stop(&mut self);

    /// The time from a sound reaching the input to the sink receiving it,
    /// as the device reports it.
    fn latency(&self) -> Duration;
}

/// Why a source stopped working.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFailure {
    /// The device disappeared (unplugged, or the system took it away).
    DeviceLost,
    /// The device's sample rate or format changed under the stream.
    RateChanged,
}

/// A sample type a source can deliver.
pub trait InputSample: Copy + Send + 'static {
    fn to_f32(self) -> f32;
}

impl InputSample for f32 {
    #[inline]
    fn to_f32(self) -> f32 {
        self
    }
}

impl InputSample for f64 {
    #[inline]
    fn to_f32(self) -> f32 {
        self as f32
    }
}

impl InputSample for i8 {
    #[inline]
    fn to_f32(self) -> f32 {
        f32::from(self) / 128.0
    }
}

impl InputSample for i16 {
    #[inline]
    fn to_f32(self) -> f32 {
        f32::from(self) / 32_768.0
    }
}

impl InputSample for i32 {
    #[inline]
    fn to_f32(self) -> f32 {
        (f64::from(self) / 2_147_483_648.0) as f32
    }
}

impl InputSample for u8 {
    #[inline]
    fn to_f32(self) -> f32 {
        (f32::from(self) - 128.0) / 128.0
    }
}

impl InputSample for u16 {
    #[inline]
    fn to_f32(self) -> f32 {
        (f32::from(self) - 32_768.0) / 32_768.0
    }
}

/// Reports a source's failure from any thread (a device's error callback).
#[derive(Debug, Clone)]
pub struct FailHandle(Arc<Shared>);

impl FailHandle {
    /// Ends a running take, keeping what was captured; the recorder then
    /// waits for another device.
    pub fn fail(&self, why: SourceFailure) {
        self.0.fail(match why {
            SourceFailure::DeviceLost => END_DEVICE_LOST,
            SourceFailure::RateChanged => END_RATE_CHANGED,
        });
    }
}

/// The receiving end a [`CaptureSource`] pushes into.
pub struct CaptureSink {
    shared: Arc<Shared>,
    ring: Producer<f32>,
    monitor: MonitorTap,
    in_channels: usize,
    rate: u32,
    gain: f32,
    gain_coeff: f32,
    meter: MeterState,
    // The current take, as the callback sees it.
    take: u32,
    channels: usize,
    countdown_left: u64,
    preroll: u64,
    stop_after: u64,
    pushed: u64,
}

impl std::fmt::Debug for CaptureSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureSink")
            .field("in_channels", &self.in_channels)
            .field("rate", &self.rate)
            .field("take", &self.take)
            .field("pushed", &self.pushed)
            .finish_non_exhaustive()
    }
}

/// The input gain smoother's time constant.
const GAIN_SMOOTH_SECS: f32 = 0.01;

impl CaptureSink {
    pub(crate) fn new(
        shared: Arc<Shared>,
        ring: Producer<f32>,
        monitor: MonitorTap,
        format: SourceFormat,
    ) -> CaptureSink {
        let rate = format.rate.max(1);
        let gain = f32::from_bits(shared.gain.load(Ordering::Relaxed));
        let take = shared.take.load(Ordering::SeqCst);
        CaptureSink {
            ring,
            monitor,
            in_channels: usize::from(format.channels.max(1)),
            rate,
            gain,
            gain_coeff: 1.0 - (-1.0 / (GAIN_SMOOTH_SECS * rate as f32)).exp(),
            meter: MeterState::new(rate),
            take,
            channels: 1,
            countdown_left: 0,
            preroll: 0,
            stop_after: u64::MAX,
            pushed: 0,
            shared,
        }
    }

    /// A handle for reporting that the source failed.
    pub fn fail_handle(&self) -> FailHandle {
        FailHandle(Arc::clone(&self.shared))
    }

    /// Whether frames pushed now count for something more than the meter
    /// (a countdown or a take is running). Test sources use it to pace.
    pub fn is_capturing(&self) -> bool {
        matches!(self.shared.phase(), COUNTING | PREROLL | RECORDING)
    }

    /// Free room in the writer's ring, in frames of the take.
    pub fn room_frames(&self) -> usize {
        self.ring.slots() / self.channels.max(1)
    }

    /// Takes one block of interleaved samples in the source's format.
    /// `captured_at` is when its first frame reached the input.
    ///
    /// Real-time safe: no allocation, no locks, no blocking.
    pub fn push<S: InputSample>(&mut self, data: &[S], captured_at: Instant) {
        let shared = Arc::clone(&self.shared);
        shared.busy.fetch_add(1, Ordering::SeqCst);
        let mut phase = shared.phase();
        let running = matches!(phase, COUNTING | PREROLL | RECORDING);
        if running {
            let take = shared.take.load(Ordering::SeqCst);
            if take != self.take {
                self.begin_take(&shared, take);
            }
            let latency = Instant::now().saturating_duration_since(captured_at);
            shared.latency.store(
                u64::try_from(latency.as_nanos()).unwrap_or(u64::MAX),
                Ordering::Relaxed,
            );
        } else {
            // The meter follows the channel choice before a take starts.
            self.channels = usize::from(shared.channels.load(Ordering::Relaxed).clamp(1, 2));
        }
        let target = f32::from_bits(shared.gain.load(Ordering::Relaxed));
        let monitor = shared.monitor.load(Ordering::Relaxed);
        let n = self.channels;
        let mut frame = [0f32; 2];
        for (i, input) in data.chunks_exact(self.in_channels).enumerate() {
            match input {
                [only] => frame = [only.to_f32(); 2],
                [l, r, ..] => frame = [l.to_f32(), r.to_f32()],
                [] => {}
            }
            if n == 1 && input.len() > 1 {
                frame[0] = 0.5 * (frame[0] + frame[1]);
            }
            self.gain += (target - self.gain) * self.gain_coeff;
            if (target - self.gain).abs() < 1e-7 {
                self.gain = target;
            }
            let out = &mut frame[..n];
            for s in out.iter_mut() {
                *s *= self.gain;
            }
            self.meter.frame(out);
            if monitor {
                self.monitor.push(out);
            }
            if phase == COUNTING {
                if self.countdown_left > 0 {
                    self.countdown_left -= 1;
                    continue;
                }
                let next = if self.preroll > 0 { PREROLL } else { RECORDING };
                phase = match shared.cas_phase(COUNTING, next) {
                    Ok(()) => next,
                    Err(now) => now,
                };
            }
            if phase != PREROLL && phase != RECORDING {
                continue;
            }
            if self.pushed >= self.stop_after {
                phase = shared.end_take(END_RANGE);
                continue;
            }
            if self.ring.slots() < n {
                phase = shared.end_take(END_OVERRUN);
                continue;
            }
            for &s in out.iter() {
                let _ = self.ring.push(s);
            }
            if self.pushed == 0 {
                let offset = i as u64 * 1_000_000_000 / u64::from(self.rate);
                shared.stamp_first_frame(captured_at + Duration::from_nanos(offset));
            }
            self.pushed += 1;
            if phase == PREROLL && self.pushed >= self.preroll {
                phase = match shared.cas_phase(PREROLL, RECORDING) {
                    Ok(()) => RECORDING,
                    Err(now) => now,
                };
            }
            if self.pushed >= self.stop_after {
                phase = shared.end_take(END_RANGE);
            }
        }
        self.meter.publish(&shared.meter);
        if running {
            shared.captured.store(self.pushed, Ordering::Relaxed);
            shared
                .countdown_left
                .store(self.countdown_left, Ordering::Relaxed);
        }
        shared.busy.fetch_add(1, Ordering::SeqCst);
    }

    fn begin_take(&mut self, shared: &Shared, take: u32) {
        self.take = take;
        self.channels = usize::from(shared.channels.load(Ordering::SeqCst).clamp(1, 2));
        self.countdown_left = shared.countdown_frames.load(Ordering::SeqCst);
        self.preroll = shared.preroll_frames.load(Ordering::SeqCst);
        self.stop_after = shared.stop_after.load(Ordering::SeqCst);
        self.pushed = 0;
    }
}
