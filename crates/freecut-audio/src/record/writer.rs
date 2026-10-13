//! The writer thread: drains the capture ring into the take's WAV file in
//! blocks of up to 4 096 frames, with TPDF dither to 24 bits, checks free
//! disk space every 4 096 frames (and after the first block), refreshes the header every second, and
//! finishes the take when the phase says Stopping.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use freecut_timeline::time::SampleRate;
use rtrb::Consumer;

use super::shared::{
    END_DEVICE_LOST, END_DISK_LOW, END_NONE, END_OVERRUN, END_RANGE, END_RATE_CHANGED,
    END_WRITE_FAILED, PREROLL, READY, RECORDING, REQ_CANCEL, STOPPING, Shared, WAITING,
};
use super::wav::{self, WavSpec};
use super::{Channels, EndReason, RecordError, RecorderEvent, RecorderState, SpaceProbe, TakeInfo};

/// Frames written per block.
pub(crate) const BLOCK_FRAMES: usize = 4_096;
/// How often the thread looks at the ring and the phase.
const POLL: Duration = Duration::from_millis(5);
/// The longest wait for the capture callback to leave a block.
const QUIESCE_LIMIT: Duration = Duration::from_millis(500);

/// Everything the writer needs for one take (sent by arm and start).
#[derive(Debug, Clone)]
pub(crate) struct TakeOrder {
    pub take: u32,
    pub path: PathBuf,
    pub rate: SampleRate,
    pub channels: Channels,
    pub preroll_frames: u64,
    pub dither: bool,
    pub header_refresh: Duration,
    pub rf64_threshold: u64,
    pub min_free_bytes: u64,
    pub warn_free_bytes: u64,
    pub free_space: SpaceProbe,
}

#[derive(Debug)]
pub(crate) enum Command {
    Take(TakeOrder),
    Shutdown,
}

/// A finished take: its number and what the caller's stop or cancel gets.
pub(crate) type TakeResult = (u32, Result<Option<TakeInfo>, RecordError>);

/// TPDF dither: the sum of two uniform values in [−0.5, 0.5) LSB.
#[derive(Debug)]
struct Dither(u32);

impl Dither {
    fn uniform(&mut self) -> f32 {
        // xorshift32
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    }

    fn next(&mut self) -> f32 {
        self.uniform() + self.uniform()
    }
}

/// `f32` in [−1, 1] to a 24-bit value, with optional dither.
fn to_24(sample: f32, dither: Option<&mut Dither>) -> i32 {
    const SCALE: f32 = (1 << 23) as f32;
    let noise = dither.map_or(0.0, Dither::next);
    let v = (sample * SCALE + noise).round();
    if v.is_nan() {
        return 0;
    }
    v.clamp(-SCALE, SCALE - 1.0) as i32
}

struct OpenTake {
    wav: wav::Writer,
    refreshed: Instant,
    warned_low: bool,
    /// Frames written since free space was last checked.
    unchecked: usize,
}

pub(crate) struct Writer {
    shared: Arc<Shared>,
    ring: Consumer<f32>,
    commands: Receiver<Command>,
    results: Sender<TakeResult>,
    events: Sender<RecorderEvent>,
    order: Option<TakeOrder>,
    open: Option<OpenTake>,
    failure: Option<RecordError>,
    reported: RecorderState,
    dither: Dither,
    ints: Vec<i32>,
}

impl Writer {
    pub(crate) fn new(
        shared: Arc<Shared>,
        ring: Consumer<f32>,
        commands: Receiver<Command>,
        results: Sender<TakeResult>,
        events: Sender<RecorderEvent>,
    ) -> Writer {
        let reported = shared.state();
        Writer {
            shared,
            ring,
            commands,
            results,
            events,
            order: None,
            open: None,
            failure: None,
            reported,
            dither: Dither(0x9E37_79B9),
            ints: Vec::with_capacity(BLOCK_FRAMES * 2),
        }
    }

    pub(crate) fn run(mut self) {
        let mut shutting = false;
        loop {
            match self.commands.recv_timeout(POLL) {
                Ok(command) => shutting |= self.command(command),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => shutting = true,
            }
            while let Ok(command) = self.commands.try_recv() {
                shutting |= self.command(command);
            }
            let phase = self.shared.phase();
            match phase {
                PREROLL | RECORDING => {
                    if shutting {
                        // The recorder is going away mid-take: keep it as
                        // a cancelled take.
                        self.shared.request_end(REQ_CANCEL);
                    } else {
                        self.ensure_open();
                        self.drain();
                        self.refresh_if_due();
                    }
                }
                STOPPING => self.finish(),
                _ => self.skip_ring(),
            }
            self.report();
            if shutting && !matches!(self.shared.phase(), PREROLL | RECORDING | STOPPING) {
                return;
            }
        }
    }

    /// Handles a command; `true` for shutdown.
    fn command(&mut self, command: Command) -> bool {
        match command {
            Command::Take(order) => {
                self.order = Some(order);
                false
            }
            Command::Shutdown => true,
        }
    }

    fn report(&mut self) {
        let state = self.shared.state();
        if state != self.reported {
            self.reported = state;
            let _ = self.events.send(RecorderEvent::State(state));
        }
    }

    /// The order for the take that is running, if it arrived.
    fn current_order(&mut self) -> Option<&TakeOrder> {
        let take = self.shared.take.load(Ordering::SeqCst);
        if self.order.as_ref().is_none_or(|o| o.take != take) {
            // Commands sent before the phase changed are visible now.
            while let Ok(command) = self.commands.try_recv() {
                self.command(command);
            }
        }
        self.order.as_ref().filter(|o| o.take == take)
    }

    fn ensure_open(&mut self) {
        if self.open.is_some() || self.failure.is_some() {
            return;
        }
        let Some(order) = self.current_order() else {
            // Arm and start always send the order first.
            tracing::error!("recording started without a take order");
            return;
        };
        let spec = WavSpec {
            rate: order.rate.hz(),
            channels: order.channels.count(),
        };
        let path = order.path.clone();
        let threshold = order.rf64_threshold;
        match wav::Writer::create(&path, spec) {
            Ok(w) => {
                self.open = Some(OpenTake {
                    wav: w.with_rf64_threshold(threshold),
                    refreshed: Instant::now(),
                    warned_low: false,
                    unchecked: BLOCK_FRAMES,
                });
            }
            Err(e) => self.fail_take(&path, e),
        }
    }

    /// The take can't be written: it ends, and stop or cancel reports the
    /// error. What reached the file stays (with its marker) for recovery.
    fn fail_take(&mut self, path: &std::path::Path, e: io::Error) {
        tracing::warn!(path = %path.display(), "recording failed: {e}");
        let err = RecordError::Io {
            path: path.to_owned(),
            source: Arc::new(e),
        };
        let _ = self.events.send(RecorderEvent::Failed(err.clone()));
        self.failure = Some(err);
        self.shared.end_take(END_WRITE_FAILED);
    }

    /// Writes everything in the ring, block by block.
    fn drain(&mut self) {
        let Some(order) = self.current_order() else {
            self.skip_ring();
            return;
        };
        let channels = usize::from(order.channels.count());
        let dither = order.dither;
        let (min_free, warn_free) = (order.min_free_bytes, order.warn_free_bytes);
        let probe = order.free_space.clone();
        let path = order.path.clone();
        loop {
            let whole = self.ring.slots() / channels * channels;
            let n = whole.min(BLOCK_FRAMES * channels);
            if n == 0 {
                return;
            }
            let Ok(chunk) = self.ring.read_chunk(n) else {
                return;
            };
            self.ints.clear();
            let (a, b) = chunk.as_slices();
            for &s in a.iter().chain(b) {
                let d = dither.then_some(&mut self.dither);
                self.ints.push(to_24(s, d));
            }
            chunk.commit_all();
            let Some(open) = self.open.as_mut() else {
                // The file couldn't be created: the take's audio is lost.
                continue;
            };
            if let Err(e) = open.wav.write_samples(&self.ints) {
                self.open = None;
                self.fail_take(&path, e);
                continue;
            }
            open.unchecked += n / channels;
            if open.unchecked < BLOCK_FRAMES {
                continue;
            }
            open.unchecked = 0;
            if let Some(free) = probe.free(&path) {
                if free < min_free {
                    self.shared.end_take(END_DISK_LOW);
                } else if free < warn_free && !open.warned_low {
                    open.warned_low = true;
                    let _ = self
                        .events
                        .send(RecorderEvent::LowDisk { free_bytes: free });
                }
            }
        }
    }

    fn refresh_if_due(&mut self) {
        let Some(every) = self.order.as_ref().map(|o| o.header_refresh) else {
            return;
        };
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if open.refreshed.elapsed() < every {
            return;
        }
        open.refreshed = Instant::now();
        if let Err(e) = open.wav.refresh_header() {
            let path = open.wav.path().to_owned();
            self.open = None;
            self.fail_take(&path, e);
        }
    }

    /// Drops ring contents that belong to no take.
    fn skip_ring(&mut self) {
        let n = self.ring.slots();
        if n > 0
            && let Ok(chunk) = self.ring.read_chunk(n)
        {
            chunk.commit_all();
        }
    }

    /// Completes the take once the callback has stopped pushing to it.
    fn finish(&mut self) {
        self.shared.quiesce(QUIESCE_LIMIT);
        let take = self.shared.take.load(Ordering::SeqCst);
        let end = self.shared.end.load(Ordering::SeqCst);
        let request = self.shared.request.load(Ordering::SeqCst);
        let captured = self.shared.captured.load(Ordering::SeqCst);
        let order = self.current_order().cloned();
        let result = match order {
            None => {
                self.skip_ring();
                Err(RecordError::NotRecording)
            }
            Some(order) => {
                // Cancelled before the start position: nothing is kept.
                let discard =
                    request == REQ_CANCEL && end == END_NONE && captured < order.preroll_frames;
                if !discard {
                    self.ensure_open();
                }
                self.drain();
                self.complete(&order, end, request, discard)
            }
        };
        if let Ok(Some(info)) = &result {
            let _ = self.events.send(RecorderEvent::Finished(info.clone()));
        }
        self.order = None;
        self.open = None;
        self.failure = None;
        // The recorder is ready again before stop or cancel returns: a
        // waiter that sees Ready and `finished == take` waits for the
        // result, which is sent right after.
        self.shared.finished.store(take, Ordering::SeqCst);
        self.shared.reset_take();
        let next = if self.shared.device_failed.load(Ordering::SeqCst) {
            WAITING
        } else {
            READY
        };
        self.shared.set_phase(next);
        let _ = self.results.send((take, result));
    }

    fn complete(
        &mut self,
        order: &TakeOrder,
        end: u8,
        request: u8,
        discard: bool,
    ) -> Result<Option<TakeInfo>, RecordError> {
        if let Some(err) = self.failure.take() {
            if let Some(mut open) = self.open.take() {
                let _ = open.wav.refresh_header();
            }
            return Err(err);
        }
        let Some(open) = self.open.take() else {
            return if discard {
                Ok(None)
            } else {
                Err(RecordError::NotRecording)
            };
        };
        let io_err = |e| RecordError::Io {
            path: order.path.clone(),
            source: Arc::new(e),
        };
        if discard {
            open.wav.discard().map_err(io_err)?;
            return Ok(None);
        }
        let summary = open.wav.finish().map_err(io_err)?;
        let ended_by = match end {
            END_RANGE => EndReason::RangeEnd,
            END_OVERRUN => EndReason::Overrun,
            END_DEVICE_LOST => EndReason::DeviceLost,
            END_RATE_CHANGED => EndReason::RateChanged,
            END_DISK_LOW => EndReason::DiskLow,
            _ if request == REQ_CANCEL => EndReason::Cancelled,
            _ => EndReason::User,
        };
        Ok(Some(TakeInfo {
            path: order.path.clone(),
            frames: summary.frames,
            rate: order.rate,
            channels: order.channels,
            preroll_frames: order.preroll_frames.min(summary.frames),
            input_latency: self.shared.latency(),
            started_at: self.shared.first_frame(Instant::now()),
            ended_by,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_round_to_24_bits_and_clamp() {
        assert_eq!(to_24(0.0, None), 0);
        assert_eq!(to_24(1.0, None), (1 << 23) - 1);
        assert_eq!(to_24(-1.0, None), -(1 << 23));
        assert_eq!(to_24(2.0, None), (1 << 23) - 1);
        assert_eq!(to_24(f32::NAN, None), 0);
        assert_eq!(to_24(1234.0 / 8_388_608.0, None), 1234);
    }

    #[test]
    fn dither_is_triangular_within_one_lsb() {
        let mut d = Dither(1);
        let mut sum = 0.0f64;
        for _ in 0..100_000 {
            let v = d.next();
            assert!((-1.0..1.0).contains(&v));
            sum += f64::from(v);
        }
        assert!((sum / 100_000.0).abs() < 0.01);
        // A value on the grid lands within one step of itself.
        for _ in 0..1_000 {
            let v = to_24(1234.0 / 8_388_608.0, Some(&mut d));
            assert!((1_233..=1_235).contains(&v), "{v}");
        }
    }
}
