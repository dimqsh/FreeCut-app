//! The reference evaluator of the per-pixel colour stages (CL-001's stage
//! table, CL-007, CL-008, CL-013 to CL-015, CL-017, CL-030, CL-035;
//! render-compositor.md §17): one function per stage, in the stack's order
//! (white balance, exposure, tone, wheels, curves, HSL,
//! hue/saturation/vibrance, the user LUT, Amount), with skin protection
//! blending stages 5, 11–12 and 13 back toward their inputs.
//! It is the oracle of the baked lattice and, later, of the GPU path.
//!
//! **Working space.** Display-referred Rec.709 code values, 0–1, as
//! decoded (CL-002). Linear light only inside white balance and exposure,
//! through a pure γ 2.4 power with the sign mirrored for negative values
//! ([`ws_to_linear`]). Hue rotation turns the luma/colour-difference
//! plane of code values (see [`rotate_hue`] for why not Oklab). Luma is
//! Rec.709-weighted code values ([`luma`]). Values
//! between stages may leave 0–1; only the final quantization clamps.
//!
//! **Precision.** Plain `f32` arithmetic (Rust never fuses multiply-adds on
//! its own) and the `libm` crate's transcendental functions, so results are
//! bit-identical on every OS. A stage whose controls sit at their defaults
//! is skipped, not run with neutral constants, so an identity stack returns
//! its input exactly.
//!
//! **Curves** (FreeCut defaults, CL-007 and CL-008 leave the shapes open):
//! every tone curve has a finite slope everywhere (rational and power
//! curves with bounded ends, extended linearly outside 0–1), so the
//! 33³ lattice follows it within the accuracy bound.

use std::sync::Arc;

use freecut_media::lut::Lut;
use freecut_timeline::model::{ColorStack, Section, SkinScope};

use super::curve::CurvesStage;
use super::hsl::HslStage;
use super::lut::{LutSource, LutStage, active_slot};
use super::skin::skin_mask;
use super::wheels::WheelStage;

/// Rec.709 luma weights, applied to code values (README C52).
pub const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
/// The Rec.709 colour-difference scales: `Cb = (B − Y)/KB`,
/// `Cr = (R − Y)/KR`.
pub(crate) const KB: f32 = 1.8556;
pub(crate) const KR: f32 = 1.5748;

/// Exposure ±100 is ±3 EV (CL-007).
const EXPOSURE_EV: f32 = 3.0;
/// White balance: log2 of the largest channel gain of Temperature and Tint
/// at ±100, before luminance normalization (about ±3000 K around D65 for
/// Temperature; FreeCut approximation, CL-008).
const WB_STOPS: f32 = 0.3;
/// How far Blacks and Whites move their end of the range at ±100.
const RANGE_ENDS: f32 = 0.2;
/// The strength of Highlights and Shadows at ±100: their luma bumps
/// `y⁴(1−y)²` and `y²(1−y)⁴` peak at 2/3 and 1/3 with an offset of 0.15.
const TONE_RANGE: f32 = 0.15 / (16.0 / 729.0);
/// Tone lift ±100: the offset `y(1−y)` bump's strength, 0.15 at mid-grey
/// (slope 1.6 at black and 0.4 at white for +100).
const TONE_LIFT: f32 = 0.6;
/// Brightness +100: the rational curve through (0.5, 0.707), which a power
/// of 0.5 also passes; −100 through (0.5, 0.25), as a power of 2 does
/// (CL-007's exponents, with a finite slope at the ends).
const BRIGHT_UP: f32 = core::f32::consts::SQRT_2;
const BRIGHT_DOWN: f32 = 2.0;
/// Contrast's pivot at Pivot 0: the encoded value of 18 % grey (CL-007),
/// moved by ±0.2 at Pivot ±100.
const PIVOT: f32 = 0.41;
const PIVOT_RANGE: f32 = 0.2;
/// Hue: 1.8° per step (CL-008).
const HUE_DEGREES_PER_STEP: f32 = 1.8;
/// How much less Vibrance acts across the skin band (CL-008).
const SKIN_RELIEF: f32 = 0.7;

/// `v` from code values to linear light: `sign(v)·|v|^2.4` (G5).
pub fn ws_to_linear(v: f32) -> f32 {
    if v == 0.0 {
        return 0.0;
    }
    let m = libm::powf(v.abs(), 2.4);
    if v < 0.0 { -m } else { m }
}

/// The inverse of [`ws_to_linear`].
pub fn linear_to_ws(l: f32) -> f32 {
    if l == 0.0 {
        return 0.0;
    }
    let m = libm::powf(l.abs(), 1.0 / 2.4);
    if l < 0.0 { -m } else { m }
}

/// Rec.709 luma of code values.
pub fn luma(c: [f32; 3]) -> f32 {
    LUMA[0] * c[0] + LUMA[1] * c[1] + LUMA[2] * c[2]
}

/// Tone lift's offset shape: `y(1 − y)` on 0–1, continued past both ends
/// with the same slope and decaying to −1, so it is smooth and bounded for
/// any luma (exposure can push luma far above 1).
fn lift_bump(y: f32) -> f32 {
    if y > 1.0 {
        -(1.0 - libm::expf(1.0 - y))
    } else if y < 0.0 {
        -(1.0 - libm::expf(y))
    } else {
        y * (1.0 - y)
    }
}

pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A curve through (0, 0) and (1, 1) with finite end slopes, extended
/// linearly outside `[0, 1]`: `x(1+a)/(1+ax)` (lifts) or its mirror
/// `x/(1+a(1−x))` (lowers).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rational {
    pub(crate) a: f32,
    pub(crate) lift: bool,
}

impl Rational {
    pub(crate) fn apply(self, x: f32) -> f32 {
        let a = self.a;
        if self.lift {
            if x < 0.0 {
                x * (1.0 + a)
            } else if x > 1.0 {
                1.0 + (x - 1.0) / (1.0 + a)
            } else {
                x * (1.0 + a) / (1.0 + a * x)
            }
        } else if x < 0.0 {
            x / (1.0 + a)
        } else if x > 1.0 {
            1.0 + (x - 1.0) * (1.0 + a)
        } else {
            x / (1.0 + a * (1.0 - x))
        }
    }
}

/// Contrast's S-curve (CL-007): through the pivot with slope `k` there,
/// soft toe and shoulder, mirrored onto both sides of the pivot.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Contrast {
    pivot: f32,
    k: f32,
}

impl Contrast {
    /// One side's shape on `u` (1 at the pivot, 0 at the end of the range):
    /// `g(0) = 0`, `g(1) = 1`, `g'(1) = k`.
    fn side(self, u: f32) -> f32 {
        if self.k >= 1.0 {
            // A toe: slope 0 at the end, k at the pivot.
            let m = libm::powf(u.abs(), self.k);
            if u < 0.0 { -m } else { m }
        } else {
            // A lifted end: slope 1/k there, k at the pivot.
            Rational {
                a: 1.0 / self.k - 1.0,
                lift: true,
            }
            .apply(u)
        }
    }

    fn apply(self, x: f32) -> f32 {
        let p = self.pivot;
        if x < p {
            p * self.side(x / p)
        } else {
            1.0 - (1.0 - p) * self.side((1.0 - x) / (1.0 - p))
        }
    }
}

/// The stack's per-pixel stages with their constants worked out once: the
/// bake evaluates thousands of pixels with one of these.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluator {
    /// Stages 5 and 6: per-channel gains in linear light.
    gains: Option<[f32; 3]>,
    /// Stage 5 alone and stage 6 alone, for when skin protection blends
    /// white balance back on skin (CL-030).
    wb: Option<[f32; 3]>,
    exposure: Option<f32>,
    /// The Color scope's skin Amount, 0–1, while it is active (CL-030).
    skin_color: Option<f32>,
    /// Stage 7.
    ends: Option<(f32, f32)>,
    shadows: f32,
    highlights: f32,
    tone_lift: f32,
    brightness: Option<Rational>,
    contrast: Option<Contrast>,
    /// Stage 8, the colour wheels (CL-015).
    wheels: Option<WheelStage>,
    /// Stage 9, the curves (CL-014).
    curves: Option<CurvesStage>,
    /// Stage 11, HSL per hue band (CL-013).
    hsl: Option<HslStage>,
    /// Stage 12.
    hue: Option<(f32, f32)>,
    saturation: Option<f32>,
    vibrance: Option<f32>,
    /// Stage 13, the user LUT (CL-017).
    lut: Option<LutStage>,
    /// Stage 19: the share of the graded picture (1 shows it alone).
    amount: f32,
}

fn level(v: i16) -> f32 {
    f32::from(v) / 100.0
}

impl Evaluator {
    /// The evaluator of `stack` without a LUT table: a LUT slot grades
    /// nothing, as when its file is missing (CL-017).
    pub fn new(stack: &ColorStack) -> Evaluator {
        Evaluator::with_table(stack, None)
    }

    /// The evaluator of `stack` with its LUT looked up in `luts`.
    pub fn with_luts(stack: &ColorStack, luts: &dyn LutSource) -> Evaluator {
        let table = active_slot(stack).and_then(|slot| luts.lut(&slot.lut.hash));
        Evaluator::with_table(stack, table)
    }

    /// The evaluator of `stack` with `table` as its LUT slot's table.
    pub fn with_table(stack: &ColorStack, table: Option<Arc<Lut>>) -> Evaluator {
        let b = &stack.basic;
        let light = stack.sections_enabled.is_on(Section::Light);
        let color = stack.sections_enabled.is_on(Section::Color);
        let on = |section_on: bool, v: i16| if section_on { v } else { 0 };

        // Stage 5, white balance: warm raises red and lowers blue, positive
        // tint lowers green (toward magenta); the gains keep the luminance
        // of greys (Rec.709 weights in linear light).
        let (t, g) = (level(on(color, b.temperature)), level(on(color, b.tint)));
        let wb = (t != 0.0 || g != 0.0).then(|| {
            let r = libm::exp2f(WB_STOPS * t);
            let bl = libm::exp2f(-WB_STOPS * t);
            let gr = libm::exp2f(-WB_STOPS * g);
            let y = LUMA[0] * r + LUMA[1] * gr + LUMA[2] * bl;
            [r / y, gr / y, bl / y]
        });
        // Stage 6, exposure.
        let ev = level(on(light, b.exposure)) * EXPOSURE_EV;
        let exposure = (ev != 0.0).then(|| libm::exp2f(ev));
        let gains = match (wb, exposure) {
            (None, None) => None,
            (wb, e) => {
                let e = e.unwrap_or(1.0);
                let w = wb.unwrap_or([1.0; 3]);
                Some([w[0] * e, w[1] * e, w[2] * e])
            }
        };

        // Stage 7, tone.
        let (blacks, whites) = (level(on(light, b.blacks)), level(on(light, b.whites)));
        let ends = (blacks != 0.0 || whites != 0.0)
            .then_some((RANGE_ENDS * blacks, 1.0 + RANGE_ENDS * whites));
        let shadows = level(on(light, b.shadows)) * TONE_RANGE;
        let highlights = level(on(light, b.highlights)) * TONE_RANGE;
        // Tone lift (CL-007): positive brightens shadows and midtones and
        // flattens the highlights; negative does the reverse.
        let tone_lift = level(on(light, b.tone_lift)) * TONE_LIFT;
        let bright = level(on(light, b.brightness));
        let brightness = (bright != 0.0).then(|| Rational {
            a: if bright > 0.0 {
                BRIGHT_UP * bright
            } else {
                BRIGHT_DOWN * -bright
            },
            lift: bright > 0.0,
        });
        let c = level(on(light, b.contrast));
        let contrast = (c != 0.0).then(|| Contrast {
            pivot: PIVOT + PIVOT_RANGE * level(b.contrast_pivot),
            k: libm::exp2f(c),
        });

        // Stage 12: hue rotation, saturation, vibrance.
        let h = on(color, b.hue);
        let hue = (h != 0).then(|| {
            // ±100 is the same half turn either way (CL-008): normalized
            // into (−180°, 180°] so both give the same bits.
            let mut deg = f32::from(h) * HUE_DEGREES_PER_STEP;
            if deg <= -180.0 {
                deg += 360.0;
            }
            let (s, c) = libm::sincosf(deg.to_radians());
            (c, s)
        });
        let sat = on(color, b.saturation);
        let saturation = (sat != 0).then(|| 1.0 + level(sat));
        let vib = on(color, b.vibrance);
        let vibrance = (vib != 0).then(|| level(vib));

        let sections = stack.sections_enabled;
        let wheels = sections
            .is_on(Section::Wheels)
            .then(|| WheelStage::new(&stack.wheels))
            .flatten();
        let curves = sections
            .is_on(Section::Curves)
            .then(|| CurvesStage::new(&stack.curves))
            .flatten();
        let hsl = sections
            .is_on(Section::Hsl)
            .then(|| HslStage::new(&stack.hsl))
            .flatten();
        // Skin protection of the Color scope (CL-030) only matters while
        // something it blends back is active, and it belongs to the Color
        // section's switch.
        let color_skin = stack.skin(SkinScope::Color);
        let skin_color = (color
            && color_skin.is_active()
            && (wb.is_some()
                || hsl.is_some()
                || hue.is_some()
                || saturation.is_some()
                || vibrance.is_some()))
        .then(|| f32::from(color_skin.amount.min(100)) / 100.0);
        let lut = table.and_then(|t| LutStage::new(stack, t));

        Evaluator {
            gains,
            wb,
            exposure,
            skin_color,
            ends,
            shadows,
            highlights,
            tone_lift,
            brightness,
            contrast,
            wheels,
            curves,
            hsl,
            hue,
            saturation,
            vibrance,
            lut,
            amount: f32::from(stack.amount.min(100)) / 100.0,
        }
    }

    /// Whether a pixel's skin mask is needed (CL-030).
    pub(crate) fn needs_mask(&self) -> bool {
        self.skin_color.is_some() || self.lut.as_ref().is_some_and(|l| l.skin.is_some())
    }

    /// Whether stages 5–6 act on each channel alone, so the bake can work
    /// them out once per node value ([`Evaluator::balance`]): not while
    /// skin protection blends white balance back by a mask of the whole
    /// pixel.
    pub(crate) fn balance_is_per_channel(&self) -> bool {
        !(self.skin_color.is_some() && self.wb.is_some())
    }

    /// Whether the evaluator has a LUT stage.
    pub(crate) fn has_lut(&self) -> bool {
        self.lut.is_some()
    }

    /// The LUT stage (stage 13), when there is one.
    pub(crate) fn lut_stage(&self) -> Option<&LutStage> {
        self.lut.as_ref()
    }

    /// The size of the LUT's 3D table (0 without one).
    pub(crate) fn lut_cube_size(&self) -> u32 {
        self.lut.as_ref().map_or(0, LutStage::cube_size)
    }

    /// The stack's per-pixel stages on one pixel of code values.
    pub fn eval(&self, input: [f32; 3]) -> [f32; 3] {
        let (balanced, m) = self.balanced(input);
        self.eval_rest(input, balanced, m)
    }

    /// Stages 5–6 on `input`, and its skin mask (0 when nothing needs it).
    fn balanced(&self, input: [f32; 3]) -> ([f32; 3], f32) {
        if self.balance_is_per_channel() {
            let m = if self.needs_mask() {
                skin_mask(input)
            } else {
                0.0
            };
            return ([0, 1, 2].map(|i| self.balance(i, input[i])), m);
        }
        // Stage 5, then its skin blend (CL-030), then stage 6.
        let m = skin_mask(input);
        let t = m * self.skin_color.unwrap_or(0.0);
        let wb = self.wb.unwrap_or([1.0; 3]);
        let balanced = [0, 1, 2].map(|i| {
            let five = linear_to_ws(ws_to_linear(input[i]) * wb[i]);
            let blended = five + t * (input[i] - five);
            match self.exposure {
                Some(e) => linear_to_ws(ws_to_linear(blended) * e),
                None => blended,
            }
        });
        (balanced, m)
    }

    /// Stages 5–6 on channel `i` alone: white balance and exposure are
    /// per-channel gains, so the bake works them out once per node value.
    pub(crate) fn balance(&self, i: usize, v: f32) -> f32 {
        match self.gains {
            Some(g) => linear_to_ws(ws_to_linear(v) * g[i]),
            None => v,
        }
    }

    /// The rest of [`Evaluator::eval`] after [`Evaluator::balance`] (only
    /// while [`Evaluator::balance_is_per_channel`]).
    pub(crate) fn eval_balanced(&self, input: [f32; 3], balanced: [f32; 3]) -> [f32; 3] {
        let m = if self.needs_mask() {
            skin_mask(input)
        } else {
            0.0
        };
        self.eval_rest(input, balanced, m)
    }

    /// Stages 7–13 and the Amount blend, with `m` the pixel's skin mask.
    fn eval_rest(&self, input: [f32; 3], balanced: [f32; 3], m: f32) -> [f32; 3] {
        let before = self.before_hsl(balanced);
        let c = match &self.hsl {
            Some(stage) => stage.apply(before),
            None => before,
        };
        let mut c = self.color(c);
        if let Some(a) = self.skin_color {
            // Stages 11–12 back toward their input on skin (CL-030).
            let t = m * a;
            if t > 0.0 {
                c = [0, 1, 2].map(|i| c[i] + t * (before[i] - c[i]));
            }
        }
        if let Some(lut) = &self.lut {
            c = lut.apply(c, m);
        }
        amount_blend(input, c, self.amount)
    }

    /// Stages 7–9 after the balance: what the HSL stage receives.
    fn before_hsl(&self, balanced: [f32; 3]) -> [f32; 3] {
        let c = self.tone(balanced);
        let c = match &self.wheels {
            Some(stage) => stage.apply(c),
            None => c,
        };
        match &self.curves {
            Some(stage) => stage.apply(c),
            None => c,
        }
    }

    /// The pixel as it enters the HSL stage (stage 11): what CL-013's
    /// eyedropper samples.
    pub fn hsl_input(&self, input: [f32; 3]) -> [f32; 3] {
        self.before_hsl(self.balanced(input).0)
    }

    /// The steepest slope of the active curve tables (0 without curves):
    /// what decides the lattice size.
    pub(crate) fn curve_slope(&self) -> f32 {
        self.curves.as_ref().map_or(0.0, CurvesStage::max_slope)
    }

    fn tone(&self, mut c: [f32; 3]) -> [f32; 3] {
        if let Some((b, w)) = self.ends {
            c = c.map(|x| b + x * (w - b));
        }
        if self.shadows != 0.0 || self.highlights != 0.0 || self.tone_lift != 0.0 {
            // Offsets by the pixel's luma, added to every channel so colour
            // differences stay. The two range bumps are flat at both ends,
            // so the clamp of `y` adds no kink; the lift's bump continues
            // smoothly past them.
            let y = luma(c).clamp(0.0, 1.0);
            let inv = 1.0 - y;
            let (y2, inv2) = (y * y, inv * inv);
            let off = self.shadows * y2 * inv2 * inv2
                + self.highlights * y2 * y2 * inv2
                + self.tone_lift * lift_bump(luma(c));
            c = c.map(|x| x + off);
        }
        if let Some(curve) = self.brightness {
            c = c.map(|x| curve.apply(x));
        }
        if let Some(curve) = self.contrast {
            c = c.map(|x| curve.apply(x));
        }
        c
    }

    fn color(&self, mut c: [f32; 3]) -> [f32; 3] {
        if let Some((cos, sin)) = self.hue {
            c = rotate_hue(c, cos, sin);
        }
        if let Some(s) = self.saturation {
            c = mix_from_luma(c, s);
        }
        if let Some(v) = self.vibrance {
            let max = c[0].max(c[1]).max(c[2]);
            let min = c[0].min(c[1]).min(c[2]);
            let sat = (max - min).clamp(0.0, 1.0);
            let s = (1.0 + v * (1.0 - sat) * (1.0 - SKIN_RELIEF * skin_band(c))).max(0.0);
            c = mix_from_luma(c, s);
        }
        c
    }
}

/// [`Evaluator::eval`] for one pixel: the reference the lattice is checked
/// against (render-compositor.md §17). A LUT slot grades nothing here; see
/// [`eval_pixel_with`].
pub fn eval_pixel(stack: &ColorStack, rgb: [f32; 3]) -> [f32; 3] {
    Evaluator::new(stack).eval(rgb)
}

/// [`eval_pixel`] with the stack's LUT looked up in `luts`.
pub fn eval_pixel_with(stack: &ColorStack, luts: &dyn LutSource, rgb: [f32; 3]) -> [f32; 3] {
    Evaluator::with_luts(stack, luts).eval(rgb)
}

/// `y + s·(c − y)` with `y` the pixel's luma: 0 is grey (R = G = B
/// exactly), 2 doubles the distance from grey (CL-008).
pub(crate) fn mix_from_luma(c: [f32; 3], s: f32) -> [f32; 3] {
    let y = luma(c);
    c.map(|x| y + s * (x - y))
}

/// How much `c` sits in the skin-tone band (CL-008's 15°–50°, as a hue
/// angle of code values with red largest), 0 to 1. Continuous everywhere.
fn skin_band(c: [f32; 3]) -> f32 {
    let [r, g, b] = c;
    if r <= g || r <= b {
        return 0.0;
    }
    let min = g.min(b);
    let span = r - min;
    if span <= 0.0 || g < b {
        return 0.0;
    }
    // Hue in degrees, 0 (red) to 60 (yellow) in this sextant.
    let h = 60.0 * (g - b) / span;
    smoothstep(5.0, 18.0, h) * (1.0 - smoothstep(45.0, 58.0, h))
}

/// Hue rotation (CL-008): the chroma plane of Rec.709 luma and colour
/// differences (`Cb = (B − Y)/1.8556`, `Cr = (R − Y)/1.5748`, code values)
/// turns about the grey axis, so luma stays exactly where it was and greys
/// never move.
///
/// *FreeCut decision (K1, measured):* CL-001 planned this turn in Oklab of
/// γ 2.4-linear light. Measured against a 33³ lattice that is up to 19/255
/// off at Hue ±100: a turn pushes saturated colours out of gamut, and both
/// re-encoding a linear value that crosses 0 and Oklab's cube root near the
/// primaries are too steep to interpolate. The luma/colour-difference turn
/// is linear in code values, so the lattice reproduces it exactly, and it
/// keeps CL-008's promises (lightness as luma stays, ±100 agree, greys
/// stay).
pub(crate) fn rotate_hue(c: [f32; 3], cos: f32, sin: f32) -> [f32; 3] {
    let y = luma(c);
    let cb = (c[2] - y) / KB;
    let cr = (c[0] - y) / KR;
    let (cb, cr) = (cb * cos - cr * sin, cb * sin + cr * cos);
    let r = y + KR * cr;
    let b = y + KB * cb;
    let g = (y - LUMA[0] * r - LUMA[2] * b) / LUMA[1];
    [r, g, b]
}

/// Stage 19 (CL-035): `base·(1 − t) + graded·t`; at `t = 1` exactly the
/// graded value.
fn amount_blend(base: [f32; 3], graded: [f32; 3], t: f32) -> [f32; 3] {
    if t >= 1.0 {
        return graded;
    }
    [
        base[0] * (1.0 - t) + graded[0] * t,
        base[1] * (1.0 - t) + graded[1] * t,
        base[2] * (1.0 - t) + graded[2] * t,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecut_timeline::model::ColorParamId;

    #[test]
    fn the_identity_stack_returns_its_input_bit_for_bit() {
        let e = Evaluator::new(&ColorStack::default());
        for v in [0.0, 1e-6, 0.25, 0.5, 0.999, 1.0] {
            assert_eq!(e.eval([v, 1.0 - v, v / 2.0]), [v, 1.0 - v, v / 2.0]);
        }
    }

    #[test]
    fn linear_conversions_mirror_the_sign_and_invert() {
        for v in [-0.7f32, -0.01, 0.0, 0.2, 0.5, 1.0, 2.0] {
            let back = linear_to_ws(ws_to_linear(v));
            assert!((back - v).abs() < 1e-5, "{v} → {back}");
            assert_eq!(ws_to_linear(-v), -ws_to_linear(v));
        }
        // A half turn keeps luma.
        let turned = rotate_hue([0.8, 0.3, 0.1], -1.0, 0.0);
        assert!((luma(turned) - luma([0.8, 0.3, 0.1])).abs() < 1e-6);
    }

    #[test]
    fn curves_pass_through_their_ends_and_pivot() {
        for lift in [true, false] {
            let c = Rational { a: 1.5, lift };
            assert_eq!(c.apply(0.0), 0.0);
            assert!((c.apply(1.0) - 1.0).abs() < 1e-6);
        }
        for k in [0.5, 0.8, 1.0, 1.5, 2.0] {
            let c = Contrast { pivot: 0.41, k };
            assert!((c.apply(0.41) - 0.41).abs() < 1e-6);
            assert!(c.apply(0.0).abs() < 1e-6);
            assert!((c.apply(1.0) - 1.0).abs() < 1e-6);
        }
        let s = ColorStack::default().with(ColorParamId::Brightness, 100);
        let up = eval_pixel(&s, [0.5; 3])[0];
        assert!((up - 0.707).abs() < 0.01, "{up}");
    }
}
