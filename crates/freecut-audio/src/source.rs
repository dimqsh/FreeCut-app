//! What the mixer plays, decoupled from the edit model.
//!
//! The mixer doesn't read the timeline itself. A [`Schedule`] answers "what
//! is audible in this window" as plain [`MixInput`]s, and a [`SourceOpener`]
//! turns a [`SourceKey`] into a [`SourceReader`] of stereo samples at the mix
//! rate. [`DocSchedule`](crate::DocSchedule) builds the schedule from a
//! sequence (TL-008's mix view) and `FileOpener` the readers from the media
//! library's files; tests use synthetic ones.

use freecut_timeline::time::{SampleRate, Tick, TimeRange};

use crate::envelope::GainEnvelope;
use crate::gain::{Fade, balance_gains, sanitize};

/// One audio stream of one asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceKey {
    /// The asset (media library entry) the stream belongs to: its
    /// `MediaId` value for library media.
    pub asset: u64,
    /// Index of the audio stream inside the asset's file.
    pub stream: u32,
}

/// One clip's contribution to the mix, already resolved to plain values.
///
/// Clip mute, track mute and disabled clips are the schedule's business: a
/// silent clip is simply not listed (or listed silent, so the mixer can ramp
/// to it). Build one with [`MixInput::new`] and set what differs from the
/// defaults.
///
/// Per sample the mixer applies, in this order of the AU signal chain: the
/// clip volume or its keyframes ([`GainEnvelope`]), the fades, the clip pan,
/// the stream-mix factor and the track stage's volume and pan. Changes to
/// everything but keyframed volume ramp over about 10 ms during playback.
#[derive(Debug, Clone, PartialEq)]
pub struct MixInput {
    /// Stable per clip (and stream) across edits; the mixer keeps one reader
    /// per key and reads it sequentially while the key stays scheduled.
    pub key: u64,
    /// The clip the input plays. The inputs of a clip playing "All streams
    /// mixed" (AU-038) share it, with distinct keys: while one of them
    /// waits for its reader, a change of level of the others waits too, so
    /// the clip's level doesn't dip when it switches to them during
    /// playback. [`MixInput::new`] sets it to `key`.
    pub clip: u64,
    /// Where the samples come from.
    pub source: SourceKey,
    /// The clip's placement on the timeline.
    pub range: TimeRange,
    /// Media time heard at `range.start()`.
    pub source_start: Tick,
    /// Clip volume (AU-005) or volume keyframes (AU-008). Keyframe times are
    /// media times, on the same axis as `source_start`.
    pub gain: GainEnvelope,
    /// Stored fade-in (AU-006); its length is clamped to the clip when
    /// evaluated.
    pub fade_in: Fade,
    /// Stored fade-out (AU-006); its length is clamped to the clip when
    /// evaluated.
    pub fade_out: Fade,
    /// Clip pan (AU-014) from −1 (left) to +1 (right), balance law
    /// ([`balance_gains`]).
    pub pan: f32,
    /// Gain of this stream in an "All streams mixed" sum (AU-038): 1 for a
    /// single stream, [`stream_mix_factor`](crate::stream_mix_factor)`(n)` for each of `n` streams.
    pub stream_mix: f32,
    /// The track stage the clip goes through (AU-013).
    pub track: TrackStage,
}

impl MixInput {
    /// An input of its own clip (`clip` is `key`) at unity gain, without
    /// fades, centred, on track 0 at unity.
    pub fn new(key: u64, source: SourceKey, range: TimeRange, source_start: Tick) -> MixInput {
        MixInput {
            key,
            clip: key,
            source,
            range,
            source_start,
            gain: GainEnvelope::default(),
            fade_in: Fade::NONE,
            fade_out: Fade::NONE,
            pan: 0.0,
            stream_mix: 1.0,
            track: TrackStage::default(),
        }
    }

    /// Whether nothing of the input can be heard: its volume, stream-mix
    /// factor or track volume is silent.
    pub fn is_silent(&self) -> bool {
        self.gain.is_silent() || self.static_volume() == 0.0
    }

    /// The part of the volume that is constant over the clip, as linear
    /// amplitude: the static clip volume (unity when keyframed), the
    /// stream-mix factor and the track volume. The mixer smooths it.
    pub(crate) fn static_volume(&self) -> f32 {
        let clip = match self.gain {
            GainEnvelope::Static(g) => sanitize(g),
            GainEnvelope::Keyframes(_) => 1.0,
        };
        clip * sanitize(self.stream_mix) * sanitize(self.track.gain)
    }

    /// Left and right gains of the clip pan and the track pan together.
    pub(crate) fn channel_gains(&self) -> [f32; 2] {
        let [cl, cr] = balance_gains(self.pan);
        let [tl, tr] = balance_gains(self.track.pan);
        [cl * tl, cr * tr]
    }
}

/// The track stage of a clip's track (AU-013): after all clip processing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackStage {
    /// The track, for per-track monitoring (solo, meters); not used in the
    /// sum itself.
    pub id: u64,
    /// Track volume as linear amplitude; negative and non-finite values
    /// count as 0.
    pub gain: f32,
    /// Track pan from −1 to +1, balance law.
    pub pan: f32,
}

impl Default for TrackStage {
    fn default() -> TrackStage {
        TrackStage {
            id: 0,
            gain: 1.0,
            pan: 0.0,
        }
    }
}

/// What is audible on the timeline.
pub trait Schedule: Send + Sync {
    /// Every input whose `range` overlaps `window`, as heard in preview.
    /// Extra inputs outside the window are harmless; missing ones are
    /// silent.
    fn inputs(&self, window: TimeRange) -> Vec<MixInput>;

    /// What an export renders of `window`: [`Schedule::inputs`] without
    /// monitoring-only state such as track solo (AU-012). The default is
    /// [`Schedule::inputs`], for schedules without such state.
    fn export_inputs(&self, window: TimeRange) -> Vec<MixInput> {
        self.inputs(window)
    }
}

/// Why a [`SourceReader`] couldn't deliver samples. The mixer plays silence
/// for that block and opens the source again after the next jump.
#[derive(Debug, thiserror::Error)]
#[error("reading the audio source failed")]
pub struct ReadError(#[source] Box<dyn std::error::Error + Send + Sync>);

impl ReadError {
    /// Wraps any error (or a message) as a read failure.
    pub fn new(cause: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> ReadError {
        ReadError(cause.into())
    }
}

/// Stereo samples of one source at the rate it was opened with.
pub trait SourceReader: Send {
    /// Fills `out` (interleaved stereo, `2 * frames` values) with the
    /// `frames` samples starting at the one covering media time `at`. Time
    /// before the source's first sample or after its last is silence.
    ///
    /// Successive calls usually continue where the last one ended; any other
    /// `at` is a seek.
    fn read(&mut self, at: Tick, frames: usize, out: &mut [f32]) -> Result<(), ReadError>;

    /// Gets ready for a [`read`](SourceReader::read) at `at`, so that read
    /// is sequential rather than a seek. Playback calls this on a worker
    /// thread right after opening the reader, before the mixer needs it.
    ///
    /// The default reads one frame at `at`: readers keep what they just
    /// read, so reading from there again continues where they are.
    fn prepare(&mut self, at: Tick) -> Result<(), ReadError> {
        let mut one = [0.0; 2];
        self.read(at, 1, &mut one)
    }
}

/// Opens readers for sources.
pub trait SourceOpener: Send + Sync {
    /// A reader for `source` delivering samples at `rate`, or `None` when the
    /// source is offline or unreadable (it then contributes silence).
    fn open(&self, source: &SourceKey, rate: SampleRate) -> Option<Box<dyn SourceReader>>;
}

/// Nothing is audible: the schedule of an empty timeline.
#[derive(Debug, Clone, Copy, Default)]
pub struct Silence;

impl Schedule for Silence {
    fn inputs(&self, _window: TimeRange) -> Vec<MixInput> {
        Vec::new()
    }
}

impl SourceOpener for Silence {
    fn open(&self, _source: &SourceKey, _rate: SampleRate) -> Option<Box<dyn SourceReader>> {
        None
    }
}
