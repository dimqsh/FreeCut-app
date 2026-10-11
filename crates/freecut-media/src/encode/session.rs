//! One export's encoders and muxer (export-pipeline §7.1).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ffmpeg_next::format::Pixel;
use ffmpeg_next::format::sample::Type as SampleType;
use ffmpeg_next::{
    ChannelLayout, Codec, Dictionary, Packet, Rational, codec, color, encoder, format, frame,
};

use super::EncodeError;
use super::fifo::SampleFifo;
use super::spec::{EncodeSpec, aac_options, x264_options};
use crate::av;
use crate::av::sws::YuvConverter;
use crate::cancel::CancelToken;
use crate::init::ensure_init;

/// The muxer FreeCut's video exports use (EX-029). Named explicitly, never
/// guessed from the file extension.
const MUXER: &str = "mp4";
const VIDEO_ENCODER: &str = "libx264";
const AUDIO_ENCODER: &str = "aac";

/// What a finished encode wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MuxStats {
    /// Video frames pushed (= video packets written).
    pub video_frames: u64,
    /// Audio sample frames pushed (per channel).
    pub audio_samples: u64,
    /// Video packets written to the file.
    pub video_packets: u64,
    /// Audio packets written to the file.
    pub audio_packets: u64,
    /// Size of the finished file in bytes.
    pub bytes: u64,
}

/// One stream's encoder and where its packets go.
struct StreamWriter {
    enc: encoder::Encoder,
    /// Output stream index.
    index: usize,
    enc_tb: Rational,
    /// The muxer's stream time base, known after the header is written.
    st_tb: Rational,
    /// Duration given to packets the encoder leaves without one.
    default_duration: i64,
    packets: u64,
}

impl StreamWriter {
    /// Writes every packet the encoder has ready. With `cancel`, stops
    /// early (returning `Cancelled`) when it fires.
    fn drain(
        &mut self,
        out: &mut format::context::Output,
        cancel: Option<&CancelToken>,
    ) -> Result<(), EncodeError> {
        let mut pkt = Packet::empty();
        loop {
            if cancel.is_some_and(CancelToken::is_cancelled) {
                return Err(EncodeError::Cancelled);
            }
            match self.enc.receive_packet(&mut pkt) {
                Ok(()) => {}
                Err(ffmpeg_next::Error::Eof) => return Ok(()),
                Err(ffmpeg_next::Error::Other { errno }) if errno == ffmpeg_next::error::EAGAIN => {
                    return Ok(());
                }
                Err(e) => return Err(ff("avcodec_receive_packet", e)),
            }
            pkt.set_stream(self.index);
            if pkt.duration() <= 0 {
                pkt.set_duration(self.default_duration);
            }
            pkt.rescale_ts(self.enc_tb, self.st_tb);
            pkt.write_interleaved(out)
                .map_err(|e| ff("av_interleaved_write_frame", e))?;
            self.packets += 1;
        }
    }
}

struct VideoWriter {
    stream: StreamWriter,
    width: u32,
    height: u32,
    /// The caller's pixels, copied in with FFmpeg's row alignment.
    rgba: frame::Video,
    /// The encoder's input frame, reused (made writable before each fill).
    yuv: frame::Video,
    convert: YuvConverter,
    /// The frame index expected next.
    next: u64,
}

struct AudioWriter {
    stream: StreamWriter,
    fifo: SampleFifo,
    rate: u32,
    /// Pts (in samples) of the next frame sent to the encoder.
    next_pts: i64,
    samples_in: u64,
}

/// Where the session is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Open,
    /// Finished or abandoned: nothing left to clean up.
    Closed,
}

/// Encodes one export into an MP4 file: H.264 High (libx264) 4:2:0 8-bit
/// with BT.709 tags, plus AAC-LC stereo when the spec has audio (EX-029,
/// EX-031, EX-032), with fast start.
///
/// All FFmpeg contexts of the export live here, so one thread owns the
/// session (it is `Send`, not `Sync`). Frames go in with
/// [`push_video`](EncodeSession::push_video) and
/// [`push_audio`](EncodeSession::push_audio); each call encodes what is ready
/// and returns, so the caller can check its cancel flag between calls.
/// Packets are interleaved by FFmpeg's `av_interleaved_write_frame`; pushing
/// whichever stream is behind (see
/// [`video_due_before_audio`](super::video_due_before_audio)) keeps its
/// queue short.
///
/// [`finish`](EncodeSession::finish) completes the file;
/// [`abandon`](EncodeSession::abandon), or dropping an unfinished session,
/// deletes it, so a cancelled or failed export leaves no partial file
/// (EX-022).
pub struct EncodeSession {
    path: PathBuf,
    video: VideoWriter,
    audio: Option<AudioWriter>,
    // Declared after the encoders, so it is dropped after them.
    out: format::context::Output,
    state: State,
}

impl std::fmt::Debug for EncodeSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodeSession")
            .field("path", &self.path)
            .field("frames_pushed", &self.video.next)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// Wraps an ffmpeg-next error for operation `op`.
fn ff(op: &'static str, e: ffmpeg_next::Error) -> EncodeError {
    EncodeError::from_av(op, i32::from(e), e.to_string())
}

/// `path` as the URL handed to FFmpeg: `file:` plus the UTF-8 path, so a
/// name that looks like `scheme:…` is never taken for a protocol.
fn file_url(path: &Path) -> Result<String, EncodeError> {
    let s = path
        .to_str()
        .ok_or(EncodeError::UnsupportedPath("it is not valid UTF-8"))?;
    if s.contains('\0') {
        return Err(EncodeError::UnsupportedPath("it contains a NUL character"));
    }
    if s.is_empty() {
        return Err(EncodeError::UnsupportedPath("it is empty"));
    }
    Ok(format!("file:{s}"))
}

/// Option list → FFmpeg dictionary.
fn dictionary(options: &[(&str, String)]) -> Dictionary<'static> {
    let mut d = Dictionary::new();
    for (k, v) in options {
        d.set(k, v);
    }
    d
}

fn find_encoder(name: &'static str) -> Result<Codec, EncodeError> {
    encoder::find_by_name(name).ok_or(EncodeError::EncoderUnavailable(name))
}

/// Threads for the video encoder: one per core, at most 16 (P17).
fn encoder_threads() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZero::get)
        .unwrap_or(1)
        .min(16)
}

/// Fails with `UnknownOption` when an encoder or the muxer left options
/// unused.
fn reject_leftovers(component: &'static str, left: Vec<String>) -> Result<(), EncodeError> {
    match left.into_iter().next() {
        Some(key) => Err(EncodeError::UnknownOption { component, key }),
        None => Ok(()),
    }
}

impl EncodeSession {
    /// Creates `path` (truncating it if it exists) and opens the encoders
    /// and the MP4 muxer for `spec`, writing the file header.
    ///
    /// The caller passes the job's temporary `.part` path (EX-022); the path
    /// must be valid UTF-8 (FFmpeg reopens it by name for the fast-start
    /// pass). On any error the file is removed again.
    pub fn create(path: &Path, spec: &EncodeSpec) -> Result<EncodeSession, EncodeError> {
        spec.validate()?;
        ensure_init()?;
        let url = file_url(path)?;
        let x264 = find_encoder(VIDEO_ENCODER)?;
        let aac = spec
            .audio
            .map(|_| find_encoder(AUDIO_ENCODER))
            .transpose()?;

        let mut out =
            format::output_as(&url, MUXER).map_err(|e| ff("opening the output file", e))?;
        match Self::open_streams(&mut out, spec, x264, aac) {
            Ok((video, audio)) => Ok(EncodeSession {
                path: path.to_path_buf(),
                video,
                audio,
                out,
                state: State::Open,
            }),
            Err(e) => {
                drop(out);
                if let Err(rm) = std::fs::remove_file(path) {
                    tracing::warn!("could not remove {}: {rm}", path.display());
                }
                Err(e)
            }
        }
    }

    /// Steps 3–6 of the open order: encoders (with `GLOBAL_HEADER` before
    /// opening), streams, metadata, header.
    fn open_streams(
        out: &mut format::context::Output,
        spec: &EncodeSpec,
        x264: Codec,
        aac: Option<Codec>,
    ) -> Result<(VideoWriter, Option<AudioWriter>), EncodeError> {
        let global = out.format().flags().contains(format::Flags::GLOBAL_HEADER);
        let rate = spec.frame_rate;
        let (num, den) = (
            i32::try_from(rate.num()).map_err(|_| EncodeError::InvalidSpec("frame rate"))?,
            i32::try_from(rate.den()).map_err(|_| EncodeError::InvalidSpec("frame rate"))?,
        );

        // Video: libx264, yuv420p, BT.709 limited, left chroma siting.
        let video_tb = Rational(den, num);
        let mut v = codec::context::Context::new_with_codec(x264)
            .encoder()
            .video()
            .map_err(|e| ff("allocating the video encoder", e))?;
        v.set_width(spec.width);
        v.set_height(spec.height);
        v.set_format(Pixel::YUV420P);
        v.set_time_base(video_tb);
        v.set_frame_rate(Some(Rational(num, den)));
        v.set_colorspace(color::Space::BT709);
        v.set_color_range(color::Range::MPEG);
        v.set_color_primaries(color::Primaries::BT709);
        v.set_color_transfer_characteristic(color::TransferCharacteristic::BT709);
        av::encode::set_codec_chroma_left(&mut v);
        if global {
            v.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let options = x264_options(spec.video_kbps(), spec.gop_frames(), encoder_threads());
        let left = av::encode::open_codec_strict(&mut v, dictionary(&options))
            .map_err(|e| ff("opening the H.264 encoder", e))?;
        reject_leftovers("video encoder", left)?;
        let video_enc = v.0;

        let video_index = {
            let mut ost = out
                .add_stream(x264)
                .map_err(|e| ff("adding the video stream", e))?;
            ost.set_parameters(&video_enc);
            ost.set_time_base(video_tb);
            ost.index()
        };

        // Audio: native AAC-LC, planar float, stereo.
        let audio = match (spec.audio, aac) {
            (Some(a), Some(aac)) => {
                let sr = i32::try_from(a.sample_rate.hz())
                    .map_err(|_| EncodeError::InvalidSpec("sample rate"))?;
                let audio_tb = Rational(1, sr);
                let mut ctx = codec::context::Context::new_with_codec(aac)
                    .encoder()
                    .audio()
                    .map_err(|e| ff("allocating the audio encoder", e))?;
                ctx.set_rate(sr);
                ctx.set_channel_layout(ChannelLayout::STEREO);
                ctx.set_format(format::Sample::F32(SampleType::Planar));
                ctx.set_time_base(audio_tb);
                if global {
                    ctx.set_flags(codec::Flags::GLOBAL_HEADER);
                }
                let left =
                    av::encode::open_codec_strict(&mut ctx, dictionary(&aac_options(a.kbps)))
                        .map_err(|e| ff("opening the AAC encoder", e))?;
                reject_leftovers("audio encoder", left)?;
                let opened = encoder::audio::Encoder(ctx);
                let frame_size = usize::try_from(opened.frame_size()).unwrap_or(0);
                // AAC always has 1024; 0 would mean "any size".
                let frame_size = if frame_size == 0 { 1024 } else { frame_size };
                let audio_enc = (opened.0).0;
                let index = {
                    let mut ost = out
                        .add_stream(aac)
                        .map_err(|e| ff("adding the audio stream", e))?;
                    ost.set_parameters(&audio_enc);
                    ost.set_time_base(audio_tb);
                    ost.index()
                };
                Some(AudioWriter {
                    stream: StreamWriter {
                        enc: audio_enc,
                        index,
                        enc_tb: audio_tb,
                        st_tb: audio_tb,
                        default_duration: i64::try_from(frame_size).unwrap_or(1024),
                        packets: 0,
                    },
                    fifo: SampleFifo::new(2, frame_size),
                    rate: a.sample_rate.hz(),
                    next_pts: 0,
                    samples_in: 0,
                })
            }
            _ => None,
        };

        // EX-040: technical metadata only. FFmpeg adds its own `encoder` tag.
        let mut meta = Dictionary::new();
        meta.set("creation_time", &creation_time_now());
        if let Some(title) = &spec.title {
            meta.set("title", title);
        }
        out.set_metadata(meta);

        let mut mux_opts = Dictionary::new();
        mux_opts.set("movflags", "+faststart");
        let left: Vec<String> = out
            .write_header_with(mux_opts)
            .map_err(|e| ff("writing the file header", e))?
            .iter()
            .map(|(k, _)| k.to_owned())
            .collect();
        reject_leftovers("muxer", left)?;

        let stream_tb = |i: usize, fallback: Rational| {
            out.stream(i)
                .map(|s| s.time_base())
                .filter(|tb| tb.numerator() > 0 && tb.denominator() > 0)
                .unwrap_or(fallback)
        };
        let video = VideoWriter {
            stream: StreamWriter {
                enc: video_enc,
                index: video_index,
                enc_tb: video_tb,
                st_tb: stream_tb(video_index, video_tb),
                default_duration: 1,
                packets: 0,
            },
            width: spec.width,
            height: spec.height,
            rgba: frame::Video::new(Pixel::RGBA, spec.width, spec.height),
            yuv: frame::Video::new(Pixel::YUV420P, spec.width, spec.height),
            convert: YuvConverter::new().map_err(|e| ff("sws_alloc_context", e))?,
            next: 0,
        };
        let audio = audio.map(|mut a| {
            a.stream.st_tb = stream_tb(a.stream.index, a.stream.enc_tb);
            a
        });
        Ok((video, audio))
    }

    /// Encodes frame `frame_index`: `rgba` is `width × height × 4` bytes of
    /// straight-alpha RGBA8, rows top to bottom, already composited over
    /// black (any pixel that is not opaque is composited over black here).
    ///
    /// Frames are pushed in order from 0 without gaps; frame `n` is
    /// presented at `n / frame_rate`. The pixels are converted to BT.709
    /// limited-range 4:2:0 (no dither: the input is already 8-bit, and the
    /// renderer's float → 8-bit step owns EX-032's dither).
    pub fn push_video(&mut self, rgba: &[u8], frame_index: u64) -> Result<(), EncodeError> {
        self.ensure_open()?;
        let v = &mut self.video;
        if frame_index != v.next {
            return Err(EncodeError::OutOfOrder {
                expected: v.next,
                got: frame_index,
            });
        }
        let row = v.width as usize * 4;
        let rows = v.height as usize;
        if rgba.len() != row * rows {
            return Err(EncodeError::InvalidInput(format!(
                "a {}×{} frame needs {} bytes of RGBA, got {}",
                v.width,
                v.height,
                row * rows,
                rgba.len()
            )));
        }
        let pts = i64::try_from(frame_index)
            .map_err(|_| EncodeError::InvalidInput("frame index too large".into()))?;

        let stride = v.rgba.stride(0);
        let dst = v.rgba.data_mut(0);
        for (y, src_row) in rgba.chunks_exact(row).enumerate() {
            let dst_row = &mut dst[y * stride..y * stride + row];
            dst_row.copy_from_slice(src_row);
            for px in dst_row.as_chunks_mut::<4>().0 {
                let a = u16::from(px[3]);
                if a != 255 {
                    for c in &mut px[..3] {
                        // Over black: c·a/255, rounded. Fits u8 (≤ 255).
                        *c = ((u16::from(*c) * a + 127) / 255) as u8;
                    }
                    px[3] = 255;
                }
            }
        }
        av::encode::make_writable(&mut v.yuv).map_err(|e| ff("av_frame_make_writable", e))?;
        v.convert
            .convert(&mut v.rgba, &mut v.yuv)
            .map_err(|e| ff("sws_scale_frame", e))?;
        v.yuv.set_pts(Some(pts));
        v.stream
            .enc
            .send_frame(&v.yuv)
            .map_err(|e| ff("avcodec_send_frame (video)", e))?;
        v.stream.drain(&mut self.out, None)?;
        v.next += 1;
        Ok(())
    }

    /// Queues interleaved stereo `f32` samples (left, right, left, …) and
    /// encodes every full AAC frame. The mix is in the output rate already;
    /// the last, possibly short, frame is sent by
    /// [`finish`](EncodeSession::finish). Non-finite samples become silence.
    pub fn push_audio(&mut self, interleaved_stereo: &[f32]) -> Result<(), EncodeError> {
        self.ensure_open()?;
        let a = self
            .audio
            .as_mut()
            .ok_or_else(|| EncodeError::InvalidInput("this export has no audio stream".into()))?;
        if !interleaved_stereo.len().is_multiple_of(2) {
            return Err(EncodeError::InvalidInput(
                "stereo audio needs an even number of samples".into(),
            ));
        }
        a.fifo.push(interleaved_stereo);
        a.samples_in += (interleaved_stereo.len() / 2) as u64;
        while let Some(chunk) = a.fifo.pop_frame() {
            Self::send_audio(a, &chunk, &mut self.out, None)?;
        }
        Ok(())
    }

    /// Encodes one chunk of interleaved stereo as a planar frame.
    fn send_audio(
        a: &mut AudioWriter,
        chunk: &[f32],
        out: &mut format::context::Output,
        cancel: Option<&CancelToken>,
    ) -> Result<(), EncodeError> {
        let n = chunk.len() / 2;
        let mut f = frame::Audio::new(
            format::Sample::F32(SampleType::Planar),
            n,
            ChannelLayout::STEREO,
        );
        f.set_rate(a.rate);
        for ch in 0..2 {
            let plane = f.plane_mut::<f32>(ch);
            for (dst, src) in plane.iter_mut().zip(chunk.iter().skip(ch).step_by(2)) {
                *dst = if src.is_finite() { *src } else { 0.0 };
            }
        }
        f.set_pts(Some(a.next_pts));
        a.next_pts = a
            .next_pts
            .checked_add(i64::try_from(n).unwrap_or(i64::MAX))
            .ok_or_else(|| EncodeError::InvalidInput("too much audio".into()))?;
        a.stream
            .enc
            .send_frame(&f)
            .map_err(|e| ff("avcodec_send_frame (audio)", e))?;
        a.stream.drain(out, cancel)
    }

    /// Video frames pushed so far (the next expected frame index).
    pub fn frames_pushed(&self) -> u64 {
        self.video.next
    }

    /// Video packets written so far. It trails
    /// [`frames_pushed`](EncodeSession::frames_pushed) by the encoder's
    /// lookahead and catches up during [`finish`](EncodeSession::finish),
    /// so it is the progress measure (export-pipeline §7.1).
    pub fn video_packets_written(&self) -> u64 {
        self.video.stream.packets
    }

    /// The file being written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn ensure_open(&self) -> Result<(), EncodeError> {
        match self.state {
            State::Open => Ok(()),
            State::Closed => Err(EncodeError::InvalidInput("the session is closed".into())),
        }
    }

    /// Flushes both encoders (P15: video and audio), writes the trailer (the
    /// fast-start rewrite that moves the index before the media happens
    /// here) and closes the file, checking the final flush.
    ///
    /// `cancel` is checked between packets while the encoders drain; when it
    /// fires the file is deleted and `Cancelled` returned. The fast-start
    /// rewrite itself is one FFmpeg call that copies the file once.
    ///
    /// On any error the partial file is deleted. Success here is not the
    /// end of the job: [`verify_output`](super::verify_output) reads the file
    /// back (P6).
    pub fn finish(mut self, cancel: &CancelToken) -> Result<MuxStats, EncodeError> {
        match self.finish_inner(cancel) {
            Ok(stats) => Ok(stats),
            Err(e) => {
                if let Err(rm) = self.discard() {
                    tracing::warn!("could not remove {}: {rm}", self.path.display());
                }
                Err(e)
            }
        }
    }

    fn finish_inner(&mut self, cancel: &CancelToken) -> Result<MuxStats, EncodeError> {
        self.ensure_open()?;
        if self.video.next == 0 {
            return Err(EncodeError::InvalidInput(
                "no video frames were pushed".into(),
            ));
        }
        let check = || {
            if cancel.is_cancelled() {
                Err(EncodeError::Cancelled)
            } else {
                Ok(())
            }
        };
        check()?;
        self.video
            .stream
            .enc
            .send_eof()
            .map_err(|e| ff("flushing the video encoder", e))?;
        self.video.stream.drain(&mut self.out, Some(cancel))?;
        if let Some(a) = self.audio.as_mut() {
            if let Some(rest) = a.fifo.pop_rest() {
                Self::send_audio(a, &rest, &mut self.out, Some(cancel))?;
            }
            a.stream
                .enc
                .send_eof()
                .map_err(|e| ff("flushing the audio encoder", e))?;
            a.stream.drain(&mut self.out, Some(cancel))?;
        }
        check()?;
        self.out
            .write_trailer()
            .map_err(|e| ff("writing the file trailer", e))?;
        av::encode::close_output_checked(&mut self.out)
            .map_err(|e| ff("closing the output file", e))?;
        self.state = State::Closed;
        let bytes = std::fs::metadata(&self.path).map(|m| m.len())?;
        Ok(MuxStats {
            video_frames: self.video.next,
            audio_samples: self.audio.as_ref().map_or(0, |a| a.samples_in),
            video_packets: self.video.stream.packets,
            audio_packets: self.audio.as_ref().map_or(0, |a| a.stream.packets),
            bytes,
        })
    }

    /// Stops without finishing: closes the file and deletes it (cancel or
    /// failure, EX-022). No flush, no trailer.
    pub fn abandon(mut self) -> Result<(), EncodeError> {
        self.discard().map_err(EncodeError::Io)
    }

    /// Closes and deletes the file unless the session is already closed.
    fn discard(&mut self) -> std::io::Result<()> {
        if self.state == State::Closed {
            return Ok(());
        }
        self.state = State::Closed;
        // The file is deleted next; a failed final flush doesn't matter.
        let _ = av::encode::close_output_checked(&mut self.out);
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

impl Drop for EncodeSession {
    /// An unfinished session leaves no partial file behind.
    fn drop(&mut self) {
        if let Err(e) = self.discard() {
            tracing::warn!("could not remove {}: {e}", self.path.display());
        }
    }
}

/// The export time as FFmpeg's ISO 8601 `creation_time` (UTC, µs).
fn creation_time_now() -> String {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    iso8601_utc(since.as_secs(), since.subsec_micros())
}

/// `secs` since 1970-01-01 UTC as `YYYY-MM-DDTHH:MM:SS.ffffffZ`.
fn iso8601_utc(secs: u64, micros: u32) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{micros:06}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 → (year, month, day) in the proleptic Gregorian
/// calendar (Howard Hinnant's `civil_from_days`, for non-negative days).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_time_format() {
        assert_eq!(iso8601_utc(0, 0), "1970-01-01T00:00:00.000000Z");
        assert_eq!(
            iso8601_utc(1_700_000_000, 123_456),
            "2023-11-14T22:13:20.123456Z"
        );
        // A leap day.
        assert_eq!(iso8601_utc(951_782_400, 0), "2000-02-29T00:00:00.000000Z");
        assert_eq!(iso8601_utc(4_102_444_799, 0), "2099-12-31T23:59:59.000000Z");
    }

    #[test]
    fn urls_are_file_urls() {
        assert_eq!(
            file_url(Path::new("/tmp/a:b.mp4")).ok().as_deref(),
            Some("file:/tmp/a:b.mp4")
        );
        assert!(matches!(
            file_url(Path::new("")),
            Err(EncodeError::UnsupportedPath(_))
        ));
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            let bad = Path::new(OsStr::from_bytes(b"/tmp/\xff.mp4"));
            assert!(matches!(
                file_url(bad),
                Err(EncodeError::UnsupportedPath(_))
            ));
        }
    }

    #[test]
    fn sessions_are_send() {
        fn send<T: Send>() {}
        send::<EncodeSession>();
    }
}
