//! State shared by the caller, the capture callback and the writer thread:
//! atomics only.
//!
//! **Phase.** One `AtomicU8` holds the recorder's state. Every party moves
//! it with compare-and-swap, so a transition happens once:
//! - the caller: Ready → Counting down / Pre-roll / Recording (arm, start),
//!   Counting down → Ready (cancel), Pre-roll / Recording → Stopping (stop,
//!   cancel);
//! - the capture callback: Counting down → Pre-roll / Recording (the
//!   countdown ran out), Pre-roll → Recording, Pre-roll / Recording →
//!   Stopping (range end, overrun);
//! - the source's error path: anything → Waiting or Stopping (device lost);
//! - the writer: Pre-roll / Recording → Stopping (disk low, write error),
//!   Stopping → Ready / Waiting (the take is finished).
//!
//! **Why a take ends.** `end` is set once per take (first wins) by the
//! callback, the device or the writer; `request` is what the caller asked
//! for (stop or cancel). The writer reads `end` first.
//!
//! **Quiescence.** The callback increments `busy` on entry and on exit, so
//! it is odd while a block is being pushed. After seeing Stopping, the
//! writer waits for `busy` to leave the odd value it read (or to be even):
//! every later block sees Stopping and pushes nothing, so the ring then
//! holds the whole take.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::meter::MeterCells;
use super::{ClockStamp, RecorderState};

pub(crate) const WAITING: u8 = 0;
pub(crate) const READY: u8 = 1;
pub(crate) const COUNTING: u8 = 2;
pub(crate) const PREROLL: u8 = 3;
pub(crate) const RECORDING: u8 = 4;
pub(crate) const STOPPING: u8 = 5;

/// `end` codes.
pub(crate) const END_NONE: u8 = 0;
pub(crate) const END_RANGE: u8 = 1;
pub(crate) const END_OVERRUN: u8 = 2;
pub(crate) const END_DEVICE_LOST: u8 = 3;
pub(crate) const END_RATE_CHANGED: u8 = 4;
pub(crate) const END_DISK_LOW: u8 = 5;
pub(crate) const END_WRITE_FAILED: u8 = 6;

/// `request` codes.
pub(crate) const REQ_NONE: u8 = 0;
pub(crate) const REQ_STOP: u8 = 1;
pub(crate) const REQ_CANCEL: u8 = 2;

/// `stop_after` when the take has no end.
pub(crate) const NO_END: u64 = u64::MAX;

/// What [`Shared::request_end`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndRequest {
    /// A take is ending; its result will come from the writer.
    Stopping,
    /// The countdown was cancelled; nothing was written.
    CountdownCancelled,
    /// Nothing was running.
    Idle,
}

#[derive(Debug)]
pub(crate) struct Shared {
    phase: AtomicU8,
    pub(crate) end: AtomicU8,
    pub(crate) request: AtomicU8,
    /// The device is gone: after the take, the recorder waits.
    pub(crate) device_failed: AtomicBool,
    /// Odd while the callback is inside a block.
    pub(crate) busy: AtomicU64,
    /// The current take's number (bumped by arm and start).
    pub(crate) take: AtomicU32,
    /// The last take the writer finished (its result is sent first).
    pub(crate) finished: AtomicU32,
    // The take's parameters, written before `take` is bumped.
    pub(crate) countdown_frames: AtomicU64,
    pub(crate) preroll_frames: AtomicU64,
    pub(crate) stop_after: AtomicU64,
    pub(crate) channels: AtomicU8,
    // Live controls.
    /// Linear input gain, `f32` bits.
    pub(crate) gain: AtomicU32,
    pub(crate) monitor: AtomicBool,
    // Published by the callback once per block.
    pub(crate) captured: AtomicU64,
    pub(crate) countdown_left: AtomicU64,
    /// Nanoseconds from `epoch` to the take's first frame; `i64::MIN`
    /// until it is captured.
    pub(crate) first_frame: AtomicI64,
    /// The last measured input latency, in nanoseconds.
    pub(crate) latency: AtomicU64,
    pub(crate) meter: MeterCells,
    epoch: Instant,
}

impl Shared {
    pub(crate) fn new(channels: u8, gain: f32, monitor: bool) -> Shared {
        Shared {
            phase: AtomicU8::new(READY),
            end: AtomicU8::new(END_NONE),
            request: AtomicU8::new(REQ_NONE),
            device_failed: AtomicBool::new(false),
            busy: AtomicU64::new(0),
            take: AtomicU32::new(0),
            finished: AtomicU32::new(0),
            countdown_frames: AtomicU64::new(0),
            preroll_frames: AtomicU64::new(0),
            stop_after: AtomicU64::new(NO_END),
            channels: AtomicU8::new(channels),
            gain: AtomicU32::new(gain.to_bits()),
            monitor: AtomicBool::new(monitor),
            captured: AtomicU64::new(0),
            countdown_left: AtomicU64::new(0),
            first_frame: AtomicI64::new(i64::MIN),
            latency: AtomicU64::new(0),
            meter: MeterCells::default(),
            epoch: Instant::now(),
        }
    }

    pub(crate) fn phase(&self) -> u8 {
        self.phase.load(Ordering::SeqCst)
    }

    pub(crate) fn set_phase(&self, phase: u8) {
        self.phase.store(phase, Ordering::SeqCst);
    }

    /// Moves `from` → `to`; the phase found when it wasn't `from`.
    pub(crate) fn cas_phase(&self, from: u8, to: u8) -> Result<(), u8> {
        self.phase
            .compare_exchange(from, to, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
    }

    pub(crate) fn state(&self) -> RecorderState {
        match self.phase() {
            WAITING => RecorderState::Waiting,
            READY => RecorderState::Ready,
            COUNTING => RecorderState::CountingDown,
            PREROLL => RecorderState::PreRoll,
            RECORDING => RecorderState::Recording,
            _ => RecorderState::Stopping,
        }
    }

    /// Records why the take ends, unless something already did.
    pub(crate) fn set_end(&self, code: u8) {
        let _ = self
            .end
            .compare_exchange(END_NONE, code, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Ends a running take for `code` (callback, device or writer): sets the
    /// reason and moves Pre-roll / Recording to Stopping. Returns the phase
    /// after.
    pub(crate) fn end_take(&self, code: u8) -> u8 {
        self.set_end(code);
        loop {
            let phase = self.phase();
            if phase != PREROLL && phase != RECORDING {
                return phase;
            }
            if self.cas_phase(phase, STOPPING).is_ok() {
                return STOPPING;
            }
        }
    }

    /// The source stopped working: a running take ends (keeping what was
    /// captured); otherwise the recorder waits for another device.
    pub(crate) fn fail(&self, code: u8) {
        self.device_failed.store(true, Ordering::SeqCst);
        self.set_end(code);
        loop {
            let phase = self.phase();
            let to = match phase {
                READY | COUNTING => WAITING,
                PREROLL | RECORDING => STOPPING,
                _ => return,
            };
            if self.cas_phase(phase, to).is_ok() {
                return;
            }
        }
    }

    /// The caller's stop (`REQ_STOP`) or cancel (`REQ_CANCEL`).
    pub(crate) fn request_end(&self, request: u8) -> EndRequest {
        loop {
            match self.phase() {
                phase @ (PREROLL | RECORDING) => {
                    self.request.store(request, Ordering::SeqCst);
                    if self.cas_phase(phase, STOPPING).is_ok() {
                        return EndRequest::Stopping;
                    }
                }
                STOPPING => return EndRequest::Stopping,
                COUNTING => {
                    if self.cas_phase(COUNTING, READY).is_ok() {
                        return EndRequest::CountdownCancelled;
                    }
                }
                _ => return EndRequest::Idle,
            }
        }
    }

    pub(crate) fn stamp_first_frame(&self, at: Instant) {
        let nanos = if at >= self.epoch {
            i64::try_from(at.duration_since(self.epoch).as_nanos()).unwrap_or(i64::MAX)
        } else {
            -i64::try_from(self.epoch.duration_since(at).as_nanos()).unwrap_or(i64::MAX)
        };
        self.first_frame.store(nanos, Ordering::Relaxed);
    }

    /// The take's first frame, or `fallback` when none was captured.
    pub(crate) fn first_frame(&self, fallback: Instant) -> ClockStamp {
        let nanos = self.first_frame.load(Ordering::Relaxed);
        if nanos == i64::MIN {
            return ClockStamp(fallback);
        }
        let d = Duration::from_nanos(nanos.unsigned_abs());
        let at = if nanos >= 0 {
            self.epoch.checked_add(d)
        } else {
            self.epoch.checked_sub(d)
        };
        ClockStamp(at.unwrap_or(fallback))
    }

    pub(crate) fn latency(&self) -> Duration {
        Duration::from_nanos(self.latency.load(Ordering::Relaxed))
    }

    /// Clears the per-take values before a take starts or after it ends.
    pub(crate) fn reset_take(&self) {
        self.end.store(END_NONE, Ordering::SeqCst);
        self.request.store(REQ_NONE, Ordering::SeqCst);
        self.captured.store(0, Ordering::SeqCst);
        self.first_frame.store(i64::MIN, Ordering::SeqCst);
    }

    /// Waits until the callback is not inside a block it started before
    /// now (bounded, in case a device stopped mid-callback).
    pub(crate) fn quiesce(&self, limit: Duration) {
        let seen = self.busy.load(Ordering::SeqCst);
        if seen.is_multiple_of(2) {
            return;
        }
        let start = Instant::now();
        while self.busy.load(Ordering::SeqCst) == seen && start.elapsed() < limit {
            std::thread::sleep(Duration::from_micros(200));
        }
    }
}
