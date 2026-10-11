//! Codec identity, decoder lookup and decoder opening without ffmpeg-next's
//! enum getters (README §5.7).
//!
//! `codecpar->codec_type` and `codecpar->codec_id` are read as raw integers,
//! and the libavcodec functions that take a codec id are declared here with
//! the enum's underlying integer type, so an id added by a newer libavcodec
//! minor is never materialized as an out-of-range Rust enum. Decoders are
//! opened here as well: ffmpeg-next's `Decoder::video()`/`audio()` read the
//! codec context's `codec_id` and `codec_type` as enums.

use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

use ffmpeg_next::codec::Context;
use ffmpeg_next::codec::decoder::{Decoder, Opened};
use ffmpeg_next::format::context::Input;
use ffmpeg_next::{Codec, ffi};

// SAFETY: these are libavcodec's own `avcodec_get_name` and
// `avcodec_find_decoder` (linked through ffmpeg-sys-next), declared with
// `unsigned int` in place of `enum AVCodecID`: the enum's values fit, and C
// passes an enum exactly like its integer type on every supported target.
// Both functions accept any id; unknown ids give "unknown_codec" and null.
unsafe extern "C" {
    #[link_name = "avcodec_get_name"]
    fn avcodec_get_name_raw(id: c_uint) -> *const c_char;
    #[link_name = "avcodec_find_decoder"]
    fn avcodec_find_decoder_raw(id: c_uint) -> *const ffi::AVCodec;
}

/// What a stream carries, from `codecpar->codec_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaKind {
    Video,
    Audio,
    /// Subtitles, data, attachments and kinds this build doesn't know.
    Other,
}

impl MediaKind {
    fn from_raw(raw: c_int) -> MediaKind {
        if raw == ffi::AVMediaType::AVMEDIA_TYPE_VIDEO as c_int {
            MediaKind::Video
        } else if raw == ffi::AVMediaType::AVMEDIA_TYPE_AUDIO as c_int {
            MediaKind::Audio
        } else {
            MediaKind::Other
        }
    }
}

/// A codec id as libavcodec stores it, possibly one these bindings' headers
/// don't list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CodecId(c_uint);

impl CodecId {
    /// `AV_CODEC_ID_NONE`.
    pub(crate) const NONE: CodecId = CodecId(ffi::AVCodecID::AV_CODEC_ID_NONE as c_uint);
    pub(crate) const VP8: CodecId = CodecId(ffi::AVCodecID::AV_CODEC_ID_VP8 as c_uint);
    pub(crate) const VP9: CodecId = CodecId(ffi::AVCodecID::AV_CODEC_ID_VP9 as c_uint);
    pub(crate) const VORBIS: CodecId = CodecId(ffi::AVCodecID::AV_CODEC_ID_VORBIS as c_uint);

    /// Wraps a raw `codec_id` value.
    pub(crate) const fn from_raw(raw: c_uint) -> CodecId {
        CodecId(raw)
    }

    /// The raw `codec_id` value.
    pub(crate) const fn raw(self) -> c_uint {
        self.0
    }

    /// libavcodec's name for the codec (`"h264"`, `"aac"`), or `None` when
    /// the running libavcodec doesn't know the id either.
    pub(crate) fn name(self) -> Option<String> {
        // SAFETY: `avcodec_get_name` accepts any id and returns a pointer to
        // a static NUL-terminated string (never null in FFmpeg 8).
        let name = unsafe { avcodec_get_name_raw(self.0) };
        if name.is_null() {
            return None;
        }
        // SAFETY: non-null, NUL-terminated and static (see above).
        let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
        (name != "unknown_codec").then(|| name.into_owned())
    }

    /// The decoder libavcodec picks for this id, if the build has one.
    pub(crate) fn decoder(self) -> Option<Codec> {
        // SAFETY: `avcodec_find_decoder` accepts any id and returns null or a
        // pointer to a static `AVCodec` that lives as long as the process.
        let codec = unsafe { avcodec_find_decoder_raw(self.0) };
        // SAFETY: `codec` is a non-null pointer to a static `AVCodec`.
        (!codec.is_null()).then(|| unsafe { Codec::wrap(codec) })
    }
}

/// The codec parameters of stream `stream`, when it exists.
fn codecpar(input: &Input, stream: usize) -> Option<*const ffi::AVCodecParameters> {
    // SAFETY: read-only access to the context owned by `input`, borrowed for
    // the duration; only streams below `nb_streams` are dereferenced.
    unsafe {
        let ctx = input.as_ptr();
        if stream >= (*ctx).nb_streams as usize {
            return None;
        }
        let st = *(*ctx).streams.add(stream);
        if st.is_null() || (*st).codecpar.is_null() {
            return None;
        }
        Some((*st).codecpar.cast_const())
    }
}

/// The kind of stream `stream` (Other when it doesn't exist).
pub(crate) fn stream_kind(input: &Input, stream: usize) -> MediaKind {
    let Some(par) = codecpar(input, stream) else {
        return MediaKind::Other;
    };
    // SAFETY: `par` points to the live codec parameters of a stream of
    // `input` (borrowed); the enum field is read as its `int` representation.
    let raw = unsafe { ptr::addr_of!((*par).codec_type).cast::<c_int>().read() };
    MediaKind::from_raw(raw)
}

/// The codec id of stream `stream` (`NONE` when it doesn't exist).
pub(crate) fn stream_codec(input: &Input, stream: usize) -> CodecId {
    let Some(par) = codecpar(input, stream) else {
        return CodecId::NONE;
    };
    // SAFETY: as in `stream_kind`; the enum field is read as its underlying
    // `unsigned int`.
    CodecId(unsafe { ptr::addr_of!((*par).codec_id).cast::<c_uint>().read() })
}

/// Opens `codec` for stream `stream` of `input`: codec parameters copied,
/// packet time base set to the stream's, `threads == 1` decoding on the
/// calling thread and anything else frame threads (`0` lets FFmpeg pick the
/// count).
pub(crate) fn open_decoder(
    input: &Input,
    stream: usize,
    codec: Codec,
    threads: usize,
) -> Result<Opened, ffmpeg_next::Error> {
    let invalid = ffmpeg_next::Error::StreamNotFound;
    let par = codecpar(input, stream).ok_or(invalid)?;
    let count = c_int::try_from(threads).unwrap_or(0);
    // SAFETY: `codec` points to a static `AVCodec`. The new context is owned
    // by the `Context` wrapper (no owner), which frees it on every path,
    // including the error returns. `par` is valid for the call (`input` is
    // borrowed); the time base is a plain `AVRational` of the same stream.
    unsafe {
        let raw = ffi::avcodec_alloc_context3(codec.as_ptr());
        if raw.is_null() {
            return Err(ffmpeg_next::Error::Other { errno: ffi::ENOMEM });
        }
        let mut context = Context::wrap(raw, None);
        let ret = ffi::avcodec_parameters_to_context(raw, par);
        if ret < 0 {
            return Err(ffmpeg_next::Error::from(ret));
        }
        let ctx = input.as_ptr();
        let st = *(*ctx).streams.add(stream);
        (*raw).pkt_timebase = (*st).time_base;
        (*raw).thread_count = count;
        (*raw).thread_type = if threads == 1 {
            0
        } else {
            ffi::FF_THREAD_FRAME
        };
        let ret = ffi::avcodec_open2(context.as_mut_ptr(), codec.as_ptr(), ptr::null_mut());
        if ret < 0 {
            return Err(ffmpeg_next::Error::from(ret));
        }
        Ok(Opened(Decoder(context)))
    }
}

/// `av_guess_frame_rate` for stream `stream` as `(num, den)` (`(0, 1)` when
/// the stream doesn't exist or the rate is unknown).
pub(crate) fn guess_frame_rate(input: &Input, stream: usize) -> (i32, i32) {
    // SAFETY: the context and stream belong to `input`, borrowed for the
    // call; the index is checked against `nb_streams`.
    // `av_guess_frame_rate` only reads them (its pointers are non-const in C
    // for historical reasons), and a null frame is allowed.
    let r = unsafe {
        let ctx = input.as_ptr();
        if stream >= (*ctx).nb_streams as usize {
            return (0, 1);
        }
        let st = *(*ctx).streams.add(stream);
        if st.is_null() {
            return (0, 1);
        }
        ffi::av_guess_frame_rate(ctx.cast_mut(), st, ptr::null_mut())
    };
    (r.num, r.den)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;

    #[test]
    fn raw_media_types_map_to_kinds() {
        assert_eq!(
            MediaKind::from_raw(ffi::AVMediaType::AVMEDIA_TYPE_VIDEO as c_int),
            MediaKind::Video
        );
        assert_eq!(
            MediaKind::from_raw(ffi::AVMediaType::AVMEDIA_TYPE_AUDIO as c_int),
            MediaKind::Audio
        );
        assert_eq!(
            MediaKind::from_raw(ffi::AVMediaType::AVMEDIA_TYPE_SUBTITLE as c_int),
            MediaKind::Other
        );
        // Values a newer libavutil might add are just "other".
        assert_eq!(MediaKind::from_raw(77), MediaKind::Other);
        assert_eq!(MediaKind::from_raw(-5), MediaKind::Other);
    }

    #[test]
    fn codec_ids_unknown_to_the_headers_are_harmless() {
        crate::init::ensure_init().unwrap();
        let h264 = CodecId(ffi::AVCodecID::AV_CODEC_ID_H264 as c_uint);
        assert_eq!(h264.name().as_deref(), Some("h264"));
        assert!(h264.decoder().is_some());
        // An id no libavcodec defines: no name, no decoder, no UB.
        let future = CodecId(0x7fff_fff0);
        assert_eq!(future.name(), None);
        assert!(future.decoder().is_none());
        assert_eq!(CodecId::NONE.name().as_deref(), Some("none"));
    }
}
