//! Probing (PM-035): what a file contains, in the timeline's plain types.
//!
//! [`probe`] opens the file through [`crate::source`], then reads packets from
//! the start of every usable audio and video stream once, decoding until each
//! stream has produced its first frame. That pass gives, per stream, the
//! first presentable timestamp (the media origin and stream starts), the
//! real output pixel format (alpha, bit depth), the decoded size, and the
//! frame display matrix (EXIF orientation of MJPEG stills). Packet timestamps
//! of the default video stream (up to 300) feed the VFR heuristic.
//!
//! Durations come from the stream headers, else from the per-track
//! `DURATION` tag Matroska muxers write (Matroska has no per-stream duration
//! field), else from the container, converted to media time; a duration
//! estimated from the bit rate, or taken from a tag or the container, is
//! flagged with `duration_exact = false` on audio streams.
//!
//! Audio streams also carry what the stream picker shows (AU-038): the
//! file's default flag (the stream's `default` disposition), its language
//! and title tags (a language of "und", undetermined, counts as none) and
//! FFmpeg's name for its channel layout (none when the file doesn't say which
//! speakers the channels feed).
//!
//! Stills (`MediaInfo::still` set) get the nominal duration
//! [`STILL_DURATION`]: one second, only so the item has a positive length.
//! A placed still lasts the project's still duration (PM-010), which the
//! timeline decides; nothing should read meaning into this value.

use std::ffi::CStr;
use std::path::Path;
use std::time::Duration;

use ffmpeg_next::decoder;
use ffmpeg_next::format::stream::Disposition;
use ffmpeg_next::frame;
use ffmpeg_next::util::frame::side_data::Type as SideDataType;
use freecut_timeline::model::{AudioStreamInfo, Fingerprint, MediaInfo, Rotation, VideoStreamInfo};
use freecut_timeline::time::{
    Rational, Rounding, StreamTimeBase, TICKS_PER_SECOND, Tick, ticks_from_stream,
};

use crate::av;
use crate::cancel::CancelToken;
use crate::color::HdrTransfer;
use crate::error::{DecodeError, ProbeError};
use crate::fingerprint::fingerprint;
use crate::geometry::orientation_from_matrix;
use crate::source::{FIRST_FRAME_PACKET_LIMIT, Source, cancelled_probe, open_source};
use crate::time::{StreamClock, is_variable_rate, nominal_rate};

/// The nominal duration given to still images (one second; see the module
/// docs).
pub const STILL_DURATION: Tick = Tick::from_raw(TICKS_PER_SECOND);

/// How long [`probe`] may go without finishing (PM-030's 30 s timeout).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Packet timestamps of the default video stream used for VFR detection.
const VFR_PACKETS: usize = 300;

/// Total packets read before giving up on streams without a first frame.
/// The readers allow as many packets of one stream (they agree with the
/// probe about which files decode, M2-MEDIA-01).
const SCAN_PACKET_LIMIT: usize = FIRST_FRAME_PACKET_LIMIT;

/// The result of probing a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probed {
    /// Streams, durations and geometry, in media time.
    pub info: MediaInfo,
    /// The content key (PM-038), computed with the probed duration.
    pub fingerprint: Fingerprint,
}

/// Probes `path` with the default [`PROBE_TIMEOUT`].
///
/// Returns the streams FreeCut can decode with their codec, coded size,
/// sample aspect ratio, display rotation and mirroring, nominal rate, VFR
/// flag, alpha, bit depth, sample rate and channels, and every start and
/// duration in media time (0 = the first presentable frame of the default
/// video stream). Content decides, not the extension (PM-036).
///
/// Stills set `info.still` to their displayed size (after EXIF orientation)
/// and get the nominal duration [`STILL_DURATION`].
///
/// Errors follow PM-037: [`ProbeError::Io`] (missing file, permissions),
/// [`ProbeError::Empty`], [`ProbeError::NotMedia`], [`ProbeError::NoStreams`],
/// [`ProbeError::UnsupportedCodec`], [`ProbeError::Incomplete`] (MP4/MOV
/// without its index), [`ProbeError::Corrupt`], [`ProbeError::UnsupportedPath`]
/// (folders and other non-files) and [`ProbeError::Timeout`].
pub fn probe(path: &Path) -> Result<Probed, ProbeError> {
    probe_with_cancel(path, &CancelToken::new().with_timeout(PROBE_TIMEOUT))
}

/// Probes `path`, giving up when `cancel` fires
/// ([`ProbeError::Cancelled`] or, for a deadline, [`ProbeError::Timeout`]).
pub fn probe_with_cancel(path: &Path, cancel: &CancelToken) -> Result<Probed, ProbeError> {
    let (info, _, _) = probe_info(path, cancel)?;
    let fingerprint = fingerprint(path, info.duration)?;
    Ok(Probed { info, fingerprint })
}

/// The probe result, the default video stream's index, and that stream's
/// HDR transfer from its codec parameters (PQ or HLG, which the decoders
/// convert to SDR, CL-002; `None` for SDR).
type Detail = (MediaInfo, Option<usize>, Option<HdrTransfer>);

/// The probe result with the [`Detail`] decoding entry points need.
fn probe_info(path: &Path, cancel: &CancelToken) -> Result<Detail, ProbeError> {
    let mut src = open_source(path, cancel)?;
    let info = scan(&mut src, path).map_err(|e| match e {
        DecodeError::Cancelled | DecodeError::Timeout => cancelled_probe(cancel),
        DecodeError::Open(p) => p,
        DecodeError::Io(io) => ProbeError::Io(io),
        other => ProbeError::Corrupt {
            detail: other.to_string(),
        },
    })?;
    let hdr = src
        .default_video
        .and_then(|index| av::encode::stream_params(&src.input, index))
        .and_then(|p| av::sws::hdr_transfer(p.color_trc));
    Ok((info, src.default_video, hdr))
}

/// [`probe_info`] for decoding entry points.
pub(crate) fn probe_detail(path: &Path, cancel: &CancelToken) -> Result<Detail, DecodeError> {
    probe_info(path, cancel).map_err(DecodeError::Open)
}

/// What the first decoded frame of a stream tells us.
#[derive(Debug, Clone)]
struct First {
    ts: i64,
    width: u32,
    height: u32,
    has_alpha: bool,
    bit_depth: u8,
    matrix: Option<[i32; 9]>,
    sample_rate: u32,
    channels: u16,
}

enum Dec {
    Video(decoder::Video),
    Audio(decoder::Audio),
}

struct StreamScan {
    index: usize,
    dec: Dec,
    first: Option<First>,
    /// Gave up (undecodable within the limits).
    failed: bool,
}

impl StreamScan {
    fn done(&self) -> bool {
        self.first.is_some() || self.failed
    }

    /// Pulls one frame if available and records it as the first.
    fn receive(&mut self) {
        if self.first.is_some() {
            return;
        }
        match &mut self.dec {
            Dec::Video(d) => {
                let mut f = frame::Video::empty();
                if d.receive_frame(&mut f).is_ok() {
                    let info = av::pixfmt::frame_info(&f);
                    let matrix = f
                        .side_data(SideDataType::DisplayMatrix)
                        .filter(|sd| sd.data().len() >= 36)
                        .map(|sd| av::stream::matrix_from_bytes(sd.data()));
                    self.first = Some(First {
                        ts: f.timestamp().or_else(|| f.pts()).unwrap_or(0),
                        width: f.width(),
                        height: f.height(),
                        has_alpha: info.is_some_and(|i| i.has_alpha),
                        bit_depth: info.map_or(8, |i| i.bit_depth),
                        matrix,
                        sample_rate: 0,
                        channels: 0,
                    });
                }
            }
            Dec::Audio(d) => {
                let mut f = frame::Audio::empty();
                if d.receive_frame(&mut f).is_ok() {
                    self.first = Some(First {
                        ts: f.timestamp().or_else(|| f.pts()).unwrap_or(0),
                        width: 0,
                        height: 0,
                        has_alpha: false,
                        bit_depth: 0,
                        matrix: None,
                        sample_rate: f.rate(),
                        channels: f.channels(),
                    });
                }
            }
        }
    }

    fn send(&mut self, packet: Option<&ffmpeg_next::Packet>) {
        let r = match (&mut self.dec, packet) {
            (Dec::Video(d), Some(p)) => d.send_packet(p),
            (Dec::Audio(d), Some(p)) => d.send_packet(p),
            (Dec::Video(d), None) => d.send_eof(),
            (Dec::Audio(d), None) => d.send_eof(),
        };
        if let Err(e) = r {
            tracing::debug!(stream = self.index, "probe decode error: {e}");
        }
    }
}

/// The span `[start, end)` a stream's headers announce, in its own units.
fn header_span(src: &Source, index: usize, first_ts: i64) -> Option<(i64, i64)> {
    let st = src.input.stream(index)?;
    let duration = st.duration();
    if duration <= 0 || duration == ffmpeg_next::ffi::AV_NOPTS_VALUE {
        return None;
    }
    let start = match st.start_time() {
        ffmpeg_next::ffi::AV_NOPTS_VALUE => first_ts,
        s => s,
    };
    Some((start, start.checked_add(duration)?))
}

/// Parses a Matroska `DURATION` tag (`HH:MM:SS.nnnnnnnnn`, fraction
/// optional and of any length) into nanoseconds.
fn parse_duration_tag(text: &str) -> Option<i64> {
    let text = text.trim();
    let (clock, fraction) = text.split_once('.').unwrap_or((text, ""));
    let mut parts = clock.split(':');
    let (h, m, sec) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let num = |s: &str| -> Option<i64> {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let (h, m, sec) = (num(h)?, num(m)?, num(sec)?);
    if m >= 60 || sec >= 60 {
        return None;
    }
    let mut nanos = 0i64;
    for (i, digit) in fraction.bytes().take(9).enumerate() {
        nanos += i64::from(digit - b'0') * 10i64.pow(8 - i as u32);
    }
    h.checked_mul(3600)?
        .checked_add(m * 60 + sec)?
        .checked_mul(1_000_000_000)?
        .checked_add(nanos)
}

/// The end of stream `index` from a muxer's per-track `DURATION` tag, as an
/// absolute tick time of the file. FFmpeg's Matroska muxer stores the
/// track's end time there; a value before the stream's first frame can only
/// be a length (other muxers), so it is counted from that frame.
fn tagged_end(src: &Source, index: usize, first_abs: Tick) -> Option<Tick> {
    let tag = av::stream::stream_tag(&src.input, index, c"DURATION")?;
    let nanos = parse_duration_tag(&tag)?;
    let tagged = ticks_from_stream(nanos, StreamTimeBase::new(1, 1_000_000_000).ok()?).ok()?;
    if tagged > first_abs {
        Some(tagged)
    } else {
        first_abs.checked_add(tagged).ok()
    }
}

/// The container's end as an absolute tick time of the file.
fn container_end(src: &Source) -> Option<Tick> {
    let (start, duration) = av::stream::container_span(&src.input);
    let end_us = start.unwrap_or(0).checked_add(duration?)?;
    let micros = StreamTimeBase::new(1, 1_000_000).ok()?;
    ticks_from_stream(end_us, micros).ok()
}

/// End of a stream in media time, and whether it came from the stream's own
/// header: from that header, else from its `DURATION` tag (never past the
/// container's end), else the container's end.
fn stream_end(
    src: &Source,
    index: usize,
    clock: &StreamClock,
    first_ts: i64,
) -> Option<(Tick, bool)> {
    if let Some((_, end)) = header_span(src, index, first_ts) {
        return clock.media_time(end).ok().map(|t| (t, true));
    }
    let container = container_end(src);
    let first_abs = ticks_from_stream(first_ts, clock.time_base()).ok()?;
    let abs = match (tagged_end(src, index, first_abs), container) {
        (Some(tag), Some(end)) => tag.min(end),
        (Some(tag), None) => tag,
        (None, end) => end?,
    };
    clock.absolute_to_media(abs).ok().map(|t| (t, false))
}

fn scan(src: &mut Source, path: &Path) -> Result<MediaInfo, DecodeError> {
    let mut streams: Vec<usize> = src.video.iter().chain(&src.audio).copied().collect();
    streams.sort_unstable();
    src.keep_only(&streams);

    let mut scans = Vec::with_capacity(streams.len());
    for &index in &streams {
        let dec = if src.video.contains(&index) {
            src.video_decoder(index, 1).map(Dec::Video)
        } else {
            src.audio_decoder(index).map(Dec::Audio)
        };
        match dec {
            Ok(dec) => scans.push(StreamScan {
                index,
                dec,
                first: None,
                failed: false,
            }),
            Err(e) => tracing::debug!(stream = index, "cannot open decoder: {e}"),
        }
    }

    let default_video = src.default_video;
    let mut vfr_pts: Vec<i64> = Vec::new();
    let mut packets = 0usize;
    loop {
        let need_vfr = default_video.is_some() && !src.still && vfr_pts.len() < VFR_PACKETS;
        if scans.iter().all(StreamScan::done) && !need_vfr {
            break;
        }
        match src.read_packet()? {
            Some(packet) => {
                packets += 1;
                let index = packet.stream();
                if Some(index) == default_video
                    && vfr_pts.len() < VFR_PACKETS
                    && let Some(pts) = packet.pts()
                {
                    vfr_pts.push(pts);
                }
                if let Some(s) = scans.iter_mut().find(|s| s.index == index && !s.done()) {
                    s.send(Some(&packet));
                    s.receive();
                }
                if packets >= SCAN_PACKET_LIMIT {
                    scans.iter_mut().filter(|s| !s.done()).for_each(|s| {
                        s.failed = true;
                    });
                    break;
                }
            }
            None => {
                for s in scans.iter_mut().filter(|s| !s.done()) {
                    s.send(None);
                    s.receive();
                    s.failed = s.first.is_none();
                }
                break;
            }
        }
    }

    let first_of = |index: usize| {
        scans
            .iter()
            .find(|s| s.index == index)
            .and_then(|s| s.first.clone())
    };
    // Media time 0 is the first frame of the default video stream, or of the
    // default audio stream without video (I2).
    let Some(origin_stream) = default_video.or(src.default_audio) else {
        return Err(DecodeError::Open(ProbeError::NoStreams));
    };
    let Some(origin_first) = first_of(origin_stream) else {
        return Err(DecodeError::Open(ProbeError::Corrupt {
            detail: "the first frame could not be decoded".to_owned(),
        }));
    };
    let (num, den) = src.time_base(origin_stream);
    let origin = StreamClock::new(num, den, Tick::ZERO)?.media_time(origin_first.ts)?;

    let mut video = Vec::new();
    for &index in &src.video {
        let Some(first) = first_of(index) else {
            tracing::warn!(
                stream = index,
                "skipping a video stream that doesn't decode"
            );
            continue;
        };
        video.push(video_info(src, index, &first, origin, &vfr_pts, path)?);
    }
    let mut audio = Vec::new();
    for &index in &src.audio {
        let Some(first) = first_of(index) else {
            tracing::warn!(
                stream = index,
                "skipping an audio stream that doesn't decode"
            );
            continue;
        };
        audio.push(audio_info(src, index, &first, origin)?);
    }
    if video.is_empty() && audio.is_empty() {
        return Err(DecodeError::Open(ProbeError::Corrupt {
            detail: "no stream could be decoded".to_owned(),
        }));
    }

    let mut duration = Tick::ZERO;
    for (start, len) in video
        .iter()
        .map(|v| (v.start, v.duration))
        .chain(audio.iter().map(|a| (a.start, a.duration)))
    {
        duration = duration.max(start.checked_add(len)?);
    }
    let still = if src.still {
        video.first().map(|v| {
            let quarter = matches!(v.rotation, Rotation::Cw90 | Rotation::Cw270);
            if quarter {
                (v.height, v.width)
            } else {
                (v.width, v.height)
            }
        })
    } else {
        None
    };
    if still.is_some() {
        duration = STILL_DURATION;
    }
    if !duration.is_positive() {
        // A stream whose headers give no length (raw streams); the exact
        // scan (AU-001) corrects it later. One frame keeps it usable.
        duration = one_frame(video.first().and_then(|v| v.rate));
    }
    Ok(MediaInfo {
        container: src.container.clone(),
        default_video: src.default_video.and_then(|i| u32::try_from(i).ok()),
        duration,
        video,
        audio,
        still,
    })
}

/// Orientation of a still picture as the `image` crate reads it (EXIF),
/// mapped to "mirror, then rotate clockwise".
fn image_orientation(path: &Path) -> Option<(Rotation, bool)> {
    use image::ImageDecoder;
    use image::metadata::Orientation as O;
    let mut decoder = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    Some(match decoder.orientation().ok()? {
        O::NoTransforms => (Rotation::None, false),
        O::Rotate90 => (Rotation::Cw90, false),
        O::Rotate180 => (Rotation::Cw180, false),
        O::Rotate270 => (Rotation::Cw270, false),
        O::FlipHorizontal => (Rotation::None, true),
        O::FlipVertical => (Rotation::Cw180, true),
        // Rotate, then flip the result = flip the coded picture, then rotate
        // the other way.
        O::Rotate90FlipH => (Rotation::Cw270, true),
        O::Rotate270FlipH => (Rotation::Cw90, true),
    })
}

fn video_info(
    src: &Source,
    index: usize,
    first: &First,
    origin: Tick,
    default_pts: &[i64],
    path: &Path,
) -> Result<VideoStreamInfo, DecodeError> {
    let st = src
        .input
        .stream(index)
        .ok_or(DecodeError::Corrupt { at: Tick::ZERO })?;
    let (num, den) = src.time_base(index);
    let clock = StreamClock::new(num, den, origin)?;
    let avg = st.avg_frame_rate();
    let base = st.rate();
    let avg = (avg.numerator(), avg.denominator());
    let base = (base.numerator(), base.denominator());
    let guess = av::codec::guess_frame_rate(&src.input, index);

    let (rotation, mirrored) = if src.still {
        image_orientation(path)
            .or_else(|| first.matrix.as_ref().map(orientation_from_matrix))
            .unwrap_or((Rotation::None, false))
    } else {
        av::stream::display_matrix(&src.input, index)
            .or(first.matrix)
            .map(|m| orientation_from_matrix(&m))
            .unwrap_or((Rotation::None, false))
    };

    let sar = av::stream::stream_sar(&src.input, index)
        .or_else(|| {
            // The codec-level SAR, when the container has none.
            let (n, d) = av::stream::coded_params(&src.input, index)?.sar;
            (n > 0 && d > 0).then_some((n, d))
        })
        .map(|(n, d)| reduce(n, d))
        .unwrap_or((1, 1));

    let codec = src.codec_name(index);
    if src.still {
        return Ok(VideoStreamInfo {
            index: index as u32,
            codec,
            width: first.width,
            height: first.height,
            sar,
            rotation,
            mirrored,
            rate: None,
            variable_rate: false,
            has_alpha: first.has_alpha,
            bit_depth: first.bit_depth,
            start: Tick::ZERO,
            duration: STILL_DURATION,
        });
    }

    let rate = nominal_rate(guess, avg);
    let pts: &[i64] = if Some(index) == src.default_video {
        default_pts
    } else {
        &[]
    };
    let start = clock.media_time(first.ts)?;
    let end = stream_end(src, index, &clock, first.ts).map_or(start, |(end, _)| end);
    let mut duration = end.checked_sub(start)?;
    if !duration.is_positive() {
        // One frame (PM-035: a one-frame video is valid).
        duration = one_frame(rate);
    }
    Ok(VideoStreamInfo {
        index: index as u32,
        codec,
        width: first.width,
        height: first.height,
        sar,
        rotation,
        mirrored,
        rate,
        variable_rate: is_variable_rate(pts, avg, base, (num, den)),
        has_alpha: first.has_alpha,
        bit_depth: first.bit_depth,
        start,
        duration,
    })
}

fn audio_info(
    src: &Source,
    index: usize,
    first: &First,
    origin: Tick,
) -> Result<AudioStreamInfo, DecodeError> {
    if src.input.stream(index).is_none() {
        return Err(DecodeError::Corrupt { at: Tick::ZERO });
    }
    let (num, den) = src.time_base(index);
    let clock = StreamClock::new(num, den, origin)?;
    let start = clock.media_time(first.ts)?;
    let (end, from_header) = stream_end(src, index, &clock, first.ts).unwrap_or((start, false));
    let duration = end.checked_sub(start)?.max(Tick::ZERO);
    let coded = av::stream::coded_params(&src.input, index).unwrap_or_default();
    let sample_rate = if first.sample_rate > 0 {
        first.sample_rate
    } else {
        u32::try_from(coded.sample_rate).unwrap_or(0)
    };
    let channels = if first.channels > 0 {
        first.channels
    } else {
        u16::try_from(coded.channels).unwrap_or(0)
    };
    let default = src
        .input
        .stream(index)
        .is_some_and(|st| st.disposition().contains(Disposition::DEFAULT));
    Ok(AudioStreamInfo {
        index: index as u32,
        codec: src.codec_name(index),
        sample_rate,
        channels,
        start,
        duration,
        duration_exact: from_header && !av::stream::duration_from_bitrate(&src.input),
        default,
        language: tag_text(src, index, c"language").filter(|l| !l.eq_ignore_ascii_case("und")),
        title: tag_text(src, index, c"title"),
        layout: av::layout::stream_layout_name(&src.input, index),
    })
}

/// Stream `index`'s metadata tag `key`, trimmed; `None` when missing or
/// blank.
fn tag_text(src: &Source, index: usize, key: &CStr) -> Option<String> {
    let text = av::stream::stream_tag(&src.input, index, key)?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// One frame at `rate` (rounded up), or one tick when the rate is unknown.
pub(crate) fn one_frame(rate: Option<Rational>) -> Tick {
    rate.and_then(|r| {
        Tick::from_raw(TICKS_PER_SECOND)
            .div_rational(r, Rounding::Ceil)
            .ok()
    })
    .filter(|t| t.is_positive())
    .unwrap_or(Tick::from_raw(1))
}

fn reduce(n: i32, d: i32) -> (u32, u32) {
    let (mut a, mut b) = (n.unsigned_abs(), d.unsigned_abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let g = a.max(1);
    (n.unsigned_abs() / g, d.unsigned_abs() / g)
}
