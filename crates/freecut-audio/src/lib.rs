//! Audio mixing and playback for FreeCut (AU-010).
//!
//! - [`Mixer`]: renders the timeline's audio as interleaved stereo `f32` at
//!   one sample rate, block by block. Playback and export share it. It
//!   applies clip volume or volume keyframes, fades with their curves, clip
//!   pan, the stream-mix factor and the track stage (AU-005, AU-006,
//!   AU-008, AU-013, AU-014, AU-038), sums every input on a stereo bus,
//!   then applies the master gain and the −1 dBFS safety limiter.
//! - [`Schedule`], [`SourceOpener`], [`SourceReader`]: what the mixer plays,
//!   as plain values, so the mixer itself doesn't depend on the shape of the
//!   edit model.
//! - [`DocSchedule`]: a document's sequence as a [`Schedule`], read through
//!   TL's mix view (`freecut_timeline::mix`): keyframed volume, fade curves,
//!   the track stage and the clip's stream selection ("All streams mixed"
//!   as one input per stream), with track solo as a playback parameter
//!   (AU-012).
//! - [`OfflineMix`]: the sound of one export range, exactly
//!   `ExportClock::sample_count` frames, through the same mixer without a
//!   device. It ignores solo unless asked to export as heard.
//! - `FileOpener` (feature `media`): sources decoded with `freecut-media`,
//!   built from the app's resolved media paths with `FileOpener::from_paths`.
//! - `AudioEngine` (feature `output`): real-time playback on the chosen
//!   output device or the system default, with fallback and a status the
//!   app shows (AU-010), and the audio clock the player syncs video to.
//!   `list_output_devices` lists the devices for Preferences (UI-029).
//! - [`record`]: voiceover recording (AU-027): a [`record::Recorder`]
//!   writes takes from a capture source (a microphone through
//!   `record::CpalInput`, feature `record`, or [`record::SyntheticInput`])
//!   to 24-bit WAV/RF64 files with FreeCut's own crash-tolerant writer,
//!   with level meters and monitoring.
//!
//! **Time.** Positions are [`Tick`](freecut_timeline::time::Tick)s; sample
//! indices come from them exactly ([`sample_at`], [`sample_start`]). Floats
//! are used only for sample values and gains.
//!
//! **Features.** `media` (default) links FFmpeg through `freecut-media`;
//! `output` and `record` (default) link `cpal` for playback and for
//! microphones. Without them the mixer and the recorder (with synthetic
//! sources) still build and test.
#![forbid(unsafe_code)]

mod document;
mod envelope;
mod gain;
mod limiter;
mod mixer;
mod offline;
mod playback;
mod prefetch;
pub mod record;
mod samples;
mod source;

#[cfg(feature = "media")]
mod file;
#[cfg(feature = "output")]
mod output;
#[cfg(test)]
mod testing;

pub use document::{DocSchedule, stream_input_key};
pub use envelope::{GainEnvelope, GainKey, GainKeys, KeyInterp};
pub use gain::{
    Fade, FadeCurve, balance_gains, db_to_amplitude, effective_fades, gain_amplitude,
    stream_mix_factor,
};
pub use mixer::Mixer;
pub use offline::{DEFAULT_BLOCK_FRAMES, OfflineMix, OfflineMixError};
pub use samples::{sample_at, sample_start};
pub use source::{
    MixInput, ReadError, Schedule, Silence, SourceKey, SourceOpener, SourceReader, TrackStage,
};

#[cfg(feature = "media")]
pub use file::FileOpener;
#[cfg(feature = "output")]
pub use output::{AudioEngine, list_output_devices};
#[cfg(feature = "output")]
pub use playback::{DeviceIssue, EngineConfig, EngineStatus, OutputDeviceInfo, OutputError};
