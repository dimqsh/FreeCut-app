//! The export settings model (EX-021, EX-023, EX-024, EX-026, EX-027,
//! EX-031) and the pure resolution steps that turn it into concrete numbers.

use std::path::PathBuf;

use freecut_timeline::model::{Document, ProjectSettings};
use freecut_timeline::time::{FrameRate, SampleRate};

use super::bitrate::tier_kbps;

/// The largest output side any MVP encoder accepts (x264 within level 6.2,
/// EX-029).
pub const MAX_OUTPUT_SIDE: u32 = 8192;

/// The AAC bitrates offered (EX-031), in kbps.
pub const AAC_KBPS_CHOICES: [u32; 5] = [96, 128, 192, 256, 320];

/// The default AAC bitrate (EX-031).
pub const DEFAULT_AAC_KBPS: u32 = 192;

/// The output sample rates offered (EX-031, PM-007).
pub const AUDIO_SAMPLE_RATES: [u32; 2] = [44_100, 48_000];

/// The default output sample rate when the project's isn't offered.
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// The output frame rates EX-026 lists, as exact rationals: 23.976, 24, 25,
/// 29.97, 30, 48, 50, 59.94 and 60. The project rate is always offered too.
pub const FRAME_RATE_CHOICES: [(u32, u32); 9] = [
    (24_000, 1001),
    (24, 1),
    (25, 1),
    (30_000, 1001),
    (30, 1),
    (48, 1),
    (50, 1),
    (60_000, 1001),
    (60, 1),
];

/// The output container. The MVP writes MP4 only (EX-029).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Container {
    #[default]
    Mp4,
}

impl Container {
    /// The file extension, without the dot (EX-023).
    pub const fn extension(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
        }
    }
}

/// The video codec. The MVP encodes H.264 only (EX-029).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    #[default]
    H264,
}

impl VideoCodec {
    /// EX-027's codec factor in the bitrate formula, in per-mille (H.264
    /// 1.0; HEVC and VP9 0.6 and AV1 0.5 arrive with EX-030).
    pub(crate) const fn factor_per_mille(self) -> u32 {
        match self {
            VideoCodec::H264 => 1000,
        }
    }
}

/// Whether `codec` can be written into `container` (README C39: the pure
/// table; the FFmpeg cross-check lives in the app).
pub const fn compatible(codec: VideoCodec, container: Container) -> bool {
    match (codec, container) {
        (VideoCodec::H264, Container::Mp4) => true,
    }
}

/// A resolution preset, named by the output's short side (EX-024).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShortSide {
    P480,
    P720,
    P1080,
    P1440,
    P2160,
}

impl ShortSide {
    /// Every preset, smallest first.
    pub const ALL: [ShortSide; 5] = [
        ShortSide::P480,
        ShortSide::P720,
        ShortSide::P1080,
        ShortSide::P1440,
        ShortSide::P2160,
    ];

    /// The short side in pixels.
    pub const fn px(self) -> u32 {
        match self {
            ShortSide::P480 => 480,
            ShortSide::P720 => 720,
            ShortSide::P1080 => 1080,
            ShortSide::P1440 => 1440,
            ShortSide::P2160 => 2160,
        }
    }

    /// The preset with a short side of `px`, if there is one.
    pub fn from_px(px: u32) -> Option<ShortSide> {
        ShortSide::ALL.into_iter().find(|s| s.px() == px)
    }
}

/// The output size choice (EX-024). Custom sizes and 8K are EX-025 (v1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum OutputSize {
    /// The canvas size, rounded to even sides ("Canvas size (W×H)").
    #[default]
    Canvas,
    Preset(ShortSide),
}

/// The output frame rate choice (EX-026). The MVP dialog offers only
/// `Project`; `Fixed` holds one of [`FRAME_RATE_CHOICES`] or the project
/// rate, and is stored so v1 needs no format change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FrameRateChoice {
    /// Whatever the project rate is when the export starts.
    #[default]
    Project,
    Fixed(FrameRate),
}

impl FrameRateChoice {
    /// The rate to export at for a project running at `project`.
    pub const fn resolve(self, project: FrameRate) -> FrameRate {
        match self {
            FrameRateChoice::Project => project,
            FrameRateChoice::Fixed(rate) => rate,
        }
    }

    /// Whether this is a rate the export dialog offers for a project at
    /// `project` (EX-026).
    pub fn is_offered(self, project: FrameRate) -> bool {
        match self {
            FrameRateChoice::Project => true,
            FrameRateChoice::Fixed(rate) => {
                rate == project
                    || FRAME_RATE_CHOICES
                        .iter()
                        .any(|&(num, den)| rate.num() == num && rate.den() == den)
            }
        }
    }
}

/// The bitrate tiers (EX-027). Custom rate control is EX-028 (v1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum QualityTier {
    /// Half the recommended bitrate.
    Smaller,
    #[default]
    Recommended,
    /// Twice the recommended bitrate.
    Higher,
}

impl QualityTier {
    pub const ALL: [QualityTier; 3] = [
        QualityTier::Smaller,
        QualityTier::Recommended,
        QualityTier::Higher,
    ];

    /// The tier's factor on the recommended bitrate, in per-mille.
    pub(crate) const fn factor_per_mille(self) -> u32 {
        match self {
            QualityTier::Smaller => 500,
            QualityTier::Recommended => 1000,
            QualityTier::Higher => 2000,
        }
    }
}

/// Output name and folder (EX-023).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct FileTarget {
    /// The name as the user typed it, without extension. It is sanitized
    /// ([`super::sanitize_file_stem`]) when the export starts, never
    /// silently while typing.
    pub name: String,
    /// The destination folder; `None` until one is chosen.
    pub folder: Option<PathBuf>,
}

/// Video settings (EX-021 §4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct VideoSettings {
    pub size: OutputSize,
    pub frame_rate: FrameRateChoice,
    /// Read-only in the MVP: H.264.
    pub codec: VideoCodec,
    /// Read-only in the MVP: MP4.
    pub container: Container,
    pub quality: QualityTier,
}

/// Audio of a video export (EX-031): AAC-LC stereo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioExportSettings {
    /// Whether the file gets an audio stream at all.
    pub include: bool,
    /// One of [`AAC_KBPS_CHOICES`].
    pub kbps: u32,
    /// One of [`AUDIO_SAMPLE_RATES`].
    pub sample_rate: SampleRate,
}

/// The settings of one export (EX-021). Saved in the project file when an
/// export starts; not an undoable edit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExportSettings {
    pub file: FileTarget,
    pub video: VideoSettings,
    pub audio: AudioExportSettings,
}

impl ExportSettings {
    /// The first-export defaults (EX-021): name = `name` (the project
    /// name), folder = `folder` (the last export folder on this machine, or
    /// the OS videos folder, chosen by the app), canvas size, project rate,
    /// Recommended, AAC 192 kbps at the project sample rate (48 kHz if that
    /// rate isn't offered). Audio is off when the root sequence has nothing
    /// audible (EX-031, "Timeline is silent").
    pub fn defaults(name: &str, folder: Option<PathBuf>, document: &Document) -> ExportSettings {
        let settings = document.settings();
        let silent = document
            .sequence(document.root())
            .is_none_or(|seq| !super::preflight::has_audible_clips(seq));
        ExportSettings {
            file: FileTarget {
                name: name.to_string(),
                folder,
            },
            video: VideoSettings::default(),
            audio: AudioExportSettings {
                include: !silent,
                kbps: DEFAULT_AAC_KBPS,
                sample_rate: default_sample_rate(settings),
            },
        }
    }

    /// The output frame size for `canvas` (EX-024).
    pub fn output_dimensions(&self, canvas: (u32, u32)) -> (u32, u32) {
        output_dimensions(canvas, self.video.size)
    }

    /// The output frame rate for a project at `project` (EX-026).
    pub fn output_rate(&self, project: FrameRate) -> FrameRate {
        self.video.frame_rate.resolve(project)
    }

    /// The target video bitrate in kbps for a project with `settings`
    /// (EX-027).
    pub fn video_kbps(&self, settings: &ProjectSettings) -> u32 {
        let (w, h) = self.output_dimensions((settings.width, settings.height));
        tier_kbps(
            self.video.quality,
            w,
            h,
            self.output_rate(settings.frame_rate),
            self.video.codec,
        )
    }

    /// The audio bitrate in kbps; 0 when audio is off.
    pub fn audio_kbps(&self) -> u32 {
        if self.audio.include {
            self.audio.kbps
        } else {
            0
        }
    }
}

/// The project's sample rate if an export may use it, else 48 kHz (EX-031).
fn default_sample_rate(settings: &ProjectSettings) -> SampleRate {
    if AUDIO_SAMPLE_RATES.contains(&settings.sample_rate.hz()) {
        settings.sample_rate
    } else {
        // 48 kHz divides the tick rate (it is a valid `SampleRate`); the
        // fallback keeps the project rate if that ever changed.
        SampleRate::new(DEFAULT_SAMPLE_RATE).unwrap_or(settings.sample_rate)
    }
}

/// The output frame size for a `canvas` of width × height and the size
/// choice `size` (EX-024). Both sides are even, as 4:2:0 chroma needs, and at
/// least 2.
///
/// - `Canvas`: each side rounded to the nearest even number.
/// - `Preset(P)`: the canvas fitted to a short side of `P` px with its
///   aspect kept: landscape `round_even(P × W / H) × P`, portrait
///   `P × round_even(P × H / W)`, square `P × P`.
///
/// Exact halves round up (1081 → 1082). The result may exceed
/// [`MAX_OUTPUT_SIDE`] for extreme canvases; preflight refuses that
/// (EX-036) rather than distorting the frame.
pub fn output_dimensions(canvas: (u32, u32), size: OutputSize) -> (u32, u32) {
    let (w, h) = (u64::from(canvas.0.max(1)), u64::from(canvas.1.max(1)));
    match size {
        OutputSize::Canvas => (round_even(w, 1), round_even(h, 1)),
        OutputSize::Preset(side) => {
            let p = u64::from(side.px());
            if w > h {
                (round_even(p * w, h), side.px())
            } else if h > w {
                (side.px(), round_even(p * h, w))
            } else {
                (side.px(), side.px())
            }
        }
    }
}

/// `num / den` rounded to the nearest even integer, exact halves up, at
/// least 2. `den` is non-zero.
fn round_even(num: u64, den: u64) -> u32 {
    // Nearest integer of num / (2·den), halves up, then doubled.
    let halves = (num + den) / (2 * den);
    u32::try_from(halves.saturating_mul(2))
        .unwrap_or(u32::MAX - 1)
        .max(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_fit_the_canvas_with_even_sides() {
        let p1080 = OutputSize::Preset(ShortSide::P1080);
        // Landscape, portrait, square.
        assert_eq!(output_dimensions((1920, 1080), p1080), (1920, 1080));
        assert_eq!(output_dimensions((1080, 1920), p1080), (1080, 1920));
        assert_eq!(output_dimensions((1000, 1000), p1080), (1080, 1080));
        assert_eq!(
            output_dimensions((1920, 1080), OutputSize::Preset(ShortSide::P720)),
            (1280, 720)
        );
        assert_eq!(
            output_dimensions((1920, 1080), OutputSize::Preset(ShortSide::P480)),
            (854, 480)
        );
        assert_eq!(
            output_dimensions((1080, 1920), OutputSize::Preset(ShortSide::P2160)),
            (2160, 3840)
        );
        // 4:5 portrait at 720: 900 exactly.
        assert_eq!(
            output_dimensions((1080, 1350), OutputSize::Preset(ShortSide::P720)),
            (720, 900)
        );
        // An odd custom canvas: 1081×1921 at 1080 gives 1080 × 1919.2 → 1920.
        assert_eq!(output_dimensions((1081, 1921), p1080), (1080, 1920));
        // 21:9-ish: 2560×1080 at 480 → 1137.8 → 1138.
        assert_eq!(
            output_dimensions((2560, 1080), OutputSize::Preset(ShortSide::P480)),
            (1138, 480)
        );
    }

    #[test]
    fn the_canvas_entry_rounds_each_side_to_even() {
        assert_eq!(
            output_dimensions((1080, 1920), OutputSize::Canvas),
            (1080, 1920)
        );
        assert_eq!(
            output_dimensions((1081, 1921), OutputSize::Canvas),
            (1082, 1922)
        );
        assert_eq!(output_dimensions((1, 1), OutputSize::Canvas), (2, 2));
        assert_eq!(output_dimensions((0, 3), OutputSize::Canvas), (2, 4));
        // Every result is even, whatever the canvas.
        for w in 1..60 {
            for h in 1..60 {
                for size in [
                    OutputSize::Canvas,
                    OutputSize::Preset(ShortSide::P480),
                    OutputSize::Preset(ShortSide::P1440),
                ] {
                    let (ow, oh) = output_dimensions((w, h), size);
                    assert!(ow.is_multiple_of(2) && oh.is_multiple_of(2) && ow >= 2 && oh >= 2);
                }
            }
        }
    }

    #[test]
    fn extreme_canvases_never_overflow() {
        let tall = output_dimensions((1, u32::MAX), OutputSize::Preset(ShortSide::P2160));
        assert_eq!(tall.0, 2160);
        assert!(tall.1.is_multiple_of(2) && tall.1 > MAX_OUTPUT_SIDE);
        let big = output_dimensions((u32::MAX, u32::MAX), OutputSize::Canvas);
        assert!(big.0.is_multiple_of(2) && big.1.is_multiple_of(2));
    }

    #[test]
    fn short_sides_and_rates() {
        for side in ShortSide::ALL {
            assert_eq!(ShortSide::from_px(side.px()), Some(side));
        }
        assert_eq!(ShortSide::from_px(1000), None);
        let p30 = FrameRate::new(30, 1).unwrap();
        let p120 = FrameRate::new(120, 1).unwrap();
        let p15 = FrameRate::new(15, 1).unwrap();
        assert_eq!(FrameRateChoice::Project.resolve(p120), p120);
        assert_eq!(FrameRateChoice::Fixed(p30).resolve(p120), p30);
        assert!(FrameRateChoice::Fixed(p30).is_offered(p120));
        assert!(
            FrameRateChoice::Fixed(p120).is_offered(p120),
            "project rate"
        );
        assert!(!FrameRateChoice::Fixed(p15).is_offered(p120));
        for (num, den) in FRAME_RATE_CHOICES {
            assert!(FrameRate::new(num, den).is_ok(), "{num}/{den} on the grid");
        }
        assert!(compatible(VideoCodec::H264, Container::Mp4));
        assert_eq!(Container::Mp4.extension(), "mp4");
    }
}
