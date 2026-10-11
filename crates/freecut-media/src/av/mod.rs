//! The FFI boundary: everything ffmpeg-next 9 does not wrap safely.
//!
//! This is the only module in the crate where `unsafe` is allowed. Each
//! function wraps one raw FFmpeg operation behind a safe signature, and every
//! `unsafe` block carries a `// SAFETY:` comment stating the obligations it
//! relies on. The rest of the crate uses these wrappers and ffmpeg-next's
//! safe API only.
//!
//! What lives here and why:
//! - [`codec`]: stream kinds and codec ids read as raw integers, decoder
//!   lookup by raw id and decoder opening (README §5.7: ffmpeg-next's
//!   `Parameters::id()`, `medium()` and `Decoder::video()`/`audio()` read
//!   enum-typed fields as Rust enums, which a newer libav minor could make
//!   undefined behaviour), plus `av_guess_frame_rate`.
//! - [`detect`]: `av_probe_input_format3` on a file's first bytes.
//! - [`samples`]: decoded audio frames as typed byte slices, with the sample
//!   format read as an integer.
//! - [`stream`]: per-stream `discard`, per-stream `avformat_seek_file`
//!   (ffmpeg-next's `Input::seek` always passes stream `-1`, i.e. µs units
//!   against the default stream), the stream display matrix from
//!   `codecpar->coded_side_data` (read with a raw integer side-data type), and
//!   `duration_estimation_method`.
//! - [`sws`]: frame → RGBA through FFmpeg 8's dynamic `sws_scale_frame`, with
//!   the CL-002 colour defaults applied to the source frame (ffmpeg-next's
//!   legacy scaler defaults to BT.601 and ignores errors), and RGBA →
//!   BT.709 limited-range Y′CbCr for the encoders.
//! - [`layout`]: a stream's channel layout (speakers as raw integers, and
//!   its name), and FFmpeg's default layout for a channel count.
//! - [`swr`]: planar `f32` → interleaved `f32` remix and resample, with
//!   FreeCut's stereo matrix (AU-010).
//! - [`pixfmt`]: alpha and bit depth from the pixel-format descriptor, by
//!   raw format integer.
//! - [`encode`]: strict encoder opening (unused options reported), chroma
//!   siting, writable frames, the checked final close of an output file, and
//!   codec parameters read back for `verify_output`.
#![allow(unsafe_code)]

pub(crate) mod codec;
pub(crate) mod detect;
pub(crate) mod encode;
pub(crate) mod layout;
pub(crate) mod pixfmt;
pub(crate) mod samples;
pub(crate) mod stream;
pub(crate) mod swr;
pub(crate) mod sws;

use crate::error::DecodeError;

/// Wraps an FFmpeg error as [`DecodeError::Ffmpeg`] for operation `op`.
pub(crate) fn ff_error(op: &'static str, e: ffmpeg_next::Error) -> DecodeError {
    DecodeError::Ffmpeg {
        op,
        code: i32::from(e),
        msg: e.to_string(),
    }
}
