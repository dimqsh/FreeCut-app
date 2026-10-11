//! Container recognition on a byte prefix (`av_probe_input_format3`), used
//! to tell "FFmpeg doesn't know this content" from "a known container whose
//! header is damaged" after an open fails.

use std::ffi::{CStr, CString, c_int};
use std::ptr;

use ffmpeg_next::ffi;

/// The demuxer FFmpeg would pick for `head` (the start of a file named
/// `filename`) and its probe score (0–100; `AVPROBE_SCORE_MAX` for a clear
/// match, 50 or less for guesses from the extension alone).
pub(crate) fn detect(head: &[u8], filename: Option<&str>) -> Option<(String, i32)> {
    let size = c_int::try_from(head.len()).ok()?;
    let padding = usize::try_from(ffi::AVPROBE_PADDING_SIZE).ok()?;
    // The prober may read up to AVPROBE_PADDING_SIZE bytes past the data,
    // which must be zero.
    let mut buf = Vec::with_capacity(head.len() + padding);
    buf.extend_from_slice(head);
    buf.resize(head.len() + padding, 0);
    let name = filename.and_then(|n| CString::new(n).ok());
    let empty = c"";
    let probe = ffi::AVProbeData {
        filename: name.as_deref().unwrap_or(empty).as_ptr(),
        buf: buf.as_mut_ptr(),
        buf_size: size,
        mime_type: ptr::null(),
    };
    let mut score: c_int = 0;
    // SAFETY: `probe` points to `size` bytes of data followed by the
    // required zeroed padding, and to a NUL-terminated file name; all of it
    // outlives the call. The result is null or a static demuxer description.
    let format = unsafe { ffi::av_probe_input_format3(&raw const probe, 1, &raw mut score) };
    if format.is_null() {
        return None;
    }
    // SAFETY: `format` is a static `AVInputFormat` whose `name` is a static
    // NUL-terminated string.
    let name = unsafe {
        let n = (*format).name;
        if n.is_null() {
            return None;
        }
        CStr::from_ptr(n).to_string_lossy().into_owned()
    };
    Some((name, score))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;

    #[test]
    fn recognizes_containers_by_content_not_name() {
        crate::init::ensure_init().unwrap();
        let mut mkv = vec![0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x86, 0x81, 0x01];
        mkv.extend_from_slice(&[0x42, 0x82, 0x88]);
        mkv.extend_from_slice(b"matroska");
        mkv.resize(256, 0);
        let (name, score) = detect(&mkv, Some("clip.bin")).unwrap();
        assert!(name.contains("matroska"), "{name}");
        assert!(score > 50, "{score}");

        // Text named like an MP4 is at most a guess from the extension.
        let guess = detect(b"hello, this is plain text", Some("clip.mp4"));
        assert!(
            guess.as_ref().is_none_or(|(_, score)| *score <= 50),
            "{guess:?}"
        );
    }
}
