//! Test fixtures, generated at test time into a temporary directory. No
//! third-party media is ever committed.
//!
//! Video is encoded through ffmpeg-next (our own encode path). Each frame of
//! a "barcode" clip shows its frame number as a 4×3 grid of flat gray cells
//! (bit set → light, clear → dark), which survives lossy coding and decodes
//! back with [`read_barcode`]. Fixtures that need container metadata
//! ffmpeg-next can't write (display matrices) are remuxed with the `ffmpeg`
//! CLI when it is installed; tests using them skip with a message otherwise.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)] // test code; each test binary uses a subset

use std::f64::consts::PI;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use ff::format::Pixel;
use ff::format::sample::Type as SampleType;
use ff::{ChannelLayout, Dictionary, Packet, Rational, codec, encoder, format, frame};
use ffmpeg_next as ff;
use freecut_timeline::time::{StreamTimeBase, Tick, ticks_from_stream};

/// Barcode grid: 4 columns × 3 rows = 12 bits (frames 0..4095).
pub const COLS: u32 = 4;
pub const ROWS: u32 = 3;

static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
static BUILT: OnceLock<Mutex<Vec<(String, PathBuf)>>> = OnceLock::new();

/// A per-process directory for fixtures.
pub fn dir() -> &'static Path {
    DIR.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

/// Builds the fixture `name` once per test process with `make`.
pub fn cached(name: &str, make: impl FnOnce(&Path)) -> PathBuf {
    let built = BUILT.get_or_init(|| Mutex::new(Vec::new()));
    let mut list = built.lock().unwrap();
    if let Some((_, p)) = list.iter().find(|(n, _)| n == name) {
        return p.clone();
    }
    let path = dir().join(name);
    make(&path);
    list.push((name.to_owned(), path.clone()));
    path
}

/// Luma of the barcode cell covering `(x, y)` in frame `k`.
pub fn barcode_luma(k: u32, x: u32, y: u32, w: u32, h: u32) -> u8 {
    let col = (x * COLS / w).min(COLS - 1);
    let row = (y * ROWS / h).min(ROWS - 1);
    let bit = row * COLS + col;
    if (k >> bit) & 1 == 1 { 210 } else { 40 }
}

/// Reads the frame number back from decoded RGBA.
pub fn read_barcode(rgba: &[u8], w: u32, h: u32) -> u32 {
    let mut k = 0;
    for row in 0..ROWS {
        for col in 0..COLS {
            let (x0, x1) = (col * w / COLS, (col + 1) * w / COLS);
            let (y0, y1) = (row * h / ROWS, (row + 1) * h / ROWS);
            // The middle half of the cell, away from edges and ringing.
            let (cx0, cx1) = (x0 + (x1 - x0) / 4, x1 - (x1 - x0) / 4);
            let (cy0, cy1) = (y0 + (y1 - y0) / 4, y1 - (y1 - y0) / 4);
            let (mut sum, mut n) = (0u64, 0u64);
            for y in cy0..cy1 {
                for x in cx0..cx1 {
                    let i = ((y * w + x) * 4) as usize;
                    sum += u64::from(rgba[i]) + u64::from(rgba[i + 1]) + u64::from(rgba[i + 2]);
                    n += 3;
                }
            }
            if sum / n.max(1) > 128 {
                k |= 1 << (row * COLS + col);
            }
        }
    }
    k
}

/// What a frame shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// The frame number as a barcode.
    Barcode,
    /// Four colored quadrants: red, green / blue, white.
    Quadrants,
}

/// BT.601 limited-range Y′CbCr of an sRGB color.
pub fn rgb_to_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let (r, g, b) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    let y = 16.0 + 65.481 * r + 128.553 * g + 24.966 * b;
    let u = 128.0 - 37.797 * r - 74.203 * g + 112.0 * b;
    let v = 128.0 + 112.0 * r - 93.786 * g - 18.214 * b;
    (y.round() as u8, u.round() as u8, v.round() as u8)
}

/// The sRGB color of each quadrant (top-left, top-right, bottom-left,
/// bottom-right).
pub const QUADRANT_COLORS: [(u8, u8, u8); 4] =
    [(220, 30, 30), (30, 200, 30), (30, 30, 220), (230, 230, 230)];

/// Which quadrant `(x, y)` is in: 0 TL, 1 TR, 2 BL, 3 BR.
pub fn quadrant(x: u32, y: u32, w: u32, h: u32) -> usize {
    usize::from(x >= w / 2) + 2 * usize::from(y >= h / 2)
}

/// Average RGB of the center of each quadrant of an RGBA image.
pub fn quadrant_means(rgba: &[u8], w: u32, h: u32) -> [(u8, u8, u8); 4] {
    let mut out = [(0, 0, 0); 4];
    for (q, slot) in out.iter_mut().enumerate() {
        let (qx, qy) = ((q as u32 % 2) * w / 2, (q as u32 / 2) * h / 2);
        let (qw, qh) = (w / 2, h / 2);
        let mut sum = [0u64; 3];
        let mut n = 0u64;
        for y in qy + qh / 4..qy + 3 * qh / 4 {
            for x in qx + qw / 4..qx + 3 * qw / 4 {
                let i = ((y * w + x) * 4) as usize;
                for c in 0..3 {
                    sum[c] += u64::from(rgba[i + c]);
                }
                n += 1;
            }
        }
        let n = n.max(1);
        *slot = ((sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8);
    }
    out
}

/// Which of [`QUADRANT_COLORS`] `c` is closest to.
pub fn nearest_quadrant_color(c: (u8, u8, u8)) -> usize {
    let d = |a: (u8, u8, u8)| {
        let f = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        f(a.0, c.0) + f(a.1, c.1) + f(a.2, c.2)
    };
    (0..4).min_by_key(|&i| d(QUADRANT_COLORS[i])).unwrap()
}

#[derive(Debug, Clone)]
pub struct AudioSpec {
    pub rate: u32,
    pub channels: u16,
    /// Total samples per channel.
    pub samples: usize,
}

#[derive(Debug, Clone)]
pub struct VideoSpec {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    /// Frames per second as `num/den`.
    pub rate: (i32, i32),
    pub bframes: u32,
    pub gop: u32,
    /// Ask the muxer for this stream time base (MP4 honours it).
    pub stream_time_base: Option<(i32, i32)>,
    /// Per-frame timestamps in the encoder time base `1/90000` (VFR);
    /// default: frame `k` at `k / rate`.
    pub pts: Option<Vec<i64>>,
    pub pattern: Pattern,
    pub audio: Option<AudioSpec>,
    /// Sample aspect ratio written into the stream.
    pub sar: Option<(i32, i32)>,
}

impl VideoSpec {
    pub fn barcode(frames: u32, rate: (i32, i32)) -> VideoSpec {
        VideoSpec {
            width: 192,
            height: 144,
            frames,
            rate,
            bframes: 3,
            gop: 12,
            stream_time_base: None,
            pts: None,
            pattern: Pattern::Barcode,
            audio: None,
            sar: None,
        }
    }
}

/// A deterministic test signal: two tones plus a little LCG noise.
pub fn signal(n: usize, ch: usize, rate: u32) -> f32 {
    let t = n as f64 / f64::from(rate);
    let tone = 0.4 * (2.0 * PI * 441.0 * t + ch as f64).sin() + 0.2 * (2.0 * PI * 1234.5 * t).sin();
    let mut x = (n as u64)
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(ch as u64 * 1_442_695_040_888_963_407 + 1);
    x ^= x >> 33;
    let noise = ((x % 2001) as f64 / 1000.0 - 1.0) * 0.05;
    (tone + noise) as f32
}

fn video_encoder() -> ff::Codec {
    encoder::find_by_name("libx264")
        .or_else(|| encoder::find_by_name("mpeg4"))
        .expect("neither libx264 nor mpeg4 encoder in this FFmpeg build")
}

fn fill_video(f: &mut frame::Video, k: u32, spec: &VideoSpec) {
    let (w, h) = (spec.width, spec.height);
    let colors: Vec<(u8, u8, u8)> = QUADRANT_COLORS
        .iter()
        .map(|&(r, g, b)| rgb_to_yuv(r, g, b))
        .collect();
    let stride = f.stride(0);
    let data = f.data_mut(0);
    for y in 0..h {
        for x in 0..w {
            data[y as usize * stride + x as usize] = match spec.pattern {
                Pattern::Barcode => barcode_luma(k, x, y, w, h),
                Pattern::Quadrants => colors[quadrant(x, y, w, h)].0,
            };
        }
    }
    for plane in 1..3 {
        let stride = f.stride(plane);
        let data = f.data_mut(plane);
        for y in 0..h / 2 {
            for x in 0..w / 2 {
                data[y as usize * stride + x as usize] = match spec.pattern {
                    Pattern::Barcode => 128,
                    Pattern::Quadrants => {
                        let c = colors[quadrant(x * 2, y * 2, w, h)];
                        if plane == 1 { c.1 } else { c.2 }
                    }
                };
            }
        }
    }
}

struct Muxing {
    index: usize,
    enc: encoder::Encoder,
    enc_tb: Rational,
    /// Samples per audio frame (0 for video).
    frame_size: usize,
    /// Packet duration in `enc_tb` when the encoder leaves it unset.
    default_duration: i64,
}

fn drain(m: &mut Muxing, octx: &mut format::context::Output) {
    let ost_tb = octx.stream(m.index).unwrap().time_base();
    let mut pkt = Packet::empty();
    while m.enc.receive_packet(&mut pkt).is_ok() {
        pkt.set_stream(m.index);
        if pkt.duration() <= 0 {
            // Without durations the MP4 edit list would end at the last
            // frame's start and mark that frame discarded.
            pkt.set_duration(m.default_duration);
        }
        pkt.rescale_ts(m.enc_tb, ost_tb);
        pkt.write_interleaved(octx).unwrap();
    }
}

/// Encodes a video (and optional AAC audio) clip to `path` (container from
/// the extension).
pub fn encode_video(path: &Path, spec: &VideoSpec) {
    ff::init().unwrap();
    ff::log::set_level(ff::log::Level::Quiet);
    let mut octx = format::output(&path).unwrap();
    let global = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);

    let codec = video_encoder();
    let enc_tb = if spec.pts.is_some() {
        Rational(1, 90_000)
    } else {
        Rational(spec.rate.1, spec.rate.0)
    };
    let mut video = {
        let mut ost = octx.add_stream(codec).unwrap();
        let mut enc = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .unwrap();
        enc.set_width(spec.width);
        enc.set_height(spec.height);
        enc.set_format(Pixel::YUV420P);
        enc.set_time_base(enc_tb);
        enc.set_frame_rate(Some(Rational(spec.rate.0, spec.rate.1)));
        enc.set_gop(spec.gop);
        if let Some((n, d)) = spec.sar {
            enc.set_aspect_ratio(Rational(n, d));
        }
        enc.set_max_b_frames(spec.bframes as usize);
        if global {
            enc.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let mut opts = Dictionary::new();
        if codec.name() == "libx264" {
            opts.set("preset", "veryfast");
            opts.set("crf", "20");
            opts.set(
                "x264-params",
                &format!(
                    "bframes={}:keyint={}:min-keyint={}:scenecut=0",
                    spec.bframes, spec.gop, spec.gop
                ),
            );
        } else {
            opts.set("qscale", "3");
        }
        let enc = enc.open_with(opts).unwrap();
        ost.set_parameters(&enc);
        if let Some((n, d)) = spec.stream_time_base {
            ost.set_time_base(Rational(n, d));
        }
        Muxing {
            index: ost.index(),
            enc: enc.0.0,
            enc_tb,
            frame_size: 0,
            // One nominal frame in the encoder time base.
            default_duration: (i64::from(enc_tb.denominator()) * i64::from(spec.rate.1))
                / (i64::from(enc_tb.numerator()) * i64::from(spec.rate.0)),
        }
    };

    let mut audio = spec.audio.as_ref().map(|a| {
        let codec = encoder::find_by_name("aac").expect("aac encoder");
        let mut ost = octx.add_stream(codec).unwrap();
        let mut enc = codec::context::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .unwrap();
        enc.set_rate(a.rate as i32);
        enc.set_channel_layout(if a.channels == 1 {
            ChannelLayout::MONO
        } else {
            ChannelLayout::STEREO
        });
        enc.set_format(format::Sample::F32(SampleType::Planar));
        enc.set_time_base(Rational(1, a.rate as i32));
        if global {
            enc.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let enc = enc.open_with(aac_options()).unwrap();
        ost.set_parameters(&enc);
        let frame_size = enc.frame_size() as usize;
        Muxing {
            index: ost.index(),
            enc: enc.0.0,
            enc_tb: Rational(1, a.rate as i32),
            frame_size,
            default_duration: frame_size as i64,
        }
    });

    octx.write_header().unwrap();

    let mut next_audio = 0usize;
    for k in 0..spec.frames {
        let mut f = frame::Video::new(Pixel::YUV420P, spec.width, spec.height);
        fill_video(&mut f, k, spec);
        let pts = match &spec.pts {
            Some(list) => list[k as usize],
            None => i64::from(k),
        };
        f.set_pts(Some(pts));
        video.enc.send_frame(&f).unwrap();
        drain(&mut video, &mut octx);
        // Keep audio roughly in step with the picture.
        if let (Some(m), Some(a)) = (audio.as_mut(), spec.audio.as_ref()) {
            let until = ((u64::from(k + 1) * u64::from(a.rate) * spec.rate.1 as u64)
                / spec.rate.0 as u64) as usize;
            next_audio = feed_audio(m, a, next_audio, until.min(a.samples), &mut octx);
        }
    }
    if let (Some(m), Some(a)) = (audio.as_mut(), spec.audio.as_ref()) {
        next_audio = feed_audio(m, a, next_audio, a.samples, &mut octx);
        let _ = next_audio;
        m.enc.send_eof().unwrap();
        drain(m, &mut octx);
    }
    video.enc.send_eof().unwrap();
    drain(&mut video, &mut octx);
    octx.write_trailer().unwrap();
}

/// AAC encoder options. Perceptual noise substitution is off: its noise
/// comes from decoder state that a seek resets, so random access to
/// PNS-coded bands can't match a sequential decode (documented in
/// `freecut_media::AudioReader`).
fn aac_options() -> Dictionary<'static> {
    let mut d = Dictionary::new();
    d.set("aac_pns", "0");
    d
}

/// Encodes AAC frames covering samples `[from, until)`; returns the next
/// sample to encode.
fn feed_audio(
    m: &mut Muxing,
    a: &AudioSpec,
    mut from: usize,
    until: usize,
    octx: &mut format::context::Output,
) -> usize {
    let frame_size = m.frame_size.max(1);
    while from + frame_size <= until || (until == a.samples && from < until) {
        let n = frame_size.min(a.samples - from);
        let layout = if a.channels == 1 {
            ChannelLayout::MONO
        } else {
            ChannelLayout::STEREO
        };
        let mut f = frame::Audio::new(format::Sample::F32(SampleType::Planar), n, layout);
        f.set_rate(a.rate);
        for ch in 0..usize::from(a.channels) {
            let plane = f.plane_mut::<f32>(ch);
            for (i, s) in plane.iter_mut().enumerate().take(n) {
                *s = signal(from + i, ch, a.rate);
            }
        }
        f.set_pts(Some(from as i64));
        m.enc.send_frame(&f).unwrap();
        drain(m, octx);
        from += n;
    }
    from
}

/// Encodes an audio-only AAC clip (`.m4a`/`.mp4`).
pub fn encode_aac(path: &Path, a: &AudioSpec) {
    ff::init().unwrap();
    ff::log::set_level(ff::log::Level::Quiet);
    let mut octx = format::output(&path).unwrap();
    let global = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);
    let codec = encoder::find_by_name("aac").expect("aac encoder");
    let mut m = {
        let mut ost = octx.add_stream(codec).unwrap();
        let mut enc = codec::context::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .unwrap();
        enc.set_rate(a.rate as i32);
        enc.set_channel_layout(if a.channels == 1 {
            ChannelLayout::MONO
        } else {
            ChannelLayout::STEREO
        });
        enc.set_format(format::Sample::F32(SampleType::Planar));
        enc.set_time_base(Rational(1, a.rate as i32));
        if global {
            enc.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let enc = enc.open_with(aac_options()).unwrap();
        ost.set_parameters(&enc);
        let frame_size = enc.frame_size() as usize;
        Muxing {
            index: ost.index(),
            enc: enc.0.0,
            enc_tb: Rational(1, a.rate as i32),
            frame_size,
            default_duration: frame_size as i64,
        }
    };
    octx.write_header().unwrap();
    feed_audio(&mut m, a, 0, a.samples, &mut octx);
    m.enc.send_eof().unwrap();
    drain(&mut m, &mut octx);
    octx.write_trailer().unwrap();
}

/// The interleaved 16-bit samples [`write_wav`] writes.
pub fn wav_samples(rate: u32, channels: u16, frames: usize) -> Vec<i16> {
    let mut samples = Vec::with_capacity(frames * usize::from(channels));
    for n in 0..frames {
        for ch in 0..usize::from(channels) {
            samples.push((signal(n, ch, rate) * 32767.0).round() as i16);
        }
    }
    samples
}

/// Writes 16-bit PCM WAV with [`signal`] samples; returns the samples
/// (interleaved) as written.
pub fn write_wav(path: &Path, rate: u32, channels: u16, frames: usize) -> Vec<i16> {
    let samples = wav_samples(rate, channels, frames);
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in &samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::File::create(path)
        .unwrap()
        .write_all(&out)
        .unwrap();
    samples
}

/// Presentation times of every frame of the first video stream, in media
/// ticks (first frame at 0), from a plain sequential decode with
/// ffmpeg-next: the test oracle. Frames outside an edit list are dropped by
/// the decoder, as in any player.
pub fn frame_starts(path: &Path) -> Vec<Tick> {
    ff::init().unwrap();
    let mut ictx = format::input(&path).unwrap();
    let stream = ictx.streams().best(ff::media::Type::Video).unwrap();
    let index = stream.index();
    let tb = stream.time_base();
    let tb = StreamTimeBase::new(tb.numerator(), tb.denominator()).unwrap();
    let mut dec = codec::context::Context::from_parameters(stream.parameters())
        .unwrap()
        .decoder()
        .video()
        .unwrap();
    let mut pts = Vec::new();
    let mut f = frame::Video::empty();
    for (s, p) in ictx.packets() {
        if s.index() == index {
            dec.send_packet(&p).unwrap();
            while dec.receive_frame(&mut f).is_ok() {
                pts.push(f.timestamp().unwrap());
            }
        }
    }
    dec.send_eof().unwrap();
    while dec.receive_frame(&mut f).is_ok() {
        pts.push(f.timestamp().unwrap());
    }
    let ticks: Vec<Tick> = pts
        .iter()
        .map(|&p| ticks_from_stream(p, tb).unwrap())
        .collect();
    let origin = ticks[0];
    let starts: Vec<Tick> = ticks
        .iter()
        .map(|t| t.checked_sub(origin).unwrap())
        .collect();
    assert!(
        starts.windows(2).all(|w| w[0] < w[1]),
        "oracle timestamps must increase"
    );
    starts
}

/// The `ffmpeg` CLI, when installed.
pub fn ffmpeg_cli() -> Option<&'static str> {
    static FOUND: OnceLock<bool> = OnceLock::new();
    let ok = *FOUND.get_or_init(|| {
        Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success())
    });
    ok.then_some("ffmpeg")
}

/// Runs the `ffmpeg` CLI with `args`; panics with its stderr on failure.
pub fn run_ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "ffmpeg {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Skips the current test (returns from it) when the CLI is missing.
#[macro_export]
macro_rules! require_ffmpeg_cli {
    () => {
        if support::ffmpeg_cli().is_none() {
            eprintln!("skipping: the ffmpeg CLI is not installed");
            return;
        }
    };
}

/// A deterministic xorshift generator for random test targets.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % (hi - lo) as u64) as i64
    }

    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = (self.next_u64() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
    }
}

/// A JPEG of the quadrant pattern carrying EXIF orientation `orientation`
/// (written as a minimal APP1 segment after SOI).
pub fn jpeg_with_orientation(path: &Path, w: u32, h: u32, orientation: u16) {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let (r, g, b) = QUADRANT_COLORS[quadrant(x, y, w, h)];
        image::Rgb([r, g, b])
    });
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
        .encode_image(&img)
        .unwrap();
    // TIFF (little endian): header, one IFD entry (0x0112 SHORT 1 value).
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"II*\0");
    tiff.extend_from_slice(&8u32.to_le_bytes());
    tiff.extend_from_slice(&1u16.to_le_bytes());
    tiff.extend_from_slice(&0x0112u16.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&orientation.to_le_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_le_bytes());
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(&tiff);
    let mut out = jpeg[..2].to_vec(); // SOI
    out.extend_from_slice(&[0xff, 0xe1]);
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg[2..]);
    std::fs::write(path, out).unwrap();
}

/// A PNG with an alpha gradient (straight alpha).
pub fn png_with_alpha(path: &Path, w: u32, h: u32) {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([
            (x * 255 / w) as u8,
            (y * 255 / h) as u8,
            128,
            (x * 255 / w.max(1)) as u8,
        ])
    });
    img.save(path).unwrap();
}

/// Rewrites `src` into `dst` with the CLI, keeping the streams (`-c copy`),
/// after `input_args` (e.g. `-display_rotation:v:0 90`).
pub fn remux_with(src: &Path, dst: &Path, input_args: &[&str]) {
    let mut args: Vec<&str> = input_args.to_vec();
    args.push("-i");
    args.push(src.to_str().unwrap());
    args.extend_from_slice(&["-c", "copy"]);
    args.push(dst.to_str().unwrap());
    run_ffmpeg(&args);
}
