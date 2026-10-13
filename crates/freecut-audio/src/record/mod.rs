//! Voiceover recording (AU-027; audio-engine.md §12, architecture C53).
//!
//! A [`Recorder`] takes frames from a [`CaptureSource`] (a microphone,
//! [`CpalInput`] with the feature `record`, or [`SyntheticInput`] in tests)
//! and writes takes as 24-bit WAV files with FreeCut's own writer
//! ([`wav`]), switching to RF64 past 4 GiB and keeping the file readable if
//! the app dies mid-take.
//!
//! Three threads share the work:
//! - the **capture callback** (the source's) converts to `f32`, applies
//!   the input gain through a 10 ms smoother, updates the meter atomics,
//!   feeds monitoring, runs the countdown, and pushes the take's frames
//!   into a lock-free ring of 2 s. It never blocks, allocates or locks; a
//!   full ring ends the take ([`EndReason::Overrun`]) rather than leave a
//!   gap;
//! - the **writer** drains the ring into the file, checks free space,
//!   refreshes the header every second and finishes the take;
//! - the **caller** (the app) only sends commands and reads atomics and
//!   [`RecorderEvent`]s. [`Recorder::stop`] and [`Recorder::cancel`] wait
//!   for the writer to finish the take (normally well under 200 ms); the
//!   UI thread uses [`Recorder::request_stop`] and the
//!   [`RecorderEvent::Finished`] event instead.
//!
//! **States** ([`RecorderState`]): *Ready* (device open, meter live,
//! nothing written) → *Counting down* (frames counted, not written) →
//! *Pre-roll* (written as a handle before the start position) →
//! *Recording* → *Stopping* (draining and finishing the file) → *Ready*.
//! A lost device leads to *Waiting*; the app then opens a new recorder.
//!
//! **Files.** A take's file is created when its first frame is captured,
//! never replacing an existing file, with a `<file>.recording` marker
//! beside it until it is finished (PM-043). A take cancelled during the
//! countdown or before its pre-roll is over leaves nothing; one cancelled
//! while recording is finished and kept ([`EndReason::Cancelled`]): FreeCut
//! never deletes a user's audio without asking.

mod meter;
mod monitor;
mod shared;
mod source;
mod synthetic;
pub mod wav;
mod writer;

#[cfg(feature = "record")]
mod cpal_in;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use freecut_timeline::model::GainDb;
use freecut_timeline::time::SampleRate;
use rtrb::RingBuffer;

use crate::gain::gain_amplitude;
use shared::{
    COUNTING, EndRequest, NO_END, PREROLL, READY, RECORDING, REQ_CANCEL, REQ_STOP, Shared,
};
use writer::{Command, TakeOrder, TakeResult, Writer};

#[cfg(feature = "record")]
pub use cpal_in::{CpalInput, InputDeviceInfo, list_inputs};
pub use meter::{CLIP_DB, METER_FLOOR_DB, MeterReading};
pub use monitor::MonitorFeed;
pub use source::{
    CaptureSink, CaptureSource, FailHandle, InputSample, SourceFailure, SourceFormat,
};
pub use synthetic::{Signal, SyntheticDriver, SyntheticInput};

pub(crate) use monitor::MAX_BACKLOG_FRAMES as MONITOR_BACKLOG_FRAMES;
#[cfg(test)]
pub(crate) use monitor::channel as monitor_channel;

/// The ring between the callback and the writer holds this much audio.
const RING_SECS: u32 = 2;
/// The longest [`Recorder::stop`] waits for the writer.
const FINISH_TIMEOUT: Duration = Duration::from_secs(10);
/// Takes shorter than this (after the pre-roll) count as accidental
/// (AU-027).
pub const MIN_TAKE: Duration = Duration::from_millis(300);
/// The input level range (AU-027), in hundredths of a decibel.
const INPUT_GAIN_RANGE: (i32, i32) = (-2_000, 2_000);

/// Mono or stereo takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Channels {
    #[default]
    Mono,
    Stereo,
}

impl Channels {
    pub fn count(self) -> u16 {
        match self {
            Channels::Mono => 1,
            Channels::Stereo => 2,
        }
    }
}

/// Free bytes on the volume holding a path, if the system can tell.
type FreeBytes = dyn Fn(&Path) -> Option<u64> + Send + Sync;

/// How the writer learns the free space on the take's disk.
#[derive(Clone)]
pub struct SpaceProbe(Arc<FreeBytes>);

impl std::fmt::Debug for SpaceProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SpaceProbe")
    }
}

impl SpaceProbe {
    /// The system's free space on the file's volume.
    pub fn system() -> SpaceProbe {
        SpaceProbe(Arc::new(|path: &Path| {
            let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
            fs4::available_space(dir.unwrap_or(path)).ok()
        }))
    }

    /// A probe of the caller's (tests: a scripted disk).
    pub fn new(probe: impl Fn(&Path) -> Option<u64> + Send + Sync + 'static) -> SpaceProbe {
        SpaceProbe(Arc::new(probe))
    }

    pub(crate) fn free(&self, path: &Path) -> Option<u64> {
        (self.0)(path)
    }
}

impl Default for SpaceProbe {
    fn default() -> SpaceProbe {
        SpaceProbe::system()
    }
}

/// What a recorder records and how. The rate is the source's (open the
/// source at the project's rate when the device offers it).
#[derive(Debug, Clone)]
pub struct RecordConfig {
    /// The next take's file. It must not exist yet.
    pub path: PathBuf,
    pub channels: Channels,
    /// Input level, −20 to +20 dB (clamped).
    pub gain: GainDb,
    /// Counted in captured frames before the take starts ([`Recorder::arm`]).
    pub countdown: Duration,
    /// Frames written before the start position, as a handle.
    pub preroll_frames: u64,
    /// The take stops by itself with this many frames in the file
    /// (pre-roll included): the punch range's Out plus its handle.
    pub stop_after_frames: Option<u64>,
    /// The take stops cleanly when the disk has less free space.
    pub min_free_bytes: u64,
    /// Below this, one [`RecorderEvent::LowDisk`] warns.
    pub warn_free_bytes: u64,
    /// "Hear yourself": feed the gained input to [`Recorder::monitor_feed`].
    pub monitor: bool,
    /// TPDF dither when reducing to 24 bits (tests turn it off to compare
    /// exact values).
    pub dither: bool,
    /// How often the header is brought up to date.
    pub header_refresh: Duration,
    /// The RIFF size past which the file becomes RF64 (4 GiB; tests lower
    /// it).
    pub rf64_threshold: u64,
    pub free_space: SpaceProbe,
}

impl RecordConfig {
    /// FreeCut's defaults (AU-027): mono, 0 dB, 3 s countdown, no pre-roll,
    /// stop at 50 MB free, warn at 500 MB, monitoring off.
    pub fn new(path: impl Into<PathBuf>) -> RecordConfig {
        RecordConfig {
            path: path.into(),
            channels: Channels::Mono,
            gain: GainDb::ZERO,
            countdown: Duration::from_secs(3),
            preroll_frames: 0,
            stop_after_frames: None,
            min_free_bytes: 50_000_000,
            warn_free_bytes: 500_000_000,
            monitor: false,
            dither: true,
            header_refresh: Duration::from_secs(1),
            rf64_threshold: wav::RIFF_LIMIT,
            free_space: SpaceProbe::system(),
        }
    }
}

/// The recorder's state, as the panel shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecorderState {
    /// No usable input (the device was lost).
    Waiting,
    /// The device is open and the meter is live; nothing is written.
    Ready,
    CountingDown,
    /// Writing the handle before the start position.
    PreRoll,
    Recording,
    /// Draining the ring and finishing the file.
    Stopping,
}

/// Why a take ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndReason {
    /// Stop.
    User,
    /// Cancel while recording: the file is kept but the take is not placed.
    Cancelled,
    /// The punch range's end ([`RecordConfig::stop_after_frames`]).
    RangeEnd,
    /// The device was unplugged; what was captured is kept.
    DeviceLost,
    /// The disk reached [`RecordConfig::min_free_bytes`].
    DiskLow,
    /// The writer fell 2 s behind; what was captured is kept.
    Overrun,
    /// The device changed its rate or format mid-take.
    RateChanged,
}

/// A point on the system's monotonic clock, as the audio clock relates to
/// it (`AudioEngine` positions are interpolated on the same clock).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClockStamp(pub(crate) Instant);

impl ClockStamp {
    pub fn instant(self) -> Instant {
        self.0
    }
}

/// A finished take.
#[derive(Debug, Clone, PartialEq)]
pub struct TakeInfo {
    pub path: PathBuf,
    /// Frames in the file, pre-roll included.
    pub frames: u64,
    pub rate: SampleRate,
    pub channels: Channels,
    /// The leading frames that are the pre-roll handle.
    pub preroll_frames: u64,
    /// The input latency measured during the take.
    pub input_latency: Duration,
    /// When the first frame reached the input.
    pub started_at: ClockStamp,
    pub ended_by: EndReason,
}

impl TakeInfo {
    /// Shorter than [`MIN_TAKE`] after the pre-roll: an accidental click;
    /// the app removes the file and places nothing (AU-027).
    pub fn is_too_short(&self) -> bool {
        let min = u64::from(self.rate.hz()) * 3 / 10;
        self.frames.saturating_sub(self.preroll_frames) < min
    }
}

/// What the recorder reports to the panel.
#[derive(Debug, Clone)]
pub enum RecorderEvent {
    State(RecorderState),
    /// A take ended and its file is complete.
    Finished(TakeInfo),
    /// A take couldn't be written; what reached the file stays, with its
    /// marker, for recovery.
    Failed(RecordError),
    /// The disk is below [`RecordConfig::warn_free_bytes`] (once per take).
    LowDisk {
        free_bytes: u64,
    },
}

/// Recording errors.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RecordError {
    #[error("no microphone found")]
    NoDevice,
    #[error("the microphone {0:?} isn't connected")]
    DeviceNotFound(String),
    #[error("the microphone is in use by another app: {0}")]
    Busy(String),
    #[error("FreeCut isn't allowed to use the microphone: {0}")]
    Denied(String),
    #[error("the microphone failed: {0}")]
    Device(String),
    #[error("the microphone's sample format isn't supported: {0}")]
    UnsupportedFormat(String),
    #[error("the microphone's sample rate ({0} Hz) isn't supported")]
    UnsupportedRate(u32),
    #[error("the recorder can't do that while {0:?}")]
    WrongState(RecorderState),
    #[error("nothing is being recorded")]
    NotRecording,
    #[error("the take was cancelled before its start position")]
    NothingRecorded,
    #[error("can't write {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: Arc<std::io::Error>,
    },
    #[error("the recorder didn't finish the take in time")]
    Timeout,
    #[error("can't start a recording thread: {0}")]
    Thread(Arc<std::io::Error>),
}

/// Linear input gain for a level, clamped to the input range.
fn input_gain(gain: GainDb) -> f32 {
    let (lo, hi) = INPUT_GAIN_RANGE;
    gain_amplitude(GainDb::from_hundredths(gain.hundredths().clamp(lo, hi)))
}

/// Frames in `d` at `rate`, rounded down.
fn frames_in(d: Duration, rate: SampleRate) -> u64 {
    let frames = d.as_nanos() * u128::from(rate.hz()) / 1_000_000_000;
    u64::try_from(frames).unwrap_or(u64::MAX)
}

/// Records takes from one capture source. See the module docs.
pub struct Recorder {
    shared: Arc<Shared>,
    source: Box<dyn CaptureSource>,
    config: RecordConfig,
    rate: SampleRate,
    commands: Sender<Command>,
    results: Receiver<TakeResult>,
    events: Receiver<RecorderEvent>,
    writer: Option<JoinHandle<()>>,
    monitor: Option<MonitorFeed>,
    /// The last take whose result stop or cancel returned (take numbers
    /// start at 1).
    returned: Option<u32>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder")
            .field("state", &self.state())
            .field("rate", &self.rate)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Recorder {
    /// Starts `source` and the writer thread. The recorder is then Ready:
    /// the meter is live and nothing is written until [`Recorder::arm`] or
    /// [`Recorder::start`].
    pub fn open(
        config: RecordConfig,
        mut source: Box<dyn CaptureSource>,
    ) -> Result<Recorder, RecordError> {
        let format = source.format();
        let rate =
            SampleRate::new(format.rate).map_err(|_| RecordError::UnsupportedRate(format.rate))?;
        if format.channels == 0 {
            return Err(RecordError::UnsupportedFormat("no channels".to_owned()));
        }
        let channels = u8::try_from(config.channels.count()).unwrap_or(1);
        let shared = Arc::new(Shared::new(
            channels,
            input_gain(config.gain),
            config.monitor,
        ));
        // Room for 2 s of stereo, whichever channel count a take uses.
        let ring_len = usize::try_from(format.rate * RING_SECS * 2).unwrap_or(1 << 20);
        let (producer, consumer) = RingBuffer::new(ring_len);
        let (tap, feed) = monitor::channel(format.rate);
        let (commands, command_rx) = crossbeam_channel::unbounded();
        let (result_tx, results) = crossbeam_channel::unbounded();
        let (event_tx, events) = crossbeam_channel::unbounded();
        let writer = Writer::new(
            Arc::clone(&shared),
            consumer,
            command_rx,
            result_tx,
            event_tx,
        );
        let handle = std::thread::Builder::new()
            .name("freecut-record-writer".to_owned())
            .spawn(move || writer.run())
            .map_err(|e| RecordError::Thread(Arc::new(e)))?;
        let sink = source::CaptureSink::new(Arc::clone(&shared), producer, tap, format);
        if let Err(e) = source.start(sink) {
            let _ = commands.send(Command::Shutdown);
            let _ = handle.join();
            return Err(e);
        }
        Ok(Recorder {
            shared,
            source,
            config,
            rate,
            commands,
            results,
            events,
            writer: Some(handle),
            monitor: Some(feed),
            // Take 0 is "no take yet".
            returned: Some(0),
        })
    }

    pub fn state(&self) -> RecorderState {
        self.shared.state()
    }

    /// The level meter (atomics; cheap from the UI thread).
    pub fn meter(&self) -> MeterReading {
        self.shared.meter.read()
    }

    /// Clears the clip light (also cleared when a take starts).
    pub fn reset_clip(&self) {
        self.shared.meter.reset_clip();
    }

    /// The recorder's events. There is one queue: use one receiver.
    pub fn events(&self) -> Receiver<RecorderEvent> {
        self.events.clone()
    }

    /// The source's sample rate, which every take has.
    pub fn rate(&self) -> SampleRate {
        self.rate
    }

    pub fn config(&self) -> &RecordConfig {
        &self.config
    }

    /// Frames captured in the current take so far, pre-roll included.
    pub fn captured_frames(&self) -> u64 {
        self.shared.captured.load(Ordering::Relaxed)
    }

    /// What is left of the countdown, while counting down.
    pub fn countdown_left(&self) -> Option<Duration> {
        if self.shared.phase() != COUNTING {
            return None;
        }
        let frames = self.shared.countdown_left.load(Ordering::Relaxed);
        let nanos = u128::from(frames) * 1_000_000_000 / u128::from(self.rate.hz());
        Some(Duration::from_nanos(
            u64::try_from(nanos).unwrap_or(u64::MAX),
        ))
    }

    /// The input latency the source reports.
    pub fn input_latency(&self) -> Duration {
        self.source.latency()
    }

    /// Changes the input level at once (smoothed over 10 ms).
    pub fn set_gain(&mut self, gain: GainDb) {
        self.config.gain = gain;
        self.shared
            .gain
            .store(input_gain(gain).to_bits(), Ordering::Relaxed);
    }

    /// Turns "Hear yourself" on or off at once.
    pub fn set_monitor(&mut self, on: bool) {
        self.config.monitor = on;
        self.shared.monitor.store(on, Ordering::Relaxed);
    }

    /// The monitoring feed for the output engine (`AudioEngine::set_monitor`).
    /// There is one; later calls return `None`.
    pub fn monitor_feed(&mut self) -> Option<MonitorFeed> {
        self.monitor.take()
    }

    /// Replaces the configuration for the next take (its path, channels,
    /// countdown, pre-roll, punch end…). Only while Ready.
    pub fn update(&mut self, config: RecordConfig) -> Result<(), RecordError> {
        self.require(READY)?;
        let channels = u8::try_from(config.channels.count()).unwrap_or(1);
        self.shared.channels.store(channels, Ordering::SeqCst);
        self.set_gain(config.gain);
        self.set_monitor(config.monitor);
        self.config = config;
        Ok(())
    }

    fn require(&self, phase: u8) -> Result<(), RecordError> {
        if self.shared.phase() == phase {
            Ok(())
        } else {
            Err(RecordError::WrongState(self.state()))
        }
    }

    /// The phase a take starts in once the countdown is over.
    fn first_phase(&self) -> u8 {
        if self.config.preroll_frames > 0 {
            PREROLL
        } else {
            RECORDING
        }
    }

    /// Hands the next take to the writer and the callback (before the
    /// phase lets the callback start it).
    fn begin_take(&mut self, countdown_frames: u64) -> Result<(), RecordError> {
        let shared = &self.shared;
        let take = shared.take.load(Ordering::SeqCst).wrapping_add(1);
        let order = TakeOrder {
            take,
            path: self.config.path.clone(),
            rate: self.rate,
            channels: self.config.channels,
            preroll_frames: self.config.preroll_frames,
            dither: self.config.dither,
            header_refresh: self.config.header_refresh,
            rf64_threshold: self.config.rf64_threshold,
            min_free_bytes: self.config.min_free_bytes,
            warn_free_bytes: self.config.warn_free_bytes,
            free_space: self.config.free_space.clone(),
        };
        self.commands
            .send(Command::Take(order))
            .map_err(|_| RecordError::Thread(Arc::new(std::io::Error::other("writer stopped"))))?;
        shared.reset_take();
        shared
            .countdown_frames
            .store(countdown_frames, Ordering::SeqCst);
        shared
            .countdown_left
            .store(countdown_frames, Ordering::SeqCst);
        shared
            .preroll_frames
            .store(self.config.preroll_frames, Ordering::SeqCst);
        shared.stop_after.store(
            self.config.stop_after_frames.unwrap_or(NO_END),
            Ordering::SeqCst,
        );
        let channels = u8::try_from(self.config.channels.count()).unwrap_or(1);
        shared.channels.store(channels, Ordering::SeqCst);
        shared.meter.reset_clip();
        shared.take.store(take, Ordering::SeqCst);
        Ok(())
    }

    /// Starts a take after the countdown ([`RecordConfig::countdown`],
    /// counted in captured frames). Only while Ready.
    pub fn arm(&mut self) -> Result<(), RecordError> {
        self.require(READY)?;
        let countdown = frames_in(self.config.countdown, self.rate);
        self.begin_take(countdown)?;
        let to = if countdown > 0 {
            COUNTING
        } else {
            self.first_phase()
        };
        self.shared
            .cas_phase(READY, to)
            .map_err(|_| RecordError::WrongState(self.state()))
    }

    /// Starts a take now: from Ready without a countdown, or cuts a running
    /// countdown short.
    pub fn start(&mut self) -> Result<(), RecordError> {
        let first = self.first_phase();
        match self.shared.phase() {
            READY => {
                self.begin_take(0)?;
                self.shared
                    .cas_phase(READY, first)
                    .map_err(|_| RecordError::WrongState(self.state()))
            }
            COUNTING => match self.shared.cas_phase(COUNTING, first) {
                // The countdown may have just run out by itself.
                Ok(()) | Err(PREROLL | RECORDING) => Ok(()),
                Err(_) => Err(RecordError::WrongState(self.state())),
            },
            _ => Err(RecordError::WrongState(self.state())),
        }
    }

    /// Asks the running take to stop without waiting; the take arrives as
    /// [`RecorderEvent::Finished`] (and from a later [`Recorder::stop`]). A
    /// countdown is cancelled. Returns `false` when nothing was running.
    pub fn request_stop(&self) -> bool {
        self.shared.request_end(REQ_STOP) != EndRequest::Idle
    }

    /// Stops the take and waits for its file to be finished. A countdown
    /// is cancelled ([`RecordError::NothingRecorded`]).
    ///
    /// If the take already ended by itself (punch range, lost device, full
    /// disk…), returns that take.
    pub fn stop(&mut self) -> Result<TakeInfo, RecordError> {
        let take = self.shared.take.load(Ordering::SeqCst);
        match self.shared.request_end(REQ_STOP) {
            EndRequest::CountdownCancelled => Err(RecordError::NothingRecorded),
            EndRequest::Stopping | EndRequest::Idle => match self.wait_result(take)? {
                Some(info) => Ok(info),
                None => Err(RecordError::NothingRecorded),
            },
        }
    }

    /// Cancels: a countdown or a pre-roll ends with nothing kept
    /// (`Ok(None)`); a take that is recording is finished and kept, marked
    /// [`EndReason::Cancelled`] (`Ok(Some(..))`), for the Takes list. In
    /// any other state there is nothing to cancel (`Ok(None)`).
    pub fn cancel(&mut self) -> Result<Option<TakeInfo>, RecordError> {
        let take = self.shared.take.load(Ordering::SeqCst);
        match self.shared.request_end(REQ_CANCEL) {
            EndRequest::CountdownCancelled => Ok(None),
            EndRequest::Stopping | EndRequest::Idle => match self.wait_result(take) {
                Err(RecordError::NotRecording) => Ok(None),
                other => other,
            },
        }
    }

    /// The writer's result for take `take`, waiting while it is being
    /// finished. [`RecordError::NotRecording`] when that take never ran or
    /// was already returned.
    fn wait_result(&mut self, take: u32) -> Result<Option<TakeInfo>, RecordError> {
        let deadline = Instant::now() + FINISH_TIMEOUT;
        loop {
            while let Ok((id, result)) = self.results.try_recv() {
                if id == take {
                    self.returned = Some(id);
                    return result;
                }
            }
            let phase = self.shared.phase();
            let idle = !matches!(phase, PREROLL | RECORDING) && phase != shared::STOPPING;
            if idle {
                let finished = self.shared.finished.load(Ordering::SeqCst);
                if finished != take || self.returned == Some(take) {
                    return Err(RecordError::NotRecording);
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(RecordError::Timeout);
            }
            match self.results.recv_timeout((deadline - now).min(POLL_RESULT)) {
                Ok((id, result)) if id == take => {
                    self.returned = Some(id);
                    return result;
                }
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(RecordError::Timeout),
            }
        }
    }
}

/// How often a waiting stop looks at the phase.
const POLL_RESULT: Duration = Duration::from_millis(5);

impl Drop for Recorder {
    /// Stops the source; a take in progress is cancelled (kept if it was
    /// past its pre-roll), and the writer finishes it before the thread
    /// ends.
    fn drop(&mut self) {
        self.source.stop();
        self.shared.request_end(REQ_CANCEL);
        let _ = self.commands.send(Command::Shutdown);
        if let Some(handle) = self.writer.take() {
            let _ = handle.join();
        }
    }
}
