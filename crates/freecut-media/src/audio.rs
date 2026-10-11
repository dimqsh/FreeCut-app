//! Sample-accurate audio reading (AU-001, AU-002) and waveform peaks
//! (PM-072, AU-009).
//!
//! Decoded frames are placed on the stream's native sample grid by their
//! presentation timestamps, converted through media time (so audio keeps its
//! offset to the picture, I2). Inside a contiguous run the reader trusts
//! sample counting and re-anchors on the timestamp only when the two
//! disagree by more than 20 ms or two frames (TS/AAC timestamp jitter).
//! Priming and edit lists are left to libavcodec/libavformat's defaults.
//!
//! [`AudioReader`] converts to interleaved `f32` at the requested rate and
//! channel count. Output sample `n` covers media time `n / out_rate`; samples
//! before the stream's first sample and after its last are zero.
//!
//! Random reads equal the same span of a sequential read:
//! - at the native rate and channel count, samples bypass libswresample, so
//!   PCM is bit-exact;
//! - otherwise a fresh resampler starts on an input sample that is a whole
//!   number of resampling periods (`in_rate / gcd`), so its output phases
//!   match the ones a sequential pass uses, and enough input before the
//!   target (500 ms) is decoded and discarded for the decoder and the
//!   filter history to converge. Results then agree to float rounding.
//!   Codecs whose decoder state is not a function of recent input can
//!   differ in noise-filled parts: AAC perceptual noise substitution and
//!   AC-3 mantissa dither (about 2e-5) draw from generators a seek resets.
//!
//! Timestamps coarser than a sample (Matroska and WebM store milliseconds)
//! are up to half a unit off the counted position, so a frame reached by
//! seeking is snapped onto the lattice of frame starts a sequential pass
//! counts (`first + len0 + k · frame length`) when the codec's frames have a
//! fixed length (AAC, MP3, Opus, AC-3). Streams with coarse timestamps and
//! frames of varying length (Vorbis) can't be placed after a seek and are
//! read forward only, like the streams below.
//!
//! Streams whose post-seek timestamps are estimates (raw MP3, ADTS AAC, Ogg)
//! are never seeked (C11): going backwards rewinds and decodes forward, and
//! going forward keeps decoding from where the decoder is.

use std::path::Path;

use ffmpeg_next::decoder;
use ffmpeg_next::frame;
use freecut_timeline::time::{SampleRate, TICKS_PER_SECOND, Tick};

use crate::av;
use crate::av::codec::CodecId;
use crate::av::samples::{SampleType, SamplesError, audio_samples};
use crate::av::swr::Resampler;
use crate::cancel::CancelToken;
use crate::error::{DecodeError, StreamKind};
use crate::source::{Source, open_source};
use crate::time::{
    StreamClock, rescale_ceil, rescale_floor, sample_floor, sample_nearest, sample_time,
};

/// Input decoded and discarded before a random-access target: 1/2 s. Opus
/// (CELT) predicts band energies from the previous frame, so a decoder
/// flushed by a seek converges by about half per 20 ms frame; 500 ms brings
/// it to float rounding (200 ms left differences near 4e-5).
const PREROLL_SECONDS_DIV: u32 = 2;

/// Zero input fed after the end so the resampler's filter flushes.
const TAIL_ZEROS: usize = 8192;

/// Forward distance (output samples, in seconds) decoded rather than seeked.
const FORWARD_SECONDS: i64 = 5;

/// Consecutive decode errors tolerated.
const MAX_DECODE_ERRORS: usize = 10;

/// Timestamp jumps beyond this many seconds are treated as broken
/// timestamps (sample counting continues) rather than gaps to fill with
/// silence, which keeps memory bounded on damaged files.
const MAX_GAP_SECONDS: i64 = 30;

/// Demuxers whose post-seek timestamps are estimates (C11).
fn seek_unreliable(container: &str) -> bool {
    matches!(container, "mp3" | "aac" | "ogg" | "ac3" | "eac3" | "mp2")
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// Appends the samples of frame `f`, converted to `f32`, to `planes` (one
/// per channel; extra planes get zeros, extra frame channels are dropped).
/// Non-finite floats become 0. Returns the number of samples per channel.
fn convert_frame(f: &frame::Audio, planes: &mut [Vec<f32>]) -> Result<usize, DecodeError> {
    let view = audio_samples(f).map_err(|e| DecodeError::Ffmpeg {
        op: "decode audio",
        code: 0,
        msg: match e {
            SamplesError::UnknownFormat(raw) => format!("unsupported sample format {raw}"),
            SamplesError::Inconsistent => "a decoded frame is smaller than it says".to_owned(),
        },
    })?;
    let (n, channels, bytes) = (view.samples, view.channels, view.kind.bytes());
    if n == 0 || channels == 0 {
        return Ok(0);
    }
    let kind = view.kind;
    let read = |data: &[u8], i: usize| -> f32 {
        let s = &data[i * bytes..(i + 1) * bytes];
        match kind {
            SampleType::U8 => (f32::from(s[0]) - 128.0) / 128.0,
            SampleType::I16 => f32::from(i16::from_ne_bytes([s[0], s[1]])) / 32_768.0,
            SampleType::I32 => {
                (f64::from(i32::from_ne_bytes([s[0], s[1], s[2], s[3]])) / 2_147_483_648.0) as f32
            }
            SampleType::I64 => {
                let v = i64::from_ne_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]);
                (v as f64 / 9_223_372_036_854_775_808.0) as f32
            }
            SampleType::F32 => {
                let v = f32::from_ne_bytes([s[0], s[1], s[2], s[3]]);
                if v.is_finite() { v } else { 0.0 }
            }
            SampleType::F64 => {
                let v = f64::from_ne_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]);
                if v.is_finite() { v as f32 } else { 0.0 }
            }
        }
    };
    for (ch, plane) in planes.iter_mut().enumerate() {
        if ch >= channels {
            plane.extend(std::iter::repeat_n(0.0, n));
        } else if view.planar {
            let data = view.planes[ch];
            plane.extend((0..n).map(|i| read(data, i)));
        } else {
            let data = view.planes[0];
            plane.extend((0..n).map(|i| read(data, i * channels + ch)));
        }
    }
    Ok(n)
}

/// How frames are placed on the sample grid when nothing has been counted
/// yet (after opening, a rewind or a seek).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grid {
    /// Timestamps resolve single samples (MP4's `1/rate`, TS's `1/90000`):
    /// a frame starts at the sample its timestamp names.
    Exact,
    /// Timestamps are coarser than a sample (Matroska's 1 ms), and frames
    /// have one length `step` (learned from the stream's second frame, or
    /// from the first frame reached by a seek). A sequential pass puts the
    /// stream's first frame at `first` and every later one on
    /// `base + k · step`; a timestamp within `slack` samples of that lattice
    /// is snapped onto it, so random reads equal sequential ones.
    Lattice {
        first: i64,
        base: i64,
        step: Option<i64>,
        slack: i64,
    },
    /// Coarse timestamps and frames of varying length: the position of a
    /// frame reached by seeking can't be recovered, so the stream is read
    /// forward from its start only.
    Unreliable,
}

/// Decodes one audio stream and places its frames on the native sample grid.
struct StreamDecoder {
    src: Source,
    stream: usize,
    decoder: decoder::Audio,
    clock: StreamClock,
    rate: u32,
    channels: u16,
    /// The speaker of each channel (FFmpeg `AVChannel` numbers), when the
    /// file says; it decides the stereo downmix (AU-010).
    channel_ids: Option<Vec<i32>>,
    /// Where counting says the next frame starts (None after a reset).
    expect: Option<i64>,
    sent_eof: bool,
    errors: usize,
    /// Planar samples of the last decoded frame.
    planes: Vec<Vec<f32>>,
    grid: Grid,
    /// Set when decoding found the grid to be [`Grid::Unreliable`]; taken
    /// by the reader, which then redoes a read that relied on a seek.
    grid_broke: bool,
    /// Every frame since the stream's first was decoded in order (no seek
    /// since the last rewind), so positions were counted from the start.
    from_start: bool,
}

impl StreamDecoder {
    /// Opens `stream` and decodes its first frame. Returns the decoder (with
    /// that frame in `planes`) and the frame's first sample index.
    fn open(
        path: &Path,
        stream: usize,
        cancel: &CancelToken,
    ) -> Result<(StreamDecoder, i64, usize), DecodeError> {
        let mut src = open_source(path, cancel)?;
        src.require(stream, StreamKind::Audio)?;
        let origin = src.origin()?;
        src.keep_only(&[stream]);
        src.rewind(stream)?;
        let decoder = src.audio_decoder(stream)?;
        let (num, den) = src.time_base(stream);
        let coded = av::stream::coded_params(&src.input, stream).unwrap_or_default();
        let codec = av::codec::stream_codec(&src.input, stream);
        let channel_ids = av::layout::stream_channel_ids(&src.input, stream);
        let mut this = StreamDecoder {
            src,
            stream,
            decoder,
            clock: StreamClock::new(num, den, origin)?,
            rate: u32::try_from(coded.sample_rate).unwrap_or(0),
            channels: u16::try_from(coded.channels).unwrap_or(0),
            channel_ids,
            expect: None,
            sent_eof: false,
            errors: 0,
            planes: Vec::new(),
            grid: Grid::Exact,
            grid_broke: false,
            from_start: true,
        };
        let Some((first, len)) = this.next_frame(true)? else {
            return Err(DecodeError::Corrupt { at: Tick::ZERO });
        };
        this.grid = this.initial_grid(first, len, codec);
        Ok((this, first, len))
    }

    /// The grid for a stream whose first frame is `[first, first + len)`.
    fn initial_grid(&self, first: i64, len: usize, codec: CodecId) -> Grid {
        let tb = self.clock.time_base();
        // One time-base unit in samples is `num * rate / den`.
        let units = i64::from(tb.num()) * i64::from(self.rate);
        let den = i64::from(tb.den());
        if units <= den {
            return Grid::Exact;
        }
        if codec == CodecId::VORBIS {
            // Vorbis frames follow its block sizes (128 to 2048 samples).
            return Grid::Unreliable;
        }
        Grid::Lattice {
            first,
            base: first + len as i64,
            step: None,
            // Half a unit of muxer rounding, the same again for a decoder's
            // own timestamp arithmetic, and our nearest-sample rounding.
            slack: (units + den - 1) / den + 1,
        }
    }

    /// True when the stream's frames can be placed after a seek.
    fn grid_allows_seeking(&self) -> bool {
        self.grid != Grid::Unreliable
    }

    /// Whether the grid broke since the last call.
    fn take_grid_broke(&mut self) -> bool {
        std::mem::take(&mut self.grid_broke)
    }

    fn reset(&mut self) {
        self.decoder.flush();
        self.expect = None;
        self.sent_eof = false;
        self.errors = 0;
    }

    /// Decodes the next frame into `planes`; returns its first sample index
    /// and length, or `None` at the end of the stream. With `adopt_format`,
    /// the frame's rate and channel count become the stream's (first frame).
    fn next_frame(&mut self, adopt_format: bool) -> Result<Option<(i64, usize)>, DecodeError> {
        loop {
            let mut f = frame::Audio::empty();
            match self.decoder.receive_frame(&mut f) {
                Ok(()) => {
                    self.errors = 0;
                    if adopt_format {
                        self.rate = f.rate();
                        self.channels = f.channels();
                    }
                    if f.samples() == 0 {
                        continue;
                    }
                    if f.rate() != self.rate || self.rate == 0 || self.channels == 0 {
                        return Err(DecodeError::Ffmpeg {
                            op: "decode audio",
                            code: 0,
                            msg: format!(
                                "the sample rate changed mid-stream ({} → {} Hz)",
                                self.rate,
                                f.rate()
                            ),
                        });
                    }
                    return self.place(&f).map(Some);
                }
                Err(ffmpeg_next::Error::Eof) => return Ok(None),
                Err(ffmpeg_next::Error::Other { errno }) if errno == ffmpeg_next::error::EAGAIN => {
                    if self.sent_eof {
                        return Ok(None);
                    }
                }
                Err(e) => self.count_error("avcodec_receive_frame", e)?,
            }
            match self.src.next_packet(self.stream)? {
                Some(p) => {
                    if let Err(e) = self.decoder.send_packet(&p) {
                        self.count_error("avcodec_send_packet", e)?;
                    }
                }
                None => {
                    if let Err(e) = self.decoder.send_eof() {
                        tracing::debug!("send_eof: {e}");
                    }
                    self.sent_eof = true;
                }
            }
        }
    }

    fn count_error(&mut self, op: &'static str, e: ffmpeg_next::Error) -> Result<(), DecodeError> {
        self.errors += 1;
        tracing::debug!("{op}: {e}");
        if self.errors > MAX_DECODE_ERRORS {
            Err(av::ff_error(op, e))
        } else {
            Ok(())
        }
    }

    /// Converts `f` into `planes` and anchors it on the sample grid.
    fn place(&mut self, f: &frame::Audio) -> Result<(i64, usize), DecodeError> {
        let channels = usize::from(self.channels);
        self.planes.resize_with(channels, Vec::new);
        for p in &mut self.planes {
            p.clear();
        }
        let mut n = convert_frame(f, &mut self.planes)?;
        // An edit list that ends inside the last frame shortens that packet's
        // duration but not the decoded frame (encoder padding). Trim it when
        // durations are sample-accurate (time base 1/rate, as in MP4/MOV).
        let tb = self.clock.time_base();
        let duration = f.packet().duration;
        if tb.num() == 1
            && i64::from(tb.den()) == i64::from(self.rate)
            && let Ok(d) = usize::try_from(duration)
            && d > 0
            && d < n
        {
            n = d;
            self.planes.iter_mut().for_each(|p| p.truncate(d));
        }
        let by_pts = match f.timestamp().or_else(|| f.pts()) {
            Some(ts) => Some(sample_nearest(self.clock.media_time(ts)?, self.rate)),
            None => None,
        };
        let tolerance = i64::from(self.rate / 50).max(2 * n as i64);
        let max_gap = i64::from(self.rate) * MAX_GAP_SECONDS;
        let first = match (self.expect, by_pts) {
            (Some(e), Some(p)) if (p - e).abs() <= tolerance => self.counted(e, n),
            (Some(e), Some(p)) if p - e > max_gap => {
                tracing::warn!(
                    expected = e,
                    timestamp = p,
                    "ignoring an implausible audio timestamp jump"
                );
                self.counted(e, n)
            }
            (_, Some(p)) => {
                if let Some(e) = self.expect {
                    tracing::debug!(
                        expected = e,
                        timestamp = p,
                        "audio re-anchored on its timestamp"
                    );
                }
                self.anchored(p, n)
            }
            (Some(e), None) => self.counted(e, n),
            (None, None) => 0,
        };
        self.expect = Some(first + n as i64);
        Ok((first, n))
    }

    /// A frame of `n` samples placed at `e` by counting: checks it against
    /// the lattice (the second frame of the stream sets its step).
    fn counted(&mut self, e: i64, n: usize) -> i64 {
        if let Grid::Lattice { base, step, .. } = &mut self.grid {
            match *step {
                None if e == *base => *step = Some(n as i64),
                Some(s) if (e - *base).rem_euclid(s) != 0 => {
                    tracing::debug!(
                        stream = self.stream,
                        "audio frames vary in length; reading forward only"
                    );
                    self.grid = Grid::Unreliable;
                    self.grid_broke = true;
                }
                _ => {}
            }
        }
        e
    }

    /// A frame of `n` samples whose timestamp says it starts at `p`, with
    /// nothing counted before it: snapped onto the lattice when there is one.
    fn anchored(&mut self, p: i64, n: usize) -> i64 {
        let Grid::Lattice {
            first,
            base,
            step,
            slack,
        } = &mut self.grid
        else {
            return p;
        };
        if p == *first {
            // The stream's first frame again (after a rewind).
            return p;
        }
        let s = *step.get_or_insert(n as i64);
        if s <= 2 * *slack {
            // Frames shorter than the timestamps' uncertainty: nothing to snap to.
            return p;
        }
        let on = *base + (p - *base + s / 2).div_euclid(s) * s;
        if on >= *base && (on - p).abs() <= *slack {
            on
        } else {
            p
        }
    }

    /// Repositions the demuxer so decoding resumes at or before input
    /// sample `m`. Returns whether it seeked (rather than rewinding to the
    /// start of the stream).
    fn seek_to_sample(
        &mut self,
        m: i64,
        first_in: i64,
        seekable: bool,
    ) -> Result<bool, DecodeError> {
        self.reset();
        if !seekable || m <= first_in {
            // From the start, decode with a new decoder: a flushed one keeps
            // state (AAC's previous window shape) that changes the first
            // frame, which no preroll can hide at the start of the stream.
            self.src.rewind(self.stream)?;
            self.decoder = self.src.audio_decoder(self.stream)?;
            self.from_start = true;
            return Ok(false);
        }
        let t = sample_time(m, self.rate);
        let ts = self.clock.ts_at_or_before(t)?;
        self.from_start = false;
        self.src.seek_at_or_before(self.stream, ts)?;
        Ok(true)
    }
}

/// Output being produced from a position: decoded input → resampler →
/// interleaved output samples `[out_start, out_start + len)`.
struct Pipe {
    resampler: Option<Resampler>,
    /// Next input sample index the pipe expects.
    next_in: i64,
    /// Output index of `out[0]`.
    out_start: i64,
    out: Vec<f32>,
    /// One past the last real (decoded) input sample fed so far.
    last_real: Option<i64>,
    /// The stream ended and the filter tail was flushed.
    done: bool,
    /// Positioned by a seek (not from the stream's start or by decoding on).
    from_seek: bool,
}

/// Reads one audio stream as interleaved `f32` at a chosen rate and channel
/// count, at any sample position.
pub struct AudioReader {
    dec: StreamDecoder,
    out_rate: u32,
    out_channels: u16,
    /// Resampling period: `step_in` input samples ↔ `step_out` output ones.
    step_in: i64,
    step_out: i64,
    /// First input sample of the stream.
    first_in: i64,
    /// One past the last input sample, once the end has been decoded.
    end_in: Option<i64>,
    /// The demuxer's timestamps after a seek are exact (C11).
    container_seekable: bool,
    pipe: Option<Pipe>,
}

impl std::fmt::Debug for AudioReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioReader")
            .field("source", &self.dec.src)
            .field("stream", &self.dec.stream)
            .field("in_rate", &self.dec.rate)
            .field("out_rate", &self.out_rate)
            .field("out_channels", &self.out_channels)
            .finish_non_exhaustive()
    }
}

impl AudioReader {
    /// Opens audio stream `stream_index` of `path` for output at `out_rate`
    /// with `out_channels` channels.
    ///
    /// Stereo output follows AU-010: mono is copied to both channels at
    /// unity gain, and more than two channels are downmixed with the ITU
    /// coefficients (front left and right at unity, centre and surrounds at
    /// −3 dB, LFE dropped), by the speakers the stream's channel layout names
    /// (FFmpeg's default layout for the count when it names none). Other
    /// output counts use libswresample's default matrices.
    pub fn open(
        path: &Path,
        stream_index: usize,
        out_rate: SampleRate,
        out_channels: u16,
    ) -> Result<AudioReader, DecodeError> {
        if out_channels == 0 {
            return Err(DecodeError::InvalidArgument(
                "out_channels must be at least 1",
            ));
        }
        let (dec, first_in, _) = StreamDecoder::open(path, stream_index, &CancelToken::new())?;
        let g = gcd(u64::from(dec.rate), u64::from(out_rate.hz()));
        let container_seekable = !seek_unreliable(&dec.src.container);
        Ok(AudioReader {
            step_in: i64::try_from(u64::from(dec.rate) / g).unwrap_or(1),
            step_out: i64::try_from(u64::from(out_rate.hz()) / g).unwrap_or(1),
            out_rate: out_rate.hz(),
            out_channels,
            first_in,
            end_in: None,
            container_seekable,
            dec,
            pipe: None,
        })
    }

    /// The stream's native sample rate.
    pub fn native_rate(&self) -> u32 {
        self.dec.rate
    }

    /// The stream's native channel count.
    pub fn native_channels(&self) -> u16 {
        self.dec.channels
    }

    /// Reads `frames` output frames (`frames * out_channels` interleaved
    /// samples) starting at output sample `floor(start * out_rate / TPS)`.
    ///
    /// A `frames` whose sample count overflows or can't be allocated is
    /// [`DecodeError::InvalidArgument`].
    pub fn read(&mut self, start: Tick, frames: usize) -> Result<Vec<f32>, DecodeError> {
        let out = self.read_once(start, frames)?;
        if self.dec.take_grid_broke() && self.pipe.as_ref().is_some_and(|p| p.from_seek) {
            // This read placed audio after a seek in a stream that turned out
            // to be readable forward only: read it again from the start.
            self.pipe = None;
            return self.read_once(start, frames);
        }
        Ok(out)
    }

    fn read_once(&mut self, start: Tick, frames: usize) -> Result<Vec<f32>, DecodeError> {
        let ch = usize::from(self.out_channels);
        let too_large = DecodeError::InvalidArgument("frames is too large");
        let len = frames.checked_mul(ch).ok_or(too_large)?;
        let mut out = Vec::new();
        out.try_reserve_exact(len)
            .map_err(|_| DecodeError::InvalidArgument("frames is too large"))?;
        out.resize(len, 0.0f32);
        if frames == 0 {
            return Ok(out);
        }
        let n0 = sample_floor(start, self.out_rate);
        let n1 = n0.saturating_add(i64::try_from(frames).unwrap_or(i64::MAX));
        let o_first = rescale_ceil(self.first_in, self.dec.rate, self.out_rate);
        if n1 <= o_first || self.o_end().is_some_and(|end| n0 >= end) {
            return Ok(out);
        }
        let from = n0.max(o_first);
        self.ensure_positioned(from)?;
        self.produce(n1)?;
        let o_end = self.o_end().unwrap_or(i64::MAX);
        let Some(pipe) = self.pipe.as_mut() else {
            return Ok(out);
        };
        let avail_end = pipe.out_start + (pipe.out.len() / ch) as i64;
        let lo = from.max(pipe.out_start);
        let hi = n1.min(avail_end).min(o_end);
        if lo < hi {
            let src = ((lo - pipe.out_start) as usize) * ch;
            let dst = ((lo - n0) as usize) * ch;
            let len = ((hi - lo) as usize) * ch;
            out[dst..dst + len].copy_from_slice(&pipe.out[src..src + len]);
        }
        // Keep the span just read, so reading it again is free.
        if n0 > pipe.out_start {
            let drop = (((n0 - pipe.out_start) as usize) * ch).min(pipe.out.len());
            pipe.out.drain(..drop);
            pipe.out_start += (drop / ch) as i64;
        }
        Ok(out)
    }

    /// Whether positions after a seek are known: the demuxer's timestamps
    /// are exact (C11) and the stream's frames can be placed (see the module
    /// docs).
    fn seekable(&self) -> bool {
        self.container_seekable && self.dec.grid_allows_seeking()
    }

    /// One past the last output sample carrying stream audio, once known.
    fn o_end(&self) -> Option<i64> {
        self.end_in
            .map(|end| rescale_ceil(end, self.dec.rate, self.out_rate))
    }

    fn needs_resampler(&self) -> bool {
        self.dec.rate != self.out_rate || self.dec.channels != self.out_channels
    }

    fn ensure_positioned(&mut self, from: i64) -> Result<(), DecodeError> {
        let ch = i64::from(self.out_channels);
        let limit = FORWARD_SECONDS * i64::from(self.out_rate);
        let ok = self.pipe.as_ref().is_some_and(|p| {
            let end = p.out_start + p.out.len() as i64 / ch;
            p.out_start <= from && (from <= end.saturating_add(limit) || p.done)
        });
        if ok { Ok(()) } else { self.reposition(from) }
    }

    /// A pipe whose input starts at sample `m_s` (output `o_s`).
    fn new_pipe(&self, m_s: i64, o_s: i64, from_seek: bool) -> Result<Pipe, DecodeError> {
        let resampler = if self.needs_resampler() {
            Some(
                Resampler::new(
                    self.dec.channels,
                    self.dec.rate,
                    self.out_channels,
                    self.out_rate,
                    self.dec.channel_ids.as_deref(),
                )
                .map_err(|e| av::ff_error("swr_init", e))?,
            )
        } else {
            None
        };
        Ok(Pipe {
            resampler,
            next_in: m_s,
            out_start: o_s,
            out: Vec::new(),
            last_real: None,
            done: false,
            from_seek,
        })
    }

    /// Starts a new pipe whose output covers `o_target`.
    fn reposition(&mut self, o_target: i64) -> Result<(), DecodeError> {
        let rate = self.dec.rate;
        let m_target = rescale_floor(o_target, self.out_rate, rate);
        let preroll = i64::from(rate / PREROLL_SECONDS_DIV);
        let start = |back: i64| {
            let want = m_target.saturating_sub(preroll).saturating_sub(back);
            let m_s = want.div_euclid(self.step_in) * self.step_in;
            (m_s, m_s / self.step_in * self.step_out)
        };
        let seekable = self.seekable();
        if !seekable {
            // Read forward only: when the decoder has decoded continuously
            // from the start and is still before the new start, keep
            // decoding from where it is instead of rewinding; its output is
            // what a rewind would produce.
            let (m_s, o_s) = start(0);
            if self.dec.from_start
                && !self.dec.sent_eof
                && self.dec.expect.is_some_and(|next| next <= m_s)
            {
                self.pipe = Some(self.new_pipe(m_s, o_s, false)?);
                return Ok(());
            }
        }
        let mut back = 0i64;
        loop {
            let (m_s, o_s) = start(back);
            let seeked = self.dec.seek_to_sample(m_s, self.first_in, seekable)?;
            let mut pipe = self.new_pipe(m_s, o_s, seeked)?;
            match self.dec.next_frame(false)? {
                Some((f0, len)) => {
                    let at_start = f0 <= self.first_in;
                    if f0 > m_s && !at_start && seekable {
                        // Landed after the wanted position (L4): go further back.
                        back = (back * 2).max(i64::from(rate));
                        continue;
                    }
                    self.feed(&mut pipe, f0, len)?;
                }
                None => self.finish(&mut pipe)?,
            }
            self.pipe = Some(pipe);
            return Ok(());
        }
    }

    /// Decodes until output reaches `until` or the stream ends.
    fn produce(&mut self, until: i64) -> Result<(), DecodeError> {
        let ch = i64::from(self.out_channels);
        let Some(mut pipe) = self.pipe.take() else {
            return Ok(());
        };
        let result = (|| {
            while pipe.out_start + pipe.out.len() as i64 / ch < until && !pipe.done {
                match self.dec.next_frame(false)? {
                    Some((f0, len)) => self.feed(&mut pipe, f0, len)?,
                    None => self.finish(&mut pipe)?,
                }
            }
            Ok(())
        })();
        self.pipe = Some(pipe);
        result
    }

    /// The stream ended: record its end and flush the resampler with zeros.
    fn finish(&mut self, pipe: &mut Pipe) -> Result<(), DecodeError> {
        // Only a pipe that decoded something knows where the stream ends.
        if let Some(end) = pipe.last_real {
            self.end_in = Some(end.max(self.first_in));
        }
        let zeros = vec![0.0f32; TAIL_ZEROS];
        self.push(pipe, &zeros, None)?;
        pipe.done = true;
        Ok(())
    }

    /// Feeds the decoded frame `[f0, f0 + len)` (in `dec.planes`), with
    /// silence for any gap before it and skipping any overlap.
    fn feed(&mut self, pipe: &mut Pipe, f0: i64, len: usize) -> Result<(), DecodeError> {
        if f0 > pipe.next_in {
            let mut gap = f0 - pipe.next_in;
            let zeros = vec![0.0f32; 4096];
            while gap > 0 {
                let n = gap.min(4096) as usize;
                self.push(pipe, &zeros[..n], None)?;
                gap -= n as i64;
            }
        }
        let end = f0 + len as i64;
        pipe.last_real = Some(pipe.last_real.map_or(end, |e| e.max(end)));
        let skip = (pipe.next_in - f0).clamp(0, len as i64) as usize;
        if skip < len {
            let planes = std::mem::take(&mut self.dec.planes);
            let result = self.push(pipe, &[], Some((&planes, skip, len)));
            self.dec.planes = planes;
            result?;
        }
        Ok(())
    }

    /// Pushes either `zeros` (on every channel) or `planes[..][skip..len]`
    /// through the pipe.
    fn push(
        &mut self,
        pipe: &mut Pipe,
        zeros: &[f32],
        planes: Option<(&Vec<Vec<f32>>, usize, usize)>,
    ) -> Result<(), DecodeError> {
        let in_ch = usize::from(self.dec.channels);
        let slices: Vec<&[f32]> = match planes {
            Some((p, skip, len)) => p.iter().take(in_ch).map(|v| &v[skip..len]).collect(),
            None => (0..in_ch).map(|_| zeros).collect(),
        };
        let n = slices.first().map_or(0, |s| s.len());
        pipe.next_in += n as i64;
        match pipe.resampler.as_mut() {
            Some(r) => {
                r.convert(&slices, &mut pipe.out)
                    .map_err(|e| av::ff_error("swr_convert", e))?;
            }
            None => {
                pipe.out.reserve(n * in_ch);
                for i in 0..n {
                    for s in &slices {
                        pipe.out.push(s[i]);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Min/max waveform peaks of audio stream `stream` of `path`.
///
/// Bucket `k` covers media time `[k * bucket, (k + 1) * bucket)`; each entry
/// is the minimum and maximum sample over all channels of the samples whose
/// start falls in it (native rate, before any resampling). Samples before
/// media time 0 are excluded; buckets without samples are `(0, 0)`. The
/// last bucket is the one holding the stream's last sample.
///
/// Decodes the whole stream sequentially (never seeks), checking `cancel`
/// between packets ([`DecodeError::Cancelled`]).
pub fn audio_peaks(
    path: &Path,
    stream: usize,
    bucket: Tick,
    cancel: &CancelToken,
) -> Result<Vec<(f32, f32)>, DecodeError> {
    if !bucket.is_positive() {
        return Err(DecodeError::InvalidArgument("bucket must be positive"));
    }
    if cancel.is_cancelled() {
        return Err(crate::source::cancelled_decode(cancel));
    }
    let (mut dec, mut f0, mut len) = StreamDecoder::open(path, stream, cancel)?;
    // Bucket k starts at the first sample m with m * TPS >= k * bucket * rate,
    // i.e. m = ceil(k * bucket * rate / TPS).
    let rate = i128::from(dec.rate);
    let span = i128::from(bucket.raw()) * rate;
    let tps = i128::from(TICKS_PER_SECOND);
    let bucket_of = |m: i64| (i128::from(m) * tps).div_euclid(span);
    let start_of = |k: i128| {
        let n = k * span;
        n.div_euclid(tps) + i128::from(n.rem_euclid(tps) != 0)
    };
    let mut peaks: Vec<Option<(f32, f32)>> = Vec::new();
    loop {
        let first = f0.max(0);
        let end = f0 + len as i64;
        let mut m = first;
        while m < end {
            let k = bucket_of(m);
            let next = i64::try_from(start_of(k + 1)).unwrap_or(i64::MAX).min(end);
            let Ok(k) = usize::try_from(k) else {
                break;
            };
            if k >= peaks.len() {
                peaks.resize(k + 1, None);
            }
            let (mut lo, mut hi) = peaks[k].unwrap_or((f32::INFINITY, f32::NEG_INFINITY));
            let (a, b) = ((m - f0) as usize, (next - f0) as usize);
            for plane in &dec.planes {
                for &v in &plane[a..b] {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            peaks[k] = Some((lo, hi));
            m = next;
        }
        match dec.next_frame(false)? {
            Some((a, b)) => (f0, len) = (a, b),
            None => break,
        }
    }
    Ok(peaks.into_iter().map(|p| p.unwrap_or((0.0, 0.0))).collect())
}
