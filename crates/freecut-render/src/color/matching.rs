//! Stage 4, the colour match (CL-020): a gain and an offset per channel of
//! Rec.709 luma and colour differences of code values (`Y'`, `Cb =
//! (B − Y')/1.8556`, `Cr = (R − Y')/1.5748`), blended toward the input by
//! Intensity.
//!
//! *FreeCut decision (K5, measured):* CL-020 planned the transfer in Oklab
//! of γ 2.4-linear light. Against the 33³ lattice that was up to 34/255 off
//! (18/255 at 65³) for ordinary transforms: a gain on a/b pushes saturated
//! colours through zero in linear light, where re-encoding has an infinite
//! slope. A gain and offset in Y'CbCr of code values is affine in code
//! values, so the lattice reproduces it exactly, as K1 decided for Hue.
//! The statistics in [`super::stats`] use the same space.

use freecut_timeline::model::{ColorStack, MATCH_ONE, Section};

use super::eval::{KB, KR, LUMA, luma};

/// `Y'`, `Cb` and `Cr` of a pixel of code values.
pub fn ycc_from_code(c: [f32; 3]) -> [f32; 3] {
    let y = luma(c);
    [y, (c[2] - y) / KB, (c[0] - y) / KR]
}

/// Code values of a `Y'CbCr` colour (the inverse of [`ycc_from_code`]).
pub fn code_from_ycc(v: [f32; 3]) -> [f32; 3] {
    let [y, cb, cr] = v;
    let r = y + KR * cr;
    let b = y + KB * cb;
    let g = (y - LUMA[0] * r - LUMA[2] * b) / LUMA[1];
    [r, g, b]
}

/// Stage 4 (CL-020): `gain · v + offset` per `Y'CbCr` channel, then
/// `lerp(input, matched, intensity)` in code values.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MatchStage {
    gain: [f32; 3],
    offset: [f32; 3],
    /// Intensity, 0–1.
    t: f32,
}

impl MatchStage {
    /// The stage of `stack`: `None` while the Match section is off or
    /// changes nothing.
    pub(crate) fn new(stack: &ColorStack) -> Option<MatchStage> {
        if !stack.sections_enabled.is_on(Section::Match) {
            return None;
        }
        let m = stack.match_xf.as_ref().filter(|m| m.changes())?;
        let scale = |v: i32| v as f32 / MATCH_ONE as f32;
        Some(MatchStage {
            gain: m.gain().map(scale),
            offset: m.offset().map(scale),
            t: f32::from(m.intensity.min(100)) / 100.0,
        })
    }

    pub(crate) fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        let v = ycc_from_code(c);
        let out = code_from_ycc([0, 1, 2].map(|i| self.gain[i] * v[i] + self.offset[i]));
        if self.t >= 1.0 {
            return out;
        }
        [0, 1, 2].map(|i| c[i] + self.t * (out[i] - c[i]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ycc_round_trips_and_greys_have_no_colour() {
        let grey = ycc_from_code([0.4; 3]);
        assert!((grey[0] - 0.4).abs() < 1e-6 && grey[1].abs() < 1e-6 && grey[2].abs() < 1e-6);
        for c in [[0.0, 0.0, 0.0], [0.2, 0.5, 0.9], [1.0, 0.0, 0.0], [0.5; 3]] {
            let back = code_from_ycc(ycc_from_code(c));
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() < 1e-5, "{c:?} → {back:?}");
            }
        }
    }
}
