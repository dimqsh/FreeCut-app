//! Frame-accurate video decoding (TR-036, EX-004).
//!
//! [`VideoDecoder::frame_at`] returns the frame *displayed* at media time `t`:
//! the last frame whose media-time start is `<= t`, the first frame when `t`
//! is before it, and the last frame when `t` is past the end. Frames are
//! identified by their presentation timestamps converted to ticks (I1, I3);
//! targets are compared in tick space only (L2), so time bases that don't
//! divide the tick rate (1/15360) can't pick the previous frame.
//!
//! The decoder keeps the last frame at or before the previous target and
//! the first frame after it. A later target within a short distance is
//! reached by decoding forward (L5); anything else seeks: to the keyframe at
//! or before `t` minus a safety margin (reordering puts a keyframe's
//! presentation time after its decode time), stepping further back when the
//! first decoded frame still lands after `t` (L4), down to the start of the
//! stream. Decoder shortcuts that change pixels (`skip_loop_filter`,
//! `skip_frame`) are never used, so exact frames are identical however they
//! are reached (README §5.5).
//!
//! A still (`MediaInfo::still`) shows its first frame at every time, also
//! when it is an animated GIF, APNG or WebP: animations are stills of their
//! first frame in the MVP (PM-034), as in the probe and the poster.

use std::path::Path;

use ffmpeg_next::decoder;
use ffmpeg_next::frame;
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};

use crate::av;
use crate::av::sws::Scaler;
use crate::cancel::CancelToken;
use crate::error::{DecodeError, StreamKind};
use crate::probe::one_frame;
use crate::source::{Source, open_source};
use crate::time::{StreamClock, nominal_rate};

/// How [`VideoDecoder::frame_at`] trades accuracy for speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SeekMode {
    /// The exact frame displayed at `t`.
    Exact,
    /// A nearby frame at or before `t`, quickly: the current frame when the
    /// decoder is already there, else the first frame decoded after seeking
    /// to the keyframe at or before `t` (EX-004 fast drags).
    Approximate,
}

/// A decoded frame converted to RGBA8 at its coded size (no rotation or
/// pixel-aspect correction; those are display geometry, applied by render).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    /// Width in pixels (the coded width unless a size was requested).
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Media time at which this frame starts being displayed.
    pub pts: Tick,
    /// How long it is displayed: up to the next frame's start when that is
    /// known, else the frame's own duration, else `None`.
    pub duration: Option<Tick>,
    /// `width * height * 4` bytes, rows top to bottom, straight alpha
    /// (opaque formats have alpha 255).
    pub rgba: Vec<u8>,
}

/// One decoded frame and where it sits in media time.
struct Decoded {
    frame: frame::Video,
    start: Tick,
    /// From the frame's own duration field, when set.
    own_duration: Option<Tick>,
}

/// Which decoded frame answers a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Current,
    Next,
}

/// Consecutive decode errors tolerated before a request fails (MLT's 10).
const MAX_DECODE_ERRORS: usize = 10;

/// Seek attempts before giving up on a target.
const MAX_SEEK_ATTEMPTS: usize = 64;

/// Decodes one video stream of one file, frame-accurately.
///
/// Not `Sync`: one thread uses a decoder at a time (I5).
pub struct VideoDecoder {
    src: Source,
    stream: usize,
    decoder: decoder::Video,
    clock: StreamClock,
    scaler: Scaler,
    /// Nominal frame duration, for margins and timestamp repair.
    frame: Tick,
    /// Media time of the stream's first frame.
    first_start: Tick,
    /// A still picture: the first frame answers every request.
    still: bool,
    /// The last decoded frame at or before the previous target.
    cur: Option<Decoded>,
    /// The first decoded frame after the previous target.
    next: Option<Decoded>,
    sent_eof: bool,
    /// The decoder has returned every frame up to the end of the stream.
    drained: bool,
    last_start: Option<Tick>,
    errors: usize,
    /// RGBA of the most recently returned frame, by start time and size.
    converted: Option<(Tick, u32, u32, Vec<u8>)>,
}

impl std::fmt::Debug for VideoDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoDecoder")
            .field("source", &self.src)
            .field("stream", &self.stream)
            .field("first_start", &self.first_start)
            .finish_non_exhaustive()
    }
}

impl VideoDecoder {
    /// Opens video stream `stream_index` of `path`.
    ///
    /// Decodes the stream's first frame, which also fixes the media origin
    /// when this is the default video stream (I2).
    pub fn open(path: &Path, stream_index: usize) -> Result<VideoDecoder, DecodeError> {
        let mut src = open_source(path, &CancelToken::new())?;
        src.require(stream_index, StreamKind::Video)?;
        let is_default = src.default_video == Some(stream_index);
        // The origin of another stream comes from the default one.
        let origin = if is_default {
            Tick::ZERO
        } else {
            src.origin()?
        };
        src.keep_only(&[stream_index]);
        src.rewind(stream_index)?;
        let decoder =
            src.video_decoder(stream_index, crate::resources::current().decoder_threads)?;
        let (num, den) = src.time_base(stream_index);
        let guess = av::codec::guess_frame_rate(&src.input, stream_index);
        let rate = src.input.stream(stream_index).and_then(|st| {
            let avg = st.avg_frame_rate();
            nominal_rate(guess, (avg.numerator(), avg.denominator()))
        });
        let still = src.still;
        let mut this = VideoDecoder {
            src,
            stream: stream_index,
            decoder,
            clock: StreamClock::new(num, den, origin)?,
            scaler: Scaler::new().map_err(|e| av::ff_error("sws_alloc_context", e))?,
            frame: one_frame(rate),
            first_start: Tick::ZERO,
            still,
            cur: None,
            next: None,
            sent_eof: false,
            drained: false,
            last_start: None,
            errors: 0,
            converted: None,
        };
        let Some(mut first) = this.decode_one()? else {
            return Err(DecodeError::Corrupt { at: Tick::ZERO });
        };
        if is_default {
            // Media time 0 is this frame (same rule as `Source::origin`).
            let ts = first
                .frame
                .timestamp()
                .or_else(|| first.frame.pts())
                .unwrap_or(0);
            let abs = StreamClock::new(num, den, Tick::ZERO)?.media_time(ts)?;
            this.clock = StreamClock::new(num, den, abs)?;
            first.start = Tick::ZERO;
            this.last_start = Some(Tick::ZERO);
        }
        this.first_start = first.start;
        this.next = Some(first);
        Ok(this)
    }

    /// The media time of this stream's first frame (0 for the default
    /// video stream).
    pub fn first_frame_time(&self) -> Tick {
        self.first_start
    }

    /// The frame displayed at media time `t` (see the module docs), as RGBA8
    /// at the coded size.
    pub fn frame_at(&mut self, t: Tick, mode: SeekMode) -> Result<VideoFrame, DecodeError> {
        let which = self.locate(t, mode)?;
        self.output(t, which, None)
    }

    /// The stream's coded picture size (what [`VideoDecoder::frame_at`]
    /// returns), as the decoder reports it.
    pub fn coded_size(&self) -> (u32, u32) {
        (self.decoder.width(), self.decoder.height())
    }

    /// Like [`VideoDecoder::frame_at`], scaled to `width × height` during
    /// the RGBA conversion (cheaper than converting at the coded size when
    /// the picture is shown smaller, and filtered for the downscale).
    pub fn frame_at_size(
        &mut self,
        t: Tick,
        mode: SeekMode,
        width: u32,
        height: u32,
    ) -> Result<VideoFrame, DecodeError> {
        let which = self.locate(t, mode)?;
        self.output(t, which, Some((width, height)))
    }

    /// Converts the located frame to RGBA at `size` (default: coded size).
    fn output(
        &mut self,
        t: Tick,
        which: Answer,
        size: Option<(u32, u32)>,
    ) -> Result<VideoFrame, DecodeError> {
        let duration = self.display_duration(which);
        let d = match which {
            Answer::Current => self.cur.as_ref(),
            Answer::Next => self.next.as_ref(),
        }
        .ok_or(DecodeError::Corrupt { at: t })?;
        let start = d.start;
        let (width, height) = size.unwrap_or((d.frame.width(), d.frame.height()));
        let rgba = match &self.converted {
            Some((s, w, h, data)) if *s == start && *w == width && *h == height => data.clone(),
            _ => {
                let data = self
                    .scaler
                    .convert_rgba(&d.frame, width, height)
                    .map_err(|e| av::ff_error("sws_scale_frame", e))?;
                self.converted = Some((start, width, height, data.clone()));
                data
            }
        };
        Ok(VideoFrame {
            width,
            height,
            pts: start,
            duration,
            rgba,
        })
    }

    fn display_duration(&self, which: Answer) -> Option<Tick> {
        if self.still {
            return None;
        }
        match which {
            Answer::Current => {
                let cur = self.cur.as_ref()?;
                match &self.next {
                    Some(n) => n.start.checked_sub(cur.start).ok(),
                    None => cur.own_duration,
                }
            }
            Answer::Next => self.next.as_ref()?.own_duration,
        }
    }

    /// Which held frame answers `t` without decoding, if any.
    fn answer(&self, t: Tick) -> Option<Answer> {
        match (&self.cur, &self.next) {
            (Some(c), Some(n)) if c.start <= t && t < n.start => Some(Answer::Current),
            (Some(c), None) if self.drained && c.start <= t => Some(Answer::Current),
            // Before the first frame: show the first frame.
            (None, Some(n)) if n.start <= self.first_start && t < n.start => Some(Answer::Next),
            _ => None,
        }
    }

    /// Start of the most recently decoded frame.
    fn latest(&self) -> Option<Tick> {
        self.next.as_ref().or(self.cur.as_ref()).map(|d| d.start)
    }

    /// Whether `t` is reached by decoding forward within `limit`.
    fn forward_reaches(&self, t: Tick, limit: Tick) -> bool {
        let Some(latest) = self.latest() else {
            return false;
        };
        if latest > t || self.cur.as_ref().is_some_and(|c| c.start > t) {
            return false;
        }
        t.checked_sub(latest).is_ok_and(|gap| gap <= limit)
    }

    /// Forward decoding beats seeking up to min(2 s, 64 frames) (L5).
    fn forward_limit(&self) -> Tick {
        let frames = self.frame.checked_mul_int(64).unwrap_or(Tick::MAX);
        frames.min(Tick::from_raw(2 * TICKS_PER_SECOND))
    }

    /// The seek margin: enough for a keyframe's presentation time to trail
    /// its decode time under B-frame reordering.
    fn margin(&self) -> Tick {
        self.frame
            .checked_mul_int(4)
            .unwrap_or(Tick::MAX)
            .max(Tick::from_raw(TICKS_PER_SECOND / 10))
    }

    /// Positions the decoder so the frame answering `t` is held.
    fn locate(&mut self, t: Tick, mode: SeekMode) -> Result<Answer, DecodeError> {
        if self.still {
            // `open` holds the first frame in `next`, and a still never
            // decodes again.
            return Ok(Answer::Next);
        }
        if let Some(a) = self.answer(t) {
            return Ok(a);
        }
        match mode {
            SeekMode::Exact => {
                if self.forward_reaches(t, self.forward_limit()) {
                    self.advance_to(t)?;
                    if let Some(a) = self.answer(t) {
                        return Ok(a);
                    }
                }
                self.seek_exact(t)
            }
            SeekMode::Approximate => {
                if self.forward_reaches(t, self.frame.checked_mul_int(4).unwrap_or(Tick::MAX)) {
                    self.advance_to(t)?;
                    if let Some(a) = self.answer(t) {
                        return Ok(a);
                    }
                }
                self.seek_approximate(t)
            }
        }
    }

    /// Decodes forward until the held frames bracket `t` (or the stream ends).
    fn advance_to(&mut self, t: Tick) -> Result<(), DecodeError> {
        loop {
            if self.next.is_none() {
                match self.decode_one()? {
                    Some(d) => self.next = Some(d),
                    None => return Ok(()),
                }
            }
            if self.next.as_ref().is_some_and(|n| n.start <= t) {
                self.cur = self.next.take();
            } else {
                return Ok(());
            }
        }
    }

    /// Drops all decoder state before a reposition.
    fn reset(&mut self) {
        self.decoder.flush();
        self.cur = None;
        self.next = None;
        self.sent_eof = false;
        self.drained = false;
        self.last_start = None;
        self.errors = 0;
    }

    /// Repositions the demuxer at the keyframe at or before `target`, or at
    /// the start of the stream when `target` is at or before its first frame.
    fn reposition(&mut self, target: Tick) -> Result<(), DecodeError> {
        self.reset();
        if target <= self.first_start {
            // From the start, decode with a new decoder, exactly as `open`
            // did, so the first frames can't depend on earlier history.
            self.src.rewind(self.stream)?;
            self.decoder = self.src.video_decoder(self.stream, 0)?;
            return Ok(());
        }
        // A target too late to express in stream units is past every frame:
        // the last keyframe is the right place to start.
        let ts = self.clock.ts_at_or_before(target).unwrap_or(i64::MAX);
        self.src.seek_at_or_before(self.stream, ts)
    }

    fn seek_exact(&mut self, t: Tick) -> Result<Answer, DecodeError> {
        let mut back = self.margin();
        for _ in 0..MAX_SEEK_ATTEMPTS {
            let target = t.checked_sub(back).unwrap_or(Tick::from_raw(i64::MIN));
            self.reposition(target)?;
            match self.decode_one()? {
                Some(d) if d.start <= t || d.start <= self.first_start => {
                    self.next = Some(d);
                    self.advance_to(t)?;
                    if let Some(a) = self.answer(t) {
                        return Ok(a);
                    }
                    // Only reachable when the stream ended before `t` on a
                    // frame before the target: the last frame answers.
                    if self.cur.is_some() {
                        return Ok(Answer::Current);
                    }
                }
                // Overshoot (L4) or nothing decodable after the seek point.
                _ if target <= self.first_start => return Err(DecodeError::Corrupt { at: t }),
                _ => {}
            }
            back = back
                .checked_mul_int(2)
                .unwrap_or(Tick::MAX)
                .max(Tick::from_raw(TICKS_PER_SECOND));
        }
        Err(DecodeError::Corrupt { at: t })
    }

    fn seek_approximate(&mut self, t: Tick) -> Result<Answer, DecodeError> {
        self.reposition(t)?;
        match self.decode_one()? {
            Some(d) if d.start <= t => {
                self.cur = Some(d);
                Ok(Answer::Current)
            }
            Some(d) if d.start <= self.first_start => {
                self.next = Some(d);
                Ok(Answer::Next)
            }
            _ => self.seek_exact(t),
        }
    }

    /// Decodes the next frame of the stream, or `None` at its end.
    fn decode_one(&mut self) -> Result<Option<Decoded>, DecodeError> {
        loop {
            let mut f = frame::Video::empty();
            match self.decoder.receive_frame(&mut f) {
                Ok(()) => {
                    self.errors = 0;
                    return self.wrap(f).map(Some);
                }
                Err(ffmpeg_next::Error::Eof) => {
                    self.drained = true;
                    return Ok(None);
                }
                Err(ffmpeg_next::Error::Other { errno }) if errno == ffmpeg_next::error::EAGAIN => {
                    if self.sent_eof {
                        self.drained = true;
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

    /// Places a decoded frame in media time, repairing timestamps that don't
    /// increase (broken muxers, packed B-frames) to the previous start plus
    /// one nominal frame.
    fn wrap(&mut self, frame: frame::Video) -> Result<Decoded, DecodeError> {
        let ts = frame.timestamp().or_else(|| frame.pts());
        let mut start = match ts {
            Some(ts) => self.clock.media_time(ts)?,
            None => match self.last_start {
                Some(last) => last.checked_add(self.frame)?,
                None => self.first_start,
            },
        };
        if let Some(last) = self.last_start
            && start <= last
        {
            start = last.checked_add(self.frame)?;
        }
        self.last_start = Some(start);
        let own_duration = match (ts, frame.packet().duration) {
            (Some(ts), d) if d > 0 => ts
                .checked_add(d)
                .and_then(|end| self.clock.media_time(end).ok())
                .and_then(|end| end.checked_sub(start).ok())
                .filter(|d| d.is_positive()),
            _ => None,
        };
        Ok(Decoded {
            frame,
            start,
            own_duration,
        })
    }
}
