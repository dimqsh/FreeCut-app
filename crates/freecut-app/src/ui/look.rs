//! The Look lens's words (CL-007, CL-008, CL-013 to CL-017, CL-019,
//! CL-020, CL-030, CL-043):
//! slider and section labels, tooltips, folded-section summaries and the
//! pages' empty states. FreeCut's own wording. Pure, so it is tested
//! without a window; the lens itself is `ui::inspector::look`.

use freecut_timeline::model::{
    AutoCorrection, AutoSource, AutoStyle, ColorMaskParam, ColorPage, ColorParamId, ColorStack,
    CurveChannel, HslChannel, HueBand, LutInterp, LutRef, MaskCombine, MaskParam, MaskShapeKind,
    Section, SkinScope, WheelId,
};

/// The full names of the HSL sliders, by band and channel (the keyframe
/// lists and summaries name the band; the HSL page's rows don't).
const HSL_LABELS: [[&str; 3]; 8] = [
    ["Red hue", "Red saturation", "Red lightness"],
    ["Orange hue", "Orange saturation", "Orange lightness"],
    ["Yellow hue", "Yellow saturation", "Yellow lightness"],
    ["Green hue", "Green saturation", "Green lightness"],
    ["Cyan hue", "Cyan saturation", "Cyan lightness"],
    ["Blue hue", "Blue saturation", "Blue lightness"],
    ["Purple hue", "Purple saturation", "Purple lightness"],
    ["Magenta hue", "Magenta saturation", "Magenta lightness"],
];

/// A colour band's name.
pub fn band_label(band: HueBand) -> &'static str {
    match band {
        HueBand::Red => "Red",
        HueBand::Orange => "Orange",
        HueBand::Yellow => "Yellow",
        HueBand::Green => "Green",
        HueBand::Cyan => "Cyan",
        HueBand::Blue => "Blue",
        HueBand::Purple => "Purple",
        HueBand::Magenta => "Magenta",
    }
}

/// A wheel's name.
pub fn wheel_label(wheel: WheelId) -> &'static str {
    match wheel {
        WheelId::Shadows => "Shadows",
        WheelId::Midtones => "Midtones",
        WheelId::Highlights => "Highlights",
        WheelId::Offset => "Offset",
    }
}

/// What a wheel tints, for its tooltip.
pub fn wheel_hint(wheel: WheelId) -> &'static str {
    match wheel {
        WheelId::Shadows => "Tints the dark parts; drag toward a colour, double-click to centre",
        WheelId::Midtones => "Tints the middle tones; drag toward a colour, double-click to centre",
        WheelId::Highlights => {
            "Tints the bright parts; drag toward a colour, double-click to centre"
        }
        WheelId::Offset => "Tints everything alike; drag toward a colour, double-click to centre",
    }
}

/// A wheel puck's name in keyframe lists.
pub fn puck_label(wheel: WheelId) -> &'static str {
    match wheel {
        WheelId::Shadows => "Shadows colour",
        WheelId::Midtones => "Midtones colour",
        WheelId::Highlights => "Highlights colour",
        WheelId::Offset => "Offset colour",
    }
}

/// A curve tab's name.
pub fn curve_label(channel: CurveChannel) -> &'static str {
    match channel {
        CurveChannel::Luma => "Luma",
        CurveChannel::Red => "Red",
        CurveChannel::Green => "Green",
        CurveChannel::Blue => "Blue",
    }
}

/// The label of a slider's own row: the HSL page names the channel only
/// (the swatch above names the band) and a wheel's luminance sits under
/// its wheel; everything else as [`param_label`].
pub fn row_label(id: ColorParamId) -> &'static str {
    match id {
        ColorParamId::Hsl { channel, .. } => match channel {
            HslChannel::Hue => "Hue",
            HslChannel::Saturation => "Saturation",
            HslChannel::Lightness => "Lightness",
        },
        ColorParamId::WheelLuma(_) => "Luminance",
        ColorParamId::LutIntensity | ColorParamId::MatchIntensity => "Intensity",
        ColorParamId::SkinAmount(_) => "Amount",
        ColorParamId::AutoStrength => "Strength",
        other => param_label(other),
    }
}

/// A slider's full name (keyframe lists, summaries).
pub fn param_label(id: ColorParamId) -> &'static str {
    match id {
        ColorParamId::Hsl { band, channel } => HSL_LABELS[band.index()][channel as usize],
        ColorParamId::WheelLuma(w) => match w {
            WheelId::Shadows => "Shadows luminance",
            WheelId::Midtones => "Midtones luminance",
            WheelId::Highlights => "Highlights luminance",
            WheelId::Offset => "Offset luminance",
        },
        ColorParamId::WheelStrength => "Strength",
        ColorParamId::Exposure => "Exposure",
        ColorParamId::Brightness => "Brightness",
        ColorParamId::Contrast => "Contrast",
        ColorParamId::ContrastPivot => "Pivot",
        ColorParamId::Highlights => "Highlights",
        ColorParamId::Shadows => "Shadows",
        ColorParamId::Whites => "Whites",
        ColorParamId::Blacks => "Blacks",
        ColorParamId::ToneLift => "Tone lift",
        ColorParamId::Temperature => "Temperature",
        ColorParamId::Tint => "Tint",
        ColorParamId::Hue => "Hue",
        ColorParamId::Saturation => "Saturation",
        ColorParamId::Vibrance => "Vibrance",
        ColorParamId::Amount => "Amount",
        ColorParamId::LutIntensity => "LUT intensity",
        ColorParamId::SkinAmount(SkinScope::Color) => "Skin protection",
        ColorParamId::SkinAmount(SkinScope::Lut) => "LUT skin protection",
        ColorParamId::AutoStrength => "Auto strength",
        ColorParamId::MatchIntensity => "Match intensity",
    }
}

/// What a slider does, for its row's tooltip.
pub fn param_hint(id: ColorParamId) -> &'static str {
    match id {
        ColorParamId::Exposure => "Brighter or darker overall, like opening the lens (±3 stops)",
        ColorParamId::Brightness => "Lifts or lowers the midtones; black and white stay",
        ColorParamId::Contrast => "Spreads tones apart or draws them together around the pivot",
        ColorParamId::ContrastPivot => {
            "The tone Contrast turns around: lower brightens more of the picture"
        }
        ColorParamId::Highlights => "Brightens or recovers the bright parts",
        ColorParamId::Shadows => "Lifts or deepens the dark parts",
        ColorParamId::Whites => "Moves the top of the range: up toward clipping, down to soften",
        ColorParamId::Blacks => "Moves the bottom of the range: down crushes, up lifts",
        ColorParamId::ToneLift => {
            "Brightens shadows and midtones while easing the highlights, for backlit or flat shots"
        }
        ColorParamId::Temperature => "Cooler (blue) or warmer (amber)",
        ColorParamId::Tint => "Toward green or toward magenta",
        ColorParamId::Hue => {
            "Turns every colour around the colour wheel; to change one colour, use HSL"
        }
        ColorParamId::Saturation => "Less colour, down to black and white, or more",
        ColorParamId::Vibrance => "More colour where there is little, easy on skin tones",
        ColorParamId::Amount => "How much of the whole grade shows",
        ColorParamId::Hsl { channel, .. } => match channel {
            HslChannel::Hue => "Shifts this colour toward its neighbours, up to 30°",
            HslChannel::Saturation => "Less of this colour, down to grey, or more of it",
            HslChannel::Lightness => "Darkens or lightens this colour only",
        },
        ColorParamId::WheelLuma(w) => match w {
            WheelId::Shadows => "Lifts or lowers the dark end of the range",
            WheelId::Midtones => "Brightens or darkens the middle tones; black and white stay",
            WheelId::Highlights => "Raises or lowers the bright end of the range",
            WheelId::Offset => "Moves every tone up or down alike",
        },
        ColorParamId::WheelStrength => "How much of all four wheels shows",
        ColorParamId::LutIntensity => "How much of the LUT shows; 0 keeps the LUT but hides it",
        ColorParamId::SkinAmount(_) => SKIN_HINT,
        ColorParamId::AutoStrength => {
            "How much of the automatic correction shows; your own sliders keep their values"
        }
        ColorParamId::MatchIntensity => "How much of the match shows; 0 keeps it but hides it",
    }
}

/// The Auto button (CL-019); Re-run once there is a correction.
pub const AUTO_BUTTON: &str = "Auto";
pub const RERUN: &str = "Re-run";
pub const APPLY_TO_SLIDERS: &str = "Apply to sliders";
/// What the Auto button does, for its tooltip.
pub const AUTO_HINT: &str = "Measures the footage and sets a neutral, well-exposed starting point \
     under your own sliders. Footage meant to look warm or dark may be corrected too: lower \
     Strength or switch Auto off";
/// What Apply to sliders does.
pub const APPLY_HINT: &str = "Adds the correction at its Strength into the Light and Color \
     sliders and removes it, so you edit plain values";
/// Why Auto and Match can't run on an adjustment clip.
pub const NO_FOOTAGE: &str = "Auto and Match measure a clip's own footage; adjustment clips \
     have none";
/// Why Auto and Match run on one clip at a time in this build.
pub const ONE_AT_A_TIME: &str = "Select one clip: Auto and Match measure one clip at a time";

/// What Auto analyses, for its select.
pub fn auto_source_label(source: AutoSource) -> &'static str {
    match source {
        AutoSource::WholeClip => "Whole clip",
        AutoSource::Playhead => "Frame at playhead",
    }
}

pub fn auto_source_hint(source: AutoSource) -> &'static str {
    match source {
        AutoSource::WholeClip => {
            "Five frames spread over the clip, so one odd frame doesn't decide"
        }
        AutoSource::Playhead => "Only the frame under the playhead",
    }
}

/// Auto's style, for its select.
pub fn auto_style_label(style: AutoStyle) -> &'static str {
    match style {
        AutoStyle::Natural => "Natural",
        AutoStyle::Vivid => "Vivid",
    }
}

pub fn auto_style_hint(style: AutoStyle) -> &'static str {
    match style {
        AutoStyle::Natural => "Corrects only",
        AutoStyle::Vivid => "Corrects, then adds Vibrance +20 and Saturation +5",
    }
}

/// One read-only row of a correction (CL-019): the slider's name and the
/// offset at Strength 100, e.g. ("Exposure", "+12").
pub fn auto_rows(auto: &AutoCorrection) -> Vec<(ColorParamId, &'static str, String)> {
    freecut_timeline::model::AutoOffsets::PARAMS
        .into_iter()
        .filter(|id| auto.offsets.get(*id) != 0)
        .map(|id| (id, param_label(id), signed(auto.offsets.get(id))))
        .collect()
}

/// Whether the correction wants more than slider `id`'s range allows on
/// top of `manual` (CL-019's "limited" hint).
pub fn auto_limited(auto: &AutoCorrection, id: ColorParamId, manual: i16) -> bool {
    let wanted = i32::from(manual) + i32::from(auto.added(id));
    let spec = id.spec();
    wanted < i32::from(spec.min) || wanted > i32::from(spec.max)
}

/// The Match section's button before a match (CL-020).
pub const MATCH_BUTTON: &str = "Match color…";
pub const REMATCH: &str = "Re-match";
pub const CHANGE_REFERENCE: &str = "Change reference…";
pub const MATCH_HINT: &str = "Makes this clip's colours resemble a frame of another clip: \
     pick the frame on the timeline";
/// The pick bar's sentence (CL-020's pick mode).
pub const PICK_SENTENCE: &str = "Click a frame of another clip on the timeline to match to";
pub const USE_PLAYHEAD: &str = "Use frame at playhead";
pub const USE_PLAYHEAD_HINT: &str = "Matches to the topmost other clip under the playhead";
pub const UNGRADED: &str = "Use ungraded source";
pub const UNGRADED_HINT: &str = "Measure the reference without its own colour settings";

/// What an analysis that found nothing to do says (CL-019, CL-020).
pub fn analysis_failure(error: &freecut_render::color::stats::ColorStatsError) -> &'static str {
    use freecut_render::color::stats::ColorStatsError as E;
    match error {
        E::NoPixels => "Nothing to measure: the frames are fully transparent",
        E::NothingToCorrect => "Nothing to correct: the frames are almost one colour",
        E::NotEnoughDetail => "Not enough detail to match: a frame is black or one colour",
    }
}

/// Skin protection's tooltip (CL-030).
pub const SKIN_HINT: &str = "Keeps skin tones out of this section's colour change. It works by      colour, so orange and wooden things are kept too";

/// The Skin protection switch's label (CL-030).
pub const SKIN_LABEL: &str = "Skin protection";

/// What a scope's Skin protection switch keeps, for its row's tooltip.
pub fn skin_switch_hint(scope: SkinScope) -> &'static str {
    match scope {
        SkinScope::Color => {
            "Keeps white balance, hue, saturation, vibrance and HSL changes off faces and hands"
        }
        SkinScope::Lut => "Keeps the LUT's colour off faces and hands",
    }
}

/// An interpolation's name in the LUT section's More (CL-017).
pub fn interp_label(interp: LutInterp) -> &'static str {
    match interp {
        LutInterp::Tetrahedral => "Tetrahedral",
        LutInterp::Trilinear => "Trilinear",
    }
}

/// What an interpolation does, for its tooltip.
pub fn interp_hint(interp: LutInterp) -> &'static str {
    match interp {
        LutInterp::Tetrahedral => "Smoothest between the LUT's points (the usual choice)",
        LutInterp::Trilinear => "Simpler blending between the LUT's points, as some other tools do",
    }
}

/// The LUT section's name for a file it can't name better: its file name
/// without the extension.
pub fn lut_file_label(lut: &LutRef) -> &str {
    lut.file_name
        .rsplit_once('.')
        .map_or(lut.file_name.as_str(), |(stem, _)| stem)
}

/// A section's title.
pub fn section_label(section: Section) -> &'static str {
    match section {
        Section::Auto => "Auto",
        Section::Match => "Match",
        Section::Light => "Light",
        Section::Color => "Color",
        Section::Lut => "LUT",
        Section::Filter => "Filter",
        Section::Detail => "Detail & finish",
        Section::Hsl => "HSL",
        Section::Curves => "Curves",
        Section::Wheels => "Wheels",
        Section::Masks => "Masks",
    }
}

/// A page's label in the page strip.
pub fn page_label(page: ColorPage) -> &'static str {
    match page {
        ColorPage::Basic => "Basic",
        ColorPage::Hsl => "HSL",
        ColorPage::Curves => "Curves",
        ColorPage::Wheels => "Wheels",
        ColorPage::Masks => "Masks",
    }
}

/// A mask shape's name (CL-031, TR-017): the chip, the picker and the
/// Add mask menu.
pub fn mask_shape_label(kind: MaskShapeKind) -> &'static str {
    match kind {
        MaskShapeKind::Linear => "Linear",
        MaskShapeKind::Band => "Band",
        MaskShapeKind::Ellipse => "Ellipse",
        MaskShapeKind::Rectangle => "Rectangle",
    }
}

/// What a mask shape keeps, for its tooltip.
pub fn mask_shape_hint(kind: MaskShapeKind) -> &'static str {
    match kind {
        MaskShapeKind::Linear => "One straight edge: the grade applies on one side of it",
        MaskShapeKind::Band => "A strip between two parallel edges",
        MaskShapeKind::Ellipse => "An oval or a circle",
        MaskShapeKind::Rectangle => "A box, with corners as round as you like",
    }
}

/// A mask chip's label: its place in the list and its shape, "1 · Ellipse".
pub fn mask_chip_label(index: usize, kind: MaskShapeKind) -> String {
    format!("{} · {}", index + 1, mask_shape_label(kind))
}

/// A combine mode's name (TR-020).
pub fn mask_mode_label(mode: MaskCombine) -> &'static str {
    match mode {
        MaskCombine::Add => "Add",
        MaskCombine::Subtract => "Subtract",
        MaskCombine::Intersect => "Intersect",
    }
}

/// What a combine mode does with the masks above it in the list.
pub fn mask_mode_hint(mode: MaskCombine) -> &'static str {
    match mode {
        MaskCombine::Add => "Grades this area as well as the masks before it",
        MaskCombine::Subtract => "Cuts this area out of the masks before it",
        MaskCombine::Intersect => "Keeps only where this area and the masks before it overlap",
    }
}

/// A mask value's row label.
pub fn mask_param_label(param: ColorMaskParam) -> &'static str {
    match param {
        ColorMaskParam::Shape(MaskParam::Center) => "Position",
        ColorMaskParam::Shape(MaskParam::Size) => "Size",
        ColorMaskParam::Shape(MaskParam::Rotation) => "Rotation",
        ColorMaskParam::Shape(MaskParam::Feather) => "Feather",
        ColorMaskParam::Shape(MaskParam::Roundness) => "Roundness",
        ColorMaskParam::Shape(MaskParam::BandWidth) => "Band width",
        ColorMaskParam::Expand => "Expand",
    }
}

/// A mask value's tooltip.
pub fn mask_param_hint(param: ColorMaskParam) -> &'static str {
    match param {
        ColorMaskParam::Shape(MaskParam::Center) => {
            "Where the mask sits, from the picture's centre, in % of its shorter side"
        }
        ColorMaskParam::Shape(MaskParam::Size) => "Width and height, in % of the shorter side",
        ColorMaskParam::Shape(MaskParam::Rotation) => "Turns the mask clockwise",
        ColorMaskParam::Shape(MaskParam::Feather) => {
            "Softens the edge: 100 % fades over a quarter of the shorter side"
        }
        ColorMaskParam::Shape(MaskParam::Roundness) => "Rounds the corners",
        ColorMaskParam::Shape(MaskParam::BandWidth) => {
            "How wide the strip is, in % of the shorter side"
        }
        ColorMaskParam::Expand => "Grows or shrinks the edge without moving the mask",
    }
}

/// The Masks page with no mask yet: its sentence (CL-031).
pub const NO_MASKS: &str =
    "No masks: the grade covers the whole picture. Add a mask to grade only part of it.";

/// Why Add mask is greyed out at the cap (CL-031).
pub const MASKS_FULL: &str = "A clip holds at most 8 masks";

/// Why the Masks page edits one clip at a time.
pub const ONE_CLIP_FOR_MASKS: &str = "Select one clip to edit its masks";

/// "1 mask", "3 masks".
pub fn masks_count(n: usize) -> String {
    if n == 1 {
        "1 mask".to_owned()
    } else {
        format!("{n} masks")
    }
}

/// What Reset all asks before it removes masks (CL-024, CL-031): short
/// enough for the footer's row.
pub fn reset_all_masks_question(masks: usize) -> String {
    format!("Also removes {}?", masks_count(masks))
}

/// A slider value as the summary writes it: "+12", "−5" (a true minus).
fn signed(v: i16) -> String {
    if v < 0 {
        format!("\u{2212}{}", v.unsigned_abs())
    } else {
        format!("+{v}")
    }
}

/// A folded section's summary (CL-043): "Exposure +12 · Contrast −5",
/// "off" while switched off, empty at the defaults. At most three values,
/// then "+N".
pub fn section_summary(stack: &ColorStack, section: Section) -> String {
    if !stack.sections_enabled.is_on(section) {
        return "off".to_owned();
    }
    let changed: Vec<String> = match section {
        // The bands that changed: "Orange · Blue".
        Section::Hsl => HueBand::ALL
            .into_iter()
            .filter(|b| !stack.hsl.band(*b).is_default())
            .map(|b| band_label(b).to_owned())
            .collect(),
        // The curves off the diagonal: "Luma · Red".
        Section::Curves => CurveChannel::ALL
            .into_iter()
            .filter(|c| !stack.curves.get(*c).is_default())
            .map(|c| curve_label(c).to_owned())
            .collect(),
        // The wheels that tint or move a level, then Strength.
        Section::Wheels => WheelId::ALL
            .into_iter()
            .filter(|w| !stack.wheels.wheel(*w).is_default())
            .map(|w| wheel_label(w).to_owned())
            .chain(
                (stack.wheels.strength != 100)
                    .then(|| format!("Strength {} %", stack.wheels.strength)),
            )
            .collect(),
        // The LUT and its intensity: "teal-night · 60 %".
        Section::Lut => stack
            .lut
            .iter()
            .flat_map(|l| {
                std::iter::once(lut_file_label(&l.lut).to_owned())
                    .chain((l.intensity != 100).then(|| format!("{} %", l.intensity)))
                    .chain(l.skin.on.then(|| SKIN_LABEL.to_owned()))
            })
            .collect(),
        // The offsets at Strength 100: "Exposure +12 · Temperature −8".
        Section::Auto => stack
            .auto
            .iter()
            .flat_map(|a| {
                auto_rows(a)
                    .into_iter()
                    .map(|(_, name, v)| format!("{name} {v}"))
                    .chain((a.strength != 100).then(|| format!("{} %", a.strength)))
            })
            .collect(),
        Section::Match => stack
            .match_xf
            .iter()
            .flat_map(|m| {
                std::iter::once("Matched".to_owned())
                    .chain((m.intensity != 100).then(|| format!("{} %", m.intensity)))
            })
            .collect(),
        // How many masks, while there are any: "2 masks".
        Section::Masks => (!stack.masks.is_empty())
            .then(|| masks_count(stack.masks.len()))
            .into_iter()
            .collect(),
        Section::Color => ColorParamId::COLOR
            .iter()
            .filter(|id| stack.get(**id) != id.spec().default)
            .map(|id| format!("{} {}", param_label(*id), signed(stack.get(*id))))
            .chain(stack.basic.skin.on.then(|| SKIN_LABEL.to_owned()))
            .collect(),
        _ => section_values(stack, section),
    };
    match changed.len() {
        0..=3 => changed.join(" · "),
        n => format!("{} · +{}", changed[..2].join(" · "), n - 2),
    }
}

/// The sliders of `section` away from their defaults, as "Exposure +12".
fn section_values(stack: &ColorStack, section: Section) -> Vec<String> {
    section
        .params()
        .iter()
        .filter(|id| stack.get(**id) != id.spec().default)
        .map(|id| format!("{} {}", param_label(*id), signed(stack.get(*id))))
        .collect()
}

/// Whether a page holds a value away from its default (its dot, CL-043).
pub fn page_changed(stack: &ColorStack, page: ColorPage) -> bool {
    Section::ALL
        .into_iter()
        .filter(|s| s.page() == page)
        .any(|s| !stack.section_is_default(s) || !stack.sections_enabled.is_on(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_name_changed_sliders_with_true_minus_signs() {
        let s = ColorStack::default()
            .with(ColorParamId::Exposure, 12)
            .with(ColorParamId::Contrast, -5);
        assert_eq!(
            section_summary(&s, Section::Light),
            "Exposure +12 · Contrast \u{2212}5"
        );
        assert_eq!(section_summary(&s, Section::Color), "");
        let busy = s
            .with(ColorParamId::Shadows, 1)
            .with(ColorParamId::Whites, 2);
        assert_eq!(
            section_summary(&busy, Section::Light),
            "Exposure +12 · Contrast \u{2212}5 · +2"
        );
        let off = ColorStack {
            sections_enabled: s.sections_enabled.with(Section::Light, false),
            ..s.clone()
        };
        assert_eq!(section_summary(&off, Section::Light), "off");
        assert!(page_changed(&s, ColorPage::Basic));
        assert!(!page_changed(&s, ColorPage::Curves));
    }

    #[test]
    fn every_param_and_page_has_words() {
        for id in ColorParamId::ALL {
            assert!(!param_label(id).is_empty() && !param_hint(id).is_empty());
        }
        for page in ColorPage::ALL {
            assert!(!page_label(page).is_empty());
        }
        for s in Section::ALL {
            assert!(!section_label(s).is_empty());
        }
        // Every full slider name is distinct, so keyframe lists can tell
        // them apart.
        let names: std::collections::BTreeSet<&str> =
            ColorParamId::ALL.into_iter().map(param_label).collect();
        assert_eq!(names.len(), ColorParamId::ALL.len());
        for w in WheelId::ALL {
            assert!(!wheel_hint(w).is_empty() && !puck_label(w).is_empty());
        }
    }

    #[test]
    fn k3_summaries_name_the_lut_and_skin_protection() {
        use freecut_timeline::model::LutSlot;
        let s = ColorStack::default()
            .with_lut(Some(LutSlot::new(LutRef::new([1; 16], "teal-night.cube"))))
            .with(ColorParamId::LutIntensity, 60)
            .with_skin(SkinScope::Lut, true)
            .with(ColorParamId::Temperature, -10)
            .with_skin(SkinScope::Color, true);
        assert_eq!(
            section_summary(&s, Section::Lut),
            "teal-night · 60 % · Skin protection"
        );
        assert_eq!(
            section_summary(&s, Section::Color),
            "Temperature \u{2212}10 · Skin protection"
        );
        assert_eq!(row_label(ColorParamId::LutIntensity), "Intensity");
        assert_eq!(lut_file_label(&LutRef::new([0; 16], "noext")), "noext");
        for i in LutInterp::ALL {
            assert!(!interp_label(i).is_empty() && !interp_hint(i).is_empty());
        }
        for scope in SkinScope::ALL {
            assert!(!skin_switch_hint(scope).is_empty());
        }
    }

    #[test]
    fn k2_summaries_name_bands_curves_and_wheels() {
        use freecut_timeline::model::{Curve, CurvePoint};
        let s = ColorStack::default()
            .with(
                ColorParamId::Hsl {
                    band: HueBand::Orange,
                    channel: HslChannel::Saturation,
                },
                -20,
            )
            .with(
                ColorParamId::Hsl {
                    band: HueBand::Blue,
                    channel: HslChannel::Hue,
                },
                10,
            );
        assert_eq!(section_summary(&s, Section::Hsl), "Orange · Blue");
        assert!(page_changed(&s, ColorPage::Hsl));
        let bent = Curve::new(vec![
            CurvePoint::new(0, 0),
            CurvePoint::new(5_000, 6_000),
            CurvePoint::new(10_000, 10_000),
        ])
        .unwrap();
        let c = ColorStack::default().with_curve(CurveChannel::Red, bent);
        assert_eq!(section_summary(&c, Section::Curves), "Red");
        let w = ColorStack::default()
            .with_puck(WheelId::Shadows, 100, 0)
            .with(ColorParamId::WheelStrength, 60);
        assert_eq!(
            section_summary(&w, Section::Wheels),
            "Shadows · Strength 60 %"
        );
        assert_eq!(
            row_label(ColorParamId::WheelLuma(WheelId::Offset)),
            "Luminance"
        );
        assert_eq!(
            param_label(ColorParamId::WheelLuma(WheelId::Offset)),
            "Offset luminance"
        );
    }
}
