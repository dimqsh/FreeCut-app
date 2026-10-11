//! Raw per-stream operations on an open format context.

use std::ffi::{CStr, c_int};
use std::ptr;

use ffmpeg_next::ffi;
use ffmpeg_next::format::context::Input;

/// Makes the demuxer return packets of `keep` only: every other stream gets
/// `discard = AVDISCARD_ALL` (L7), `keep` gets `AVDISCARD_DEFAULT`.
pub(crate) fn keep_only_streams(input: &mut Input, keep: &[usize]) {
    let count = input.nb_streams() as usize;
    // SAFETY: `input` is borrowed mutably, so nothing else uses the format
    // context. `streams` holds `nb_streams` valid stream pointers owned by the
    // context, and only indices below that count are dereferenced. The values
    // written are valid `AVDiscard` variants.
    unsafe {
        let ctx = input.as_mut_ptr();
        for i in 0..count {
            let st = *(*ctx).streams.add(i);
            if st.is_null() {
                continue;
            }
            (*st).discard = if keep.contains(&i) {
                ffi::AVDiscard::AVDISCARD_DEFAULT
            } else {
                ffi::AVDiscard::AVDISCARD_ALL
            };
        }
    }
}

/// `avformat_seek_file` on stream `stream`, with timestamps in that stream's
/// time base. Any interrupt latched by an earlier cancel is cleared first.
pub(crate) fn seek_file(
    input: &mut Input,
    stream: usize,
    min_ts: i64,
    ts: i64,
    max_ts: i64,
) -> Result<(), ffmpeg_next::Error> {
    let index = c_int::try_from(stream).map_err(|_| ffmpeg_next::Error::StreamNotFound)?;
    if stream >= input.nb_streams() as usize {
        return Err(ffmpeg_next::Error::StreamNotFound);
    }
    input.clear_interrupt();
    // SAFETY: the format context is exclusively borrowed through `input` and
    // stays alive for the call; `index` was checked against `nb_streams`.
    let ret = unsafe { ffi::avformat_seek_file(input.as_mut_ptr(), index, min_ts, ts, max_ts, 0) };
    if ret < 0 {
        Err(ffmpeg_next::Error::from(ret))
    } else {
        Ok(())
    }
}

/// The 3×3 display matrix stored with stream `stream` (container rotation
/// and flips), if any. FFmpeg 8 keeps it in `codecpar->coded_side_data`.
pub(crate) fn display_matrix(input: &Input, stream: usize) -> Option<[i32; 9]> {
    // SAFETY: read-only access to the context owned by `input`, which is
    // borrowed for the duration. Only streams below `nb_streams` and side-data
    // entries below `nb_coded_side_data` are dereferenced. The side-data type
    // is read as a raw integer (never as the Rust enum, which a newer libav
    // could extend), and the payload is copied only when it holds at least the
    // nine `int32_t`s of a display matrix.
    unsafe {
        let ctx = input.as_ptr();
        if stream >= (*ctx).nb_streams as usize {
            return None;
        }
        let st = *(*ctx).streams.add(stream);
        if st.is_null() || (*st).codecpar.is_null() {
            return None;
        }
        let par = (*st).codecpar;
        let entries = (*par).coded_side_data;
        let count = usize::try_from((*par).nb_coded_side_data).unwrap_or(0);
        if entries.is_null() {
            return None;
        }
        let wanted = ffi::AVPacketSideDataType::AV_PKT_DATA_DISPLAYMATRIX as c_int;
        for i in 0..count {
            let entry = entries.add(i);
            let kind = ptr::addr_of!((*entry).type_).cast::<c_int>().read();
            if kind == wanted && !(*entry).data.is_null() && (*entry).size >= 36 {
                let mut bytes = [0u8; 36];
                ptr::copy_nonoverlapping((*entry).data, bytes.as_mut_ptr(), 36);
                return Some(matrix_from_bytes(&bytes));
            }
        }
        None
    }
}

/// Parses nine native-endian `int32_t`s (the layout FFmpeg uses for display
/// matrices in packet and frame side data).
pub(crate) fn matrix_from_bytes(bytes: &[u8]) -> [i32; 9] {
    let mut m = [0i32; 9];
    let (chunks, _) = bytes.as_chunks::<4>();
    for (v, chunk) in m.iter_mut().zip(chunks) {
        *v = i32::from_ne_bytes(*chunk);
    }
    m
}

/// True when the container duration was estimated from the bit rate
/// (`AVFMT_DURATION_FROM_BITRATE`), so it is a guess (L25).
pub(crate) fn duration_from_bitrate(input: &Input) -> bool {
    // SAFETY: read-only access to a field of the context owned by `input`,
    // read as a raw integer rather than as the Rust enum.
    let method = unsafe {
        ptr::addr_of!((*input.as_ptr()).duration_estimation_method)
            .cast::<c_int>()
            .read()
    };
    method == ffi::AVDurationEstimationMethod::AVFMT_DURATION_FROM_BITRATE as c_int
}

/// The container's `start_time` and `duration` in `AV_TIME_BASE` units
/// (µs), when known.
pub(crate) fn container_span(input: &Input) -> (Option<i64>, Option<i64>) {
    // SAFETY: read-only access to two plain `int64_t` fields of the context
    // owned by `input`.
    let (start, duration) = unsafe {
        let ctx = input.as_ptr();
        ((*ctx).start_time, (*ctx).duration)
    };
    let known = |v: i64| (v != ffi::AV_NOPTS_VALUE).then_some(v);
    (known(start), known(duration).filter(|d| *d > 0))
}

/// The value of metadata tag `key` (case-insensitive) of stream `stream`,
/// decoded lossily: tag bytes come from the file and need not be UTF-8
/// (ffmpeg-next's `DictionaryRef::get` assumes they are).
pub(crate) fn stream_tag(input: &Input, stream: usize, key: &CStr) -> Option<String> {
    // SAFETY: read-only access to the context owned by `input`, borrowed for
    // the duration; only streams below `nb_streams` are dereferenced.
    // `av_dict_get` accepts a null dictionary, `key` is NUL-terminated, and a
    // returned entry's `value` is a NUL-terminated string owned by the
    // dictionary, copied before the borrow ends.
    unsafe {
        let ctx = input.as_ptr();
        if stream >= (*ctx).nb_streams as usize {
            return None;
        }
        let st = *(*ctx).streams.add(stream);
        if st.is_null() {
            return None;
        }
        let entry = ffi::av_dict_get((*st).metadata, key.as_ptr(), ptr::null(), 0);
        if entry.is_null() || (*entry).value.is_null() {
            return None;
        }
        Some(
            CStr::from_ptr((*entry).value)
                .to_string_lossy()
                .into_owned(),
        )
    }
}

/// Plain codec parameters of a stream, read without touching enum fields.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CodedParams {
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) sample_rate: i32,
    pub(crate) channels: i32,
    /// Bits per raw sample when the container says (0 when unknown).
    pub(crate) bits_per_raw_sample: i32,
    /// The container's codec tag (an MP4/AVI FourCC), little-endian.
    pub(crate) codec_tag: u32,
    /// The codec-level sample aspect ratio (`(0, 1)` when unknown).
    pub(crate) sar: (i32, i32),
}

/// The codec parameters of stream `stream`, if it exists.
pub(crate) fn coded_params(input: &Input, stream: usize) -> Option<CodedParams> {
    // SAFETY: read-only access to plain `int` fields of the codec parameters
    // of a stream below `nb_streams`, owned by the context `input` borrows.
    unsafe {
        let ctx = input.as_ptr();
        if stream >= (*ctx).nb_streams as usize {
            return None;
        }
        let st = *(*ctx).streams.add(stream);
        if st.is_null() || (*st).codecpar.is_null() {
            return None;
        }
        let par = (*st).codecpar;
        Some(CodedParams {
            width: (*par).width,
            height: (*par).height,
            sample_rate: (*par).sample_rate,
            channels: (*par).ch_layout.nb_channels,
            bits_per_raw_sample: (*par).bits_per_raw_sample,
            codec_tag: (*par).codec_tag,
            sar: (
                (*par).sample_aspect_ratio.num,
                (*par).sample_aspect_ratio.den,
            ),
        })
    }
}

/// The stream's sample aspect ratio as stored by the container (which wins
/// over the codec's, as in `av_guess_sample_aspect_ratio`), if set.
pub(crate) fn stream_sar(input: &Input, stream: usize) -> Option<(i32, i32)> {
    // SAFETY: read-only access to a plain `AVRational` of a stream below
    // `nb_streams` owned by `input`.
    let sar = unsafe {
        let ctx = input.as_ptr();
        if stream >= (*ctx).nb_streams as usize {
            return None;
        }
        let st = *(*ctx).streams.add(stream);
        if st.is_null() {
            return None;
        }
        (*st).sample_aspect_ratio
    };
    (sar.num > 0 && sar.den > 0).then_some((sar.num, sar.den))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::orientation_from_matrix;
    use freecut_timeline::model::Rotation;

    /// A display matrix built by FFmpeg itself.
    fn ffmpeg_matrix(clockwise_degrees: f64, hflip: bool, vflip: bool) -> [i32; 9] {
        let mut m = [0i32; 9];
        // SAFETY: both functions write exactly nine `int32_t`s into the
        // array, which outlives the calls.
        unsafe {
            ffi::av_display_rotation_set(m.as_mut_ptr(), clockwise_degrees);
            ffi::av_display_matrix_flip(m.as_mut_ptr(), c_int::from(hflip), c_int::from(vflip));
        }
        m
    }

    #[test]
    fn orientation_convention_matches_ffmpeg_matrices() {
        // `av_display_rotation_set` takes a clockwise angle (L8).
        let cases = [
            (0.0, false, false, Rotation::None, false),
            (90.0, false, false, Rotation::Cw90, false),
            (180.0, false, false, Rotation::Cw180, false),
            (270.0, false, false, Rotation::Cw270, false),
            (-90.0, false, false, Rotation::Cw270, false),
            (0.0, true, false, Rotation::None, true),
            (0.0, false, true, Rotation::Cw180, true),
            (0.0, true, true, Rotation::Cw180, false),
            (89.0, false, false, Rotation::Cw90, false),
        ];
        for (deg, h, v, rot, mir) in cases {
            let m = ffmpeg_matrix(deg, h, v);
            assert_eq!(orientation_from_matrix(&m), (rot, mir), "{deg} {h} {v}");
        }
        // EXIF orientation 6 ("rotate 90° clockwise") reads back as -90°.
        // SAFETY: the matrix holds nine `int32_t`s.
        let angle =
            unsafe { ffi::av_display_rotation_get(ffmpeg_matrix(90.0, false, false).as_ptr()) };
        assert!((angle + 90.0).abs() < 1e-6);
    }

    #[test]
    fn matrices_parse_native_endian() {
        let mut bytes = Vec::new();
        for v in [65536i32, -1, 0, 3, 4, 5, 6, 7, 1 << 30] {
            bytes.extend_from_slice(&v.to_ne_bytes());
        }
        assert_eq!(
            matrix_from_bytes(&bytes),
            [65536, -1, 0, 3, 4, 5, 6, 7, 1 << 30]
        );
    }
}
