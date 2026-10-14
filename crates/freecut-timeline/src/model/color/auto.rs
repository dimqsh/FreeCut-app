//! Auto color (CL-019) and color match (CL-020): results of an analysis
//! of the clip's own footage, stored with the stack. Plain fixed-point
//! values; the statistics that produce them live in
//! `freecut-render::color::stats` and the app runs the analysis.
//!
//! - [`AutoCorrection`] holds slider *offsets*, not slider values: the
//!   renderer sees `clamp(manual + offset × strength / 100)` for each
//!   slider ([`super::ColorStack::effective_basic`]), so the user's own
//!   sliders stay theirs and switching Auto off leaves them as they were.
//! - [`MatchTransform`] is a per-channel gain and offset in Rec.709 luma
//!   and colour differences of code values (`Y'`, `Cb`, `Cr`; K5's measured
//!   replacement for CL-020's Oklab), ×10⁴, applied at stage 4 and blended
//!   by its Intensity.

use crate::ids::{ClipId, MediaId};
use crate::time::Tick;

use super::ColorParamId;

/// The slider offsets of an auto correction (CL-019), each −100..=+100 in
/// the units of its slider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AutoOffsets {
    pub exposure: i16,
    pub contrast: i16,
    pub whites: i16,
    pub blacks: i16,
    pub temperature: i16,
    pub tint: i16,
    pub vibrance: i16,
    pub saturation: i16,
}

impl AutoOffsets {
    /// The sliders an auto correction moves, in the order the Auto section
    /// lists them (Light, then Color).
    pub const PARAMS: [ColorParamId; 8] = [
        ColorParamId::Exposure,
        ColorParamId::Contrast,
        ColorParamId::Whites,
        ColorParamId::Blacks,
        ColorParamId::Temperature,
        ColorParamId::Tint,
        ColorParamId::Vibrance,
        ColorParamId::Saturation,
    ];

    /// The offset of slider `id`; 0 for sliders Auto doesn't move.
    pub const fn get(&self, id: ColorParamId) -> i16 {
        match id {
            ColorParamId::Exposure => self.exposure,
            ColorParamId::Contrast => self.contrast,
            ColorParamId::Whites => self.whites,
            ColorParamId::Blacks => self.blacks,
            ColorParamId::Temperature => self.temperature,
            ColorParamId::Tint => self.tint,
            ColorParamId::Vibrance => self.vibrance,
            ColorParamId::Saturation => self.saturation,
            _ => 0,
        }
    }

    /// The offsets with slider `id`'s set to `v` (clamped to ±100); other
    /// sliders are left alone.
    #[must_use]
    pub fn with(mut self, id: ColorParamId, v: i16) -> AutoOffsets {
        let v = v.clamp(-100, 100);
        match id {
            ColorParamId::Exposure => self.exposure = v,
            ColorParamId::Contrast => self.contrast = v,
            ColorParamId::Whites => self.whites = v,
            ColorParamId::Blacks => self.blacks = v,
            ColorParamId::Temperature => self.temperature = v,
            ColorParamId::Tint => self.tint = v,
            ColorParamId::Vibrance => self.vibrance = v,
            ColorParamId::Saturation => self.saturation = v,
            _ => {}
        }
        self
    }

    /// Every offset limited to ±100.
    #[must_use]
    pub fn clamped(self) -> AutoOffsets {
        AutoOffsets::PARAMS
            .into_iter()
            .fold(self, |o, id| o.with(id, o.get(id)))
    }

    /// Whether every offset is 0 (the correction changes nothing).
    pub fn is_zero(&self) -> bool {
        AutoOffsets::PARAMS.iter().all(|id| self.get(*id) == 0)
    }

    /// The largest offset by size: what CL-019's "Only clips that need it"
    /// compares with its threshold.
    pub fn largest(&self) -> i16 {
        AutoOffsets::PARAMS
            .iter()
            .map(|id| self.get(*id).saturating_abs())
            .max()
            .unwrap_or(0)
    }
}

/// `offset × strength / 100`, rounded half away from zero: what an offset
/// adds to its slider at `strength` percent.
pub const fn scaled_offset(offset: i16, strength: u8) -> i16 {
    let s = if strength > 100 { 100 } else { strength } as i32;
    let p = offset as i32 * s;
    // |q| ≤ |offset| because s ≤ 100, so the cast back to i16 is exact.
    let q = if p >= 0 {
        (p + 50) / 100
    } else {
        (p - 50) / 100
    };
    q as i16
}

/// How Auto corrects (CL-019).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AutoStyle {
    /// Corrects only.
    #[default]
    Natural,
    /// Corrects, then adds Vibrance +20 and Saturation +5.
    Vivid,
}

impl AutoStyle {
    pub const ALL: [AutoStyle; 2] = [AutoStyle::Natural, AutoStyle::Vivid];

    /// The stable name in project files.
    pub const fn as_str(self) -> &'static str {
        match self {
            AutoStyle::Natural => "natural",
            AutoStyle::Vivid => "vivid",
        }
    }

    pub fn parse(s: &str) -> Option<AutoStyle> {
        AutoStyle::ALL.into_iter().find(|x| x.as_str() == s)
    }
}

/// What Auto analysed (CL-019).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AutoSource {
    /// Five frames evenly spaced across the used range.
    #[default]
    WholeClip,
    /// The frame at the playhead.
    Playhead,
}

impl AutoSource {
    pub const ALL: [AutoSource; 2] = [AutoSource::WholeClip, AutoSource::Playhead];

    /// The stable name in project files.
    pub const fn as_str(self) -> &'static str {
        match self {
            AutoSource::WholeClip => "clip",
            AutoSource::Playhead => "playhead",
        }
    }

    pub fn parse(s: &str) -> Option<AutoSource> {
        AutoSource::ALL.into_iter().find(|x| x.as_str() == s)
    }
}

/// A stored auto correction (CL-019): offsets under the user's sliders,
/// scaled by Strength (0–100 %, keyable as `ColorParamId::AutoStrength`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AutoCorrection {
    pub offsets: AutoOffsets,
    pub style: AutoStyle,
    pub source: AutoSource,
    pub strength: u8,
}

impl AutoCorrection {
    /// A correction at Strength 100.
    pub fn new(offsets: AutoOffsets, style: AutoStyle, source: AutoSource) -> AutoCorrection {
        AutoCorrection {
            offsets: offsets.clamped(),
            style,
            source,
            strength: 100,
        }
    }

    /// What it adds to slider `id` at its Strength.
    pub const fn added(&self, id: ColorParamId) -> i16 {
        scaled_offset(self.offsets.get(id), self.strength)
    }

    /// Whether it changes any slider (offsets not all 0, Strength above 0).
    pub fn changes(&self) -> bool {
        self.strength > 0 && !self.offsets.is_zero()
    }
}

/// The fixed-point scale of a match transform's gains and offsets.
pub const MATCH_ONE: i32 = 10_000;
/// A match gain's limits (CL-020: 0.5–2×), ×10⁴.
pub const MATCH_GAIN_MIN: i32 = 5_000;
pub const MATCH_GAIN_MAX: i32 = 20_000;
/// A match offset's limits, ×10⁴ (a channel moves at most ±1).
pub const MATCH_OFFSET_LIMIT: i32 = 10_000;

/// Where a match's reference frame came from (CL-020), so **Re-match** can
/// measure it again.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MatchRef {
    /// A frame of a clip on the timeline: the clip, its content time (TL's
    /// rational ticks; source time for media), and whether the reference
    /// was measured as graded (its own colour stack applied, the default)
    /// or as its ungraded source.
    Clip {
        clip: ClipId,
        at: Tick,
        graded: bool,
    },
    /// A frame of a library item (an image file, or a video at `at`).
    Media { media: MediaId, at: Tick },
}

/// A colour match (CL-020): out = gain · in + offset per channel of
/// Rec.709 `Y'CbCr` of code values, ×10⁴, blended toward the input by
/// Intensity (0–100 %, keyable as `ColorParamId::MatchIntensity`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MatchTransform {
    gain: [i32; 3],
    offset: [i32; 3],
    pub intensity: u8,
    pub reference: MatchRef,
}

impl MatchTransform {
    /// A transform at Intensity 100 with `gain` limited to 0.5–2× and
    /// `offset` to ±1 (×10⁴).
    pub fn new(gain: [i32; 3], offset: [i32; 3], reference: MatchRef) -> MatchTransform {
        MatchTransform {
            gain: gain.map(|g| g.clamp(MATCH_GAIN_MIN, MATCH_GAIN_MAX)),
            offset: offset.map(|o| o.clamp(-MATCH_OFFSET_LIMIT, MATCH_OFFSET_LIMIT)),
            intensity: 100,
            reference,
        }
    }

    /// The identity transform (a clip matched to itself).
    pub fn identity(reference: MatchRef) -> MatchTransform {
        MatchTransform::new([MATCH_ONE; 3], [0; 3], reference)
    }

    /// The gains of `Y'`, `Cb` and `Cr`, ×10⁴.
    pub const fn gain(&self) -> [i32; 3] {
        self.gain
    }

    /// The offsets of `Y'`, `Cb` and `Cr`, ×10⁴.
    pub const fn offset(&self) -> [i32; 3] {
        self.offset
    }

    /// Whether the transform leaves every colour where it is.
    pub fn is_identity(&self) -> bool {
        self.gain == [MATCH_ONE; 3] && self.offset == [0; 3]
    }

    /// Whether it changes the picture (not the identity, Intensity above
    /// 0).
    pub fn changes(&self) -> bool {
        self.intensity > 0 && !self.is_identity()
    }

    /// The transform with another Intensity (limited to 100).
    #[must_use]
    pub fn with_intensity(&self, intensity: u8) -> MatchTransform {
        MatchTransform {
            intensity: intensity.min(100),
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_scale_with_strength_and_round_half_away_from_zero() {
        assert_eq!(scaled_offset(12, 100), 12);
        assert_eq!(scaled_offset(12, 50), 6);
        assert_eq!(scaled_offset(-12, 50), -6);
        assert_eq!(scaled_offset(5, 50), 3, "2.5 rounds up");
        assert_eq!(scaled_offset(-5, 50), -3, "−2.5 rounds down");
        assert_eq!(scaled_offset(100, 0), 0);
        assert_eq!(scaled_offset(100, 250), 100, "Strength is limited to 100");
        let o = AutoOffsets::default()
            .with(ColorParamId::Exposure, 300)
            .with(ColorParamId::Hue, 40);
        assert_eq!(o.exposure, 100);
        assert_eq!(o.get(ColorParamId::Hue), 0, "Auto never moves Hue");
        assert_eq!(o.largest(), 100);
        assert!(AutoOffsets::default().is_zero());
    }

    #[test]
    fn match_transforms_are_limited_and_know_the_identity() {
        let r = MatchRef::Media {
            media: MediaId::from_raw(core::num::NonZeroU64::new(3).unwrap()),
            at: Tick::ZERO,
        };
        let m = MatchTransform::new([40_000, 1_000, 10_000], [0, 30_000, -2], r.clone());
        assert_eq!(m.gain(), [20_000, 5_000, 10_000]);
        assert_eq!(m.offset(), [0, 10_000, -2]);
        assert!(!m.is_identity() && m.changes());
        assert!(!m.with_intensity(0).changes());
        assert_eq!(m.with_intensity(200).intensity, 100);
        assert!(MatchTransform::identity(r).is_identity());
        assert_eq!(AutoStyle::parse("vivid"), Some(AutoStyle::Vivid));
        assert_eq!(AutoSource::parse("playhead"), Some(AutoSource::Playhead));
    }
}
