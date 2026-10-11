//! Media I/O for FreeCut: the only crate that talks to FFmpeg.
//!
//! What it provides (milestone M2):
//! - [`init`]: one-time FFmpeg setup, a runtime version check and the
//!   capability table of the linked build (PM-036);
//! - [`probe`]: what a file contains, as the timeline's plain
//!   [`MediaInfo`](freecut_timeline::model::MediaInfo), plus its
//!   [`fingerprint`] (PM-035, PM-038);
//! - [`VideoDecoder`]: the frame displayed at any media time, as RGBA;
//! - [`AudioReader`]: sample-accurate interleaved `f32` at any rate;
//! - [`poster`]: a tile-sized, display-oriented thumbnail (PM-070);
//! - [`audio_peaks`]: min/max waveform buckets (PM-072);
//! - [`CancelToken`]: cooperative cancellation for all of the above;
//! - [`encode`]: H.264 + AAC MP4 export encoding and its read-back check
//!   (EX-029);
//! - [`color`]: HDR → SDR conversion (CL-002), applied by the decoders to
//!   PQ, HLG and BT.2020 frames (pure Rust, always built);
//! - [`lut`]: `.cube` and `.3dl` LUT parsing and sampling (CL-016, CL-017;
//!   pure Rust, always built).
//!
//! **Time.** Every time in the API is a [`Tick`](freecut_timeline::time::Tick)
//! of *media time*: 0 is the first presentable frame of the file's default
//! video stream (or its first audio sample when there is no video), and
//! every stream keeps its offset to that origin. Stream timestamps are
//! converted only with `freecut_timeline::time::ticks_from_stream`, and frames
//! are chosen by comparing ticks.
//!
//! **Threads.** Everything here blocks; call it from worker threads, never
//! the UI thread. A decoder or reader is used by one thread at a time.
//!
//! **Features.** Without the default `ffmpeg` feature only the FFmpeg-free
//! parts are built: [`CancelToken`], the error types, [`fingerprint`],
//! [`color`] and [`lut`].
//!
//! `unsafe` code lives only in the private `av` module (the FFI boundary),
//! with a `// SAFETY:` comment on every block.
#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod cancel;
pub mod color;
pub mod encode;
pub mod error;
mod fingerprint;
pub mod lut;

#[cfg(feature = "ffmpeg")]
mod audio;
#[cfg(feature = "ffmpeg")]
mod av;
#[cfg(feature = "ffmpeg")]
mod geometry;
#[cfg(feature = "ffmpeg")]
mod init;
#[cfg(feature = "ffmpeg")]
mod probe;
#[cfg(feature = "ffmpeg")]
mod source;
#[cfg(feature = "ffmpeg")]
mod thumb;
#[cfg(feature = "ffmpeg")]
mod time;
#[cfg(feature = "ffmpeg")]
mod video;

pub use cancel::{CancelReason, CancelToken};
pub use error::{DecodeError, LibVersion, MediaError, NotMediaKind, ProbeError, StreamKind};
pub use fingerprint::{FINGERPRINT_SPAN, fingerprint};

#[cfg(feature = "ffmpeg")]
pub use audio::{AudioReader, audio_peaks};
#[cfg(feature = "ffmpeg")]
pub use init::{Capabilities, CodecSupport, init};
#[cfg(feature = "ffmpeg")]
pub use probe::{PROBE_TIMEOUT, Probed, STILL_DURATION, probe, probe_with_cancel};
#[cfg(feature = "ffmpeg")]
pub use thumb::{RgbaImage, poster};
#[cfg(feature = "ffmpeg")]
pub use video::{SeekMode, VideoDecoder, VideoFrame};

#[cfg(all(test, feature = "ffmpeg"))]
mod tests {
    /// Readers move between worker threads (one user at a time, I5).
    #[test]
    fn readers_are_send() {
        fn send<T: Send>() {}
        send::<crate::VideoDecoder>();
        send::<crate::AudioReader>();
        send::<crate::CancelToken>();
    }
}
