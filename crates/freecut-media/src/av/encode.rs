//! Encoder and muxer operations that ffmpeg-next 9 does not wrap, or wraps
//! lossily (export-pipeline §7.1).
//!
//! - [`open_codec_strict`]: `avcodec_open2` that hands back the option keys
//!   the codec did not consume. ffmpeg-next's `open_with` drops that
//!   dictionary, so a misspelled option would be ignored silently.
//! - [`set_codec_chroma_left`] / [`set_frame_chroma_left`]: the chroma
//!   siting field, which ffmpeg-next exposes read-only.
//! - [`make_writable`]: `av_frame_make_writable` before a pooled frame is
//!   refilled (P13): the encoder may still hold a reference to its buffer.
//! - [`close_output_checked`]: `avio_closep` with its result checked.
//!   ffmpeg-next's `Output` destructor calls `avio_close` and ignores the
//!   result, which would lose a write error at the final flush.
//! - [`stream_params`] and the `*_name` helpers: codec parameters read back
//!   as raw integers for `verify_output` (README §5.7: no enum getters), and
//!   FFmpeg's names for them.

use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

use ffmpeg_next::format::context::{Input, Output};
use ffmpeg_next::{Dictionary, codec, ffi, frame};

use super::codec::CodecId;

// SAFETY: libavutil's/libavcodec's own name lookups (linked through
// ffmpeg-sys-next), declared with the enums' underlying integer types in
// place of `enum AVPixelFormat`, `enum AVColorPrimaries`, … and
// `enum AVCodecID`, which C passes exactly like those integers. Every one
// accepts any value and returns null (or "unknown") for values it doesn't
// know; non-null results are static NUL-terminated strings.
unsafe extern "C" {
    #[link_name = "av_get_pix_fmt_name"]
    fn av_get_pix_fmt_name_raw(format: c_int) -> *const c_char;
    #[link_name = "av_color_primaries_name"]
    fn av_color_primaries_name_raw(value: c_int) -> *const c_char;
    #[link_name = "av_color_transfer_name"]
    fn av_color_transfer_name_raw(value: c_int) -> *const c_char;
    #[link_name = "av_color_space_name"]
    fn av_color_space_name_raw(value: c_int) -> *const c_char;
    #[link_name = "av_color_range_name"]
    fn av_color_range_name_raw(value: c_int) -> *const c_char;
    #[link_name = "av_chroma_location_name"]
    fn av_chroma_location_name_raw(value: c_int) -> *const c_char;
    #[link_name = "avcodec_profile_name"]
    fn avcodec_profile_name_raw(id: c_uint, profile: c_int) -> *const c_char;
}

/// Copies a static C string returned by one of the lookups above.
fn static_name(name: *const c_char) -> Option<String> {
    if name.is_null() {
        return None;
    }
    // SAFETY: non-null results of the lookups above are static,
    // NUL-terminated strings (see the extern block).
    let s = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    (s != "unknown").then(|| s.into_owned())
}

/// Opens the encoder `ctx` was allocated for with `options`, and returns the
/// keys of the options it did not recognize (empty when all were used).
pub(crate) fn open_codec_strict(
    ctx: &mut codec::Context,
    options: Dictionary<'_>,
) -> Result<Vec<String>, ffmpeg_next::Error> {
    // SAFETY: `disown` hands over the dictionary pointer (possibly null),
    // which is owned again by `Dictionary::own` right after the call, on
    // every path. `ctx` is an exclusively borrowed, allocated codec context
    // whose `codec` field was set at allocation (`new_with_codec`), so a
    // null codec argument is allowed.
    let (ret, left) = unsafe {
        let mut raw = options.disown();
        let ret = ffi::avcodec_open2(ctx.as_mut_ptr(), ptr::null(), &mut raw);
        (ret, Dictionary::own(raw))
    };
    if ret < 0 {
        return Err(ffmpeg_next::Error::from(ret));
    }
    Ok(left.iter().map(|(k, _)| k.to_owned()).collect())
}

/// Tags an (unopened) codec context's chroma samples as left-sited
/// (MPEG-2 / H.264 default siting, EX-032).
pub(crate) fn set_codec_chroma_left(ctx: &mut codec::Context) {
    // SAFETY: `ctx` is exclusively borrowed and allocated; the value written
    // is a valid `AVChromaLocation`.
    unsafe {
        (*ctx.as_mut_ptr()).chroma_sample_location = ffi::AVChromaLocation::AVCHROMA_LOC_LEFT;
    }
}

/// Tags a frame's chroma samples as left-sited.
pub(crate) fn set_frame_chroma_left(f: &mut frame::Video) {
    // SAFETY: `f` is an exclusively borrowed, allocated frame; the value
    // written is a valid `AVChromaLocation`.
    unsafe {
        (*f.as_mut_ptr()).chroma_location = ffi::AVChromaLocation::AVCHROMA_LOC_LEFT;
    }
}

/// Makes the frame's buffers exclusively owned, copying them when an
/// encoder still holds a reference.
pub(crate) fn make_writable(f: &mut frame::Video) -> Result<(), ffmpeg_next::Error> {
    // SAFETY: `f` is an exclusively borrowed frame with allocated buffers
    // (or none, which FFmpeg reports as an error).
    let ret = unsafe { ffi::av_frame_make_writable(f.as_mut_ptr()) };
    if ret < 0 {
        Err(ffmpeg_next::Error::from(ret))
    } else {
        Ok(())
    }
}

/// Closes the output's file and reports the final flush's result. Safe to
/// call once the trailer is written; the `Output` destructor then finds no
/// I/O context left to close.
pub(crate) fn close_output_checked(out: &mut Output) -> Result<(), ffmpeg_next::Error> {
    // SAFETY: the format context is exclusively borrowed through `out`.
    // `avio_closep` flushes, closes and frees `pb` and sets it to null, so
    // the destructor's later `avio_close(NULL)` is a no-op.
    let ret = unsafe {
        let ctx = out.as_mut_ptr();
        if (*ctx).pb.is_null() {
            return Ok(());
        }
        ffi::avio_closep(&raw mut (*ctx).pb)
    };
    if ret < 0 {
        Err(ffmpeg_next::Error::from(ret))
    } else {
        Ok(())
    }
}

/// Codec parameters of one stream, as raw integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawParams {
    pub(crate) codec: CodecId,
    pub(crate) profile: c_int,
    /// `AVPixelFormat` (video) or `AVSampleFormat` (audio).
    pub(crate) format: c_int,
    pub(crate) width: c_int,
    pub(crate) height: c_int,
    pub(crate) color_primaries: c_int,
    pub(crate) color_trc: c_int,
    pub(crate) color_space: c_int,
    pub(crate) color_range: c_int,
    pub(crate) chroma_location: c_int,
    pub(crate) sample_rate: c_int,
    pub(crate) channels: c_int,
}

/// The codec parameters of stream `stream` of `input`, if it exists.
pub(crate) fn stream_params(input: &Input, stream: usize) -> Option<RawParams> {
    // SAFETY: read-only access to the context owned by `input`, borrowed for
    // the duration; only streams below `nb_streams` are dereferenced, and
    // every enum-typed field is read through a pointer cast to its
    // underlying integer type, so no out-of-range Rust enum is created.
    unsafe {
        let ctx = input.as_ptr();
        if stream >= (*ctx).nb_streams as usize {
            return None;
        }
        let st = *(*ctx).streams.add(stream);
        if st.is_null() || (*st).codecpar.is_null() {
            return None;
        }
        let par = (*st).codecpar.cast_const();
        let int = |p: *const c_int| p.read();
        Some(RawParams {
            codec: CodecId::from_raw(ptr::addr_of!((*par).codec_id).cast::<c_uint>().read()),
            profile: (*par).profile,
            format: (*par).format,
            width: (*par).width,
            height: (*par).height,
            color_primaries: int(ptr::addr_of!((*par).color_primaries).cast::<c_int>()),
            color_trc: int(ptr::addr_of!((*par).color_trc).cast::<c_int>()),
            color_space: int(ptr::addr_of!((*par).color_space).cast::<c_int>()),
            color_range: int(ptr::addr_of!((*par).color_range).cast::<c_int>()),
            chroma_location: int(ptr::addr_of!((*par).chroma_location).cast::<c_int>()),
            sample_rate: (*par).sample_rate,
            channels: (*par).ch_layout.nb_channels,
        })
    }
}

/// FFmpeg's name of a pixel format (`"yuv420p"`).
pub(crate) fn pix_fmt_name(format: c_int) -> Option<String> {
    // SAFETY: accepts any value (see the extern block).
    static_name(unsafe { av_get_pix_fmt_name_raw(format) })
}

/// FFmpeg's names of a stream's colour tags, in the order primaries,
/// transfer, matrix, range, chroma location (`"bt709"`, …, `"tv"`,
/// `"left"`). `None` for unspecified values.
pub(crate) fn color_names(p: &RawParams) -> [Option<String>; 5] {
    // SAFETY: each lookup accepts any value (see the extern block).
    unsafe {
        [
            static_name(av_color_primaries_name_raw(p.color_primaries)),
            static_name(av_color_transfer_name_raw(p.color_trc)),
            static_name(av_color_space_name_raw(p.color_space)),
            static_name(av_color_range_name_raw(p.color_range)),
            static_name(av_chroma_location_name_raw(p.chroma_location)),
        ]
    }
}

/// FFmpeg's name of a stream's profile (`"High"`, `"LC"`).
pub(crate) fn profile_name(p: &RawParams) -> Option<String> {
    // SAFETY: accepts any id and profile (see the extern block).
    static_name(unsafe { avcodec_profile_name_raw(p.codec.raw(), p.profile) })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;

    #[test]
    fn names_of_known_and_unknown_values() {
        crate::init::ensure_init().unwrap();
        assert_eq!(
            pix_fmt_name(ffi::AVPixelFormat::AV_PIX_FMT_YUV420P as c_int).as_deref(),
            Some("yuv420p")
        );
        assert_eq!(pix_fmt_name(-1), None);
        assert_eq!(pix_fmt_name(0x7fff_0000), None);
        let p = RawParams {
            codec: CodecId::from_raw(ffi::AVCodecID::AV_CODEC_ID_H264 as c_uint),
            profile: 100,
            format: 0,
            width: 0,
            height: 0,
            color_primaries: 1,
            color_trc: 1,
            color_space: 1,
            color_range: 1,
            chroma_location: 1,
            sample_rate: 0,
            channels: 0,
        };
        let names = color_names(&p);
        assert_eq!(names[0].as_deref(), Some("bt709"));
        assert_eq!(names[1].as_deref(), Some("bt709"));
        assert_eq!(names[2].as_deref(), Some("bt709"));
        assert_eq!(names[3].as_deref(), Some("tv"));
        assert_eq!(names[4].as_deref(), Some("left"));
        assert_eq!(profile_name(&p).as_deref(), Some("High"));
        let odd = RawParams {
            color_primaries: 9999,
            color_range: -3,
            profile: 9999,
            ..p
        };
        let names = color_names(&odd);
        assert_eq!(names[0], None);
        assert_eq!(names[3], None);
        assert_eq!(profile_name(&odd), None);
    }
}
