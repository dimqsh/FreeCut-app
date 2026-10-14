//! Color wheels (CL-015, stage 8): lift, gamma, gain and offset.
//!
//! *FreeCut decisions (K2, color-and-filters.md › CL-001 › K2 as built):*
//! a puck is a colour-difference vector (x along Cb, y along Cr, as on a
//! vectorscope) turned into an RGB offset with zero Rec.709 luma, added to
//! every pixel weighted by the pixel's luma the way lift, gamma and gain
//! weigh their range: `1 − y`, `4y(1 − y)`, `y` and 1. So a puck changes
//! hue and never luma. The luminance sliders are the classic per-channel
//! curves applied to all three channels alike; the midtone one is a
//! rational curve, which keeps a finite slope at black where a power
//! would not.

use freecut_timeline::model::{ColorWheels, PUCK_RADIUS, Wheel, WheelId};

use super::eval::{KB, KR, LUMA, Rational, luma};

/// The colour offset at the rim, per wheel (CL-015's rim maximums):
/// shadows ±0.10 at black, midtones ±0.10 at mid-grey (about gamma
/// 0.75–1.33), highlights ±0.25 at white (gain 0.75–1.25×), offset ±0.10.
const PUCK_REACH: [f32; 4] = [0.10, 0.10, 0.25, 0.10];
/// Luminance ±100: lift ±0.10, gain ±25 %, offset ±0.10.
const LIFT: f32 = 0.10;
const GAIN: f32 = 0.25;
const OFFSET: f32 = 0.10;
/// Midtone luminance ±100: the rational curve through (0.5, 0.595) and
/// (0.5, 0.398), where gamma 0.75 and 1.33 pass.
const MID_UP: f32 = 0.469;
const MID_DOWN: f32 = 0.513;

/// A puck at `(x, y)` as an RGB offset with zero luma, 1.0 on the blue
/// channel for a puck at the rim along +x.
pub fn puck_vector(x: i16, y: i16) -> [f32; 3] {
    let r = f32::from(PUCK_RADIUS);
    let (cb, cr) = (f32::from(x) / r, f32::from(y) / r);
    let red = KR * cr;
    let blue = KB * cb;
    let green = -(LUMA[0] * red + LUMA[2] * blue) / LUMA[1];
    [red / KB, green / KB, blue / KB]
}

/// Stage 8 with its constants worked out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WheelStage {
    /// Colour offsets per wheel, scaled by reach and Strength.
    pucks: [[f32; 3]; 4],
    any_puck: bool,
    lift: f32,
    gain: f32,
    mid: Option<Rational>,
    offset: f32,
}

impl WheelStage {
    /// The stage for `wheels`; `None` when it changes nothing.
    pub(crate) fn new(wheels: &ColorWheels) -> Option<WheelStage> {
        let s = f32::from(wheels.strength.min(100)) / 100.0;
        if s == 0.0 || !wheels.any_set() {
            return None;
        }
        let w = |id: WheelId| -> &Wheel { wheels.wheel(id) };
        let level = |id: WheelId| f32::from(w(id).luma()) / 100.0 * s;
        let mut pucks = [[0.0; 3]; 4];
        for (i, id) in WheelId::ALL.into_iter().enumerate() {
            let v = puck_vector(w(id).x(), w(id).y());
            let k = PUCK_REACH[i] * s;
            pucks[i] = v.map(|c| c * k);
        }
        let any_puck = WheelId::ALL.iter().any(|id| !w(*id).puck_is_centred());
        let m = level(WheelId::Midtones);
        let mid = (m != 0.0).then(|| Rational {
            a: if m > 0.0 { MID_UP * m } else { MID_DOWN * -m },
            lift: m > 0.0,
        });
        Some(WheelStage {
            pucks,
            any_puck,
            lift: LIFT * level(WheelId::Shadows),
            gain: GAIN * level(WheelId::Highlights),
            mid,
            offset: OFFSET * level(WheelId::Offset),
        })
    }

    pub(crate) fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        let mut chroma = [0.0; 3];
        if self.any_puck {
            let y = luma(c).clamp(0.0, 1.0);
            let weights = [1.0 - y, 4.0 * y * (1.0 - y), y, 1.0];
            for (p, wt) in self.pucks.iter().zip(weights) {
                for i in 0..3 {
                    chroma[i] += p[i] * wt;
                }
            }
        }
        let mut out = c;
        for i in 0..3 {
            let mut x = out[i];
            if self.lift != 0.0 {
                x += self.lift * (1.0 - x);
            }
            if self.gain != 0.0 {
                x *= 1.0 + self.gain;
            }
            if let Some(curve) = self.mid {
                x = curve.apply(x);
            }
            out[i] = x + self.offset + chroma[i];
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn puck_vectors_have_no_luma() {
        for (x, y) in [(1000, 0), (0, 1000), (-707, 707), (300, -950), (0, 0)] {
            let v = puck_vector(x, y);
            assert!(luma(v).abs() < 1e-6, "({x}, {y}): {v:?}");
        }
        assert!((puck_vector(1000, 0)[2] - 1.0).abs() < 1e-6);
        // +y leans red, −y leans toward cyan.
        assert!(puck_vector(0, 1000)[0] > 0.5);
        assert!(puck_vector(0, -1000)[0] < -0.5);
    }
}
