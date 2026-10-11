//! What an export encodes, and how that maps to encoder settings (pure; no
//! FFmpeg calls).

use freecut_timeline::time::{FrameRate, SampleRate};

use super::EncodeError;

/// The largest width or height accepted (EX-029: larger sizes are refused;
/// H.264's highest level tops out around 8192 pixels on a side).
pub const MAX_DIMENSION: u32 = 8192;

/// AAC bitrates offered for video exports (EX-031).
pub const AAC_KBPS_CHOICES: [u32; 5] = [96, 128, 192, 256, 320];

/// The default AAC bitrate (EX-031).
pub const DEFAULT_AAC_KBPS: u32 = 192;

/// Sample rates FFmpeg's native AAC encoder accepts.
const AAC_SAMPLE_RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

/// The lowest and highest custom video target (EX-028: 0.3 Mbps floor for
/// target-size mode, 300 Mbps ceiling).
const MIN_TARGET_KBPS: u32 = 300;
const MAX_TARGET_KBPS: u32 = 300_000;

/// EX-027's quality presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QualityTier {
    /// Half the Recommended bitrate.
    Smaller,
    /// The FreeCut formula's bitrate (the default).
    Recommended,
    /// Twice the Recommended bitrate.
    Higher,
}

/// How the video bitrate is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoQuality {
    /// A preset tier: the bitrate follows from size and frame rate
    /// ([`tier_kbps`]).
    Tier(QualityTier),
    /// An explicit single-pass VBR target in kbps (EX-028's "Target
    /// bitrate" with peaks capped), 300–300 000.
    TargetKbps(u32),
}

/// The audio stream of a video export (EX-031): AAC-LC, stereo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioSpec {
    /// The project rate (48 kHz default, 44.1 kHz allowed).
    pub sample_rate: SampleRate,
    /// Must be 2 (mono is v1).
    pub channels: u16,
    /// AAC bitrate in kbps (one of [`AAC_KBPS_CHOICES`] in the UI; 32–512
    /// accepted).
    pub kbps: u32,
}

/// Everything an [`EncodeSession`](super::EncodeSession) needs to know about
/// the output: H.264 High 4:2:0 8-bit in MP4 with BT.709 tags (EX-029,
/// EX-032), plus optional AAC audio (EX-031).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EncodeSpec {
    /// Output width in pixels: even, 2–[`MAX_DIMENSION`].
    pub width: u32,
    /// Output height in pixels: even, 2–[`MAX_DIMENSION`].
    pub height: u32,
    /// Frames per second. Frame `n` is presented at `n / frame_rate`.
    pub frame_rate: FrameRate,
    /// The video bitrate.
    pub video: VideoQuality,
    /// The audio stream, if any.
    pub audio: Option<AudioSpec>,
    /// The `title` tag (EX-040: the export dialog's name field). The only
    /// other metadata written is the creation time and FFmpeg's `encoder`
    /// tag.
    pub title: Option<String>,
}

impl EncodeSpec {
    /// Checks the spec before anything is created.
    pub fn validate(&self) -> Result<(), EncodeError> {
        for d in [self.width, self.height] {
            if d == 0 {
                return Err(EncodeError::InvalidSpec("the output size is zero"));
            }
            if !d.is_multiple_of(2) {
                return Err(EncodeError::InvalidSpec(
                    "4:2:0 output needs an even width and height",
                ));
            }
            if d > MAX_DIMENSION {
                return Err(EncodeError::InvalidSpec(
                    "the output is larger than 8192 pixels on a side",
                ));
            }
        }
        if let VideoQuality::TargetKbps(k) = self.video
            && !(MIN_TARGET_KBPS..=MAX_TARGET_KBPS).contains(&k)
        {
            return Err(EncodeError::InvalidSpec(
                "the video bitrate must be between 0.3 and 300 Mbps",
            ));
        }
        if let Some(a) = &self.audio {
            if a.channels != 2 {
                return Err(EncodeError::InvalidSpec("audio must be stereo"));
            }
            if !AAC_SAMPLE_RATES.contains(&a.sample_rate.hz()) {
                return Err(EncodeError::InvalidSpec(
                    "AAC doesn't support this sample rate",
                ));
            }
            if !(32..=512).contains(&a.kbps) {
                return Err(EncodeError::InvalidSpec(
                    "the audio bitrate must be between 32 and 512 kbps",
                ));
            }
        }
        if self.title.as_deref().is_some_and(|t| t.contains('\0')) {
            return Err(EncodeError::InvalidSpec(
                "the title contains a NUL character",
            ));
        }
        Ok(())
    }

    /// The video target bitrate in kbps.
    pub fn video_kbps(&self) -> u32 {
        match self.video {
            VideoQuality::Tier(t) => tier_kbps(t, self.width, self.height, self.frame_rate),
            VideoQuality::TargetKbps(k) => k,
        }
    }

    /// The keyframe interval in frames: 2 s, rounded (EX-027).
    pub fn gop_frames(&self) -> u32 {
        gop_frames(self.frame_rate)
    }
}

/// EX-027's H.264 Recommended bitrate in kbps:
/// `10 Mbps × (W×H / 1920×1080)^0.85 × fps_factor`, with `fps_factor` 1.0 up
/// to 30 fps, 1.5 up to 60 and 2.25 above, rounded to 0.1 Mbps and at least
/// 0.5 Mbps. (Float math on a non-time quantity, as the spec allows.)
pub fn recommended_kbps(width: u32, height: u32, rate: FrameRate) -> u32 {
    scaled_kbps(width, height, rate, 1.0)
}

/// The bitrate of `tier` in kbps (Smaller ×0.5, Higher ×2, then rounded and
/// floored as for [`recommended_kbps`]).
pub fn tier_kbps(tier: QualityTier, width: u32, height: u32, rate: FrameRate) -> u32 {
    let factor = match tier {
        QualityTier::Smaller => 0.5,
        QualityTier::Recommended => 1.0,
        QualityTier::Higher => 2.0,
    };
    scaled_kbps(width, height, rate, factor)
}

fn scaled_kbps(width: u32, height: u32, rate: FrameRate, factor: f64) -> u32 {
    let (num, den) = (u64::from(rate.num()), u64::from(rate.den()));
    let fps_factor = if num <= 30 * den {
        1.0
    } else if num <= 60 * den {
        1.5
    } else {
        2.25
    };
    let pixels = f64::from(width) * f64::from(height);
    let mbps = 10.0 * (pixels / 2_073_600.0).powf(0.85) * fps_factor * factor;
    let tenths = (mbps * 10.0).round().max(5.0);
    // At most 8192² × 2.25 × 2 → ≈ 5 Gbps: far inside u32 kbps. The cast
    // saturates anyway.
    (tenths * 100.0) as u32
}

/// Frames in a 2 s keyframe interval at `rate`, rounded half up, at least 1.
pub fn gop_frames(rate: FrameRate) -> u32 {
    let (num, den) = (u64::from(rate.num()), u64::from(rate.den()));
    // round(2·num/den) = floor((4·num + den) / (2·den))
    let g = (4 * num + den) / (2 * den);
    u32::try_from(g).unwrap_or(u32::MAX).max(1)
}

/// True when video frame `frame` (presented at `frame / rate`) comes before
/// audio sample `sample` (at `sample / sample_rate`), compared exactly.
/// Producers feeding one [`EncodeSession`](super::EncodeSession) push
/// whichever stream is behind, which keeps the muxer's interleaving queue
/// short (export-pipeline §7.2).
pub fn video_due_before_audio(
    frame: u64,
    rate: FrameRate,
    sample: u64,
    sample_rate: SampleRate,
) -> bool {
    // frame·den/num < sample/sr  ⇔  frame·den·sr < sample·num
    u128::from(frame) * u128::from(rate.den()) * u128::from(sample_rate.hz())
        < u128::from(sample) * u128::from(rate.num())
}

/// libx264 options for EX-027's single-pass VBR at `kbps` (all passed to
/// `avcodec_open2` and checked for leftovers):
///
/// | option | value | why |
/// |---|---|---|
/// | `b` | target | EX-027 target bitrate |
/// | `maxrate` | 1.5 × target | peak cap |
/// | `bufsize` | 2 × target | VBV buffer |
/// | `g` | 2 s of frames | keyframe interval |
/// | `preset` | `medium` | EX-028's "balanced" speed |
/// | `profile` | `high` | EX-029: High profile, never forced higher |
/// | `threads` | cores, at most 16 | P17 |
///
/// x264 GOPs are closed by default (`open-gop=0`); B-frames and the level
/// stay at x264's choice (EX-029).
#[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
pub(crate) fn x264_options(kbps: u32, gop: u32, threads: usize) -> Vec<(&'static str, String)> {
    let bps = u64::from(kbps) * 1000;
    vec![
        ("b", bps.to_string()),
        ("maxrate", (bps * 3 / 2).to_string()),
        ("bufsize", (bps * 2).to_string()),
        ("g", gop.to_string()),
        ("preset", "medium".to_owned()),
        ("profile", "high".to_owned()),
        ("threads", threads.clamp(1, 16).to_string()),
    ]
}

/// FFmpeg's native AAC encoder options: the bitrate only. The profile is
/// AAC-LC by default (EX-029; FDK-AAC is never used).
#[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
pub(crate) fn aac_options(kbps: u32) -> Vec<(&'static str, String)> {
    vec![("b", (u64::from(kbps) * 1000).to_string())]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;

    fn rate(num: u32, den: u32) -> FrameRate {
        FrameRate::new(num, den).unwrap()
    }

    fn spec(w: u32, h: u32) -> EncodeSpec {
        EncodeSpec {
            width: w,
            height: h,
            frame_rate: rate(25, 1),
            video: VideoQuality::Tier(QualityTier::Recommended),
            audio: Some(AudioSpec {
                sample_rate: SampleRate::new(48_000).unwrap(),
                channels: 2,
                kbps: DEFAULT_AAC_KBPS,
            }),
            title: None,
        }
    }

    /// EX-027's table (H.264, ≤ 30 fps).
    #[test]
    fn recommended_bitrates_match_the_spec_table() {
        let r30 = rate(30, 1);
        let table = [
            ((854, 480), 2_500),
            ((1280, 720), 5_000),
            ((1920, 1080), 10_000),
            ((2560, 1440), 16_300),
            ((3840, 2160), 32_500),
            ((7680, 4320), 105_600),
        ];
        for ((w, h), kbps) in table {
            assert_eq!(recommended_kbps(w, h, r30), kbps, "{w}x{h}");
        }
        // Portrait has the same pixel count, so the same bitrate.
        assert_eq!(recommended_kbps(1080, 1920, r30), 10_000);
    }

    #[test]
    fn tiers_and_frame_rate_factors() {
        let (w, h) = (1920, 1080);
        assert_eq!(tier_kbps(QualityTier::Smaller, w, h, rate(25, 1)), 5_000);
        assert_eq!(tier_kbps(QualityTier::Higher, w, h, rate(25, 1)), 20_000);
        assert_eq!(recommended_kbps(w, h, rate(30_000, 1001)), 10_000);
        assert_eq!(recommended_kbps(w, h, rate(50, 1)), 15_000);
        assert_eq!(recommended_kbps(w, h, rate(60, 1)), 15_000);
        assert_eq!(recommended_kbps(w, h, rate(120, 1)), 22_500);
        // Tiny outputs get the 0.5 Mbps floor.
        assert_eq!(recommended_kbps(64, 48, rate(25, 1)), 500);
        assert_eq!(tier_kbps(QualityTier::Smaller, 64, 48, rate(25, 1)), 500);
    }

    #[test]
    fn gop_is_two_seconds() {
        assert_eq!(gop_frames(rate(25, 1)), 50);
        assert_eq!(gop_frames(rate(30_000, 1001)), 60);
        assert_eq!(gop_frames(rate(24_000, 1001)), 48);
        assert_eq!(gop_frames(rate(60, 1)), 120);
    }

    #[test]
    fn x264_options_follow_the_tier_rules() {
        let o = x264_options(10_000, 50, 64);
        let get = |k: &str| o.iter().find(|(key, _)| *key == k).unwrap().1.clone();
        assert_eq!(get("b"), "10000000");
        assert_eq!(get("maxrate"), "15000000");
        assert_eq!(get("bufsize"), "20000000");
        assert_eq!(get("g"), "50");
        assert_eq!(get("profile"), "high");
        assert_eq!(get("threads"), "16");
        assert_eq!(aac_options(192), vec![("b", "192000".to_owned())]);
    }

    #[test]
    fn validation() {
        assert!(spec(64, 48).validate().is_ok());
        for (w, h) in [(63, 48), (64, 47), (0, 48), (64, 0), (8194, 48)] {
            assert!(
                matches!(spec(w, h).validate(), Err(EncodeError::InvalidSpec(_))),
                "{w}x{h}"
            );
        }
        assert!(spec(8192, 4320).validate().is_ok());
        let mut s = spec(64, 48);
        s.audio.as_mut().unwrap().channels = 1;
        assert!(s.validate().is_err());
        let mut s = spec(64, 48);
        s.audio.as_mut().unwrap().sample_rate = SampleRate::new(70_560).unwrap();
        assert!(s.validate().is_err());
        let mut s = spec(64, 48);
        s.video = VideoQuality::TargetKbps(100);
        assert!(s.validate().is_err());
        s.video = VideoQuality::TargetKbps(8_000);
        assert!(s.validate().is_ok());
        assert_eq!(s.video_kbps(), 8_000);
        s.title = Some("a\0b".into());
        assert!(s.validate().is_err());
    }

    #[test]
    fn interleaving_order_is_exact() {
        let sr = SampleRate::new(48_000).unwrap();
        // 29.97: frame 30 starts at 1.001 s = sample 48 048.
        let r = rate(30_000, 1001);
        assert!(video_due_before_audio(30, r, 48_049, sr));
        assert!(!video_due_before_audio(30, r, 48_048, sr));
        assert!(!video_due_before_audio(30, r, 48_000, sr));
        // Ties go to audio, so a frame is pushed once the audio up to its
        // start is in.
        assert!(!video_due_before_audio(0, rate(25, 1), 0, sr));
        assert!(video_due_before_audio(0, rate(25, 1), 1, sr));
    }
}
