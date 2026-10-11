//! Decoded audio frames as byte slices of a known sample type, with the
//! sample format read as an integer (README §5.7; ffmpeg-next's
//! `frame::Audio::format()`, `planes()` and `data()` transmute it into an
//! enum).

use std::ffi::c_int;
use std::slice;

use ffmpeg_next::ffi;
use ffmpeg_next::frame;

/// The numeric type of one sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SampleType {
    U8,
    I16,
    I32,
    I64,
    F32,
    F64,
}

impl SampleType {
    /// Bytes per sample.
    pub(crate) fn bytes(self) -> usize {
        match self {
            SampleType::U8 => 1,
            SampleType::I16 => 2,
            SampleType::I32 | SampleType::F32 => 4,
            SampleType::I64 | SampleType::F64 => 8,
        }
    }
}

/// Maps a raw `AVSampleFormat` to its type and planarity; `None` for
/// `AV_SAMPLE_FMT_NONE` and values these headers don't list.
pub(crate) fn sample_format(raw: c_int) -> Option<(SampleType, bool)> {
    use ffi::AVSampleFormat as F;
    const TABLE: [(F, SampleType, bool); 12] = [
        (F::AV_SAMPLE_FMT_U8, SampleType::U8, false),
        (F::AV_SAMPLE_FMT_S16, SampleType::I16, false),
        (F::AV_SAMPLE_FMT_S32, SampleType::I32, false),
        (F::AV_SAMPLE_FMT_S64, SampleType::I64, false),
        (F::AV_SAMPLE_FMT_FLT, SampleType::F32, false),
        (F::AV_SAMPLE_FMT_DBL, SampleType::F64, false),
        (F::AV_SAMPLE_FMT_U8P, SampleType::U8, true),
        (F::AV_SAMPLE_FMT_S16P, SampleType::I16, true),
        (F::AV_SAMPLE_FMT_S32P, SampleType::I32, true),
        (F::AV_SAMPLE_FMT_S64P, SampleType::I64, true),
        (F::AV_SAMPLE_FMT_FLTP, SampleType::F32, true),
        (F::AV_SAMPLE_FMT_DBLP, SampleType::F64, true),
    ];
    TABLE
        .iter()
        .find(|(f, _, _)| *f as c_int == raw)
        .map(|&(_, t, planar)| (t, planar))
}

/// The samples of one decoded audio frame.
#[derive(Debug)]
pub(crate) struct AudioSamples<'a> {
    pub(crate) kind: SampleType,
    pub(crate) planar: bool,
    pub(crate) channels: usize,
    /// Samples per channel.
    pub(crate) samples: usize,
    /// One slice per channel (planar), or one interleaved slice (packed),
    /// each exactly as long as the samples it holds.
    pub(crate) planes: Vec<&'a [u8]>,
}

/// Why a frame's samples can't be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SamplesError {
    /// A sample format these headers don't know (raw `AVSampleFormat`).
    UnknownFormat(c_int),
    /// The frame's buffers are smaller than its sample count says.
    Inconsistent,
}

/// Views the samples of `f`.
pub(crate) fn audio_samples(f: &frame::Audio) -> Result<AudioSamples<'_>, SamplesError> {
    // SAFETY: plain `int` fields of the frame owned by `f`, which is
    // borrowed for the duration (`format` is declared `int` in `AVFrame`).
    let (raw_format, channels, samples, linesize, extended) = unsafe {
        let p = f.as_ptr();
        (
            (*p).format,
            (*p).ch_layout.nb_channels,
            (*p).nb_samples,
            (*p).linesize[0],
            (*p).extended_data,
        )
    };
    let (kind, planar) =
        sample_format(raw_format).ok_or(SamplesError::UnknownFormat(raw_format))?;
    let channels = usize::try_from(channels).unwrap_or(0);
    let samples = usize::try_from(samples).unwrap_or(0);
    let empty = AudioSamples {
        kind,
        planar,
        channels,
        samples: 0,
        planes: Vec::new(),
    };
    if channels == 0 || samples == 0 || extended.is_null() {
        return Ok(empty);
    }
    let linesize = usize::try_from(linesize).unwrap_or(0);
    let (count, plane_len) = if planar {
        (channels, samples.checked_mul(kind.bytes()))
    } else {
        (
            1,
            samples
                .checked_mul(kind.bytes())
                .and_then(|b| b.checked_mul(channels)),
        )
    };
    let plane_len = plane_len.ok_or(SamplesError::Inconsistent)?;
    if plane_len > linesize {
        return Err(SamplesError::Inconsistent);
    }
    let mut planes = Vec::with_capacity(count);
    for i in 0..count {
        // SAFETY: for planar audio `extended_data` holds one pointer per
        // channel, for packed audio at least one; each points to a buffer of
        // at least `linesize[0]` bytes (FFmpeg's frame layout) that lives as
        // long as the borrowed frame, and `plane_len <= linesize[0]`.
        let plane = unsafe {
            let data = *extended.add(i);
            if data.is_null() {
                return Err(SamplesError::Inconsistent);
            }
            slice::from_raw_parts(data.cast_const(), plane_len)
        };
        planes.push(plane);
    }
    Ok(AudioSamples {
        kind,
        planar,
        channels,
        samples,
        planes,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;
    use ffmpeg_next::ChannelLayout;
    use ffmpeg_next::format::Sample;
    use ffmpeg_next::format::sample::Type;

    #[test]
    fn raw_formats_map_and_unknown_ones_are_refused() {
        assert_eq!(
            sample_format(ffi::AVSampleFormat::AV_SAMPLE_FMT_FLTP as c_int),
            Some((SampleType::F32, true))
        );
        assert_eq!(
            sample_format(ffi::AVSampleFormat::AV_SAMPLE_FMT_S16 as c_int),
            Some((SampleType::I16, false))
        );
        assert_eq!(sample_format(-1), None);
        assert_eq!(sample_format(4242), None);
    }

    #[test]
    fn planar_and_packed_frames_are_viewed() {
        let mut planar = frame::Audio::new(Sample::F32(Type::Planar), 10, ChannelLayout::STEREO);
        planar.plane_mut::<f32>(1)[9] = 0.5;
        let v = audio_samples(&planar).unwrap();
        assert_eq!(
            (v.kind, v.planar, v.channels, v.samples),
            (SampleType::F32, true, 2, 10)
        );
        assert_eq!(v.planes.len(), 2);
        assert_eq!(v.planes[1].len(), 40);
        assert_eq!(&v.planes[1][36..40], &0.5f32.to_ne_bytes());

        let packed = frame::Audio::new(Sample::I16(Type::Packed), 7, ChannelLayout::STEREO);
        let v = audio_samples(&packed).unwrap();
        assert_eq!(
            (v.planar, v.planes.len(), v.planes[0].len()),
            (false, 1, 28)
        );

        let empty = frame::Audio::empty();
        assert!(matches!(
            audio_samples(&empty),
            Err(SamplesError::UnknownFormat(-1))
        ));
    }
}
