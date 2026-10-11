//! Export encoding end to end (EX-022, EX-027, EX-029, EX-031, EX-032,
//! EX-040): synthetic frames and a tone go through `EncodeSession`, and the
//! file is checked with `verify_output` and decoded back with the crate's own
//! readers. Tests skip (with a message) when the linked FFmpeg has no
//! libx264.
#![cfg(feature = "ffmpeg")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::f64::consts::PI;
use std::path::{Path, PathBuf};

use freecut_media::encode::{
    AudioSpec, EncodeError, EncodeSession, EncodeSpec, OutputInfo, QualityTier, VideoQuality,
    verify_output, video_due_before_audio,
};
use freecut_media::{AudioReader, CancelToken, SeekMode, VideoDecoder};
use freecut_timeline::time::{FrameRate, SampleRate, TICKS_PER_SECOND, Tick};

const W: u32 = 64;
const H: u32 = 48;
const SR: u32 = 48_000;
/// Samples per mixer block (AU-010), so the FIFO re-chunks to 1024.
const BLOCK: u64 = 1000;
/// Length of the click burst in samples.
const CLICK_LEN: u64 = 24;

fn have_x264() -> bool {
    let ok = freecut_media::init().unwrap().has_encoder("libx264");
    if !ok {
        eprintln!("skipping: this FFmpeg build has no libx264");
    }
    ok
}

fn rate(num: u32, den: u32) -> FrameRate {
    FrameRate::new(num, den).unwrap()
}

fn spec(r: FrameRate, audio: bool) -> EncodeSpec {
    EncodeSpec {
        width: W,
        height: H,
        frame_rate: r,
        video: VideoQuality::Tier(QualityTier::Recommended),
        audio: audio.then(|| AudioSpec {
            sample_rate: SampleRate::new(SR).unwrap(),
            channels: 2,
            kbps: 192,
        }),
        title: Some("Test export".into()),
    }
}

/// A test clip: `frames` frames of dark gray with a moving square, frame
/// `flash` all white; 1 kHz tone at −20 dB with a full-scale click starting
/// at the flash frame's first sample.
struct Clip {
    rate: FrameRate,
    frames: u64,
    flash: u64,
    samples: u64,
}

impl Clip {
    /// Two seconds at `r`, flash on the first frame starting at or after
    /// 1 s.
    fn two_seconds(r: FrameRate) -> Clip {
        let (num, den) = (u64::from(r.num()), u64::from(r.den()));
        Clip {
            rate: r,
            frames: (2 * num).div_ceil(den),
            flash: num.div_ceil(den),
            samples: 2 * u64::from(SR),
        }
    }

    /// First sample of frame `k` (exact for the rates used here).
    fn frame_sample(&self, k: u64) -> u64 {
        let (num, den) = (u64::from(self.rate.num()), u64::from(self.rate.den()));
        let s = k * den * u64::from(SR);
        assert_eq!(s % num, 0, "frame {k} doesn't start on a sample");
        s / num
    }

    fn click_start(&self) -> u64 {
        self.frame_sample(self.flash)
    }

    fn frame(&self, k: u64) -> Vec<u8> {
        let mut px = vec![0u8; (W * H * 4) as usize];
        for y in 0..H {
            for x in 0..W {
                let i = ((y * W + x) * 4) as usize;
                let sq = (k as u32 * 2) % (W - 8);
                let v = if k == self.flash {
                    255
                } else if x >= sq && x < sq + 8 && (16..24).contains(&y) {
                    140
                } else {
                    40
                };
                px[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        px
    }

    fn sample(&self, n: u64) -> f32 {
        let c = self.click_start();
        if (c..c + CLICK_LEN).contains(&n) {
            0.9
        } else {
            (0.1 * (2.0 * PI * 1000.0 * n as f64 / f64::from(SR)).sin()) as f32
        }
    }

    fn block(&self, start: u64, len: u64) -> Vec<f32> {
        (start..start + len)
            .flat_map(|n| {
                let v = self.sample(n);
                [v, v]
            })
            .collect()
    }
}

/// Encodes `clip` the way the export job does: whichever stream is behind
/// is pushed next, audio in mixer-sized blocks.
fn encode(path: &Path, spec: &EncodeSpec, clip: &Clip) -> freecut_media::encode::MuxStats {
    let mut s = EncodeSession::create(path, spec).unwrap();
    let sr = SampleRate::new(SR).unwrap();
    let (mut k, mut n) = (0u64, 0u64);
    let audio = spec.audio.is_some();
    while k < clip.frames || (audio && n < clip.samples) {
        let video_next = k < clip.frames
            && (!audio || n >= clip.samples || video_due_before_audio(k, clip.rate, n, sr));
        if video_next {
            s.push_video(&clip.frame(k), k).unwrap();
            k += 1;
        } else {
            let len = BLOCK.min(clip.samples - n);
            s.push_audio(&clip.block(n, len)).unwrap();
            n += len;
        }
    }
    assert_eq!(s.frames_pushed(), clip.frames);
    s.finish(&CancelToken::new()).unwrap()
}

fn mean_red(rgba: &[u8]) -> f64 {
    let sum: u64 = rgba.chunks(4).map(|p| u64::from(p[0])).sum();
    sum as f64 / (rgba.len() / 4) as f64
}

/// Decodes the file and returns (flash frame start, click start) in ticks.
fn flash_and_click(path: &Path, info: &OutputInfo, clip: &Clip) -> (Tick, Tick) {
    let v = info.video.as_ref().unwrap();
    let mut dec = VideoDecoder::open(path, v.index).unwrap();
    let mut flash = None;
    for k in 0..clip.frames {
        let t = clip.rate.frame_start(k as i64).unwrap();
        let f = dec.frame_at(t, SeekMode::Exact).unwrap();
        if mean_red(&f.rgba) > 200.0 {
            flash = Some(f.pts);
            break;
        }
    }
    let flash = flash.expect("no flash frame decoded");

    let a = info.audio.as_ref().unwrap();
    let mut reader = AudioReader::open(path, a.index, SampleRate::new(SR).unwrap(), 2).unwrap();
    let pcm = reader.read(Tick::ZERO, clip.samples as usize).unwrap();
    let left: Vec<f32> = pcm.chunks(2).map(|f| f[0]).collect();
    let (peak, level) =
        left.iter()
            .map(|v| v.abs())
            .enumerate()
            .fold(
                (0, 0.0f32),
                |best, (i, v)| if v > best.1 { (i, v) } else { best },
            );
    assert!(level > 0.5, "no click found (peak {level})");
    // The burst's leading edge: walk back from the peak while the signal
    // stays above half of it (the tone is at 0.1, the click at 0.9).
    let mut onset = peak;
    while onset > 0 && left[onset - 1].abs() > level / 2.0 {
        onset -= 1;
    }
    let click = Tick::from_raw(onset as i64 * (TICKS_PER_SECOND / i64::from(SR)));
    (flash, click)
}

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn assert_clip_file(path: &Path, spec: &EncodeSpec, clip: &Clip) {
    let info = verify_output(path).unwrap();

    // H.264 High 4:2:0 with BT.709 tags (EX-029, EX-032).
    let v = info.video.as_ref().unwrap();
    assert_eq!(v.codec, "h264");
    assert_eq!(v.profile.as_deref(), Some("High"));
    assert_eq!(v.pix_fmt.as_deref(), Some("yuv420p"));
    assert_eq!((v.width, v.height), (W, H));
    assert!(v.color.is_bt709_limited(), "{:?}", v.color);
    assert_eq!(v.color.chroma_location.as_deref(), Some("left"));
    let nclx = info.nclx.expect("no colr box");
    assert_eq!(
        (nclx.primaries, nclx.transfer, nclx.matrix, nclx.full_range),
        (1, 1, 1, false)
    );
    // One packet per frame, nothing dropped or added (EX-022, EX-040).
    assert_eq!(v.packets, clip.frames);
    assert!(v.keyframes >= 1);
    assert_eq!(
        v.duration,
        Some(clip.rate.frames(clip.frames as i64).unwrap()),
        "video duration"
    );

    // Fast start: the index precedes the media (EX-029).
    assert_eq!(
        info.index_before_media(),
        Some(true),
        "{:?}",
        info.top_level_boxes
    );
    assert_eq!(
        info.top_level_boxes.first().map(String::as_str),
        Some("ftyp")
    );

    // AAC-LC stereo, as long as what was pushed (within one AAC frame).
    let a = info.audio.as_ref().unwrap();
    assert_eq!(a.codec, "aac");
    assert_eq!(a.profile.as_deref(), Some("LC"));
    assert_eq!((a.sample_rate, a.channels), (SR, 2));
    let samples = a.duration.unwrap().raw() / (TICKS_PER_SECOND / i64::from(SR));
    assert!(
        samples.abs_diff(clip.samples as i64) <= 1024,
        "audio has {samples} samples, pushed {}",
        clip.samples
    );

    // EX-040: technical metadata only.
    let allowed = [
        "major_brand",
        "minor_version",
        "compatible_brands",
        "encoder",
        "creation_time",
        "title",
    ];
    for key in &info.metadata_keys {
        assert!(allowed.contains(&key.as_str()), "unexpected metadata {key}");
    }
    assert!(info.metadata_keys.iter().any(|k| k == "creation_time"));
    assert!(info.metadata_keys.iter().any(|k| k == "title"));

    // The job's own check agrees.
    info.check(spec, clip.frames, clip.samples).unwrap();

    // A/V sync: the white frame and the click line up within 1 ms.
    let (flash, click) = flash_and_click(path, &info, clip);
    let expected = clip.rate.frame_start(clip.flash as i64).unwrap();
    assert_eq!(flash, expected, "flash frame time");
    let skew = (click.raw() - flash.raw()).abs();
    assert!(
        skew <= TICKS_PER_SECOND / 1000,
        "click at {click:?}, flash at {flash:?}: {} µs apart",
        skew * 1_000_000 / TICKS_PER_SECOND
    );
}

#[test]
fn encodes_25_fps_with_audio() {
    if !have_x264() {
        return;
    }
    let dir = tmp();
    let path = dir.path().join("a.mp4");
    let r = rate(25, 1);
    let (spec, clip) = (spec(r, true), Clip::two_seconds(r));
    let stats = encode(&path, &spec, &clip);
    assert_eq!(stats.video_frames, 50);
    assert_eq!(stats.video_packets, 50);
    assert_eq!(stats.audio_samples, 96_000);
    assert!(stats.bytes > 0);
    assert_clip_file(&path, &spec, &clip);
}

#[test]
fn encodes_ntsc_rate_with_audio() {
    if !have_x264() {
        return;
    }
    let dir = tmp();
    let path = dir.path().join("b.mp4");
    let r = rate(30_000, 1001);
    let (spec, clip) = (spec(r, true), Clip::two_seconds(r));
    assert_eq!((clip.frames, clip.flash), (60, 30));
    assert_eq!(clip.click_start(), 48_048);
    encode(&path, &spec, &clip);
    assert_clip_file(&path, &spec, &clip);
}

#[test]
fn video_only_exports_have_no_audio_stream() {
    if !have_x264() {
        return;
    }
    let dir = tmp();
    let path = dir.path().join("v.mp4");
    let r = rate(25, 1);
    let mut spec = spec(r, false);
    spec.title = None;
    let clip = Clip {
        frames: 10,
        ..Clip::two_seconds(r)
    };
    let mut s = EncodeSession::create(&path, &spec).unwrap();
    assert!(matches!(
        s.push_audio(&[0.0, 0.0]),
        Err(EncodeError::InvalidInput(_))
    ));
    for k in 0..clip.frames {
        s.push_video(&clip.frame(k), k).unwrap();
    }
    let stats = s.finish(&CancelToken::new()).unwrap();
    assert_eq!(stats.audio_packets, 0);
    let info = verify_output(&path).unwrap();
    assert!(info.audio.is_none());
    assert!(!info.metadata_keys.iter().any(|k| k == "title"));
    info.check(&spec, 10, 0).unwrap();
    // The check notices a wrong frame count.
    assert!(matches!(
        info.check(&spec, 11, 0),
        Err(EncodeError::Verify(_))
    ));
}

#[test]
fn transparent_pixels_are_composited_over_black() {
    if !have_x264() {
        return;
    }
    let dir = tmp();
    let path = dir.path().join("alpha.mp4");
    let r = rate(25, 1);
    let spec = spec(r, false);
    let mut s = EncodeSession::create(&path, &spec).unwrap();
    // White at alpha 0 must come out black, not white.
    let clear: Vec<u8> = [255u8, 255, 255, 0].repeat((W * H) as usize);
    for k in 0..3 {
        s.push_video(&clear, k).unwrap();
    }
    s.finish(&CancelToken::new()).unwrap();
    let mut dec = VideoDecoder::open(&path, 0).unwrap();
    let f = dec.frame_at(Tick::ZERO, SeekMode::Exact).unwrap();
    assert!(mean_red(&f.rgba) < 8.0, "mean {}", mean_red(&f.rgba));
}

#[test]
fn abandon_and_drop_delete_the_file() {
    if !have_x264() {
        return;
    }
    let dir = tmp();
    let r = rate(25, 1);
    let clip = Clip::two_seconds(r);
    let spec = spec(r, true);

    let path = dir.path().join("abandoned.mp4.part");
    let mut s = EncodeSession::create(&path, &spec).unwrap();
    for k in 0..5 {
        s.push_video(&clip.frame(k), k).unwrap();
    }
    s.push_audio(&clip.block(0, 4800)).unwrap();
    assert!(path.exists());
    s.abandon().unwrap();
    assert!(!path.exists());

    let path = dir.path().join("dropped.mp4.part");
    let s = EncodeSession::create(&path, &spec).unwrap();
    assert!(path.exists());
    drop(s);
    assert!(!path.exists());

    // A cancel during the final flush also leaves nothing behind.
    let path = dir.path().join("cancelled.mp4.part");
    let mut s = EncodeSession::create(&path, &spec).unwrap();
    for k in 0..5 {
        s.push_video(&clip.frame(k), k).unwrap();
    }
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(matches!(s.finish(&cancel), Err(EncodeError::Cancelled)));
    assert!(!path.exists());

    // Finishing without frames is an error, and also cleans up.
    let path = dir.path().join("empty.mp4.part");
    let s = EncodeSession::create(&path, &spec).unwrap();
    assert!(matches!(
        s.finish(&CancelToken::new()),
        Err(EncodeError::InvalidInput(_))
    ));
    assert!(!path.exists());
}

#[test]
fn bad_specs_and_inputs_are_refused() {
    if !have_x264() {
        return;
    }
    let dir = tmp();
    let r = rate(25, 1);
    let path: PathBuf = dir.path().join("odd.mp4");
    for (w, h) in [(63, 48), (64, 47), (0, 48)] {
        let s = EncodeSpec {
            width: w,
            height: h,
            ..spec(r, true)
        };
        assert!(matches!(
            EncodeSession::create(&path, &s),
            Err(EncodeError::InvalidSpec(_))
        ));
        assert!(!path.exists(), "a refused spec must not create a file");
    }

    let path = dir.path().join("order.mp4");
    let spec = spec(r, true);
    let clip = Clip::two_seconds(r);
    let mut s = EncodeSession::create(&path, &spec).unwrap();
    assert!(matches!(
        s.push_video(&clip.frame(1), 1),
        Err(EncodeError::OutOfOrder {
            expected: 0,
            got: 1
        })
    ));
    assert!(matches!(
        s.push_video(&[0u8; 10], 0),
        Err(EncodeError::InvalidInput(_))
    ));
    assert!(matches!(
        s.push_audio(&[0.0; 3]),
        Err(EncodeError::InvalidInput(_))
    ));
    s.push_video(&clip.frame(0), 0).unwrap();
    assert!(matches!(
        s.push_video(&clip.frame(0), 0),
        Err(EncodeError::OutOfOrder { expected: 1, .. })
    ));
    s.abandon().unwrap();

    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let bad = dir.path().join(OsStr::from_bytes(b"\xff.mp4"));
        assert!(matches!(
            EncodeSession::create(&bad, &spec),
            Err(EncodeError::UnsupportedPath(_))
        ));
    }

    // A missing folder is an FFmpeg open error, not a panic.
    let missing = dir.path().join("no/such/dir/x.mp4");
    assert!(matches!(
        EncodeSession::create(&missing, &spec),
        Err(EncodeError::Ffmpeg { .. })
    ));
}
