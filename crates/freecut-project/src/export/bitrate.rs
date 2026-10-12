//! Bitrates (EX-027) and the size estimate (EX-037).
//!
//! The bitrate formula is float math on a non-time quantity, which the spec
//! allows; its result is stored and passed on as integer kbps. It matches
//! `freecut_media::encode::{recommended_kbps, tier_kbps}` for H.264 (the
//! same table test), which the app can drop once it passes
//! `VideoQuality::TargetKbps` from here.

use freecut_timeline::model::ProjectSettings;
use freecut_timeline::time::{FrameRate, TICKS_PER_SECOND, Tick};

use super::settings::{ExportSettings, QualityTier, VideoCodec};

/// The bitrate floor of every tier, in kbps (EX-027: 0.5 Mbps).
pub const MIN_VIDEO_KBPS: u32 = 500;

/// The recommended video bitrate in kbps for a `width` × `height` output at
/// `rate` (EX-027): `10 Mbps × (W·H / 1920·1080)^0.85 × fps factor × codec
/// factor`, rounded to 0.1 Mbps, at least 0.5 Mbps. The fps factor is 1 up
/// to 30 fps, 1.5 up to 60 and 2.25 above (compared exactly, so 29.97 is
/// "up to 30").
pub fn recommended_kbps(width: u32, height: u32, rate: FrameRate, codec: VideoCodec) -> u32 {
    tier_kbps(QualityTier::Recommended, width, height, rate, codec)
}

/// The video bitrate of `tier` in kbps: the recommended rate × 0.5, 1 or 2,
/// then rounded and floored like [`recommended_kbps`].
pub fn tier_kbps(
    tier: QualityTier,
    width: u32,
    height: u32,
    rate: FrameRate,
    codec: VideoCodec,
) -> u32 {
    let (num, den) = (u64::from(rate.num()), u64::from(rate.den()));
    let fps_factor = if num <= 30 * den {
        1.0
    } else if num <= 60 * den {
        1.5
    } else {
        2.25
    };
    let per_mille = |p: u32| f64::from(p) / 1000.0;
    let pixels = f64::from(width) * f64::from(height);
    let mbps = 10.0
        * (pixels / 2_073_600.0).powf(0.85)
        * fps_factor
        * per_mille(codec.factor_per_mille())
        * per_mille(tier.factor_per_mille());
    let tenths = (mbps * 10.0).round().max(f64::from(MIN_VIDEO_KBPS / 100));
    // At most 8192² × 2.25 × 2 ≈ 5 Gbps for any real output, far inside u32
    // kbps; the float-to-int cast saturates for absurd sizes anyway.
    (tenths * 100.0) as u32
}

/// A file size estimate (EX-037). Always an estimate: with VBR tiers the
/// encoder may use less than the target, so it is an upper bound, shown as
/// "up to".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SizeEstimate {
    /// Bytes, including about 1 % container overhead.
    pub bytes: u64,
}

/// The size of an export of `duration` with `settings` for a project with
/// `project` settings (EX-037): `(video kbps + audio kbps) × seconds / 8 ×
/// 1.01`. Integer math on ticks; a negative duration counts as zero.
pub fn estimate_size(
    settings: &ExportSettings,
    project: &ProjectSettings,
    duration: Tick,
) -> SizeEstimate {
    let kbps = u128::from(settings.video_kbps(project)) + u128::from(settings.audio_kbps());
    let ticks = u128::try_from(duration.raw()).unwrap_or(0);
    // bits = kbps · 1000 · ticks / TPS; bytes = bits / 8 · 101 / 100.
    let tps = u128::from(TICKS_PER_SECOND.unsigned_abs());
    let bytes = kbps * 1000 * ticks * 101 / (tps * 8 * 100);
    SizeEstimate {
        bytes: u64::try_from(bytes).unwrap_or(u64::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::settings::{
        AudioExportSettings, FileTarget, FrameRateChoice, OutputSize, ShortSide, VideoSettings,
    };
    use freecut_timeline::time::SampleRate;

    fn rate(num: u32, den: u32) -> FrameRate {
        FrameRate::new(num, den).unwrap()
    }

    /// EX-027's table (H.264, ≤ 30 fps), the same values as freecut-media's.
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
            assert_eq!(
                recommended_kbps(w, h, r30, VideoCodec::H264),
                kbps,
                "{w}x{h}"
            );
        }
        // Portrait has the same pixel count, so the same bitrate.
        assert_eq!(recommended_kbps(1080, 1920, r30, VideoCodec::H264), 10_000);
    }

    #[test]
    fn tiers_and_frame_rate_factors() {
        let h264 = VideoCodec::H264;
        let (w, h) = (1920, 1080);
        assert_eq!(
            tier_kbps(QualityTier::Smaller, w, h, rate(25, 1), h264),
            5_000
        );
        assert_eq!(
            tier_kbps(QualityTier::Higher, w, h, rate(25, 1), h264),
            20_000
        );
        assert_eq!(recommended_kbps(w, h, rate(30_000, 1001), h264), 10_000);
        assert_eq!(recommended_kbps(w, h, rate(50, 1), h264), 15_000);
        assert_eq!(recommended_kbps(w, h, rate(60, 1), h264), 15_000);
        assert_eq!(recommended_kbps(w, h, rate(60_000, 1001), h264), 15_000);
        assert_eq!(recommended_kbps(w, h, rate(120, 1), h264), 22_500);
        // Tiny outputs get the 0.5 Mbps floor, on every tier.
        assert_eq!(recommended_kbps(64, 48, rate(25, 1), h264), 500);
        assert_eq!(
            tier_kbps(QualityTier::Smaller, 64, 48, rate(25, 1), h264),
            500
        );
    }

    fn settings(size: OutputSize, include_audio: bool) -> ExportSettings {
        ExportSettings {
            file: FileTarget::default(),
            video: VideoSettings {
                size,
                frame_rate: FrameRateChoice::Project,
                ..VideoSettings::default()
            },
            audio: AudioExportSettings {
                include: include_audio,
                kbps: 192,
                sample_rate: SampleRate::new(48_000).unwrap(),
            },
        }
    }

    #[test]
    fn the_estimate_is_bitrate_times_duration_plus_one_percent() {
        let project = ProjectSettings::new_default().unwrap();
        let one_minute = Tick::from_raw(60 * TICKS_PER_SECOND);
        let s = settings(OutputSize::Preset(ShortSide::P1080), true);
        let video = s.video_kbps(&project);
        let expected = u64::from(video + 192) * 1000 * 60 / 8 * 101 / 100;
        assert_eq!(estimate_size(&s, &project, one_minute).bytes, expected);
        // Without audio only the video counts.
        let silent = settings(OutputSize::Preset(ShortSide::P1080), false);
        assert_eq!(
            estimate_size(&silent, &project, one_minute).bytes,
            u64::from(video) * 1000 * 60 / 8 * 101 / 100
        );
        assert_eq!(estimate_size(&s, &project, Tick::ZERO).bytes, 0);
        assert_eq!(estimate_size(&s, &project, Tick::from_raw(-5)).bytes, 0);
        // A day at a high rate doesn't overflow.
        let day = Tick::from_raw(86_400 * TICKS_PER_SECOND);
        assert!(estimate_size(&s, &project, day).bytes > 100_000_000_000);
    }
}
