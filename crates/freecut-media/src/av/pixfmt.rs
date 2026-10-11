//! Pixel-format facts from FFmpeg's descriptor table, looked up by the raw
//! `AVPixelFormat` integer (README §5.7: a decoder in a newer libavutil
//! minor may output a format these headers don't list).

use std::ffi::c_int;

use ffmpeg_next::ffi;
use ffmpeg_next::frame;

// SAFETY: libavutil's own `av_pix_fmt_desc_get` (linked through
// ffmpeg-sys-next), declared with `int` in place of `enum AVPixelFormat`,
// which C passes the same way. It accepts any value and returns null for
// unknown formats.
unsafe extern "C" {
    #[link_name = "av_pix_fmt_desc_get"]
    fn av_pix_fmt_desc_get_raw(format: c_int) -> *const ffi::AVPixFmtDescriptor;
}

/// What the probe and the converter need to know about a pixel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PixFmtInfo {
    /// The format carries an alpha component.
    pub(crate) has_alpha: bool,
    /// Bits per component of the first component (luma or red).
    pub(crate) bit_depth: u8,
    /// RGB (or gray) rather than Y′CbCr samples.
    pub(crate) is_rgb: bool,
    /// A hardware surface format whose data isn't in CPU memory.
    pub(crate) is_hwaccel: bool,
}

/// Looks the raw pixel format `format` up in FFmpeg's descriptor table.
pub(crate) fn info(format: c_int) -> Option<PixFmtInfo> {
    // SAFETY: `av_pix_fmt_desc_get` accepts any value; a non-null result
    // points into FFmpeg's static, immutable descriptor table, valid for the
    // life of the process.
    let ptr = unsafe { av_pix_fmt_desc_get_raw(format) };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` is a non-null descriptor (see above); plain fields.
    let (flags, depth) = unsafe { ((*ptr).flags, (*ptr).comp[0].depth) };
    let has = |flag: i32| flags & u64::from(flag.unsigned_abs()) != 0;
    Some(PixFmtInfo {
        has_alpha: has(ffi::AV_PIX_FMT_FLAG_ALPHA),
        bit_depth: u8::try_from(depth).unwrap_or(0),
        is_rgb: has(ffi::AV_PIX_FMT_FLAG_RGB),
        is_hwaccel: has(ffi::AV_PIX_FMT_FLAG_HWACCEL),
    })
}

/// The pixel format of a decoded frame as its raw integer.
pub(crate) fn frame_format(f: &frame::Video) -> c_int {
    // SAFETY: `format` is a plain `int` field of the frame owned by `f`,
    // which is borrowed for the read.
    unsafe { (*f.as_ptr()).format }
}

/// [`info`] for a decoded frame's format.
pub(crate) fn frame_info(f: &frame::Video) -> Option<PixFmtInfo> {
    info(frame_format(f))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ffi::AVPixelFormat as P;

    #[test]
    fn common_formats() {
        let yuv = info(P::AV_PIX_FMT_YUV420P as c_int);
        assert_eq!(
            yuv,
            Some(PixFmtInfo {
                has_alpha: false,
                bit_depth: 8,
                is_rgb: false,
                is_hwaccel: false
            })
        );
        let rgba = info(P::AV_PIX_FMT_RGBA as c_int).map(|i| (i.has_alpha, i.is_rgb));
        assert_eq!(rgba, Some((true, true)));
        assert_eq!(
            info(P::AV_PIX_FMT_YUV420P10LE as c_int).map(|i| i.bit_depth),
            Some(10)
        );
        assert_eq!(
            info(P::AV_PIX_FMT_YUVA420P as c_int).map(|i| i.has_alpha),
            Some(true)
        );
    }

    #[test]
    fn unknown_formats_have_no_info() {
        assert_eq!(info(-1), None);
        assert_eq!(info(0x7fff_0000), None);
    }
}
