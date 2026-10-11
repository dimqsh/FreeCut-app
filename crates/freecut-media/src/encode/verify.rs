//! Reading a finished export back: the job's success criterion (P6).

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use freecut_timeline::time::{StreamTimeBase, Tick, ticks_from_stream};

use super::EncodeError;
use super::mp4::{self, Nclx};
use super::spec::EncodeSpec;
use crate::av;
use crate::cancel::CancelToken;
use crate::error::DecodeError;
use crate::source::open_source;

/// The AAC frame size: audio may end up to one frame off the pushed length
/// (encoder priming and padding are described by the MP4 edit list, which
/// some readers round to whole frames).
const AAC_TOLERANCE_SAMPLES: u64 = 1024;

/// A stream's colour tags as FFmpeg names them (`None` = unspecified).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ColorTags {
    /// Colour primaries (`"bt709"`).
    pub primaries: Option<String>,
    /// Transfer characteristics (`"bt709"`).
    pub transfer: Option<String>,
    /// Matrix coefficients (`"bt709"`).
    pub matrix: Option<String>,
    /// Range: `"tv"` (limited) or `"pc"` (full).
    pub range: Option<String>,
    /// Chroma siting (`"left"`).
    pub chroma_location: Option<String>,
}

impl ColorTags {
    /// BT.709 primaries, transfer and matrix with limited range (EX-032).
    pub fn is_bt709_limited(&self) -> bool {
        let is = |v: &Option<String>, want: &str| v.as_deref() == Some(want);
        is(&self.primaries, "bt709")
            && is(&self.transfer, "bt709")
            && is(&self.matrix, "bt709")
            && is(&self.range, "tv")
    }
}

/// The video stream of a written file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoStreamInfo {
    /// Container stream index.
    pub index: usize,
    /// Codec name (`"h264"`).
    pub codec: String,
    /// Profile name (`"High"`).
    pub profile: Option<String>,
    /// Pixel format name (`"yuv420p"`).
    pub pix_fmt: Option<String>,
    /// Coded width.
    pub width: u32,
    /// Coded height.
    pub height: u32,
    /// Stream colour tags.
    pub color: ColorTags,
    /// Packets in the file (one per frame for H.264 in MP4).
    pub packets: u64,
    /// Packets flagged as keyframes.
    pub keyframes: u64,
    /// Stream duration, when the container states it.
    pub duration: Option<Tick>,
}

/// The audio stream of a written file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStreamInfo {
    /// Container stream index.
    pub index: usize,
    /// Codec name (`"aac"`).
    pub codec: String,
    /// Profile name (`"LC"`).
    pub profile: Option<String>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Packets in the file.
    pub packets: u64,
    /// Stream duration, when the container states it.
    pub duration: Option<Tick>,
}

/// What [`verify_output`] found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputInfo {
    /// The demuxer's name (`"mov,mp4,m4a,3gp,3g2,mj2"`).
    pub container: String,
    /// Container-level metadata keys (EX-040's whitelist check).
    pub metadata_keys: Vec<String>,
    /// Top-level ISO BMFF boxes in file order (`["ftyp", "moov", "mdat"]`);
    /// empty for other containers.
    pub top_level_boxes: Vec<String>,
    /// The `colr`/`nclx` box inside `moov`, if present.
    pub nclx: Option<Nclx>,
    /// The first video stream.
    pub video: Option<VideoStreamInfo>,
    /// The first audio stream.
    pub audio: Option<AudioStreamInfo>,
}

impl OutputInfo {
    /// For MP4/MOV: whether the index (`moov`) precedes the media (`mdat`),
    /// i.e. the file is "fast start". `None` when either box is missing.
    pub fn index_before_media(&self) -> Option<bool> {
        let pos = |kind: &str| self.top_level_boxes.iter().position(|k| k == kind);
        Some(pos("moov")? < pos("mdat")?)
    }

    /// The export job's final check: the file holds exactly what `spec`
    /// asked for, `frames` video frames and (with audio) `audio_samples`
    /// sample frames within one AAC frame.
    pub fn check(
        &self,
        spec: &EncodeSpec,
        frames: u64,
        audio_samples: u64,
    ) -> Result<(), EncodeError> {
        let fail = |what: String| Err(EncodeError::Verify(what));
        let Some(v) = &self.video else {
            return fail("no video stream".into());
        };
        if v.codec != "h264" || v.profile.as_deref() != Some("High") {
            return fail(format!(
                "video is {} {:?}, not H.264 High",
                v.codec, v.profile
            ));
        }
        if v.pix_fmt.as_deref() != Some("yuv420p") {
            return fail(format!("pixel format {:?}, not yuv420p", v.pix_fmt));
        }
        if (v.width, v.height) != (spec.width, spec.height) {
            return fail(format!("size {}x{}", v.width, v.height));
        }
        if !v.color.is_bt709_limited() {
            return fail(format!("colour tags {:?}", v.color));
        }
        if v.packets != frames {
            return fail(format!("{} video packets for {frames} frames", v.packets));
        }
        if self.index_before_media() != Some(true) {
            return fail("the index is not at the start of the file".into());
        }
        let want = spec
            .frame_rate
            .frames(i64::try_from(frames).unwrap_or(i64::MAX));
        if let (Some(d), Ok(want)) = (v.duration, want)
            && d != want
        {
            return fail(format!("video lasts {d}, expected {want}"));
        }
        match (&spec.audio, &self.audio) {
            (None, None) => Ok(()),
            (None, Some(_)) => fail("unexpected audio stream".into()),
            (Some(_), None) => fail("no audio stream".into()),
            (Some(want), Some(a)) => {
                if a.codec != "aac" || a.channels != want.channels {
                    return fail(format!("audio is {} with {} channels", a.codec, a.channels));
                }
                if a.sample_rate != want.sample_rate.hz() {
                    return fail(format!("audio rate {}", a.sample_rate));
                }
                let Some(d) = a.duration else {
                    return fail("the audio duration is unknown".into());
                };
                let got = want.sample_rate.sample_index(d).unsigned_abs();
                if got.abs_diff(audio_samples) > AAC_TOLERANCE_SAMPLES {
                    return fail(format!("audio has {got} samples, expected {audio_samples}"));
                }
                Ok(())
            }
        }
    }
}

/// Reads a finished export back: streams, codecs, profile, pixel format,
/// colour tags, packet counts, durations, metadata keys and (for MP4) the
/// box order and `colr` box. Reading every packet header takes about as
/// long as reading the file once.
pub fn verify_output(path: &Path) -> Result<OutputInfo, EncodeError> {
    verify_output_with_cancel(path, &CancelToken::new())
}

/// [`verify_output`] that stops with `Cancelled` when `cancel` fires.
pub fn verify_output_with_cancel(
    path: &Path,
    cancel: &CancelToken,
) -> Result<OutputInfo, EncodeError> {
    let mut src = open_source(path, cancel).map_err(|e| reread(DecodeError::Open(e)))?;
    let container = src.container.clone();
    let metadata_keys = src
        .input
        .metadata()
        .iter()
        .map(|(k, _)| k.to_owned())
        .collect();

    let video_index = src.default_video.or_else(|| src.video.first().copied());
    let audio_index = src.default_audio.or_else(|| src.audio.first().copied());
    let duration = |src: &crate::source::Source, index: usize| -> Option<Tick> {
        let stream = src.input.stream(index)?;
        let (d, tb) = (stream.duration(), stream.time_base());
        let tb = StreamTimeBase::new(tb.numerator(), tb.denominator()).ok()?;
        (d > 0).then(|| ticks_from_stream(d, tb).ok()).flatten()
    };

    let mut video = video_index.and_then(|index| {
        let p = av::encode::stream_params(&src.input, index)?;
        let [primaries, transfer, matrix, range, chroma_location] = av::encode::color_names(&p);
        Some(VideoStreamInfo {
            index,
            codec: src.codec_name(index),
            profile: av::encode::profile_name(&p),
            pix_fmt: av::encode::pix_fmt_name(p.format),
            width: u32::try_from(p.width).unwrap_or(0),
            height: u32::try_from(p.height).unwrap_or(0),
            color: ColorTags {
                primaries,
                transfer,
                matrix,
                range,
                chroma_location,
            },
            packets: 0,
            keyframes: 0,
            duration: duration(&src, index),
        })
    });
    let mut audio = audio_index.and_then(|index| {
        let p = av::encode::stream_params(&src.input, index)?;
        Some(AudioStreamInfo {
            index,
            codec: src.codec_name(index),
            profile: av::encode::profile_name(&p),
            sample_rate: u32::try_from(p.sample_rate).unwrap_or(0),
            channels: u16::try_from(p.channels).unwrap_or(0),
            packets: 0,
            duration: duration(&src, index),
        })
    });

    while let Some(pkt) = src.read_packet().map_err(reread)? {
        if let Some(v) = video.as_mut().filter(|v| v.index == pkt.stream()) {
            v.packets += 1;
            v.keyframes += u64::from(pkt.is_key());
        } else if let Some(a) = audio.as_mut().filter(|a| a.index == pkt.stream()) {
            a.packets += 1;
        }
    }
    drop(src);

    let mut file = BufReader::new(File::open(path)?);
    let boxes = mp4::top_level_boxes(&mut file)?;
    let is_bmff = boxes.first().is_some_and(|b| &b.kind == b"ftyp");
    let nclx = match boxes.iter().find(|b| &b.kind == b"moov") {
        Some(moov) if is_bmff => mp4::find_nclx(&mut file, moov)?,
        _ => None,
    };
    let top_level_boxes = if is_bmff {
        boxes
            .iter()
            .map(|b| String::from_utf8_lossy(&b.kind).into_owned())
            .collect()
    } else {
        Vec::new()
    };

    Ok(OutputInfo {
        container,
        metadata_keys,
        top_level_boxes,
        nclx,
        video,
        audio,
    })
}

fn reread(e: DecodeError) -> EncodeError {
    match e {
        DecodeError::Cancelled | DecodeError::Open(crate::error::ProbeError::Cancelled) => {
            EncodeError::Cancelled
        }
        e => EncodeError::Reread(e),
    }
}
