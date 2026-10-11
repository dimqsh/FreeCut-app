//! Opening a media file: stat, content sniffing, custom-I/O open, error
//! mapping, stream classification and the media origin. Shared by the probe
//! and every reader so they agree on which streams exist, which decoder each
//! uses and where media time 0 is.
//!
//! Files are opened with `std::fs::File` wrapped in FFmpeg custom I/O
//! (`StreamIo`), never by path: ffmpeg-next's `format::input(path)` panics on
//! non-UTF-8 paths, and custom I/O also handles long Windows paths and the
//! CRT descriptor limit. The [`CancelToken`] is installed as the interrupt
//! callback, so a stalled read aborts.
//!
//! Stream kinds and codec ids are read as raw integers through [`av::codec`]
//! (README §5.7), never through ffmpeg-next's enum getters.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use ffmpeg_next::decoder;
use ffmpeg_next::format::context::Input;
use ffmpeg_next::format::stream::Disposition;
use ffmpeg_next::{Codec, Dictionary, Packet, frame};
use freecut_timeline::time::{Tick, ticks_from_stream};

use crate::av;
use crate::av::codec::{CodecId, MediaKind};
use crate::cancel::{CancelReason, CancelToken};
use crate::error::{DecodeError, NotMediaKind, ProbeError, StreamKind};
use crate::init::ensure_init;

/// Custom-I/O buffer size: large reads suit local disks and network shares.
const IO_BUFFER: usize = 256 * 1024;

/// How many bytes the content sniffer looks at.
const SNIFF_LEN: usize = 4096;

/// How many bytes container recognition looks at after a failed open.
const DETECT_LEN: usize = 256 * 1024;

/// Probe scores at or below this are guesses (`AVPROBE_SCORE_RETRY`: an
/// extension match or a weak content match, "misdetection possible").
const WEAK_PROBE_SCORE: i32 = 25;

/// Packets of one stream read while looking for its first frame before the
/// stream is declared undecodable. The probe reads at most this many packets
/// of *all* streams together, so any stream the probe accepted is also
/// found by [`Source::first_frame_ts`] (the readers' media origin).
pub(crate) const FIRST_FRAME_PACKET_LIMIT: usize = 5000;

/// `analyzeduration` (µs) and `probesize` (bytes) of the second open when a
/// stream had no parameters after the first: four times FFmpeg's defaults of
/// 5 s and 5 MB (media-io probe step 4).
const RETRY_ANALYZE_DURATION_US: i64 = 4 * 5_000_000;
const RETRY_PROBE_SIZE: i64 = 4 * 5_000_000;

/// A timestamp before any real one, far enough from `i64::MIN` that
/// demuxer arithmetic on it can't overflow.
const REWIND_TS: i64 = i64::MIN / 8;

/// Consecutive failed packet reads tolerated before giving up.
const MAX_READ_RETRIES: usize = 1000;

/// How long `find_stream_info` may analyze when opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Analysis {
    /// FFmpeg's defaults.
    Default,
    /// Four times longer, after a stream reported no parameters.
    Extended,
}

/// An open media file with its usable streams classified.
pub(crate) struct Source {
    pub(crate) input: Input,
    /// Decodable video streams (attached pictures excluded), by index.
    pub(crate) video: Vec<usize>,
    /// Decodable audio streams, by index.
    pub(crate) audio: Vec<usize>,
    /// The decoder each usable stream is opened with.
    decoders: Vec<(usize, Codec)>,
    pub(crate) default_video: Option<usize>,
    pub(crate) default_audio: Option<usize>,
    /// A single still picture: PNG, JPEG, …, or an animated image (GIF,
    /// APNG, animated WebP), which is a still of its first frame in the MVP
    /// (PM-034).
    pub(crate) still: bool,
    /// The demuxer's name, e.g. `"mov,mp4,m4a,3gp,3g2,mj2"`.
    pub(crate) container: String,
    pub(crate) cancel: CancelToken,
    path: PathBuf,
    /// The analysis the classification came from; reopening uses the same.
    analysis: Analysis,
    /// No packet was read since the input was opened, so it is at the start.
    fresh: bool,
    /// The streams the demuxer returns packets for (others are discarded).
    keep: Vec<usize>,
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Source")
            .field("container", &self.container)
            .field("video", &self.video)
            .field("audio", &self.audio)
            .field("still", &self.still)
            .finish_non_exhaustive()
    }
}

/// Recognizes common non-media files from their first bytes (PM-036).
pub(crate) fn sniff(head: &[u8]) -> Option<NotMediaKind> {
    const ARCHIVES: [&[u8]; 6] = [
        b"PK\x03\x04",
        b"PK\x05\x06",
        b"Rar!\x1a\x07",
        b"7z\xbc\xaf\x27\x1c",
        b"\x1f\x8b",
        b"BZh",
    ];
    if ARCHIVES.iter().any(|m| head.starts_with(m)) {
        return Some(NotMediaKind::Archive);
    }
    if head.starts_with(b"L\0\0\0\x01\x14\x02\0") {
        return Some(NotMediaKind::Shortcut);
    }
    if head.starts_with(b"%PDF") {
        return Some(NotMediaKind::Document);
    }
    let text = head.strip_prefix(b"\xef\xbb\xbf").unwrap_or(head);
    let start = text
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(text.len());
    let text = &text[start..];
    if text.starts_with(b"#EXTM3U") || text.starts_with(b"[playlist]") {
        return Some(NotMediaKind::Playlist);
    }
    if text.starts_with(b"{")
        && text
            .windows(b"freecut".len())
            .any(|w| w.eq_ignore_ascii_case(b"freecut"))
    {
        return Some(NotMediaKind::Project);
    }
    None
}

/// True when `file` looks like an ISO-BMFF (MP4/MOV) file whose top-level
/// boxes contain no `moov`: a recording that was never finalized, or a file
/// truncated before its index.
pub(crate) fn iso_bmff_without_index(file: &mut File) -> io::Result<bool> {
    const FIRST: [&[u8; 4]; 8] = [
        b"ftyp", b"mdat", b"free", b"skip", b"wide", b"pnot", b"moov", b"styp",
    ];
    let len = file.metadata()?.len();
    let mut pos = 0u64;
    let mut first = true;
    while pos + 8 <= len {
        file.seek(SeekFrom::Start(pos))?;
        let mut header = [0u8; 16];
        let n = read_up_to(file, &mut header)?;
        if n < 8 {
            break;
        }
        let kind = [header[4], header[5], header[6], header[7]];
        if first {
            // Only a known first box identifies the file as ISO-BMFF.
            if !FIRST.iter().any(|k| **k == kind) {
                return Ok(false);
            }
            first = false;
        } else if !kind.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
            break; // garbage: the walk can't continue
        }
        if &kind == b"moov" {
            return Ok(false);
        }
        let size = match u32::from_be_bytes([header[0], header[1], header[2], header[3]]) {
            0 => break, // the box extends to the end of the file
            1 if n >= 16 => u64::from_be_bytes([
                header[8], header[9], header[10], header[11], header[12], header[13], header[14],
                header[15],
            ]),
            1 => break,
            s => u64::from(s),
        };
        if size < 8 {
            break;
        }
        pos = pos.saturating_add(size);
    }
    Ok(!first)
}

/// The demuxer that recognizes `path`'s content with confidence (a probe
/// score above [`WEAK_PROBE_SCORE`]), if any.
fn recognized_container(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut head = vec![0u8; DETECT_LEN];
    let n = read_up_to(&mut file, &mut head).ok()?;
    let hint = file_name_hint(path);
    let (name, score) = av::detect::detect(&head[..n], hint.as_deref())?;
    (score > WEAK_PROBE_SCORE).then_some(name)
}

/// Maps a failed `avformat_open_input` + `find_stream_info`.
///
/// Content no demuxer recognizes is [`ProbeError::NotMedia`]; an MP4/MOV
/// without its index is [`ProbeError::Incomplete`]; a recognized container
/// whose header is damaged or truncated is [`ProbeError::Corrupt`].
fn map_open_error(e: ffmpeg_next::Error, path: &Path, cancel: &CancelToken) -> ProbeError {
    use ffmpeg_next::Error as E;
    match e {
        E::Exit => match cancel.reason() {
            Some(CancelReason::DeadlinePassed) => ProbeError::Timeout,
            _ => ProbeError::Cancelled,
        },
        E::InvalidData | E::DemuxerNotFound | E::Eof | E::StreamNotFound => {
            let unfinished = File::open(path)
                .and_then(|mut f| iso_bmff_without_index(&mut f))
                .unwrap_or(false);
            if unfinished {
                ProbeError::Incomplete
            } else if let Some(container) = recognized_container(path) {
                ProbeError::Corrupt {
                    detail: format!("{container}: {e}"),
                }
            } else {
                ProbeError::NotMedia {
                    kind: NotMediaKind::Unrecognized,
                }
            }
        }
        E::Other { errno } => ProbeError::Io(io::Error::from_raw_os_error(errno)),
        other => ProbeError::Corrupt {
            detail: other.to_string(),
        },
    }
}

/// The file name as a format hint (NULs would make it invalid).
fn file_name_hint(path: &Path) -> Option<String> {
    let hint: String = path
        .file_name()
        .map(|n| n.to_string_lossy().replace('\0', ""))
        .unwrap_or_default();
    (!hint.is_empty()).then_some(hint)
}

fn open_input(
    path: &Path,
    file: File,
    cancel: &CancelToken,
    analysis: Analysis,
) -> Result<Input, ProbeError> {
    let io = ffmpeg_next::format::context::StreamIo::from_read_seek_with_capacity(file, IO_BUFFER)
        .map_err(|e| ProbeError::Corrupt {
            detail: format!("cannot set up file I/O: {e}"),
        })?;
    let hint = file_name_hint(path);
    let options = match analysis {
        Analysis::Default => None,
        Analysis::Extended => {
            let mut d = Dictionary::new();
            d.set("analyzeduration", &RETRY_ANALYZE_DURATION_US.to_string());
            d.set("probesize", &RETRY_PROBE_SIZE.to_string());
            Some(d)
        }
    };
    let token = cancel.clone();
    ffmpeg_next::format::input_from_stream_with_interrupt(io, hint.as_deref(), options, move || {
        token.is_cancelled()
    })
    .map_err(|e| map_open_error(e, path, cancel))
}

/// Opens `path` and classifies its streams.
///
/// Errors: missing or unreadable file → [`ProbeError::Io`]; a directory →
/// [`ProbeError::UnsupportedPath`]; 0 bytes → [`ProbeError::Empty`]; known
/// non-media or unrecognized content → [`ProbeError::NotMedia`]; MP4/MOV
/// without an index → [`ProbeError::Incomplete`]; a recognized container
/// that doesn't open → [`ProbeError::Corrupt`]; streams without a decoder in
/// this build → [`ProbeError::UnsupportedCodec`]; a container without audio
/// or video streams → [`ProbeError::NoStreams`].
///
/// A video or audio stream that `find_stream_info` left without parameters
/// (a transport stream whose first keyframe or first audio comes late)
/// triggers one more open with four times the analysis (media-io probe
/// step 4); a stream still without parameters is skipped with a warning.
pub(crate) fn open_source(path: &Path, cancel: &CancelToken) -> Result<Source, ProbeError> {
    ensure_init()?;
    let meta = std::fs::metadata(path)?;
    if meta.is_dir() {
        return Err(ProbeError::UnsupportedPath {
            reason: "it is a folder",
        });
    }
    if !meta.is_file() {
        return Err(ProbeError::UnsupportedPath {
            reason: "it is not a regular file",
        });
    }
    if meta.len() == 0 {
        return Err(ProbeError::Empty);
    }
    let mut file = File::open(path)?;
    let mut head = vec![0u8; SNIFF_LEN];
    let n = read_up_to(&mut file, &mut head)?;
    if n == 0 {
        return Err(ProbeError::Empty);
    }
    if let Some(kind) = sniff(&head[..n]) {
        return Err(ProbeError::NotMedia { kind });
    }
    file.seek(SeekFrom::Start(0))?;
    if cancel.is_cancelled() {
        return Err(cancelled_probe(cancel));
    }

    let input = open_input(path, file, cancel, Analysis::Default)?;
    let streams = classify(&input);
    if streams.missing.is_empty() {
        return into_source(input, streams, path, cancel.clone(), Analysis::Default);
    }
    tracing::debug!(
        streams = ?streams.missing,
        "streams without parameters; analyzing longer"
    );
    let retried = File::open(path)
        .map_err(ProbeError::Io)
        .and_then(|file| open_input(path, file, cancel, Analysis::Extended));
    match retried {
        Ok(wider) => {
            let more = classify(&wider);
            for index in &more.missing {
                tracing::warn!(
                    stream = index,
                    "skipping a stream FFmpeg found no parameters for"
                );
            }
            into_source(wider, more, path, cancel.clone(), Analysis::Extended)
        }
        Err(e @ (ProbeError::Cancelled | ProbeError::Timeout)) => Err(e),
        Err(e) => {
            tracing::debug!("the longer analysis failed: {e}");
            into_source(input, streams, path, cancel.clone(), Analysis::Default)
        }
    }
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(k) => filled += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

pub(crate) fn cancelled_probe(cancel: &CancelToken) -> ProbeError {
    match cancel.reason() {
        Some(CancelReason::DeadlinePassed) => ProbeError::Timeout,
        _ => ProbeError::Cancelled,
    }
}

pub(crate) fn cancelled_decode(cancel: &CancelToken) -> DecodeError {
    match cancel.reason() {
        Some(CancelReason::DeadlinePassed) => DecodeError::Timeout,
        _ => DecodeError::Cancelled,
    }
}

/// A codec's name for messages; codecs libavcodec doesn't know are named by
/// their FourCC.
pub(crate) fn codec_name(id: CodecId, tag: u32) -> String {
    if id != CodecId::NONE
        && let Some(name) = id.name()
    {
        return name;
    }
    let bytes = tag.to_le_bytes();
    if tag != 0 && bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        String::from_utf8_lossy(&bytes).trim_end().to_owned()
    } else {
        "unknown".to_owned()
    }
}

/// Demuxers that deliver one still picture, or an animation shown as its
/// first frame in the MVP (PM-034).
fn is_image_demuxer(name: &str) -> bool {
    name == "image2" || name == "gif" || name == "apng" || name.ends_with("_pipe")
}

/// The decoder for a stream with codec `id`. VP8/VP9 with Matroska
/// side-channel alpha (`alpha_mode=1`) need libvpx: FFmpeg's native decoders
/// drop the alpha plane (L9).
fn pick_decoder(input: &Input, index: usize, id: CodecId) -> Option<Codec> {
    let native = id.decoder();
    let libvpx = if id == CodecId::VP9 {
        "libvpx-vp9"
    } else if id == CodecId::VP8 {
        "libvpx"
    } else {
        return native;
    };
    let side_alpha =
        av::stream::stream_tag(input, index, c"alpha_mode").is_some_and(|v| v.trim() == "1");
    if !side_alpha {
        return native;
    }
    match decoder::find_by_name(libvpx) {
        Some(codec) => Some(codec),
        None => {
            tracing::warn!(
                stream = index,
                "{libvpx} is not in this FFmpeg build; the stream's alpha is dropped"
            );
            native
        }
    }
}

/// The streams of an opened input, sorted out.
#[derive(Default)]
struct Streams {
    video: Vec<usize>,
    audio: Vec<usize>,
    decoders: Vec<(usize, Codec)>,
    default_video: Option<usize>,
    default_audio: Option<usize>,
    /// The first audio/video stream without a decoder, by codec name.
    unsupported: Option<String>,
    /// Decodable audio/video streams without parameters (size, sample
    /// rate): `find_stream_info` saw no keyframe or no audio in time.
    missing: Vec<usize>,
}

fn classify(input: &Input) -> Streams {
    let mut found = Streams::default();
    for stream in input.streams() {
        let index = stream.index();
        let kind = av::codec::stream_kind(input, index);
        let disposition = stream.disposition();
        if kind == MediaKind::Other || disposition.contains(Disposition::ATTACHED_PIC) {
            continue;
        }
        let coded = av::stream::coded_params(input, index).unwrap_or_default();
        let id = av::codec::stream_codec(input, index);
        let Some(codec) = pick_decoder(input, index, id) else {
            found
                .unsupported
                .get_or_insert_with(|| codec_name(id, coded.codec_tag));
            continue;
        };
        let usable = match kind {
            MediaKind::Video => coded.width > 0 && coded.height > 0,
            _ => coded.sample_rate > 0 && coded.channels > 0,
        };
        if !usable {
            found.missing.push(index);
            continue;
        }
        let is_default = disposition.contains(Disposition::DEFAULT);
        found.decoders.push((index, codec));
        if kind == MediaKind::Video {
            if is_default && found.default_video.is_none() {
                found.default_video = Some(index);
            }
            found.video.push(index);
        } else {
            if is_default && found.default_audio.is_none() {
                found.default_audio = Some(index);
            }
            found.audio.push(index);
        }
    }
    found
}

fn into_source(
    input: Input,
    streams: Streams,
    path: &Path,
    cancel: CancelToken,
    analysis: Analysis,
) -> Result<Source, ProbeError> {
    let Streams {
        video,
        audio,
        decoders,
        default_video,
        default_audio,
        unsupported,
        ..
    } = streams;
    if video.is_empty() && audio.is_empty() {
        if let Some(codec) = unsupported {
            return Err(ProbeError::UnsupportedCodec { codec });
        }
        // A demuxer that claimed the data on a weak guess and found nothing
        // usable means the content isn't media at all; a container it
        // recognized for sure simply has no audio or video (subtitles only,
        // cover art only).
        if input.probe_score() <= WEAK_PROBE_SCORE {
            return Err(ProbeError::NotMedia {
                kind: NotMediaKind::Unrecognized,
            });
        }
        return Err(ProbeError::NoStreams);
    }
    let default_video = default_video.or_else(|| video.first().copied());
    let default_audio = default_audio.or_else(|| audio.first().copied());
    let container = input.format().name().to_owned();
    let still = video.len() == 1 && audio.is_empty() && is_image_demuxer(&container);
    Ok(Source {
        input,
        video,
        audio,
        decoders,
        default_video,
        default_audio,
        still,
        container,
        cancel,
        path: path.to_path_buf(),
        analysis,
        fresh: true,
        keep: Vec::new(),
    })
}

impl Source {
    /// The stream's time base as `(num, den)`.
    pub(crate) fn time_base(&self, stream: usize) -> (i32, i32) {
        self.input
            .stream(stream)
            .map(|s| {
                let tb = s.time_base();
                (tb.numerator(), tb.denominator())
            })
            .unwrap_or((0, 1))
    }

    /// Checks that `index` is a usable stream of `kind`.
    pub(crate) fn require(&self, index: usize, kind: StreamKind) -> Result<(), DecodeError> {
        let list = match kind {
            StreamKind::Video => &self.video,
            StreamKind::Audio => &self.audio,
        };
        if list.contains(&index) {
            Ok(())
        } else {
            Err(DecodeError::NoSuchStream { index, kind })
        }
    }

    /// Makes the demuxer return packets of `streams` only (L7).
    pub(crate) fn keep_only(&mut self, streams: &[usize]) {
        self.keep = streams.to_vec();
        av::stream::keep_only_streams(&mut self.input, &self.keep);
    }

    /// Repositions the demuxer at the very start of the file.
    ///
    /// Seeks with a forward search from the smallest timestamp, which also
    /// works when the first keyframe has a negative or late timestamp (L24).
    /// Demuxers that can't seek (raw streams, image pipes) are reopened.
    pub(crate) fn rewind(&mut self, stream: usize) -> Result<(), DecodeError> {
        if self.fresh {
            return Ok(());
        }
        // Not i64::MIN: demuxers offset the target (mov subtracts its DTS
        // shift), which would overflow and land at the end of the file.
        if av::stream::seek_file(&mut self.input, stream, REWIND_TS, REWIND_TS, -REWIND_TS).is_ok()
        {
            return Ok(());
        }
        self.reopen()
    }

    /// Opens the file again from scratch, keeping the classification.
    fn reopen(&mut self) -> Result<(), DecodeError> {
        let file = File::open(&self.path).map_err(DecodeError::Io)?;
        self.input = open_input(&self.path, file, &self.cancel, self.analysis)?;
        self.fresh = true;
        av::stream::keep_only_streams(&mut self.input, &self.keep);
        Ok(())
    }

    /// Repositions the demuxer at the last keyframe of `stream` whose
    /// timestamp is `<= ts`; falls back to [`Source::rewind`] when there is
    /// none (the target is before the first keyframe).
    pub(crate) fn seek_at_or_before(&mut self, stream: usize, ts: i64) -> Result<(), DecodeError> {
        // Clamped like REWIND_TS so demuxer offsets can't overflow.
        let ts = ts.clamp(REWIND_TS, -REWIND_TS);
        match av::stream::seek_file(&mut self.input, stream, REWIND_TS, ts, ts) {
            Ok(()) => Ok(()),
            Err(_) => self.rewind(stream),
        }
    }

    /// Reads the next packet of `stream`, skipping others. `Ok(None)` at the
    /// end of the file; I/O errors are reported, not mistaken for the end.
    pub(crate) fn next_packet(&mut self, stream: usize) -> Result<Option<Packet>, DecodeError> {
        loop {
            match self.read_packet()? {
                Some(p) if p.stream() == stream => return Ok(Some(p)),
                Some(_) => {}
                None => return Ok(None),
            }
        }
    }

    /// Reads the next packet of any kept stream (see [`Source::keep_only`]).
    pub(crate) fn read_packet(&mut self) -> Result<Option<Packet>, DecodeError> {
        let mut failures = 0usize;
        loop {
            if self.cancel.is_cancelled() {
                return Err(cancelled_decode(&self.cancel));
            }
            self.fresh = false;
            let mut packet = Packet::empty();
            let err = match packet.read(&mut self.input) {
                Ok(()) if self.keep.is_empty() || self.keep.contains(&packet.stream()) => {
                    return Ok(Some(packet));
                }
                Ok(()) => continue,
                Err(ffmpeg_next::Error::Eof) => return Ok(None),
                Err(ffmpeg_next::Error::Exit) => return Err(cancelled_decode(&self.cancel)),
                Err(e) => e,
            };
            // EAGAIN and damaged packets are retried (demuxers resync on the
            // next read), within reason.
            let retry = matches!(err, ffmpeg_next::Error::InvalidData)
                || err
                    == (ffmpeg_next::Error::Other {
                        errno: ffmpeg_next::error::EAGAIN,
                    });
            failures += 1;
            if !retry || failures > MAX_READ_RETRIES {
                return Err(av::ff_error("av_read_frame", err));
            }
            tracing::debug!("retrying a failed packet read: {err}");
        }
    }

    /// Opens the decoder classification chose for `stream` (see
    /// [`av::codec::open_decoder`] for `threads`).
    fn open_decoder(
        &self,
        stream: usize,
        kind: StreamKind,
        threads: usize,
    ) -> Result<decoder::Opened, DecodeError> {
        let codec = self
            .decoders
            .iter()
            .find(|(index, _)| *index == stream)
            .map(|(_, codec)| *codec)
            .ok_or(DecodeError::NoSuchStream {
                index: stream,
                kind,
            })?;
        av::codec::open_decoder(&self.input, stream, codec, threads)
            .map_err(|e| av::ff_error("avcodec_open2", e))
    }

    /// Opens a video decoder for `stream`. `threads == 1` decodes on the
    /// calling thread (probe, origin); `0` lets FFmpeg pick frame threads.
    pub(crate) fn video_decoder(
        &self,
        stream: usize,
        threads: usize,
    ) -> Result<decoder::Video, DecodeError> {
        self.require(stream, StreamKind::Video)?;
        Ok(decoder::Video(self.open_decoder(
            stream,
            StreamKind::Video,
            threads,
        )?))
    }

    /// Opens an audio decoder for `stream`, decoding on the calling thread.
    pub(crate) fn audio_decoder(&self, stream: usize) -> Result<decoder::Audio, DecodeError> {
        self.require(stream, StreamKind::Audio)?;
        Ok(decoder::Audio(self.open_decoder(
            stream,
            StreamKind::Audio,
            1,
        )?))
    }

    /// The codec name of `stream` for [`MediaInfo`](freecut_timeline::model::MediaInfo).
    pub(crate) fn codec_name(&self, stream: usize) -> String {
        let tag = av::stream::coded_params(&self.input, stream)
            .unwrap_or_default()
            .codec_tag;
        codec_name(av::codec::stream_codec(&self.input, stream), tag)
    }

    /// The best-effort timestamp of the first frame `stream` decodes to when
    /// read from the start of the file. Leaves the demuxer anywhere; callers
    /// reposition it.
    pub(crate) fn first_frame_ts(&mut self, stream: usize) -> Result<Option<i64>, DecodeError> {
        self.keep_only(&[stream]);
        self.rewind(stream)?;
        let kind = if self.video.contains(&stream) {
            StreamKind::Video
        } else {
            StreamKind::Audio
        };
        self.require(stream, kind)?;
        let mut opened = self.open_decoder(stream, kind, 1)?;
        let mut frame = frame::Video::empty();
        let mut packets = 0usize;
        let mut eof = false;
        loop {
            match opened.receive_frame(&mut frame) {
                Ok(()) => return Ok(frame.timestamp().or_else(|| frame.pts()).or(Some(0))),
                Err(ffmpeg_next::Error::Eof) => return Ok(None),
                Err(_) if eof => return Ok(None),
                Err(_) => {}
            }
            match self.next_packet(stream)? {
                Some(p) => {
                    packets += 1;
                    if packets > FIRST_FRAME_PACKET_LIMIT {
                        return Ok(None);
                    }
                    // Undecodable packets are skipped; the limit bounds it.
                    let _ = opened.send_packet(&p);
                }
                None => {
                    eof = true;
                    let _ = opened.send_eof();
                }
            }
        }
    }

    /// Media time 0 as an absolute tick time of the file (I2): the first
    /// presentable frame of the default video stream, or the first decoded
    /// sample of the default audio stream when there is no video.
    pub(crate) fn origin(&mut self) -> Result<Tick, DecodeError> {
        let Some(stream) = self.default_video.or(self.default_audio) else {
            return Err(DecodeError::Open(ProbeError::NoStreams));
        };
        let ts = self
            .first_frame_ts(stream)?
            .ok_or(DecodeError::Corrupt { at: Tick::ZERO })?;
        let (num, den) = self.time_base(stream);
        let tb = freecut_timeline::time::StreamTimeBase::new(num, den)?;
        Ok(ticks_from_stream(ts, tb)?)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;
    use std::io::Write;

    #[test]
    fn sniffing_known_non_media() {
        assert_eq!(sniff(b"PK\x03\x04rest"), Some(NotMediaKind::Archive));
        assert_eq!(sniff(b"#EXTM3U\n#EXTINF"), Some(NotMediaKind::Playlist));
        assert_eq!(
            sniff(b"\xef\xbb\xbf  #EXTM3U"),
            Some(NotMediaKind::Playlist)
        );
        assert_eq!(sniff(b"%PDF-1.7"), Some(NotMediaKind::Document));
        assert_eq!(
            sniff(b"L\0\0\0\x01\x14\x02\0\0\0"),
            Some(NotMediaKind::Shortcut)
        );
        assert_eq!(
            sniff(br#"{ "format": "FreeCut", "version": 1 }"#),
            Some(NotMediaKind::Project)
        );
        assert_eq!(sniff(br#"{ "other": 1 }"#), None);
        assert_eq!(sniff(b"\0\0\0\x20ftypisom"), None);
        assert_eq!(sniff(b""), None);
    }

    fn boxed(kind: &[u8; 4], payload: usize) -> Vec<u8> {
        let mut v = u32::try_from(payload + 8).unwrap().to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.resize(payload + 8, 0);
        v
    }

    fn file_with(bytes: &[u8]) -> (tempfile::TempDir, File) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.mp4");
        File::create(&p).unwrap().write_all(bytes).unwrap();
        (dir, File::open(&p).unwrap())
    }

    #[test]
    fn mp4_without_moov_is_detected() {
        let mut bytes = boxed(b"ftyp", 16);
        let mut mdat = boxed(b"mdat", 1000);
        mdat.truncate(500); // cut mid-box, before any moov
        bytes.extend_from_slice(&mdat);
        let (_d, mut f) = file_with(&bytes);
        assert!(iso_bmff_without_index(&mut f).unwrap());

        let mut complete = boxed(b"ftyp", 16);
        complete.extend_from_slice(&boxed(b"mdat", 100));
        complete.extend_from_slice(&boxed(b"moov", 50));
        let (_d, mut f) = file_with(&complete);
        assert!(!iso_bmff_without_index(&mut f).unwrap());

        let (_d, mut f) = file_with(b"random bytes that are not boxes at all");
        assert!(!iso_bmff_without_index(&mut f).unwrap());
    }
}
