//! Encoding exports: H.264 + AAC in MP4 (EX-029), the MVP's only format.
//!
//! - [`EncodeSpec`] says what to write: size, frame rate, the EX-027 quality
//!   tier (or an explicit bitrate), optional AAC audio (EX-031) and the
//!   title tag. It and the bitrate formula ([`recommended_kbps`],
//!   [`tier_kbps`]) are pure.
//! - [`EncodeSession`] owns the encoders and the muxer of one export. RGBA
//!   frames and interleaved `f32` audio go in; a fast-start MP4 with BT.709
//!   tags comes out. Unfinished sessions delete their file.
//! - [`verify_output`] reads the finished file back for the job's final
//!   check ([`OutputInfo::check`]).
//!
//! **Encoder settings** (EX-027, EX-029, EX-032), all passed to FFmpeg and
//! checked: an option an encoder or the muxer doesn't recognize is an error,
//! never silently dropped.
//! - Video: libx264, High profile, `yuv420p`, single-pass VBR at the tier's
//!   target with the peak capped at 1.5× and a 2× buffer, a closed GOP of
//!   2 s, preset `medium`, level and B-frames at x264's choice, up to 16
//!   threads. Tagged BT.709 primaries/transfer/matrix, limited range, left
//!   chroma siting; the MP4 gets a matching `colr` box.
//! - Audio: FFmpeg's native AAC-LC encoder, planar float, stereo, at the
//!   chosen bitrate; 1024-sample frames with one short last frame, so the
//!   stream ends at the pushed length.
//! - Container: MP4 (muxer named explicitly), `movflags=+faststart`.
//!   Metadata is technical only (EX-040): `creation_time` (export time),
//!   `title` when set, and FFmpeg's own `encoder` tag.
//!
//! **Time.** Frame `n` has pts `n` in the time base `1/frame_rate`; audio pts
//! count samples at the output rate. Both are integers from the start, never
//! floats (P3).

mod error;
mod spec;

#[cfg(feature = "ffmpeg")]
mod fifo;
#[cfg(feature = "ffmpeg")]
mod mp4;
#[cfg(feature = "ffmpeg")]
mod session;
#[cfg(feature = "ffmpeg")]
mod verify;

pub use error::EncodeError;
pub use spec::{
    AAC_KBPS_CHOICES, AudioSpec, DEFAULT_AAC_KBPS, EncodeSpec, MAX_DIMENSION, QualityTier,
    VideoQuality, gop_frames, recommended_kbps, tier_kbps, video_due_before_audio,
};

#[cfg(feature = "ffmpeg")]
pub use mp4::Nclx;
#[cfg(feature = "ffmpeg")]
pub use session::{EncodeSession, MuxStats};
#[cfg(feature = "ffmpeg")]
pub use verify::{
    AudioStreamInfo, ColorTags, OutputInfo, VideoStreamInfo, verify_output,
    verify_output_with_cancel,
};
