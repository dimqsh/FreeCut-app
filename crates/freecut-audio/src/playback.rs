//! Real-time playback, independent of the audio API (AU-010).
//!
//! Three parties share the work:
//! - the **controller** (the app's calls on the engine) owns the transport
//!   state: playing or not, the current *generation* and the timeline sample
//!   it started at;
//! - the **feeder** thread renders [`Mixer`] blocks about 200 ms ahead into
//!   a lock-free SPSC ring of frames tagged with their generation, and
//!   reopens the device when it is lost. It never opens a source: a
//!   prefetch worker ([`crate::prefetch`]) opens and positions the readers
//!   of the next ~2 s of the schedule ahead of time, and an input whose
//!   reader isn't ready yet plays silence until it is;
//! - the device callback ([`OutputTap::fill`]) only pops frames. It never
//!   blocks, allocates or decodes, and takes no lock it would wait for (the
//!   recorder's monitoring feed is only `try_lock`ed, see below). Frames of an older generation are
//!   discarded, so play and seek take effect at once without the controller
//!   touching the ring. A dry ring plays silence and counts an underrun,
//!   except while a play or seek waits for its first frames.
//!
//! A document change during playback ([`Engine::set_source`]) starts no new
//! generation: what is already queued plays out, and the feeder renders the
//! new schedule from where it is, so the change is heard after the
//! read-ahead (about 200 ms) without a gap, and readers keep reading
//! sequentially. The mixer ramps the changes themselves.
//!
//! The callback publishes how many frames of the current generation it has
//! handed to the device, when, and the device's output latency, through a
//! lock-free seqlock that never skips an update. That is the audio clock:
//! the heard position is interpolated from it, compensated for latency and
//! kept monotonic. When the device is gone, a wall clock takes over so video
//! keeps playing.
//!
//! The device is the one chosen in the [`EngineConfig`], or the system
//! default. A chosen device that is missing, busy or denied gives way to the
//! system default at once; one that fails otherwise is tried once more after
//! 1 s (playing on the wall clock meanwhile) before it does (AU-010).
//! [`EngineStatus`] says which device plays and why. A lost device is
//! reopened the same way, and once more after 1 s if nothing opens.
//!
//! Monitoring a recording (AU-027, `Engine::set_monitor`): the recorder's
//! [`MonitorFeed`] sits in a slot of the shared state; the callback adds it
//! after the master stage and preview mute, at the input's rate only, so it
//! is never in an export. The controller holds the slot's lock only to swap
//! the feed (and drops the old one itself); a callback that finds it held
//! plays that one buffer without monitoring.
//!
//! The device itself sits behind [`Backend`], so all of this is tested
//! without sound hardware.
// Without `output` only the unit tests drive this module.
#![cfg_attr(not(feature = "output"), allow(dead_code))]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering, fence};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use freecut_timeline::time::{SampleRate, Tick, TimeRange};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::mixer::{Mixer, ReaderState};
use crate::prefetch::{Prefetcher, Request};
use crate::record::{MONITOR_BACKLOG_FRAMES, MonitorFeed};
use crate::samples::{sample_at, sample_start, sample_ticks};
use crate::source::{Schedule, Silence, SourceKey, SourceOpener};

/// Ring capacity in frames: more than the read-ahead at any device rate up
/// to 192 kHz.
const RING_FRAMES: usize = 1 << 16;
/// How far ahead of the device the feeder renders.
const READ_AHEAD_SECS: f64 = 0.2;
/// Feeder block length; small so playback starts quickly.
const BLOCK_SECS: f64 = 0.01;
/// How long the idle feeder waits for a command before checking the ring.
const IDLE_WAIT: Duration = Duration::from_millis(4);
/// Preview mute ramps over this long to avoid clicks.
const MUTE_RAMP_SECS: f32 = 0.01;
/// A lost device is reopened at once, then once more after this delay.
const RETRY_DELAY: Duration = Duration::from_secs(1);
/// How far ahead of the feeder readers are opened.
const PREFETCH_SECS: f64 = 2.0;
/// A reader for a clip already under the feeder is positioned this far
/// ahead of it, so it is usually ready before the mix gets there. Until it
/// joins, the other streams of its clip keep their level (see the mixer).
const JOIN_LEAD_SECS: f64 = 0.05;
/// The feeder looks ahead for readers every this many blocks.
const PLAN_EVERY: u32 = 5;
/// After play or seek, the feeder waits at most this long for the readers
/// of the clips under the playhead before it starts without them.
const START_WAIT: Duration = Duration::from_millis(100);

/// Playback settings (AU-010, UI-029): app preferences, not project edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineConfig {
    /// The output device, by the name `list_output_devices` reports; `None`
    /// plays on the system default device.
    pub device: Option<String>,
    /// Device buffer size in frames, from [`EngineConfig::MIN_BUFFER_FRAMES`]
    /// to [`EngineConfig::MAX_BUFFER_FRAMES`] (clamped); then clamped to
    /// what the device supports.
    pub buffer_frames: u32,
}

impl EngineConfig {
    pub const MIN_BUFFER_FRAMES: u32 = 256;
    pub const MAX_BUFFER_FRAMES: u32 = 4096;
    pub const DEFAULT_BUFFER_FRAMES: u32 = 1024;

    /// The buffer size actually asked of the device.
    pub fn buffer(&self) -> u32 {
        self.buffer_frames
            .clamp(Self::MIN_BUFFER_FRAMES, Self::MAX_BUFFER_FRAMES)
    }
}

impl Default for EngineConfig {
    fn default() -> EngineConfig {
        EngineConfig {
            device: None,
            buffer_frames: Self::DEFAULT_BUFFER_FRAMES,
        }
    }
}

/// One output device as the system lists it (UI-029).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDeviceInfo {
    /// The name to put in [`EngineConfig::device`].
    pub name: String,
    /// Whether it is the system default device now.
    pub is_default: bool,
}

/// Why the chosen output device isn't the one playing (AU-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceIssue {
    /// It isn't connected.
    NotConnected,
    /// Another application holds it exclusively.
    Busy,
    /// The operating system denied access to it.
    AccessDenied,
    /// Opening it failed or timed out (and, once the system default plays
    /// instead, failed again when retried after 1 s).
    NotResponding,
    /// It offers no sample format or rate FreeCut can play.
    Unsupported,
}

impl DeviceIssue {
    /// The issue behind a failure to open a chosen device.
    fn of(error: &OutputError) -> DeviceIssue {
        match error {
            OutputError::NoDevice | OutputError::DeviceNotFound(_) => DeviceIssue::NotConnected,
            OutputError::Busy(_) => DeviceIssue::Busy,
            OutputError::Denied(_) => DeviceIssue::AccessDenied,
            OutputError::UnsupportedFormat(_) | OutputError::UnsupportedRate(_) => {
                DeviceIssue::Unsupported
            }
            OutputError::Device(_) | OutputError::Thread(_) => DeviceIssue::NotResponding,
        }
    }
}

impl std::fmt::Display for DeviceIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DeviceIssue::NotConnected => "the chosen output device isn't connected",
            DeviceIssue::Busy => "the chosen output device is in use by another app",
            DeviceIssue::AccessDenied => "access to the chosen output device was denied",
            DeviceIssue::NotResponding => "the chosen output device didn't respond",
            DeviceIssue::Unsupported => "the chosen output device can't play FreeCut's audio",
        })
    }
}

/// Which device plays, for the app's banner and Preferences (AU-010,
/// UI-029).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineStatus {
    /// The device playing now; empty while none is open.
    pub device_name: String,
    /// The device asked for ([`EngineConfig::device`]); `None` is the
    /// system default.
    pub requested: Option<String>,
    /// The requested device can't be used and the system default plays
    /// instead.
    pub using_fallback: bool,
    /// Why the requested device isn't playing: set while falling back, and
    /// while it waits to be retried after an error.
    pub reason: Option<DeviceIssue>,
}

/// Why audio output couldn't start.
#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error("no audio output device is available")]
    NoDevice,
    #[error("the audio output device \"{0}\" isn't connected")]
    DeviceNotFound(String),
    #[error("the audio output device is in use by another application: {0}")]
    Busy(String),
    #[error("access to the audio output device was denied: {0}")]
    Denied(String),
    #[error("the output device uses the unsupported sample format {0}")]
    UnsupportedFormat(String),
    #[error("the output device runs at {0} Hz, which FreeCut can't play at")]
    UnsupportedRate(u32),
    #[error("the audio output device failed: {0}")]
    Device(String),
    #[error("can't start the audio thread")]
    Thread(#[source] std::io::Error),
}

/// One stereo frame in the ring, tagged with its generation.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Frame {
    generation: u32,
    left: f32,
    right: f32,
}

/// What the device callback last reported.
#[derive(Debug, Clone, Copy)]
struct ClockSample {
    generation: u32,
    /// Frames of `generation` handed to the device so far.
    frames: u64,
    /// How many of those the last callback wrote.
    block: u64,
    /// When the last callback ran.
    at: Instant,
    /// Time from that callback until its first frame is heard.
    latency: Duration,
}

/// The last [`ClockSample`], written by the device callback and read by
/// the controller and the feeder without locks: a seqlock, so the callback
/// never waits and never skips an update. One writer at a time (the tap of
/// the open device).
#[derive(Debug)]
struct ClockCell {
    /// Odd while a write is in progress.
    seq: AtomicU32,
    generation: AtomicU32,
    frames: AtomicU64,
    block: AtomicU64,
    /// `at`, in nanoseconds after `epoch`.
    at: AtomicI64,
    latency: AtomicU64,
    epoch: Instant,
}

impl ClockCell {
    fn new() -> ClockCell {
        ClockCell {
            seq: AtomicU32::new(0),
            generation: AtomicU32::new(0),
            frames: AtomicU64::new(0),
            block: AtomicU64::new(0),
            at: AtomicI64::new(0),
            latency: AtomicU64::new(0),
            epoch: Instant::now(),
        }
    }

    /// Publishes `sample` (the callback; real-time safe).
    fn store(&self, sample: &ClockSample) {
        let seq = self.seq.load(Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.generation.store(sample.generation, Ordering::Relaxed);
        self.frames.store(sample.frames, Ordering::Relaxed);
        self.block.store(sample.block, Ordering::Relaxed);
        self.at.store(
            clamp_i64(signed_nanos(sample.at, self.epoch)),
            Ordering::Relaxed,
        );
        let latency = u64::try_from(sample.latency.as_nanos()).unwrap_or(u64::MAX);
        self.latency.store(latency, Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(2), Ordering::Release);
    }

    /// The last sample published. Retries while a write is in progress; a
    /// writer that never finished (it can't, short of a panic mid-write)
    /// gives up after a bounded number of tries rather than hang.
    fn load(&self) -> ClockSample {
        let mut tries = 0u32;
        loop {
            let before = self.seq.load(Ordering::Acquire);
            let sample = ClockSample {
                generation: self.generation.load(Ordering::Relaxed),
                frames: self.frames.load(Ordering::Relaxed),
                block: self.block.load(Ordering::Relaxed),
                at: self.instant(self.at.load(Ordering::Relaxed)),
                latency: Duration::from_nanos(self.latency.load(Ordering::Relaxed)),
            };
            fence(Ordering::Acquire);
            let after = self.seq.load(Ordering::Relaxed);
            tries += 1;
            if (before == after && before.is_multiple_of(2)) || tries > 10_000 {
                return sample;
            }
            std::hint::spin_loop();
        }
    }

    fn instant(&self, nanos: i64) -> Instant {
        let d = Duration::from_nanos(nanos.unsigned_abs());
        let t = if nanos >= 0 {
            self.epoch.checked_add(d)
        } else {
            self.epoch.checked_sub(d)
        };
        t.unwrap_or(self.epoch)
    }
}

/// State shared with the device callback; atomics only.
#[derive(Debug)]
pub(crate) struct Shared {
    /// The generation to play (low 32 bits), and whether it continues
    /// playback without a jump (bit 32: a device reopen), so silence while
    /// it starts is an underrun, unlike the wait after play or seek.
    target: AtomicU64,
    playing: AtomicBool,
    muted: AtomicBool,
    rate: AtomicU32,
    underruns: AtomicU64,
    device_ok: AtomicBool,
    clock: ClockCell,
    /// The recorder's monitoring feed, if monitoring is on.
    monitor: Mutex<Option<MonitorFeed>>,
    last_error: Mutex<Option<String>>,
    status: Mutex<EngineStatus>,
}

const CONTINUING: u64 = 1 << 32;

impl Shared {
    fn new() -> Shared {
        Shared {
            target: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            rate: AtomicU32::new(48_000),
            underruns: AtomicU64::new(0),
            device_ok: AtomicBool::new(false),
            clock: ClockCell::new(),
            monitor: Mutex::new(None),
            last_error: Mutex::new(None),
            status: Mutex::new(EngineStatus::default()),
        }
    }

    /// The target generation and whether it continues playback.
    fn target(&self) -> (u32, bool) {
        let v = self.target.load(Ordering::Acquire);
        (v as u32, v & CONTINUING != 0)
    }

    fn set_target(&self, generation: u32, continuing: bool) {
        let flag = if continuing { CONTINUING } else { 0 };
        self.target
            .store(u64::from(generation) | flag, Ordering::Release);
    }

    /// The device reported lost samples of its own (an xrun).
    pub(crate) fn note_xrun(&self) {
        self.underruns.fetch_add(1, Ordering::Relaxed);
    }
}

/// Locks a mutex, ignoring poisoning: every value guarded here stays
/// consistent between statements.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A sample type a device accepts.
pub(crate) trait DeviceSample: Copy {
    const SILENCE: Self;
    /// Converts a nominal `[-1, 1]` value, clamping integer formats.
    fn from_f32(v: f32) -> Self;
}

impl DeviceSample for f32 {
    const SILENCE: f32 = 0.0;
    fn from_f32(v: f32) -> f32 {
        v
    }
}

impl DeviceSample for f64 {
    const SILENCE: f64 = 0.0;
    fn from_f32(v: f32) -> f64 {
        f64::from(v)
    }
}

impl DeviceSample for i16 {
    const SILENCE: i16 = 0;
    fn from_f32(v: f32) -> i16 {
        (v.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
    }
}

impl DeviceSample for u16 {
    const SILENCE: u16 = 1 << 15;
    fn from_f32(v: f32) -> u16 {
        (i32::from(i16::from_f32(v)) + (1 << 15)) as u16
    }
}

impl DeviceSample for i32 {
    const SILENCE: i32 = 0;
    fn from_f32(v: f32) -> i32 {
        (f64::from(v.clamp(-1.0, 1.0)) * f64::from(i32::MAX)).round() as i32
    }
}

impl DeviceSample for i8 {
    const SILENCE: i8 = 0;
    fn from_f32(v: f32) -> i8 {
        (v.clamp(-1.0, 1.0) * f32::from(i8::MAX)).round() as i8
    }
}

impl DeviceSample for u8 {
    const SILENCE: u8 = 1 << 7;
    fn from_f32(v: f32) -> u8 {
        (i16::from(i8::from_f32(v)) + (1 << 7)) as u8
    }
}

/// The consumer side, owned by the device callback.
pub(crate) struct OutputTap {
    ring: Consumer<Frame>,
    shared: Arc<Shared>,
    generation: u32,
    /// Frames of `generation` handed to the device.
    frames: u64,
    /// Silence before `generation`'s first frame is not an underrun (it
    /// started with play or seek).
    excused: bool,
    /// Current preview-mute gain (ramps between 0 and 1).
    mute: f32,
}

impl std::fmt::Debug for OutputTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputTap")
            .field("generation", &self.generation)
            .field("frames", &self.frames)
            .finish_non_exhaustive()
    }
}

impl OutputTap {
    fn new(ring: Consumer<Frame>, shared: Arc<Shared>) -> OutputTap {
        let mute = if shared.muted.load(Ordering::Relaxed) {
            0.0
        } else {
            1.0
        };
        let (generation, continuing) = shared.target();
        OutputTap {
            ring,
            generation,
            shared,
            frames: 0,
            excused: !continuing,
            mute,
        }
    }

    pub(crate) fn shared(&self) -> Arc<Shared> {
        Arc::clone(&self.shared)
    }

    /// Fills one device buffer of `channels`-channel interleaved samples.
    /// `latency` is the time until the buffer's first frame is heard.
    ///
    /// Real-time safe: no locks, no allocation, no blocking.
    pub(crate) fn fill<S: DeviceSample>(
        &mut self,
        out: &mut [S],
        channels: usize,
        latency: Duration,
        now: Instant,
    ) {
        let channels = channels.max(1);
        let (target, continuing) = self.shared.target();
        if target != self.generation {
            self.generation = target;
            self.frames = 0;
            self.excused = !continuing;
        }
        let playing = self.shared.playing.load(Ordering::Acquire);
        let mute_target = if self.shared.muted.load(Ordering::Relaxed) {
            0.0
        } else {
            1.0
        };
        let rate = self.shared.rate.load(Ordering::Relaxed).max(1);
        let mute_step = 1.0 / (MUTE_RAMP_SECS * rate as f32);
        // Monitoring: never waits for the lock, plays only at its own rate.
        // (An `Arc` clone: the guard must not borrow `self`.)
        let shared = Arc::clone(&self.shared);
        let mut slot = shared.monitor.try_lock().ok();
        let mut monitor = slot
            .as_deref_mut()
            .and_then(Option::as_mut)
            .filter(|feed| feed.rate() == rate);
        if let Some(feed) = monitor.as_deref_mut() {
            feed.trim(MONITOR_BACKLOG_FRAMES);
        }

        let mut got = 0u64;
        let mut short = false;
        let mut frames = out.chunks_exact_mut(channels);
        for frame in &mut frames {
            let (l, r) = match self.next_frame(target) {
                Some(f) => {
                    got += 1;
                    (f.left, f.right)
                }
                None => {
                    short = true;
                    (0.0, 0.0)
                }
            };
            self.mute += (mute_target - self.mute).clamp(-mute_step, mute_step);
            let (ml, mr) = monitor
                .as_deref_mut()
                .and_then(MonitorFeed::next_frame)
                .unwrap_or((0.0, 0.0));
            let (l, r) = (l * self.mute + ml, r * self.mute + mr);
            if let [only] = frame {
                *only = S::from_f32(0.5 * (l + r));
            } else {
                frame[0] = S::from_f32(l);
                frame[1] = S::from_f32(r);
                frame[2..].fill(S::SILENCE);
            }
        }
        frames.into_remainder().fill(S::SILENCE);

        // A dry ring after this generation started playing is an underrun,
        // and so is one while a reopened device waits to continue; the wait
        // for the first frames after play or seek is not.
        if playing && short && (self.frames > 0 || !self.excused) {
            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
        self.frames += got;
        self.shared.clock.store(&ClockSample {
            generation: self.generation,
            frames: self.frames,
            block: got,
            at: now,
            latency,
        });
    }

    /// The next frame of generation `target`, discarding older ones. A frame
    /// of a newer generation stays for the next callback.
    fn next_frame(&mut self, target: u32) -> Option<Frame> {
        loop {
            let generation = self.ring.peek().ok()?.generation;
            if generation == target {
                return self.ring.pop().ok();
            }
            // Generations only grow (wrapping): the frame is older when the
            // target is ahead of it.
            if (target.wrapping_sub(generation) as i32) > 0 {
                let _ = self.ring.pop();
            } else {
                return None;
            }
        }
    }
}

/// The opened device, as the feeder sees it.
#[derive(Debug, Clone)]
pub(crate) struct DeviceInfo {
    pub rate: SampleRate,
    pub name: String,
    /// The buffer size the device was opened with, in frames; `None` when
    /// the device chose its own.
    pub buffer_frames: Option<u32>,
}

/// Called (from any thread) when the device stops working.
pub(crate) type LostHandler = Box<dyn Fn(String) + Send + 'static>;

/// An audio output device API.
pub(crate) trait Backend: Send {
    /// Opens the output device named `device` (`None`: the system default),
    /// closing any open one, and starts it pulling from `tap`.
    fn open(
        &mut self,
        device: Option<&str>,
        tap: OutputTap,
        buffer_frames: u32,
        on_lost: LostHandler,
    ) -> Result<DeviceInfo, OutputError>;

    /// Stops and releases the device.
    fn close(&mut self);
}

/// The controller's transport state.
#[derive(Debug)]
struct Control {
    generation: u32,
    playing: bool,
    /// Timeline sample at which generation `generation` starts.
    base: i64,
    /// The last position reported, in samples (monotonic guard).
    last: i64,
    rate: SampleRate,
    /// Set while there is no device: position runs on the wall clock from
    /// `base` at this instant.
    wall: Option<Instant>,
}

impl Control {
    /// Starts a new generation at sample `from`. `continuing`: it carries
    /// on playback without a jump (a device reopen), so a gap before its
    /// first frame counts as an underrun.
    fn restart(&mut self, shared: &Shared, from: i64, playing: bool, continuing: bool) -> u32 {
        self.generation = self.generation.wrapping_add(1);
        self.base = from;
        self.last = from;
        self.playing = playing;
        shared.playing.store(playing, Ordering::Release);
        shared.set_target(self.generation, continuing);
        self.generation
    }

    /// The timeline sample being heard at `now`.
    fn heard(&self, clock: &ClockSample, now: Instant) -> i64 {
        let rate = i128::from(self.rate.hz());
        if let Some(since) = self.wall {
            let k = (signed_nanos(now, since).max(0) * rate).div_euclid(1_000_000_000);
            return self.base.saturating_add(clamp_i64(k));
        }
        if clock.generation != self.generation {
            return self.base;
        }
        // The last callback's first frame is heard `latency` after it ran.
        let first = i128::from(clock.frames - clock.block);
        let since = signed_nanos(now, clock.at) - clamp_nanos(clock.latency);
        let k =
            (first + (since * rate).div_euclid(1_000_000_000)).clamp(0, i128::from(clock.frames));
        self.base.saturating_add(clamp_i64(k))
    }

    /// Where playback continues seamlessly: just after the last frame handed
    /// to the device.
    fn resume_point(&self, clock: &ClockSample, now: Instant) -> i64 {
        if self.wall.is_some() {
            return self.heard(clock, now);
        }
        if clock.generation != self.generation {
            return self.base;
        }
        self.base
            .saturating_add(i64::try_from(clock.frames).unwrap_or(i64::MAX))
    }
}

fn signed_nanos(later: Instant, earlier: Instant) -> i128 {
    match later.checked_duration_since(earlier) {
        Some(d) => clamp_nanos(d),
        None => -clamp_nanos(earlier.duration_since(later)),
    }
}

fn clamp_nanos(d: Duration) -> i128 {
    i128::try_from(d.as_nanos()).unwrap_or(i128::MAX)
}

fn clamp_i64(v: i128) -> i64 {
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

pub(crate) enum Command {
    Start {
        generation: u32,
        from: i64,
    },
    Stop,
    SetSource {
        schedule: Arc<dyn Schedule>,
        opener: Arc<dyn SourceOpener>,
    },
    MasterGain(f32),
    Reconfigure(EngineConfig),
    DeviceLost {
        stream: u64,
        reason: String,
    },
    Shutdown,
}

/// Renders ahead into the ring and manages the device.
struct Feeder {
    commands: Receiver<Command>,
    events: Sender<Command>,
    shared: Arc<Shared>,
    control: Arc<Mutex<Control>>,
    backend: Box<dyn Backend>,
    config: EngineConfig,
    retry_delay: Duration,
    ring: Option<Producer<Frame>>,
    /// Identifies the open device stream, so late errors from a replaced
    /// one are ignored.
    stream_id: u64,
    mixer: Mixer,
    master_gain: f32,
    schedule: Arc<dyn Schedule>,
    opener: Arc<dyn SourceOpener>,
    rendering: bool,
    generation: u32,
    /// The next timeline sample to render.
    next: i64,
    mix: Vec<f32>,
    frames: Vec<Frame>,
    retry: Option<(Instant, String)>,
    /// Frames of `generation` pushed into the ring.
    pushed: u64,
    prefetch: Prefetcher,
    /// Readers asked for and not arrived yet, by input key.
    pending: HashMap<u64, Pending>,
    /// Blocks rendered since the last look-ahead.
    since_plan: u32,
    /// Until when a fresh generation waits for the readers it starts with.
    start_by: Option<Instant>,
    start_wait: Duration,
    /// The open device's buffer size in frames: the read-ahead covers two of
    /// them, however long they are.
    device_buffer: u32,
}

/// A reader asked of the prefetcher.
#[derive(Debug, Clone, Copy)]
struct Pending {
    source: SourceKey,
    /// Media time it is positioned at.
    at: Tick,
    /// The timeline sample that media time plays at.
    sample: i64,
}

impl Feeder {
    /// Handles one command; `false` means shut down.
    fn handle(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Start { generation, from } => {
                let current = {
                    let c = lock(&self.control);
                    c.playing && c.generation == generation
                };
                if current {
                    self.begin(generation, from);
                    if self.ring.is_none() && self.retry.is_none() {
                        self.reopen("no audio output device".to_owned(), false, true);
                    }
                }
            }
            Command::Stop => self.rendering = false,
            Command::SetSource { schedule, opener } => {
                // Readers from another opener may read other files now:
                // each plays on until its replacement is ready.
                if !Arc::ptr_eq(&self.opener, &opener) {
                    self.mixer.invalidate_readers();
                    self.prefetch.next_epoch();
                    self.pending.clear();
                }
                self.schedule = schedule;
                self.opener = opener;
                if self.rendering {
                    self.plan();
                }
            }
            Command::MasterGain(g) => {
                self.master_gain = g;
                self.mixer.set_master_gain(g);
            }
            Command::Reconfigure(config) => {
                let settled = config == self.config
                    && self.ring.is_some()
                    && !lock(&self.shared.status).using_fallback;
                self.config = config;
                if !settled {
                    tracing::info!(device = ?self.config.device, buffer = self.config.buffer(), "switching the audio output");
                    self.retry = None;
                    self.reopen("the audio output settings changed".to_owned(), true, false);
                }
            }
            Command::DeviceLost { stream, reason } => {
                if stream == self.stream_id && self.ring.is_some() {
                    tracing::warn!(%reason, "audio output device lost; reopening");
                    self.reopen(reason, true, true);
                }
            }
            Command::Shutdown => {
                self.backend.close();
                return false;
            }
        }
        true
    }

    /// Starts rendering generation `generation` at timeline sample `from`
    /// (play, seek, or a device reopen): forgets the readers asked for at
    /// the old position and asks for the new one's.
    fn begin(&mut self, generation: u32, from: i64) {
        self.generation = generation;
        self.next = from;
        self.rendering = true;
        self.pushed = 0;
        self.prefetch.next_epoch();
        self.pending.clear();
        self.start_by = Some(Instant::now() + self.start_wait);
        self.plan();
    }

    /// Renders one block into the ring if it is below the read-ahead.
    fn fill(&mut self) -> bool {
        self.collect_ready();
        if !self.rendering {
            return false;
        }
        let Some(ring) = self.ring.as_ref() else {
            return false;
        };
        let hz = f64::from(self.mixer.sample_rate().hz());
        let block = (hz * BLOCK_SECS) as usize;
        // At least two device buffers and a block, so a callback asking for
        // a whole long buffer (4096 frames at 16 kHz is 256 ms) never finds
        // the ring short.
        let floor = 2 * self.device_buffer as usize + block;
        let target = ((hz * READ_AHEAD_SECS) as usize)
            .max(floor)
            .min(RING_FRAMES);
        // Only this generation's frames count: older ones are dropped by
        // the callback, so a seek renders its first block at once.
        let clock = self.shared.clock.load();
        let consumed = if clock.generation == self.generation {
            clock.frames
        } else {
            0
        };
        let buffered = usize::try_from(self.pushed.saturating_sub(consumed)).unwrap_or(usize::MAX);
        if buffered.saturating_add(block) > target || ring.slots() < block {
            return false;
        }
        if self.pushed == 0 && self.waiting_to_start(block) {
            return false;
        }
        self.mix.resize(block * 2, 0.0);
        self.mixer
            .render(&*self.schedule, &*self.opener, self.next, &mut self.mix);
        let generation = self.generation;
        self.frames.clear();
        self.frames
            .extend(self.mix.as_chunks::<2>().0.iter().map(|s| Frame {
                generation,
                left: s[0],
                right: s[1],
            }));
        let Some(ring) = self.ring.as_mut() else {
            return false;
        };
        // Room was checked above; the callback only ever frees slots.
        let _ = ring.push_entire_slice(&self.frames);
        self.pushed += block as u64;
        self.next = self.next.saturating_add(block as i64);
        self.since_plan += 1;
        if self.since_plan >= PLAN_EVERY {
            self.plan();
        }
        true
    }

    /// Whether a fresh generation should wait before its first block: a
    /// reader for a clip under it was asked for and isn't there yet, and
    /// the wait hasn't run out.
    fn waiting_to_start(&self, block: usize) -> bool {
        let first_block_end = self
            .next
            .saturating_add((block + self.mixer.latency()) as i64);
        self.start_by.is_some_and(|by| Instant::now() < by)
            && self.pending.values().any(|p| p.sample < first_block_end)
    }

    /// Hands the readers the prefetcher opened to the mixer.
    fn collect_ready(&mut self) {
        for ready in self.prefetch.ready() {
            let answered = self
                .pending
                .get(&ready.key)
                .is_some_and(|p| p.source == ready.source && p.at == ready.at);
            if answered {
                self.pending.remove(&ready.key);
            }
            if Arc::ptr_eq(&ready.opener, &self.opener) && ready.rate == self.mixer.sample_rate() {
                self.mixer
                    .offer(ready.key, ready.source, ready.at, ready.reader);
            }
        }
    }

    /// Looks ahead over the next [`PREFETCH_SECS`] of the schedule and asks
    /// the prefetcher for every reader the mixer will need and doesn't hold:
    /// positioned at the clip's start, or a little ahead of the feeder for a
    /// clip already under it.
    fn plan(&mut self) {
        self.since_plan = 0;
        let rate = self.mixer.sample_rate();
        let hz = f64::from(rate.hz());
        let st = sample_ticks(rate);
        let lead = if self.pushed == 0 {
            0
        } else {
            (hz * JOIN_LEAD_SECS) as i64
        };
        let horizon = self.next.saturating_add((hz * PREFETCH_SECS) as i64);
        let (Ok(t0), Ok(t1)) = (sample_start(self.next, rate), sample_start(horizon, rate)) else {
            return;
        };
        let Ok(window) = TimeRange::from_start_end(t0, t1) else {
            return;
        };
        let inputs = self.schedule.inputs(window);
        let scheduled = |key: u64| inputs.iter().any(|i| i.key == key);
        let mut wanted = Vec::with_capacity(inputs.len());
        for input in &inputs {
            if input.is_silent() {
                continue;
            }
            let s0 = sample_at(input.range.start(), rate);
            let s1 = sample_at(input.range.end(), rate);
            let first = s0.max(self.next.saturating_add(lead));
            if first >= s1 {
                continue;
            }
            let Some(at) = (first - s0)
                .checked_mul(st)
                .and_then(|k| input.source_start.checked_add(Tick::from_raw(k)).ok())
            else {
                continue;
            };
            wanted.push(input.key);
            // A clip ahead is wanted at its start exactly; for one already
            // under the feeder any position will do.
            let ahead = s0 > self.next;
            // A clip under the feeder that an edit re-keyed carries on with
            // the reader its old key holds: nothing to open.
            if !ahead && self.mixer.can_adopt(input, scheduled) {
                self.pending.remove(&input.key);
                continue;
            }
            match self.mixer.reader_state(input.key, &input.source) {
                ReaderState::Held => continue,
                ReaderState::Prepared(p) if p == at || !ahead => continue,
                ReaderState::Prepared(_) | ReaderState::Missing => {}
            }
            if let Some(p) = self.pending.get(&input.key)
                && p.source == input.source
                && (p.at == at || !ahead)
            {
                continue;
            }
            self.pending.insert(
                input.key,
                Pending {
                    source: input.source,
                    at,
                    sample: first,
                },
            );
            self.prefetch.request(Request {
                epoch: self.prefetch.epoch(),
                key: input.key,
                source: input.source,
                at,
                rate,
                opener: Arc::clone(&self.opener),
            });
        }
        self.mixer.prune_prepared(|key| wanted.contains(&key));
        self.pending.retain(|key, _| wanted.contains(key));
    }

    fn retry_if_due(&mut self) {
        if self
            .retry
            .as_ref()
            .is_some_and(|(at, _)| Instant::now() >= *at)
            && let Some((_, reason)) = self.retry.take()
        {
            self.reopen(reason, false, true);
        }
    }

    /// Closes the device and opens the configured one, or the system
    /// default (see [`Feeder::open_device`]). Until a device works the
    /// position runs on the wall clock; with `retry`, a failure is retried
    /// once after [`RETRY_DELAY`], otherwise it is reported. `continuing`:
    /// silence until the new device gets audio counts as an underrun (a lost
    /// device, not a deliberate switch).
    fn reopen(&mut self, reason: String, retry: bool, continuing: bool) {
        self.backend.close();
        self.ring = None;
        self.shared.device_ok.store(false, Ordering::Release);
        {
            // Hold the position just after the last frame the old device got.
            let mut c = lock(&self.control);
            if c.playing && c.wall.is_none() {
                let clock = self.shared.clock.load();
                let from = c.resume_point(&clock, Instant::now());
                let last = c.last;
                let generation = c.restart(&self.shared, from, true, continuing);
                c.last = last;
                drop(c);
                self.begin(generation, from);
            }
        }
        let e = match self.open_device(retry, continuing) {
            Ok(()) => {
                *lock(&self.shared.last_error) = None;
                return;
            }
            Err(OpenFailure::Retry(e) | OpenFailure::Failed(e)) => e,
        };
        {
            let mut c = lock(&self.control);
            if c.playing && c.wall.is_none() {
                c.wall = Some(Instant::now());
            }
        }
        if retry {
            tracing::warn!(error = %e, "reopening the audio output failed; retrying");
            self.retry = Some((Instant::now() + self.retry_delay, reason));
        } else {
            tracing::warn!(error = %e, "no audio output; playing on the system clock");
            *lock(&self.shared.last_error) = Some(format!("{reason}: {e}"));
        }
    }

    /// Opens the configured device and continues playback on it. A chosen
    /// device that is missing, busy, denied or unsupported gives way to the
    /// system default at once; one that fails otherwise does so only without
    /// `may_wait` (else [`OpenFailure::Retry`]: try it again later).
    fn open_device(&mut self, may_wait: bool, continuing: bool) -> Result<(), OpenFailure> {
        let mut issue = None;
        if let Some(name) = self.config.device.clone() {
            match self.open_stream(Some(&name)) {
                Ok(info) => {
                    self.attach(info, None, continuing);
                    return Ok(());
                }
                Err(e) => {
                    let i = DeviceIssue::of(&e);
                    tracing::warn!(device = %name, error = %e, "can't open the chosen audio output");
                    if i == DeviceIssue::NotResponding && may_wait {
                        self.set_status(String::new(), false, Some(i));
                        return Err(OpenFailure::Retry(e));
                    }
                    issue = Some(i);
                }
            }
        }
        match self.open_stream(None) {
            Ok(info) => {
                if let Some(i) = issue {
                    tracing::warn!(device = %info.name, "{i}; playing on the system default device");
                }
                self.attach(info, issue, continuing);
                Ok(())
            }
            Err(e) => {
                self.set_status(String::new(), false, issue);
                Err(OpenFailure::Failed(e))
            }
        }
    }

    /// Opens `device` (`None`: the system default) on a fresh ring.
    fn open_stream(&mut self, device: Option<&str>) -> Result<DeviceInfo, OutputError> {
        self.stream_id += 1;
        let (producer, consumer) = RingBuffer::new(RING_FRAMES);
        let tap = OutputTap::new(consumer, Arc::clone(&self.shared));
        let (stream, events) = (self.stream_id, self.events.clone());
        let on_lost: LostHandler = Box::new(move |reason| {
            let _ = events.send(Command::DeviceLost { stream, reason });
        });
        let info = self
            .backend
            .open(device, tap, self.config.buffer(), on_lost)?;
        self.ring = Some(producer);
        Ok(info)
    }

    fn set_status(&self, device_name: String, using_fallback: bool, reason: Option<DeviceIssue>) {
        *lock(&self.shared.status) = EngineStatus {
            device_name,
            requested: self.config.device.clone(),
            using_fallback,
            reason,
        };
    }

    /// Switches to a newly opened device, continuing where playback is.
    /// `issue`: why it is the system default rather than the chosen one.
    fn attach(&mut self, info: DeviceInfo, issue: Option<DeviceIssue>, continuing: bool) {
        self.device_buffer = info.buffer_frames.unwrap_or(self.config.buffer());
        self.set_status(info.name, issue.is_some(), issue);
        self.shared.rate.store(info.rate.hz(), Ordering::Relaxed);
        if info.rate != self.mixer.sample_rate() {
            self.mixer = playback_mixer(info.rate, self.master_gain);
        }
        self.shared.device_ok.store(true, Ordering::Release);

        let mut c = lock(&self.control);
        let old = c.rate;
        let convert = |k: i64| {
            sample_start(k, old)
                .map(|t| sample_at(t, info.rate))
                .unwrap_or(k)
        };
        if c.playing {
            let clock = self.shared.clock.load();
            let from = convert(c.resume_point(&clock, Instant::now()));
            let last = convert(c.last);
            c.rate = info.rate;
            c.wall = None;
            let generation = c.restart(&self.shared, from, true, continuing);
            c.last = last.max(from);
            drop(c);
            self.begin(generation, from);
        } else {
            c.base = convert(c.base);
            c.rate = info.rate;
        }
    }

    fn run(mut self) {
        loop {
            while let Ok(cmd) = self.commands.try_recv() {
                if !self.handle(cmd) {
                    return;
                }
            }
            if self.fill() {
                continue;
            }
            self.retry_if_due();
            match self.commands.recv_timeout(IDLE_WAIT) {
                Ok(cmd) => {
                    if !self.handle(cmd) {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// Test driver: does everything the threads would do right now.
    #[cfg(test)]
    fn pump(&mut self) {
        self.handle_commands();
        self.retry_if_due();
        loop {
            self.prefetch.serve_queued();
            if !self.fill() {
                break;
            }
        }
    }

    #[cfg(test)]
    fn handle_commands(&mut self) {
        while let Ok(cmd) = self.commands.try_recv() {
            self.handle(cmd);
        }
    }
}

/// Why no device could be opened.
#[derive(Debug)]
enum OpenFailure {
    /// The chosen device failed in a way worth trying again shortly.
    Retry(OutputError),
    /// Neither the chosen device nor the system default opened.
    Failed(OutputError),
}

/// A mixer for playback: it never opens a reader itself.
fn playback_mixer(rate: SampleRate, master_gain: f32) -> Mixer {
    let mut mixer = Mixer::new(rate);
    mixer.defer_opens();
    mixer.set_master_gain(master_gain);
    mixer
}

enum Worker {
    Thread(Option<JoinHandle<()>>),
    #[cfg(test)]
    Manual(Box<Mutex<Feeder>>),
}

type EngineParts = (Feeder, Arc<Shared>, Arc<Mutex<Control>>, Sender<Command>);

/// The playback engine over any [`Backend`].
pub(crate) struct Engine {
    shared: Arc<Shared>,
    control: Arc<Mutex<Control>>,
    commands: Sender<Command>,
    worker: Worker,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("control", &*lock(&self.control))
            .field("shared", &self.shared)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// Opens the configured device (or the system default) through
    /// `backend` and starts the feeder thread.
    pub(crate) fn start(
        backend: Box<dyn Backend>,
        config: EngineConfig,
    ) -> Result<Engine, OutputError> {
        let prefetch = Prefetcher::spawn().map_err(OutputError::Thread)?;
        let (feeder, shared, control, commands) =
            Engine::build(backend, config, RETRY_DELAY, prefetch, START_WAIT)?;
        let handle = std::thread::Builder::new()
            .name("freecut-audio-feeder".to_owned())
            .spawn(move || feeder.run())
            .map_err(OutputError::Thread)?;
        Ok(Engine {
            shared,
            control,
            commands,
            worker: Worker::Thread(Some(handle)),
        })
    }

    /// An engine whose feeder runs only when [`Engine::pump`] is called.
    #[cfg(test)]
    pub(crate) fn manual(
        backend: Box<dyn Backend>,
        config: EngineConfig,
    ) -> Result<Engine, OutputError> {
        let (feeder, shared, control, commands) = Engine::build(
            backend,
            config,
            Duration::ZERO,
            Prefetcher::manual(),
            Duration::ZERO,
        )?;
        Ok(Engine {
            shared,
            control,
            commands,
            worker: Worker::Manual(Box::new(Mutex::new(feeder))),
        })
    }

    #[cfg(test)]
    fn with_feeder(&self, f: impl FnOnce(&mut Feeder)) {
        if let Worker::Manual(feeder) = &self.worker {
            f(&mut lock(feeder));
        }
    }

    /// Runs the feeder (and serves the prefetcher, unless held) as far as
    /// it would go now.
    #[cfg(test)]
    pub(crate) fn pump(&self) {
        self.with_feeder(Feeder::pump);
    }

    /// Lets the feeder handle its commands, rendering nothing.
    #[cfg(test)]
    pub(crate) fn pump_commands(&self) {
        self.with_feeder(Feeder::handle_commands);
    }

    /// While held, readers asked for are not opened (a slow disk).
    #[cfg(test)]
    pub(crate) fn hold_prefetch(&self, held: bool) {
        self.with_feeder(|f| f.prefetch.hold(held));
    }

    fn build(
        backend: Box<dyn Backend>,
        config: EngineConfig,
        retry_delay: Duration,
        prefetch: Prefetcher,
        start_wait: Duration,
    ) -> Result<EngineParts, OutputError> {
        let shared = Arc::new(Shared::new());
        // Placeholder rate until the device reports its own; 48 kHz is on
        // the tick grid.
        let rate = SampleRate::new(48_000).map_err(|_| OutputError::UnsupportedRate(48_000))?;
        let control = Arc::new(Mutex::new(Control {
            generation: 0,
            playing: false,
            base: 0,
            last: 0,
            rate,
            wall: None,
        }));
        let (commands, inbox) = crossbeam_channel::unbounded();
        let device_buffer = config.buffer();
        let mut feeder = Feeder {
            commands: inbox,
            events: commands.clone(),
            shared: Arc::clone(&shared),
            control: Arc::clone(&control),
            backend,
            config,
            retry_delay,
            ring: None,
            stream_id: 0,
            mixer: playback_mixer(rate, 1.0),
            master_gain: 1.0,
            schedule: Arc::new(Silence),
            opener: Arc::new(Silence),
            rendering: false,
            generation: 0,
            next: 0,
            mix: Vec::new(),
            frames: Vec::new(),
            retry: None,
            pushed: 0,
            prefetch,
            pending: HashMap::new(),
            since_plan: 0,
            start_by: None,
            start_wait,
            device_buffer,
        };
        match feeder.open_device(true, true) {
            Ok(()) => {}
            // The chosen device didn't respond: start without a device and
            // try it again shortly.
            Err(OpenFailure::Retry(e)) => {
                feeder.retry = Some((Instant::now() + retry_delay, e.to_string()));
            }
            Err(OpenFailure::Failed(e)) => return Err(e),
        }
        Ok((feeder, shared, control, commands))
    }

    fn send(&self, cmd: Command) {
        // Fails only after the feeder exited, when nothing plays anyway.
        let _ = self.commands.send(cmd);
    }

    fn control(&self) -> MutexGuard<'_, Control> {
        lock(&self.control)
    }

    pub(crate) fn play(&self, from: Tick) {
        let mut c = self.control();
        let from = sample_at(from, c.rate);
        let generation = c.restart(&self.shared, from, true, false);
        c.wall = if self.shared.device_ok.load(Ordering::Acquire) {
            None
        } else {
            Some(Instant::now())
        };
        drop(c);
        self.send(Command::Start { generation, from });
    }

    pub(crate) fn pause(&self) {
        let mut c = self.control();
        let base = c.base;
        c.restart(&self.shared, base, false, false);
        c.wall = None;
        drop(c);
        self.send(Command::Stop);
    }

    pub(crate) fn seek(&self, to: Tick) {
        let mut c = self.control();
        if c.playing {
            drop(c);
            self.play(to);
        } else {
            c.base = sample_at(to, c.rate);
        }
    }

    /// Replaces what plays. During playback nothing queued is dropped and
    /// no generation starts: the feeder renders the new schedule from where
    /// it is, so the change is heard after the read-ahead, without a gap,
    /// and readers keep reading sequentially.
    pub(crate) fn set_source(&self, schedule: Arc<dyn Schedule>, opener: Arc<dyn SourceOpener>) {
        self.send(Command::SetSource { schedule, opener });
    }

    pub(crate) fn position_at(&self, now: Instant) -> Option<Tick> {
        let mut c = self.control();
        if !c.playing {
            return None;
        }
        let clock = self.shared.clock.load();
        let heard = c.heard(&clock, now).max(c.last);
        c.last = heard;
        sample_start(heard, c.rate).ok()
    }

    pub(crate) fn position(&self) -> Option<Tick> {
        self.position_at(Instant::now())
    }

    pub(crate) fn is_playing(&self) -> bool {
        self.control().playing
    }

    pub(crate) fn set_master_gain(&self, linear: f32) {
        self.send(Command::MasterGain(linear));
    }

    /// Plays `feed` (a recorder's monitoring) on the output after the
    /// master stage, or stops monitoring with `None`. Returns whether the
    /// feed's rate is the device's (otherwise it stays silent).
    pub(crate) fn set_monitor(&self, feed: Option<MonitorFeed>) -> bool {
        let rate = self.shared.rate.load(Ordering::Relaxed);
        let matches = feed.as_ref().is_some_and(|f| f.rate() == rate);
        let old = std::mem::replace(&mut *lock(&self.shared.monitor), feed);
        drop(old);
        matches
    }

    pub(crate) fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::Relaxed);
    }

    pub(crate) fn underruns(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }

    pub(crate) fn sample_rate(&self) -> SampleRate {
        self.control().rate
    }

    pub(crate) fn device_name(&self) -> String {
        lock(&self.shared.status).device_name.clone()
    }

    pub(crate) fn status(&self) -> EngineStatus {
        lock(&self.shared.status).clone()
    }

    /// Switches the output device or buffer size. Playback carries on from
    /// where it is (or stays paused at the same position) on the new device.
    pub(crate) fn reconfigure(&self, config: EngineConfig) {
        self.send(Command::Reconfigure(config));
    }

    pub(crate) fn last_error(&self) -> Option<String> {
        lock(&self.shared.last_error).clone()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.send(Command::Shutdown);
        if let Worker::Thread(handle) = &mut self.worker
            && let Some(handle) = handle.take()
        {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests;
