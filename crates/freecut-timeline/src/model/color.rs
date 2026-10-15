//! The colour stack of a visual item (CL-001, CL-007, CL-008, CL-013,
//! CL-014, CL-015, CL-023, CL-024, CL-035, CL-043): which sections it has,
//! their switches, the master bypass, the overall Amount, the Light and
//! Color sliders, HSL, Curves and Wheels.
//!
//! Everything is fixed-point (CL-001's model notes): sliders are `i16`
//! levels, Amount and Strength `u8` percentages, curve points `u16` in
//! `0..=10000` and wheel pucks `i16` on a disk of radius 1000.
//! [`color_param_spec`] is the single table of every slider's range and
//! default; the UI, the keyframe evaluator and the loader all read it. The
//! pixel math lives in `freecut-render`.
//!
//! *Build note (K6):* the stack holds Auto and Match (results of analysing
//! the footage, CL-019 and CL-020), Light, Color (with skin protection),
//! the LUT slot, HSL, Curves, Wheels and the colour masks (CL-031, built
//! on the shared [`super::mask`] core).
//! The other sections of CL-043 already have their [`Section`] bit, so a
//! switch and a reset can name them, and later slices add their values
//! through format migrations with identity defaults.

mod auto;
mod curve;
mod hsl;
mod lut;
mod masks;
mod wheels;

use std::sync::Arc;

pub use auto::{
    AutoCorrection, AutoOffsets, AutoSource, AutoStyle, MATCH_GAIN_MAX, MATCH_GAIN_MIN,
    MATCH_OFFSET_LIMIT, MATCH_ONE, MatchRef, MatchTransform, scaled_offset,
};
pub use curve::{Curve, CurveChannel, CurveError, CurvePoint, Curves};
pub use hsl::{Hsl, HslBand, HslChannel, HueBand};
pub use lut::{
    LutInterp, LutRef, LutSlot, SKIN_AMOUNT_DEFAULT, SkinProtect, SkinScope, hash_hex,
    parse_hash_hex,
};
pub use masks::{ColorMask, ColorMaskParam, EXPAND_LIMIT, MAX_COLOR_MASKS};
pub use wheels::{ColorWheels, PUCK_RADIUS, Wheel, WheelId, on_disk};

use super::keys::PropValue;
use super::mask::{Mask, MaskCombine};

/// One section of the Color tab (CL-043), in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    Auto,
    Match,
    Light,
    Color,
    Lut,
    Filter,
    Detail,
    Hsl,
    Curves,
    Wheels,
    Masks,
}

/// The pages of the Color tab (CL-043).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColorPage {
    Basic,
    Hsl,
    Curves,
    Wheels,
    Masks,
}

impl ColorPage {
    /// Every page, in strip order.
    pub const ALL: [ColorPage; 5] = [
        ColorPage::Basic,
        ColorPage::Hsl,
        ColorPage::Curves,
        ColorPage::Wheels,
        ColorPage::Masks,
    ];
}

impl Section {
    /// Every section, in display order (pages in strip order, sections top
    /// to bottom), so the UI and the reset and paste code share one list.
    pub const ALL: [Section; 11] = [
        Section::Auto,
        Section::Match,
        Section::Light,
        Section::Color,
        Section::Lut,
        Section::Filter,
        Section::Detail,
        Section::Hsl,
        Section::Curves,
        Section::Wheels,
        Section::Masks,
    ];

    /// The page the section sits on.
    pub const fn page(self) -> ColorPage {
        match self {
            Section::Auto
            | Section::Match
            | Section::Light
            | Section::Color
            | Section::Lut
            | Section::Filter
            | Section::Detail => ColorPage::Basic,
            Section::Hsl => ColorPage::Hsl,
            Section::Curves => ColorPage::Curves,
            Section::Wheels => ColorPage::Wheels,
            Section::Masks => ColorPage::Masks,
        }
    }

    /// The section's bit in a [`SectionMask`].
    pub const fn bit(self) -> u16 {
        1 << (self as u16)
    }

    /// The stable name used in project files.
    pub const fn as_str(self) -> &'static str {
        match self {
            Section::Auto => "auto",
            Section::Match => "match",
            Section::Light => "light",
            Section::Color => "color",
            Section::Lut => "lut",
            Section::Filter => "filter",
            Section::Detail => "detail",
            Section::Hsl => "hsl",
            Section::Curves => "curves",
            Section::Wheels => "wheels",
            Section::Masks => "masks",
        }
    }

    pub fn parse(s: &str) -> Option<Section> {
        Section::ALL.into_iter().find(|x| x.as_str() == s)
    }

    /// The scalar sliders of the section (CL-007, CL-008, CL-013,
    /// CL-015); empty for Curves, whose points are no sliders, and for
    /// sections this build has no values for yet.
    pub fn params(self) -> &'static [ColorParamId] {
        match self {
            Section::Light => &ColorParamId::LIGHT,
            Section::Color => &ColorParamId::COLOR_SECTION,
            Section::Lut => &ColorParamId::LUT,
            Section::Auto => &ColorParamId::AUTO,
            Section::Match => &ColorParamId::MATCH,
            Section::Hsl => &ColorParamId::HSL,
            Section::Wheels => &ColorParamId::WHEELS,
            _ => &[],
        }
    }
}

/// Which sections are switched on (CL-023): one bit per [`Section`], all on
/// by default. Switching a section off keeps its values and keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SectionMask(u16);

impl SectionMask {
    /// Every section on.
    pub const ALL_ON: SectionMask = SectionMask((1 << Section::ALL.len()) - 1);

    /// No section.
    pub const NONE: SectionMask = SectionMask(0);

    /// Exactly `sections`.
    pub fn of(sections: &[Section]) -> SectionMask {
        sections
            .iter()
            .fold(SectionMask::NONE, |m, s| m.with(*s, true))
    }

    /// The sections that are on, in display order.
    pub fn sections(self) -> impl Iterator<Item = Section> {
        Section::ALL.into_iter().filter(move |s| self.is_on(*s))
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    /// From stored bits; bits of sections this build doesn't know are
    /// dropped.
    pub const fn from_bits(bits: u16) -> SectionMask {
        SectionMask(bits & Self::ALL_ON.0)
    }

    pub const fn is_on(self, section: Section) -> bool {
        self.0 & section.bit() != 0
    }

    #[must_use]
    pub const fn with(self, section: Section, on: bool) -> SectionMask {
        if on {
            SectionMask(self.0 | section.bit())
        } else {
            SectionMask(self.0 & !section.bit())
        }
    }
}

impl Default for SectionMask {
    fn default() -> Self {
        SectionMask::ALL_ON
    }
}

/// A scalar colour control that `EditColor` and keyframes address
/// (CL-028). Every one is keyable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColorParamId {
    // Light (CL-007).
    Exposure,
    Brightness,
    Contrast,
    ContrastPivot,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    ToneLift,
    // Color (CL-008).
    Temperature,
    Tint,
    Hue,
    Saturation,
    Vibrance,
    /// The overall Amount (CL-035), Mix on adjustment clips.
    Amount,
    /// One slider of one HSL band (CL-013).
    Hsl {
        band: HueBand,
        channel: HslChannel,
    },
    /// A wheel's luminance (CL-015).
    WheelLuma(WheelId),
    /// The Wheels section's Strength (CL-015), 0–100 %.
    WheelStrength,
    /// The LUT's Intensity (CL-017), 0–100 %; nothing while the stack has
    /// no LUT.
    LutIntensity,
    /// Skin protection's Amount (CL-030) in one scope, 0–100 %.
    SkinAmount(SkinScope),
    /// Auto's Strength (CL-019), 0–100 %; nothing while the stack has no
    /// auto correction.
    AutoStrength,
    /// Match's Intensity (CL-020), 0–100 %; nothing while the stack has no
    /// match.
    MatchIntensity,
}

/// The range and default of a colour control, in its stored units
/// (CL-001's `ParamSpec` table; named apart from the effect parameters'
/// table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorParamSpec {
    pub min: i16,
    pub max: i16,
    pub default: i16,
}

impl ColorParamSpec {
    /// `v` limited to the range.
    pub fn clamp(self, v: i16) -> i16 {
        v.clamp(self.min, self.max)
    }

    pub fn contains(self, v: i16) -> bool {
        (self.min..=self.max).contains(&v)
    }
}

/// The range and default of `id` (CL-001's conventions): bipolar sliders
/// −100..=+100 around 0, Amount and Strength 0..=100 % with 100 by
/// default.
pub const fn color_param_spec(id: ColorParamId) -> ColorParamSpec {
    match id {
        ColorParamId::Amount
        | ColorParamId::WheelStrength
        | ColorParamId::LutIntensity
        | ColorParamId::AutoStrength
        | ColorParamId::MatchIntensity => ColorParamSpec {
            min: 0,
            max: 100,
            default: 100,
        },
        ColorParamId::SkinAmount(_) => ColorParamSpec {
            min: 0,
            max: 100,
            default: SKIN_AMOUNT_DEFAULT as i16,
        },
        _ => ColorParamSpec {
            min: -100,
            max: 100,
            default: 0,
        },
    }
}

/// Project-file names of the HSL sliders, by band and channel.
const HSL_NAMES: [[&str; 3]; 8] = [
    ["hsl.red.hue", "hsl.red.saturation", "hsl.red.lightness"],
    [
        "hsl.orange.hue",
        "hsl.orange.saturation",
        "hsl.orange.lightness",
    ],
    [
        "hsl.yellow.hue",
        "hsl.yellow.saturation",
        "hsl.yellow.lightness",
    ],
    [
        "hsl.green.hue",
        "hsl.green.saturation",
        "hsl.green.lightness",
    ],
    ["hsl.cyan.hue", "hsl.cyan.saturation", "hsl.cyan.lightness"],
    ["hsl.blue.hue", "hsl.blue.saturation", "hsl.blue.lightness"],
    [
        "hsl.purple.hue",
        "hsl.purple.saturation",
        "hsl.purple.lightness",
    ],
    [
        "hsl.magenta.hue",
        "hsl.magenta.saturation",
        "hsl.magenta.lightness",
    ],
];

/// Project-file names of the wheels' luminance sliders.
const WHEEL_LUMA_NAMES: [&str; 4] = [
    "wheels.shadows.luma",
    "wheels.midtones.luma",
    "wheels.highlights.luma",
    "wheels.offset.luma",
];

impl ColorParamId {
    /// The Light section's sliders, top to bottom (Pivot sits under More).
    pub const LIGHT: [ColorParamId; 9] = [
        ColorParamId::Exposure,
        ColorParamId::Brightness,
        ColorParamId::Contrast,
        ColorParamId::ContrastPivot,
        ColorParamId::Highlights,
        ColorParamId::Shadows,
        ColorParamId::Whites,
        ColorParamId::Blacks,
        ColorParamId::ToneLift,
    ];

    /// The Color section's sliders, top to bottom.
    pub const COLOR: [ColorParamId; 5] = [
        ColorParamId::Temperature,
        ColorParamId::Tint,
        ColorParamId::Hue,
        ColorParamId::Saturation,
        ColorParamId::Vibrance,
    ];

    /// Every slider of the Color section: [`ColorParamId::COLOR`] and skin
    /// protection's Amount (CL-030).
    pub const COLOR_SECTION: [ColorParamId; 6] = [
        ColorParamId::Temperature,
        ColorParamId::Tint,
        ColorParamId::Hue,
        ColorParamId::Saturation,
        ColorParamId::Vibrance,
        ColorParamId::SkinAmount(SkinScope::Color),
    ];

    /// The LUT section's sliders: Intensity and skin protection's Amount.
    pub const LUT: [ColorParamId; 2] = [
        ColorParamId::LutIntensity,
        ColorParamId::SkinAmount(SkinScope::Lut),
    ];

    /// The Auto section's slider: Strength.
    pub const AUTO: [ColorParamId; 1] = [ColorParamId::AutoStrength];

    /// The Match section's slider: Intensity.
    pub const MATCH: [ColorParamId; 1] = [ColorParamId::MatchIntensity];

    /// The HSL section's sliders, band by band (hue, saturation,
    /// lightness).
    pub const HSL: [ColorParamId; 24] = {
        let mut out = [ColorParamId::Amount; 24];
        let mut b = 0;
        while b < 8 {
            let mut c = 0;
            while c < 3 {
                out[b * 3 + c] = ColorParamId::Hsl {
                    band: HueBand::ALL[b],
                    channel: HslChannel::ALL[c],
                };
                c += 1;
            }
            b += 1;
        }
        out
    };

    /// The Wheels section's sliders: Strength, then each wheel's
    /// luminance.
    pub const WHEELS: [ColorParamId; 5] = [
        ColorParamId::WheelStrength,
        ColorParamId::WheelLuma(WheelId::Shadows),
        ColorParamId::WheelLuma(WheelId::Midtones),
        ColorParamId::WheelLuma(WheelId::Highlights),
        ColorParamId::WheelLuma(WheelId::Offset),
    ];

    /// Every control: Light, Color, Amount, HSL, Wheels, the LUT's
    /// Intensity, the two skin Amounts, Auto's Strength and Match's
    /// Intensity.
    pub const ALL: [ColorParamId; 49] = {
        let mut out = [ColorParamId::Amount; 49];
        let mut i = 0;
        while i < 9 {
            out[i] = ColorParamId::LIGHT[i];
            i += 1;
        }
        let mut j = 0;
        while j < 5 {
            out[9 + j] = ColorParamId::COLOR[j];
            j += 1;
        }
        // out[14] stays Amount.
        let mut k = 0;
        while k < 24 {
            out[15 + k] = ColorParamId::HSL[k];
            k += 1;
        }
        let mut w = 0;
        while w < 5 {
            out[39 + w] = ColorParamId::WHEELS[w];
            w += 1;
        }
        out[44] = ColorParamId::LutIntensity;
        out[45] = ColorParamId::SkinAmount(SkinScope::Color);
        out[46] = ColorParamId::SkinAmount(SkinScope::Lut);
        out[47] = ColorParamId::AutoStrength;
        out[48] = ColorParamId::MatchIntensity;
        out
    };

    /// The section holding the control; `None` for Amount, which belongs
    /// to no section (CL-035).
    pub const fn section(self) -> Option<Section> {
        match self {
            ColorParamId::Exposure
            | ColorParamId::Brightness
            | ColorParamId::Contrast
            | ColorParamId::ContrastPivot
            | ColorParamId::Highlights
            | ColorParamId::Shadows
            | ColorParamId::Whites
            | ColorParamId::Blacks
            | ColorParamId::ToneLift => Some(Section::Light),
            ColorParamId::Temperature
            | ColorParamId::Tint
            | ColorParamId::Hue
            | ColorParamId::Saturation
            | ColorParamId::Vibrance
            | ColorParamId::SkinAmount(SkinScope::Color) => Some(Section::Color),
            ColorParamId::LutIntensity | ColorParamId::SkinAmount(SkinScope::Lut) => {
                Some(Section::Lut)
            }
            ColorParamId::Amount => None,
            ColorParamId::AutoStrength => Some(Section::Auto),
            ColorParamId::MatchIntensity => Some(Section::Match),
            ColorParamId::Hsl { .. } => Some(Section::Hsl),
            ColorParamId::WheelLuma(_) | ColorParamId::WheelStrength => Some(Section::Wheels),
        }
    }

    pub const fn spec(self) -> ColorParamSpec {
        color_param_spec(self)
    }

    /// The stable name in project files (the property path is
    /// `color.<name>`).
    pub const fn as_str(self) -> &'static str {
        match self {
            ColorParamId::Exposure => "exposure",
            ColorParamId::Brightness => "brightness",
            ColorParamId::Contrast => "contrast",
            ColorParamId::ContrastPivot => "contrast_pivot",
            ColorParamId::Highlights => "highlights",
            ColorParamId::Shadows => "shadows",
            ColorParamId::Whites => "whites",
            ColorParamId::Blacks => "blacks",
            ColorParamId::ToneLift => "tone_lift",
            ColorParamId::Temperature => "temperature",
            ColorParamId::Tint => "tint",
            ColorParamId::Hue => "hue",
            ColorParamId::Saturation => "saturation",
            ColorParamId::Vibrance => "vibrance",
            ColorParamId::Amount => "amount",
            ColorParamId::Hsl { band, channel } => HSL_NAMES[band.index()][channel as usize],
            ColorParamId::WheelLuma(w) => WHEEL_LUMA_NAMES[w as usize],
            ColorParamId::WheelStrength => "wheels.strength",
            ColorParamId::LutIntensity => "lut.intensity",
            ColorParamId::SkinAmount(SkinScope::Color) => "skin_amount",
            ColorParamId::SkinAmount(SkinScope::Lut) => "lut.skin_amount",
            ColorParamId::AutoStrength => "auto.strength",
            ColorParamId::MatchIntensity => "match.intensity",
        }
    }

    pub fn parse(s: &str) -> Option<ColorParamId> {
        ColorParamId::ALL.into_iter().find(|p| p.as_str() == s)
    }
}

/// The Light and Color sliders (CL-007, CL-008), each a level in its
/// [`color_param_spec`] range.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct BasicAdjust {
    pub exposure: i16,
    pub brightness: i16,
    pub contrast: i16,
    pub contrast_pivot: i16,
    pub highlights: i16,
    pub shadows: i16,
    pub whites: i16,
    pub blacks: i16,
    pub tone_lift: i16,
    pub temperature: i16,
    pub tint: i16,
    pub hue: i16,
    pub saturation: i16,
    pub vibrance: i16,
    /// Skin protection of the Color scope (CL-030).
    pub skin: SkinProtect,
}

impl BasicAdjust {
    fn slot(&mut self, id: ColorParamId) -> Option<&mut i16> {
        Some(match id {
            ColorParamId::Exposure => &mut self.exposure,
            ColorParamId::Brightness => &mut self.brightness,
            ColorParamId::Contrast => &mut self.contrast,
            ColorParamId::ContrastPivot => &mut self.contrast_pivot,
            ColorParamId::Highlights => &mut self.highlights,
            ColorParamId::Shadows => &mut self.shadows,
            ColorParamId::Whites => &mut self.whites,
            ColorParamId::Blacks => &mut self.blacks,
            ColorParamId::ToneLift => &mut self.tone_lift,
            ColorParamId::Temperature => &mut self.temperature,
            ColorParamId::Tint => &mut self.tint,
            ColorParamId::Hue => &mut self.hue,
            ColorParamId::Saturation => &mut self.saturation,
            ColorParamId::Vibrance => &mut self.vibrance,
            ColorParamId::Amount
            | ColorParamId::Hsl { .. }
            | ColorParamId::WheelLuma(_)
            | ColorParamId::WheelStrength
            | ColorParamId::LutIntensity
            | ColorParamId::SkinAmount(_)
            | ColorParamId::AutoStrength
            | ColorParamId::MatchIntensity => return None,
        })
    }

    /// The slider `id`; `None` for the controls that aren't Light or
    /// Color sliders.
    pub fn get(&self, id: ColorParamId) -> Option<i16> {
        let mut copy = *self;
        copy.slot(id).map(|v| *v)
    }
}

/// The colour stack of a video, image or adjustment clip (CL-001). A clip
/// without one (`Clip::color` is `None`) shows its footage as it is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColorStack {
    /// The auto correction (CL-019): offsets under the Light and Color
    /// sliders.
    pub auto: Option<AutoCorrection>,
    /// The colour match (CL-020), stage 4.
    pub match_xf: Option<MatchTransform>,
    /// The per-section switches (CL-023).
    pub sections_enabled: SectionMask,
    /// The master bypass (CL-023): the whole stack is skipped.
    pub bypass: bool,
    /// The overall Amount (CL-035), 0–100 %; Mix on adjustment clips.
    pub amount: u8,
    /// Light and Color (CL-007, CL-008).
    pub basic: BasicAdjust,
    /// HSL per hue band (CL-013).
    pub hsl: Hsl,
    /// Curves (CL-014).
    pub curves: Curves,
    /// Color wheels (CL-015).
    pub wheels: ColorWheels,
    /// The user LUT (CL-017), with its skin protection (CL-030).
    pub lut: Option<LutSlot>,
    // K6
    /// Colour masks (CL-031), in combine order; at most
    /// [`MAX_COLOR_MASKS`]. Empty: the grade covers the whole item.
    pub masks: Vec<ColorMask>,
}

impl Default for ColorStack {
    fn default() -> Self {
        ColorStack {
            auto: None,
            match_xf: None,
            sections_enabled: SectionMask::ALL_ON,
            bypass: false,
            amount: 100,
            basic: BasicAdjust::default(),
            hsl: Hsl::default(),
            curves: Curves::default(),
            wheels: ColorWheels::default(),
            lut: None,
            masks: Vec::new(),
        }
    }
}

impl ColorStack {
    /// The value of `id`.
    pub fn get(&self, id: ColorParamId) -> i16 {
        match id {
            ColorParamId::Amount => i16::from(self.amount),
            ColorParamId::Hsl { band, channel } => self.hsl.band(band).get(channel),
            ColorParamId::WheelLuma(w) => self.wheels.wheel(w).luma(),
            ColorParamId::WheelStrength => i16::from(self.wheels.strength),
            ColorParamId::LutIntensity => self
                .lut
                .as_ref()
                .map_or(id.spec().default, |l| i16::from(l.intensity)),
            ColorParamId::SkinAmount(scope) => i16::from(self.skin(scope).amount),
            ColorParamId::AutoStrength => self
                .auto
                .as_ref()
                .map_or(id.spec().default, |a| i16::from(a.strength)),
            ColorParamId::MatchIntensity => self
                .match_xf
                .as_ref()
                .map_or(id.spec().default, |m| i16::from(m.intensity)),
            _ => self.basic.get(id).unwrap_or(0),
        }
    }

    /// Whether the stack has control `id` now: the LUT's sliders only with
    /// a LUT (CL-017), Strength only with an auto correction (CL-019),
    /// Intensity only with a match (CL-020); every other control always.
    pub fn has_param(&self, id: ColorParamId) -> bool {
        match id.section() {
            Some(Section::Lut) => self.lut.is_some(),
            Some(Section::Auto) => self.auto.is_some(),
            Some(Section::Match) => self.match_xf.is_some(),
            _ => true,
        }
    }

    /// The Light and Color sliders as the renderer sees them (CL-019): each
    /// slider's manual value while its section is on (its default while
    /// off), plus the auto correction's offset at its Strength while the
    /// Auto section is on, clamped into the slider's range. Contrast's
    /// Pivot and skin protection are the manual ones.
    pub fn effective_basic(&self) -> BasicAdjust {
        let on = |s: Section| self.sections_enabled.is_on(s);
        let auto = self.auto.filter(|_| on(Section::Auto));
        let mut out = self.basic;
        for id in ColorParamId::LIGHT.iter().chain(&ColorParamId::COLOR) {
            let manual = if id.section().is_some_and(on) {
                self.basic.get(*id).unwrap_or(0)
            } else {
                id.spec().default
            };
            let added = auto.map_or(0, |a| a.added(*id));
            if let Some(slot) = out.slot(*id) {
                *slot = id.spec().clamp(manual.saturating_add(added));
            }
        }
        out
    }

    /// The stack with `auto` as its auto correction (`None` removes it);
    /// offsets and Strength are limited to their ranges.
    #[must_use]
    pub fn with_auto(&self, auto: Option<AutoCorrection>) -> ColorStack {
        ColorStack {
            auto: auto.map(|a| AutoCorrection {
                offsets: a.offsets.clamped(),
                strength: a.strength.min(100),
                ..a
            }),
            ..self.clone()
        }
    }

    /// The stack with `xf` as its colour match (`None` removes it).
    #[must_use]
    pub fn with_match(&self, xf: Option<MatchTransform>) -> ColorStack {
        ColorStack {
            match_xf: xf.map(|m| {
                let intensity = m.intensity;
                m.with_intensity(intensity)
            }),
            ..self.clone()
        }
    }

    /// Why **Apply to sliders** (CL-019) would change the picture: the
    /// section whose switch is off, Auto itself or Light or Color while it
    /// would take a non-zero offset (an off section shows its defaults, so
    /// moving an offset into it would hide it). `None` when it can apply,
    /// or when there is nothing to apply.
    pub fn auto_apply_blocker(&self) -> Option<Section> {
        let auto = self.auto.filter(AutoCorrection::changes)?;
        let on = |s: Section| self.sections_enabled.is_on(s);
        if !on(Section::Auto) {
            return Some(Section::Auto);
        }
        AutoOffsets::PARAMS
            .into_iter()
            .filter(|id| auto.added(*id) != 0)
            .filter_map(ColorParamId::section)
            .find(|s| !on(*s))
    }

    /// The stack with the auto correction added into the sliders at its
    /// Strength, each clamped into its range, and the correction removed
    /// (CL-019's **Apply to sliders**). Unchanged without a correction.
    /// The picture stays the same unless [`ColorStack::auto_apply_blocker`]
    /// names a switched-off section.
    #[must_use]
    pub fn with_auto_applied(&self) -> ColorStack {
        let Some(auto) = self.auto else {
            return self.clone();
        };
        AutoOffsets::PARAMS
            .into_iter()
            .fold(self.with_auto(None), |s, id| {
                let v = s.get(id).saturating_add(auto.added(id));
                s.with(id, v)
            })
    }

    /// Skin protection of `scope` (CL-030); the default for the LUT scope
    /// while there is no LUT.
    pub fn skin(&self, scope: SkinScope) -> SkinProtect {
        match scope {
            SkinScope::Color => self.basic.skin,
            SkinScope::Lut => self
                .lut
                .as_ref()
                .map_or_else(SkinProtect::default, |l| l.skin),
        }
    }

    /// The stack with `scope`'s skin protection switched; the LUT scope
    /// changes nothing while there is no LUT.
    #[must_use]
    pub fn with_skin(&self, scope: SkinScope, on: bool) -> ColorStack {
        let mut out = self.clone();
        match scope {
            SkinScope::Color => out.basic.skin.on = on,
            SkinScope::Lut => {
                if let Some(l) = out.lut.as_mut() {
                    l.skin.on = on;
                }
            }
        }
        out
    }

    /// The stack with `lut` in its LUT slot (`None` clears it).
    #[must_use]
    pub fn with_lut(&self, lut: Option<LutSlot>) -> ColorStack {
        ColorStack {
            lut: lut.map(|l| LutSlot {
                intensity: l.intensity.min(100),
                skin: SkinProtect {
                    amount: l.skin.amount.min(100),
                    ..l.skin
                },
                ..l
            }),
            ..self.clone()
        }
    }

    /// The stack with `id` set to `value`, clamped into its range.
    #[must_use]
    pub fn with(&self, id: ColorParamId, value: i16) -> ColorStack {
        let v = id.spec().clamp(value);
        let mut out = self.clone();
        match id {
            // In range 0..=100 after the clamp.
            ColorParamId::Amount => out.amount = u8::try_from(v).unwrap_or(100),
            ColorParamId::WheelStrength => out.wheels.strength = u8::try_from(v).unwrap_or(100),
            // In range 0..=100 after the clamp.
            ColorParamId::AutoStrength => {
                if let Some(a) = out.auto.as_mut() {
                    a.strength = u8::try_from(v).unwrap_or(100);
                }
            }
            ColorParamId::MatchIntensity => {
                if let Some(m) = out.match_xf.as_mut() {
                    m.intensity = u8::try_from(v).unwrap_or(100);
                }
            }
            // In range 0..=100 after the clamp.
            ColorParamId::LutIntensity => {
                if let Some(l) = out.lut.as_mut() {
                    l.intensity = u8::try_from(v).unwrap_or(100);
                }
            }
            ColorParamId::SkinAmount(SkinScope::Color) => {
                out.basic.skin.amount = u8::try_from(v).unwrap_or(SKIN_AMOUNT_DEFAULT);
            }
            ColorParamId::SkinAmount(SkinScope::Lut) => {
                if let Some(l) = out.lut.as_mut() {
                    l.skin.amount = u8::try_from(v).unwrap_or(SKIN_AMOUNT_DEFAULT);
                }
            }
            ColorParamId::Hsl { band, channel } => *out.hsl.band_mut(band).get_mut(channel) = v,
            ColorParamId::WheelLuma(w) => {
                let wheel = out.wheels.wheel_mut(w);
                *wheel = wheel.with_luma(v);
            }
            _ => {
                if let Some(slot) = out.basic.slot(id) {
                    *slot = v;
                }
            }
        }
        out
    }

    /// The puck of wheel `w`, `(x, y)`.
    pub fn puck(&self, w: WheelId) -> (i16, i16) {
        let wheel = self.wheels.wheel(w);
        (wheel.x(), wheel.y())
    }

    /// The stack with wheel `w`'s puck at `(x, y)`, pulled onto the disk.
    #[must_use]
    pub fn with_puck(&self, w: WheelId, x: i16, y: i16) -> ColorStack {
        let mut out = self.clone();
        let wheel = out.wheels.wheel_mut(w);
        *wheel = wheel.with_puck(x, y);
        out
    }

    /// The stack with `channel`'s curve replaced.
    #[must_use]
    pub fn with_curve(&self, channel: CurveChannel, curve: Curve) -> ColorStack {
        let mut out = self.clone();
        *out.curves.get_mut(channel) = curve;
        out
    }

    /// Every value limited to its range (curves, pucks and match
    /// transforms are valid by construction).
    #[must_use]
    pub fn clamped(&self) -> ColorStack {
        let mut out = ColorParamId::ALL
            .into_iter()
            .fold(self.with_auto(self.auto), |s, id| s.with(id, s.get(id)));
        out.masks = out.masks.iter().map(ColorMask::clamped).collect();
        out
    }

    /// Whether every value is in its range.
    pub fn is_valid(&self) -> bool {
        ColorParamId::ALL
            .into_iter()
            .all(|id| id.spec().contains(self.get(id)))
            && self.auto.is_none_or(|a| a.offsets == a.offsets.clamped())
            && SectionMask::from_bits(self.sections_enabled.bits()) == self.sections_enabled
            && masks::masks_valid(&self.masks)
    }

    /// Whether masks limit the grade (CL-031): the Masks section is on and
    /// holds at least one mask. Otherwise the grade covers the whole item.
    pub fn masks_active(&self) -> bool {
        self.sections_enabled.is_on(Section::Masks) && !self.masks.is_empty()
    }

    /// The mask named `id`.
    pub fn mask(&self, id: u16) -> Option<&ColorMask> {
        self.masks.iter().find(|m| m.id == id)
    }

    /// The stack with `mask` added at the end of the list under the
    /// smallest free id, and that id; `None` when it already holds
    /// [`MAX_COLOR_MASKS`].
    pub fn with_mask_added(&self, mask: Mask) -> Option<(ColorStack, u16)> {
        if self.masks.len() >= MAX_COLOR_MASKS {
            return None;
        }
        let id = masks::free_id(&self.masks)?;
        let mut out = self.clone();
        out.masks.push(ColorMask::new(id, mask));
        Some((out, id))
    }

    /// The stack without mask `id` (unchanged when it has none).
    #[must_use]
    pub fn without_mask(&self, id: u16) -> ColorStack {
        let mut out = self.clone();
        out.masks.retain(|m| m.id != id);
        out
    }

    /// The stack with mask `id` moved to position `to` (clamped to the
    /// list); unchanged when it has no such mask.
    #[must_use]
    pub fn with_mask_moved(&self, id: u16, to: usize) -> ColorStack {
        let mut out = self.clone();
        if let Some(from) = out.masks.iter().position(|m| m.id == id) {
            let m = out.masks.remove(from);
            let to = to.min(out.masks.len());
            out.masks.insert(to, m);
        }
        out
    }

    /// The stack with the mask of `mask.id` replaced by `mask` (clamped);
    /// unchanged when it has no such mask.
    #[must_use]
    pub fn with_mask(&self, mask: ColorMask) -> ColorStack {
        let mut out = self.clone();
        if let Some(slot) = out.masks.iter_mut().find(|m| m.id == mask.id) {
            *slot = mask.clamped();
        }
        out
    }

    /// The stack with mask `id` combined by `mode`.
    #[must_use]
    pub fn with_mask_mode(&self, id: u16, mode: MaskCombine) -> ColorStack {
        match self.mask(id) {
            Some(m) => self.with_mask(ColorMask { mode, ..*m }),
            None => self.clone(),
        }
    }

    /// Mask `id`'s `param` as a keyframe holds it; `None` without the mask
    /// or the value.
    pub fn mask_param(&self, id: u16, param: ColorMaskParam) -> Option<PropValue> {
        self.mask(id)?.get(param)
    }

    /// The stack with mask `id`'s `param` at `value`, clamped; unchanged
    /// when the mask or the value is missing or the kind is wrong.
    #[must_use]
    pub fn with_mask_param(&self, id: u16, param: ColorMaskParam, value: PropValue) -> ColorStack {
        match self.mask(id).and_then(|m| m.with(param, value)) {
            Some(m) => self.with_mask(m),
            None => self.clone(),
        }
    }

    /// Whether the section's values are all at their defaults (its switch
    /// aside).
    pub fn section_is_default(&self, section: Section) -> bool {
        let extra = match section {
            Section::Curves => self.curves == Curves::default(),
            Section::Wheels => WheelId::ALL
                .iter()
                .all(|w| self.wheels.wheel(*w).puck_is_centred()),
            Section::Color => !self.basic.skin.on,
            Section::Lut => self.lut.is_none(),
            Section::Auto => self.auto.is_none(),
            Section::Match => self.match_xf.is_none(),
            Section::Masks => self.masks.is_empty(),
            _ => true,
        };
        extra
            && section
                .params()
                .iter()
                .all(|id| self.get(*id) == id.spec().default)
    }

    /// Whether the section's values change the picture, its switch aside:
    /// Pivot alone does nothing while Contrast is 0 (CL-007), curves on the
    /// diagonal do nothing, and nor do wheels at Strength 0.
    pub fn section_changes(&self, section: Section) -> bool {
        match section {
            Section::Curves => !self.curves.is_identity(),
            Section::Wheels => self.wheels.strength > 0 && self.wheels.any_set(),
            Section::Hsl => !self.hsl.is_default(),
            // Skin protection alone changes nothing (CL-030).
            Section::Color => ColorParamId::COLOR
                .iter()
                .any(|id| self.get(*id) != id.spec().default),
            Section::Lut => self.lut.as_ref().is_some_and(|l| l.intensity > 0),
            Section::Auto => self.auto.is_some_and(|a| a.changes()),
            Section::Match => self.match_xf.as_ref().is_some_and(MatchTransform::changes),
            // Masks only limit the other sections (CL-031).
            Section::Masks => false,
            _ => section.params().iter().any(|id| {
                let v = self.get(*id);
                v != id.spec().default
                    && !(*id == ColorParamId::ContrastPivot && self.basic.contrast == 0)
            }),
        }
    }

    /// Whether the section is on and changes the picture.
    pub fn section_is_active(&self, section: Section) -> bool {
        self.sections_enabled.is_on(section) && self.section_changes(section)
    }

    /// Whether the renderer can skip the stack (CL-001, CL-035): the
    /// master bypass, Amount 0 (no cleanup stages exist yet), or no
    /// section that is on and changes the picture.
    pub fn is_identity(&self) -> bool {
        self.bypass || self.amount == 0 || !Section::ALL.iter().any(|s| self.section_is_active(*s))
    }

    /// The stack with `section`'s values back at their defaults and its
    /// switch on (CL-024). Amount stays (CL-035).
    #[must_use]
    pub fn with_section_reset(&self, section: Section) -> ColorStack {
        let mut out = section
            .params()
            .iter()
            .fold(self.clone(), |s, id| s.with(*id, id.spec().default));
        match section {
            Section::Curves => out.curves = Curves::default(),
            Section::Wheels => out.wheels = ColorWheels::default(),
            Section::Color => out.basic.skin = SkinProtect::default(),
            // CL-017: the reset keeps the LUT choice.
            Section::Lut => out.lut = out.lut.as_ref().map(LutSlot::reset),
            // CL-019, CL-020: the reset badge removes the result.
            Section::Auto => out.auto = None,
            Section::Match => out.match_xf = None,
            Section::Masks => out.masks.clear(),
            _ => {}
        }
        out.sections_enabled = out.sections_enabled.with(section, true);
        out
    }

    /// The stack with one HSL band back at its defaults (CL-013's swatch
    /// reset).
    #[must_use]
    pub fn with_band_reset(&self, band: HueBand) -> ColorStack {
        let mut out = self.clone();
        *out.hsl.band_mut(band) = HslBand::default();
        out
    }

    /// The stack with one wheel's puck centred and its luminance at 0
    /// (CL-015's reset button).
    #[must_use]
    pub fn with_wheel_reset(&self, w: WheelId) -> ColorStack {
        let mut out = self.clone();
        *out.wheels.wheel_mut(w) = Wheel::default();
        out
    }

    /// How a clip stores this stack: `None` when it is the default stack
    /// (CL-001's CPU-path note: an edit that leaves the stack untouched
    /// stores nothing), so equal grades compare equal.
    pub fn stored(stack: ColorStack) -> Option<Arc<ColorStack>> {
        (stack != ColorStack::default()).then(|| Arc::new(stack))
    }

    /// The stack a clip's `color` field stands for.
    pub fn of(color: Option<&Arc<ColorStack>>) -> ColorStack {
        color.map_or_else(ColorStack::default, |s| ColorStack::clone(s))
    }

    /// The stack with the values and switches of `sections` taken from
    /// `from` (CL-025's replace, CL-026's apply); everything else, Amount
    /// and bypass included, stays.
    #[must_use]
    pub fn with_sections_from(&self, from: &ColorStack, sections: SectionMask) -> ColorStack {
        let mut out = self.clone();
        for section in Section::ALL {
            if !sections.is_on(section) {
                continue;
            }
            // The slot first: the LUT's sliders live in it.
            match section {
                Section::Color => out.basic.skin = from.basic.skin,
                Section::Lut => out.lut.clone_from(&from.lut),
                Section::Auto => out.auto = from.auto,
                Section::Match => out.match_xf.clone_from(&from.match_xf),
                Section::Hsl => out.hsl = from.hsl,
                Section::Curves => out.curves = from.curves.clone(),
                Section::Wheels => out.wheels = from.wheels,
                Section::Masks => out.masks.clone_from(&from.masks),
                _ => {}
            }
            for id in section.params() {
                out = out.with(*id, from.get(*id));
            }
            out.sections_enabled = out
                .sections_enabled
                .with(section, from.sections_enabled.is_on(section));
        }
        out
    }

    /// The sections that hold non-default values (CL-026's pre-ticked
    /// checklist), among `of`.
    pub fn changed_sections(&self, of: &[Section]) -> SectionMask {
        of.iter()
            .filter(|s| !self.section_is_default(**s))
            .fold(SectionMask::NONE, |m, s| m.with(*s, true))
    }
}

impl Section {
    /// The sections a saved look carries (CL-026): never Auto, Match,
    /// Detail's footage repair or Masks, which belong to one piece of
    /// footage or one shot's framing.
    pub const LOOK: [Section; 7] = [
        Section::Light,
        Section::Color,
        Section::Lut,
        Section::Filter,
        Section::Hsl,
        Section::Curves,
        Section::Wheels,
    ];

    /// Whether a saved look can carry the section.
    pub fn in_look(self) -> bool {
        Section::LOOK.contains(&self)
    }
}

/// The values of some sections of a stack, taken by value (CL-024,
/// CL-025, CL-026): what `ColorEdit::ApplySections` copies onto clips.
/// Keyframes are never part of it; a saved look holds the values at the
/// playhead.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SectionSnapshot {
    sections: SectionMask,
    values: ColorStack,
}

impl SectionSnapshot {
    /// The `sections` of `stack`; the other sections' values are left at
    /// their defaults, so equal snapshots compare equal. Amount and bypass
    /// are never part of a snapshot.
    pub fn new(stack: &ColorStack, sections: SectionMask) -> SectionSnapshot {
        SectionSnapshot {
            sections,
            values: ColorStack::default().with_sections_from(&stack.clamped(), sections),
        }
    }

    /// Which sections it holds.
    pub fn sections(&self) -> SectionMask {
        self.sections
    }

    /// The held sections' values (the others at their defaults, Amount 100).
    pub fn values(&self) -> &ColorStack {
        &self.values
    }

    /// Whether it holds no section.
    pub fn is_empty(&self) -> bool {
        self.sections == SectionMask::NONE
    }

    /// `stack` with the held sections replaced.
    pub fn applied_to(&self, stack: &ColorStack) -> ColorStack {
        stack.with_sections_from(&self.values, self.sections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_param_has_a_spec_with_its_default_inside() {
        for id in ColorParamId::ALL {
            let s = color_param_spec(id);
            assert!(s.min < s.max && s.contains(s.default), "{id:?}");
            assert_eq!(ColorStack::default().get(id), s.default, "{id:?}");
            assert_eq!(ColorParamId::parse(id.as_str()), Some(id));
        }
        assert_eq!(
            color_param_spec(ColorParamId::Exposure),
            ColorParamSpec {
                min: -100,
                max: 100,
                default: 0
            }
        );
        assert_eq!(color_param_spec(ColorParamId::Amount).default, 100);
        // Every slider belongs to the section that lists it.
        for section in Section::ALL {
            for id in section.params() {
                assert_eq!(id.section(), Some(section));
            }
            assert_eq!(Section::parse(section.as_str()), Some(section));
        }
        assert_eq!(ColorParamId::Amount.section(), None);
        assert_eq!(
            ColorParamId::LIGHT.len()
                + ColorParamId::COLOR_SECTION.len()
                + 1
                + ColorParamId::HSL.len()
                + ColorParamId::WHEELS.len()
                + ColorParamId::LUT.len()
                + ColorParamId::AUTO.len()
                + ColorParamId::MATCH.len(),
            ColorParamId::ALL.len()
        );
        // Every control is listed once.
        let unique: std::collections::BTreeSet<ColorParamId> =
            ColorParamId::ALL.into_iter().collect();
        assert_eq!(unique.len(), ColorParamId::ALL.len());
        assert_eq!(color_param_spec(ColorParamId::WheelStrength).default, 100);
        assert_eq!(
            ColorParamId::parse("hsl.purple.lightness"),
            Some(ColorParamId::Hsl {
                band: HueBand::Purple,
                channel: HslChannel::Lightness
            })
        );
    }

    #[test]
    fn values_are_clamped_into_range() {
        let s = ColorStack::default()
            .with(ColorParamId::Exposure, 500)
            .with(ColorParamId::Tint, i16::MIN)
            .with(ColorParamId::Amount, -3);
        assert_eq!(s.get(ColorParamId::Exposure), 100);
        assert_eq!(s.get(ColorParamId::Tint), -100);
        assert_eq!(s.amount, 0);
        let wild = ColorStack {
            amount: 250,
            basic: BasicAdjust {
                hue: 300,
                ..BasicAdjust::default()
            },
            ..ColorStack::default()
        };
        assert!(!wild.is_valid());
        let c = wild.clamped();
        assert!(c.is_valid());
        assert_eq!((c.amount, c.basic.hue), (100, 100));
    }

    #[test]
    fn identity_follows_bypass_amount_switches_and_values() {
        let plain = ColorStack::default();
        assert!(plain.is_identity());
        let warm = plain.with(ColorParamId::Temperature, 30);
        assert!(!warm.is_identity());
        assert!(
            ColorStack {
                bypass: true,
                ..warm.clone()
            }
            .is_identity()
        );
        assert!(warm.with(ColorParamId::Amount, 0).is_identity());
        let off = ColorStack {
            sections_enabled: warm.sections_enabled.with(Section::Color, false),
            ..warm.clone()
        };
        assert!(off.is_identity(), "a switched-off section changes nothing");
        // Pivot alone does nothing while Contrast is 0.
        let pivot = plain.with(ColorParamId::ContrastPivot, 40);
        assert!(pivot.is_identity());
        assert!(!pivot.with(ColorParamId::Contrast, 10).is_identity());
    }

    #[test]
    fn section_reset_keeps_the_other_section_and_amount() {
        let s = ColorStack::default()
            .with(ColorParamId::Exposure, 20)
            .with(ColorParamId::Saturation, -40)
            .with(ColorParamId::Amount, 60);
        let s = ColorStack {
            sections_enabled: s.sections_enabled.with(Section::Light, false),
            ..s
        };
        let r = s.with_section_reset(Section::Light);
        assert!(r.section_is_default(Section::Light));
        assert!(r.sections_enabled.is_on(Section::Light));
        assert_eq!(r.get(ColorParamId::Saturation), -40);
        assert_eq!(r.amount, 60);
        assert_eq!(ColorStack::stored(ColorStack::default()), None);
        assert!(ColorStack::stored(r).is_some());
    }

    #[test]
    fn hsl_curves_and_wheels_count_as_sections() {
        let red_sat = ColorParamId::Hsl {
            band: HueBand::Red,
            channel: HslChannel::Saturation,
        };
        let s = ColorStack::default().with(red_sat, -60);
        assert!(!s.is_identity() && !s.section_is_default(Section::Hsl));
        assert_eq!(s.hsl.band(HueBand::Red).sat, -60);
        assert!(s.with_band_reset(HueBand::Red).is_identity());

        let bent = Curve::new(vec![
            CurvePoint::new(0, 0),
            CurvePoint::new(5_000, 6_000),
            CurvePoint::new(10_000, 10_000),
        ])
        .unwrap();
        let s = ColorStack::default().with_curve(CurveChannel::Red, bent);
        assert!(!s.is_identity());
        assert!(s.with_section_reset(Section::Curves).is_identity());

        let s = ColorStack::default().with_puck(WheelId::Shadows, 3_000, 0);
        assert_eq!(s.puck(WheelId::Shadows), (1_000, 0));
        assert!(!s.is_identity() && !s.section_is_default(Section::Wheels));
        let none = s.with(ColorParamId::WheelStrength, 0);
        assert!(none.is_identity(), "Strength 0 turns the wheels off");
        assert!(!none.section_is_default(Section::Wheels));
        let r = none.with_section_reset(Section::Wheels);
        assert!(r.section_is_default(Section::Wheels));
        assert_eq!(r.wheels.strength, 100);
        let lum = ColorStack::default().with(ColorParamId::WheelLuma(WheelId::Offset), 300);
        assert_eq!(lum.wheels.offset.luma(), 100);
        assert!(lum.with_wheel_reset(WheelId::Offset).is_identity());
    }

    #[test]
    fn the_lut_slot_and_skin_protection_count_as_sections() {
        let slot = LutSlot::new(LutRef::new([7; 16], "teal.cube"));
        let s = ColorStack::default().with_lut(Some(slot.clone()));
        assert!(!s.is_identity() && !s.section_is_default(Section::Lut));
        assert_eq!(s.get(ColorParamId::LutIntensity), 100);
        let faint = s.with(ColorParamId::LutIntensity, 0);
        assert!(
            faint.is_identity(),
            "Intensity 0 keeps the LUT but hides it"
        );
        assert_eq!(faint.lut.as_ref().map(|l| &l.lut), Some(&slot.lut));
        // Intensity and the LUT scope's skin do nothing without a LUT.
        let none = ColorStack::default()
            .with(ColorParamId::LutIntensity, 20)
            .with(ColorParamId::SkinAmount(SkinScope::Lut), 10)
            .with_skin(SkinScope::Lut, true);
        assert_eq!(none, ColorStack::default());
        // A section reset keeps the file (CL-017).
        let tuned = s
            .with(ColorParamId::LutIntensity, 60)
            .with(ColorParamId::SkinAmount(SkinScope::Lut), 30)
            .with_skin(SkinScope::Lut, true);
        assert_eq!(
            tuned.skin(SkinScope::Lut),
            SkinProtect {
                on: true,
                amount: 30
            }
        );
        let r = tuned.with_section_reset(Section::Lut);
        assert_eq!(r.lut, Some(slot));
        // Skin protection of the Color scope alone changes nothing, but it
        // is a non-default value, and a reset switches it off.
        let skin = ColorStack::default()
            .with_skin(SkinScope::Color, true)
            .with(ColorParamId::SkinAmount(SkinScope::Color), 300);
        assert_eq!(
            skin.basic.skin,
            SkinProtect {
                on: true,
                amount: 100
            }
        );
        assert!(skin.is_identity() && !skin.section_is_default(Section::Color));
        assert!(
            skin.with_section_reset(Section::Color)
                .section_is_default(Section::Color)
        );
        assert_eq!(
            ColorParamId::parse("lut.skin_amount"),
            Some(ColorParamId::SkinAmount(SkinScope::Lut))
        );
        assert_eq!(
            color_param_spec(ColorParamId::SkinAmount(SkinScope::Color)).default,
            70
        );
    }

    #[test]
    fn mask_bits_round_trip_and_drop_unknown_ones() {
        let m = SectionMask::ALL_ON.with(Section::Masks, false);
        assert!(!m.is_on(Section::Masks) && m.is_on(Section::Auto));
        assert_eq!(SectionMask::from_bits(m.bits()), m);
        assert_eq!(SectionMask::from_bits(u16::MAX), SectionMask::ALL_ON);
        for s in Section::ALL {
            assert!(SectionMask::ALL_ON.is_on(s));
        }
    }
}
