//! A capture source that needs no microphone: test signals, delivered by
//! its own paced thread or block by block from the test
//! ([`SyntheticDriver`]), optionally failing after a number of frames.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::RecordError;
use super::source::{CaptureSink, CaptureSource, FailHandle, SourceFailure, SourceFormat};

/// What a [`SyntheticInput`] plays. Frame `n` counts from the source's
/// start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Signal {
    Silence,
    /// The same constant on every channel.
    Dc(f32),
    /// A sine of `amp` at `hz`, in phase on every channel.
    Sine {
        hz: f32,
        amp: f32,
    },
    /// `amp` on frame 0 and every `every` frames after, silence between.
    Impulse {
        every: u64,
        amp: f32,
    },
    /// Exact 24-bit steps for sample-exact checks: channel 0 of frame `n`
    /// is `((n mod 65 536) − 32 768) / 2²³`, odd channels its negation.
    Ramp,
}

impl Signal {
    /// The value of channel `channel` of frame `n` at `rate`.
    pub fn value(self, n: u64, channel: usize, rate: u32) -> f32 {
        match self {
            Signal::Silence => 0.0,
            Signal::Dc(v) => v,
            Signal::Sine { hz, amp } => {
                // The phase from the frame number alone, so long runs don't
                // drift.
                let t = n as f64 / f64::from(rate.max(1));
                amp * (std::f64::consts::TAU * f64::from(hz) * t).sin() as f32
            }
            Signal::Impulse { every, amp } => {
                if n.checked_rem(every.max(1)) == Some(0) {
                    amp
                } else {
                    0.0
                }
            }
            Signal::Ramp => {
                let v = ((n % 65_536) as f32 - 32_768.0) / 8_388_608.0;
                if channel % 2 == 1 { -v } else { v }
            }
        }
    }
}

/// Produces the signal block by block.
#[derive(Debug)]
struct Generator {
    signal: Signal,
    rate: u32,
    channels: usize,
    /// Frames produced so far.
    n: u64,
    /// The source fails once this many frames were produced.
    fail_after: Option<u64>,
    buf: Vec<f32>,
}

impl Generator {
    /// Fills the buffer with up to `frames` frames; fewer when the failure
    /// point comes first.
    fn fill(&mut self, frames: u64) -> usize {
        let frames = match self.fail_after {
            Some(limit) => frames.min(limit.saturating_sub(self.n)),
            None => frames,
        };
        self.buf.clear();
        for i in 0..frames {
            for c in 0..self.channels {
                self.buf.push(self.signal.value(self.n + i, c, self.rate));
            }
        }
        self.n += frames;
        usize::try_from(frames).unwrap_or(usize::MAX)
    }

    fn failed(&self) -> bool {
        self.fail_after.is_some_and(|limit| self.n >= limit)
    }
}

/// A started source's state.
#[derive(Debug)]
struct Running {
    sink: CaptureSink,
    fail: FailHandle,
    generator: Generator,
    failed: bool,
}

impl Running {
    /// Produces and delivers `frames` frames in blocks of `block`; returns
    /// how many were delivered (fewer once the source fails).
    fn feed(&mut self, frames: u64, block: u64, latency: Duration) -> u64 {
        let mut done = 0;
        while done < frames && !self.failed {
            let got = self.generator.fill((frames - done).min(block));
            if got > 0 {
                let now = Instant::now();
                let at = now.checked_sub(latency).unwrap_or(now);
                self.sink.push(&self.generator.buf, at);
            }
            done += got as u64;
            if self.generator.failed() {
                self.failed = true;
                self.fail.fail(SourceFailure::DeviceLost);
            }
        }
        done
    }
}

type Slot = Arc<Mutex<Option<Running>>>;

fn lock(slot: &Slot) -> MutexGuard<'_, Option<Running>> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How a [`SyntheticInput`] delivers frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pace {
    /// Its own thread, one block every block's duration, like a device.
    RealTime,
    /// Only when the test calls [`SyntheticDriver::feed`].
    Driven,
}

/// A test capture source (no microphone): `signal` at `rate` with
/// `channels`, which reports [`SourceFailure::DeviceLost`] after
/// `fail_after` frames.
#[derive(Debug)]
pub struct SyntheticInput {
    signal: Signal,
    rate: u32,
    channels: u16,
    fail_after: Option<u64>,
    latency: Duration,
    block_frames: u64,
    pace: Pace,
    slot: Slot,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// Delivers a driven [`SyntheticInput`]'s frames from the calling thread.
#[derive(Debug, Clone)]
pub struct SyntheticDriver {
    slot: Slot,
    block_frames: u64,
    latency: Duration,
}

impl SyntheticDriver {
    /// Delivers `frames` frames in device-sized blocks, as a capture
    /// callback would. Returns how many were delivered: fewer once the
    /// source has failed, none before it is started or after it stopped.
    pub fn feed(&self, frames: u64) -> u64 {
        match lock(&self.slot).as_mut() {
            Some(running) => running.feed(frames, self.block_frames, self.latency),
            None => 0,
        }
    }

    /// Whether the source is started.
    pub fn is_running(&self) -> bool {
        lock(&self.slot).is_some()
    }
}

impl SyntheticInput {
    /// A source playing `signal` in real time on its own thread, in 10 ms
    /// blocks.
    pub fn new(signal: Signal, rate: u32, channels: u16) -> SyntheticInput {
        SyntheticInput {
            signal,
            rate,
            channels,
            fail_after: None,
            latency: Duration::ZERO,
            block_frames: u64::from(rate / 100).max(1),
            pace: Pace::RealTime,
            slot: Arc::default(),
            stop: Arc::default(),
            thread: None,
        }
    }

    /// The source reports a lost device after `frames` frames.
    pub fn fail_after(mut self, frames: u64) -> SyntheticInput {
        self.fail_after = Some(frames);
        self
    }

    /// The input latency the source reports (its frames are stamped this
    /// much before they are delivered).
    pub fn with_latency(mut self, latency: Duration) -> SyntheticInput {
        self.latency = latency;
        self
    }

    /// Delivered blocks hold at most `frames` frames.
    pub fn with_block_frames(mut self, frames: u64) -> SyntheticInput {
        self.block_frames = frames.max(1);
        self
    }

    /// Frames are delivered only through the returned driver.
    pub fn driven(mut self) -> (SyntheticInput, SyntheticDriver) {
        self.pace = Pace::Driven;
        let driver = SyntheticDriver {
            slot: Arc::clone(&self.slot),
            block_frames: self.block_frames,
            latency: self.latency,
        };
        (self, driver)
    }
}

impl CaptureSource for SyntheticInput {
    fn format(&self) -> SourceFormat {
        SourceFormat {
            rate: self.rate,
            channels: self.channels,
        }
    }

    fn start(&mut self, sink: CaptureSink) -> Result<(), RecordError> {
        self.stop();
        let running = Running {
            fail: sink.fail_handle(),
            sink,
            generator: Generator {
                signal: self.signal,
                rate: self.rate.max(1),
                channels: usize::from(self.channels.max(1)),
                n: 0,
                fail_after: self.fail_after,
                buf: Vec::new(),
            },
            failed: false,
        };
        *lock(&self.slot) = Some(running);
        if self.pace == Pace::Driven {
            return Ok(());
        }
        self.stop.store(false, Ordering::SeqCst);
        let slot = Arc::clone(&self.slot);
        let stop = Arc::clone(&self.stop);
        let block = self.block_frames;
        let latency = self.latency;
        let period = Duration::from_secs_f64(block as f64 / f64::from(self.rate.max(1)));
        let handle = std::thread::Builder::new()
            .name("freecut-synthetic-input".to_owned())
            .spawn(move || {
                let mut next = Instant::now();
                while !stop.load(Ordering::SeqCst) {
                    next += period;
                    if let Some(wait) = next.checked_duration_since(Instant::now()) {
                        std::thread::sleep(wait);
                    }
                    let mut guard = lock(&slot);
                    let Some(running) = guard.as_mut() else {
                        return;
                    };
                    running.feed(block, block, latency);
                    if running.failed {
                        return;
                    }
                }
            })
            .map_err(|e| RecordError::Thread(Arc::new(e)))?;
        self.thread = Some(handle);
        Ok(())
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        *lock(&self.slot) = None;
    }

    fn latency(&self) -> Duration {
        self.latency
    }
}

impl Drop for SyntheticInput {
    fn drop(&mut self) {
        self.stop();
    }
}
