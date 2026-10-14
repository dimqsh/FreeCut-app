//! HSL per hue band (CL-013, stage 11): which bands a pixel belongs to and
//! by how much, and the band edits themselves.
//!
//! *FreeCut decisions (K2, color-and-filters.md › CL-001 › K2 as built):*
//! membership uses the HSV hue of the code values, so the band centres are
//! CL-013's degrees; weights are raised-cosine between the two
//! neighbouring centres and sum to 1. They fade with the pixel's chroma
//! (`max − min`): by `smoothstep(0, 0.25, chroma)` for hue and saturation,
//! by its square for lightness, so greys never move. The hue shift turns
//! the luma/colour-difference plane (like stage 12's Hue), so it keeps
//! luma.

use freecut_timeline::model::{Hsl, HueBand};

use super::eval::{luma, mix_from_luma, rotate_hue};

/// Hue ±100 shifts by this many degrees (CL-013).
const HUE_SHIFT_DEGREES: f32 = 30.0;
/// Lightness ±100 moves luminance by this share (CL-013's ±50 %).
const LIGHTNESS_RANGE: f32 = 0.5;
/// Chroma at which a pixel takes a band's edit in full.
const FULL_CHROMA: f32 = 0.25;
/// Below this chroma or luma the eyedropper names no band (CL-013).
const PICK_MIN_CHROMA: f32 = 0.05;
const PICK_MIN_LUMA: f32 = 0.03;

/// The band centres in degrees, with red repeated at 360 to close the
/// circle.
const CENTRES: [f32; 9] = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 270.0, 300.0, 360.0];

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The HSV hue of `c` in degrees, `0..360`, and its chroma (`max − min`);
/// `None` for a grey.
pub fn hue_and_chroma(c: [f32; 3]) -> Option<(f32, f32)> {
    let [r, g, b] = c;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let chroma = max - min;
    if chroma <= 0.0 {
        return None;
    }
    let h = if max == r {
        let h = 60.0 * (g - b) / chroma;
        if h < 0.0 { h + 360.0 } else { h }
    } else if max == g {
        60.0 * ((b - r) / chroma + 2.0)
    } else {
        60.0 * ((r - g) / chroma + 4.0)
    };
    Some((if h >= 360.0 { h - 360.0 } else { h }, chroma))
}

/// The two bands a hue sits between and their weights (summing to 1):
/// `((band, weight), (next band, weight))`.
fn neighbours(hue: f32) -> ((usize, f32), (usize, f32)) {
    let h = hue.clamp(0.0, 359.999_97);
    let k = CENTRES[1..].partition_point(|c| *c <= h).min(7);
    let t = (h - CENTRES[k]) / (CENTRES[k + 1] - CENTRES[k]);
    let w = 0.5 * (1.0 + libm::cosf(core::f32::consts::PI * t));
    ((k, w), ((k + 1) % 8, 1.0 - w))
}

/// Each band's membership at `hue` (degrees), before the chroma fade:
/// raised-cosine between neighbouring centres, summing to 1.
pub fn band_weights(hue: f32) -> [f32; 8] {
    let mut out = [0.0; 8];
    let ((a, wa), (b, wb)) = neighbours(hue);
    out[a] += wa;
    out[b] += wb;
    out
}

/// The band the eyedropper picks for a colour (CL-013): the one with the
/// highest membership, or none when it is nearly grey or nearly black.
pub fn hsl_band_at(rgb: [f32; 3]) -> Option<HueBand> {
    let (hue, chroma) = hue_and_chroma(rgb)?;
    if chroma < PICK_MIN_CHROMA || luma(rgb) < PICK_MIN_LUMA {
        return None;
    }
    let ((a, wa), (b, wb)) = neighbours(hue);
    let i = if wa >= wb { a } else { b };
    HueBand::ALL.get(i).copied()
}

/// Stage 11 with its levels as fractions: per band hue (−1..=1),
/// saturation and lightness.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HslStage {
    hue: [f32; 8],
    sat: [f32; 8],
    light: [f32; 8],
}

impl HslStage {
    /// The stage for `hsl`; `None` when every band is at its defaults.
    pub(crate) fn new(hsl: &Hsl) -> Option<HslStage> {
        if hsl.is_default() {
            return None;
        }
        let f = |v: i16| f32::from(v) / 100.0;
        Some(HslStage {
            hue: hsl.bands.map(|b| f(b.hue)),
            sat: hsl.bands.map(|b| f(b.sat)),
            light: hsl.bands.map(|b| f(b.light)),
        })
    }

    pub(crate) fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        let Some((hue, chroma)) = hue_and_chroma(c) else {
            return c;
        };
        let fade = smoothstep(0.0, FULL_CHROMA, chroma);
        if fade == 0.0 {
            return c;
        }
        let ((a, wa), (b, wb)) = neighbours(hue);
        let mix = |v: &[f32; 8]| wa * v[a] + wb * v[b];
        let (dh, ds, dl) = (mix(&self.hue), mix(&self.sat), mix(&self.light));
        let mut c = c;
        if dh != 0.0 {
            let deg = dh * HUE_SHIFT_DEGREES * fade;
            let (s, co) = libm::sincosf(deg.to_radians());
            c = rotate_hue(c, co, s);
        }
        if ds != 0.0 {
            c = mix_from_luma(c, (1.0 + ds * fade).max(0.0));
        }
        if dl != 0.0 {
            // Toward white for positive, toward black for negative.
            let k = dl * LIGHTNESS_RANGE * fade * fade;
            c = if k > 0.0 {
                c.map(|x| x + k * (1.0 - x))
            } else {
                c.map(|x| x * (1.0 + k))
            };
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_sum_to_one_and_wrap_continuously() {
        for i in 0..3600 {
            let h = i as f32 / 10.0;
            let w = band_weights(h);
            let sum: f32 = w.iter().sum();
            assert!((sum - 1.0).abs() < 1e-6, "{h}: {sum}");
            assert!(w.iter().all(|v| (0.0..=1.0).contains(v)));
        }
        for band in HueBand::ALL {
            let w = band_weights(f32::from(band.center_degrees()));
            assert!((w[band.index()] - 1.0).abs() < 1e-6, "{band:?}");
        }
        // Just below 360° is almost all red, as 0° is.
        let near = band_weights(359.99);
        assert!((near[0] - 1.0).abs() < 1e-4 && near[7] < 1e-4);
        // Half-way between two centres splits 50/50 (CL-013).
        let mid = band_weights(15.0);
        assert!((mid[0] - 0.5).abs() < 1e-6 && (mid[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn hues_of_the_primaries() {
        let h = |c| hue_and_chroma(c).unwrap().0;
        assert_eq!(h([1.0, 0.0, 0.0]), 0.0);
        assert_eq!(h([1.0, 1.0, 0.0]), 60.0);
        assert_eq!(h([0.0, 1.0, 0.0]), 120.0);
        assert_eq!(h([0.0, 0.0, 1.0]), 240.0);
        assert_eq!(h([1.0, 0.0, 1.0]), 300.0);
        assert_eq!(hue_and_chroma([0.4; 3]), None);
        assert_eq!(hsl_band_at([0.9, 0.5, 0.1]), Some(HueBand::Orange));
        assert_eq!(hsl_band_at([0.2, 0.6, 0.2]), Some(HueBand::Green));
        assert_eq!(hsl_band_at([0.5, 0.52, 0.5]), None, "nearly grey");
        assert_eq!(hsl_band_at([0.1, 0.0, 0.0]), None, "nearly black");
    }
}
