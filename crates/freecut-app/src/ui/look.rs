//! The Look lens's words (CL-007, CL-008, CL-043): slider and section
//! labels, tooltips, folded-section summaries and the pages' empty states.
//! FreeCut's own wording. Pure, so it is tested without a window; the lens
//! itself is `ui::inspector::look`.

use freecut_timeline::model::{ColorPage, ColorParamId, ColorStack, Section};

/// A slider's row label.
pub fn param_label(id: ColorParamId) -> &'static str {
    match id {
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
    }
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

/// The empty state of a page this build doesn't edit yet: its title and
/// sentence.
pub fn page_empty_state(page: ColorPage) -> Option<(&'static str, &'static str)> {
    match page {
        ColorPage::Basic => None,
        ColorPage::Hsl => Some((
            "HSL is coming in this build",
            "Hue, saturation and lightness for each of eight colour bands will be edited here.",
        )),
        ColorPage::Curves => Some((
            "Curves are coming in this build",
            "A master curve and red, green and blue curves will be drawn here.",
        )),
        ColorPage::Wheels => Some((
            "Wheels are coming in this build",
            "Shadows, midtones, highlights and offset wheels will tint each tonal range here.",
        )),
        ColorPage::Masks => Some((
            "Masks are coming in this build",
            "Shapes that limit the grade to part of the picture will be listed here.",
        )),
    }
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
    let changed: Vec<String> = section
        .params()
        .iter()
        .filter(|id| stack.get(**id) != id.spec().default)
        .map(|id| format!("{} {}", param_label(*id), signed(stack.get(*id))))
        .collect();
    match changed.len() {
        0..=3 => changed.join(" · "),
        n => format!("{} · +{}", changed[..2].join(" · "), n - 2),
    }
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
            assert_eq!(page_empty_state(page).is_none(), page == ColorPage::Basic);
        }
        for s in Section::ALL {
            assert!(!section_label(s).is_empty());
        }
    }
}
