//! Skin protection's mask (CL-030): how much a pixel looks like skin, from
//! the colour entering white balance (the stage-4 output), so it doesn't
//! move while sliders change the picture.
//!
//! With hue, saturation and value taken HSV-style from the code values,
//! the mask is the product of three smooth weights (FreeCut defaults,
//! CL-030): hue fully inside 12°–42°, falling to zero at 5° and 50°;
//! saturation fully inside 0.12–0.65, zero below 0.06 and above 0.80;
//! value full above 0.15, zero below 0.08. Every shoulder is a raised
//! cosine, so the mask is continuous (and so is its slope).

/// Raised-cosine step from 0 at `e0` to 1 at `e1`.
fn rise(e0: f32, e1: f32, x: f32) -> f32 {
    if x <= e0 {
        0.0
    } else if x >= e1 {
        1.0
    } else {
        0.5 - 0.5 * libm::cosf(core::f32::consts::PI * (x - e0) / (e1 - e0))
    }
}

/// 1 inside `[in0, in1]`, raised-cosine shoulders down to 0 at `out0` and
/// `out1`.
fn plateau(out0: f32, in0: f32, in1: f32, out1: f32, x: f32) -> f32 {
    rise(out0, in0, x) * (1.0 - rise(in1, out1, x))
}

/// The skin mask of a pixel of code values, 0 to 1 (CL-030).
pub fn skin_mask(rgb: [f32; 3]) -> f32 {
    let [r, g, b] = rgb.map(|c| if c.is_nan() { 0.0 } else { c.clamp(0.0, 1.0) });
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let span = max - min;
    if max <= 0.0 || span <= 0.0 {
        return 0.0;
    }
    let value = rise(0.08, 0.15, max);
    if value == 0.0 {
        return 0.0;
    }
    let sat = plateau(0.06, 0.12, 0.65, 0.80, span / max);
    if sat == 0.0 {
        return 0.0;
    }
    // HSV hue in degrees; skin sits between red and yellow, so only the
    // sextants around 0°–60° can weigh anything (the rest stays above 50°
    // or wraps to just below 360°).
    let hue = if max == r {
        let h = 60.0 * (g - b) / span;
        if h < 0.0 { h + 360.0 } else { h }
    } else if max == g {
        60.0 * (b - r) / span + 120.0
    } else {
        60.0 * (r - g) / span + 240.0
    };
    value * sat * plateau(5.0, 12.0, 42.0, 50.0, hue)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb8(r: u8, g: u8, b: u8) -> [f32; 3] {
        [r, g, b].map(|c| f32::from(c) / 255.0)
    }

    #[test]
    fn skin_patches_are_in_and_sky_foliage_and_grey_are_out() {
        // Light, medium and deep skin tones (FreeCut's own test patches).
        for skin in [
            rgb8(232, 190, 160),
            rgb8(210, 160, 120),
            rgb8(176, 120, 86),
            rgb8(120, 78, 52),
            rgb8(90, 56, 40),
        ] {
            assert!(skin_mask(skin) > 0.9, "{skin:?}: {}", skin_mask(skin));
        }
        for other in [
            rgb8(110, 160, 220), // sky
            rgb8(60, 120, 50),   // foliage
            rgb8(128, 128, 128), // grey
            rgb8(255, 255, 255),
            rgb8(0, 0, 0),
            rgb8(10, 6, 4),      // too dark
            rgb8(255, 0, 0),     // pure red: hue 0°, saturation 1
            rgb8(240, 236, 232), // nearly grey
        ] {
            assert_eq!(skin_mask(other), 0.0, "{other:?}");
        }
    }

    #[test]
    fn the_mask_is_continuous_across_its_shoulders() {
        // Walk the hue from 0° to 60° at fixed saturation and value.
        let mut last = skin_mask([0.8, 0.48, 0.48]);
        for step in 1..=600 {
            let h = step as f32 * 0.1;
            let (max, min) = (0.8f32, 0.48f32);
            let g = min + (max - min) * h / 60.0;
            let m = skin_mask([max, g, min]);
            assert!((m - last).abs() < 0.03, "jump at {h}°: {last} → {m}");
            assert!((0.0..=1.0).contains(&m));
            last = m;
        }
        assert_eq!(skin_mask([f32::NAN, 0.5, 0.2]), skin_mask([0.0, 0.5, 0.2]));
    }
}
