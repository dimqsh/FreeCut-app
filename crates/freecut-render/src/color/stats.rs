//! Colour statistics of sampled frames (CL-019 auto color, CL-020 colour
//! match, measured in Rec.709 `Y'CbCr` of code values): pure functions over [`Picture`]s, no model state beyond the
//! values they return.
//!
//! **Determinism.** Every sum is `f64`, taken in row-major pixel order and
//! then frame by frame in the order given; histograms are integer counts.
//! The transcendental functions come from `libm`. So the same frames give
//! the same result on every run, thread count and OS.
//!
//! Only pixels with alpha above one half are measured. Analysis pictures
//! are at most [`ANALYSIS_WIDTH`] pixels wide ([`analysis_picture`]).

use freecut_timeline::model::{AutoOffsets, AutoStyle, ColorStack, MATCH_ONE};

use super::matching::MatchStage;
use super::quantize;
use crate::cpu::Picture;

/// The widest picture the analysis looks at (CL-019, CL-020).
pub const ANALYSIS_WIDTH: u32 = 512;

/// Why an analysis has no result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ColorStatsError {
    /// No frame had an opaque pixel.
    #[error("the frames have no opaque pixels to measure")]
    NoPixels,
    /// The frames are nearly uniform (a black screen, a solid colour, a
    /// title card): CL-019's "Nothing to correct".
    #[error("nothing to correct")]
    NothingToCorrect,
    /// The target or reference is black or uniform: CL-020's "Not enough
    /// detail to match".
    #[error("not enough detail to match")]
    NotEnoughDetail,
}

/// Whether a pixel counts: alpha above one half.
fn opaque(px: &[u8]) -> bool {
    px[3] > 127
}

/// `v / 255` in linear light (γ 2.4), for each 8-bit value.
fn linear_table() -> [f64; 256] {
    let mut out = [0.0; 256];
    for (i, v) in out.iter_mut().enumerate() {
        *v = libm::pow(i as f64 / 255.0, 2.4);
    }
    out
}

fn to_linear(v: f64) -> f64 {
    if v <= 0.0 { 0.0 } else { libm::pow(v, 2.4) }
}

fn to_code(l: f64) -> f64 {
    if l <= 0.0 {
        0.0
    } else {
        libm::pow(l, 1.0 / 2.4)
    }
}

/// Rec.709 luma of 8-bit code values, rounded to an 8-bit bin, in integer
/// arithmetic.
fn luma_bin(r: u8, g: u8, b: u8) -> usize {
    ((2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b) + 5000) / 10_000) as usize
}

/// `picture` box-filtered down by a whole factor until it is at most
/// [`ANALYSIS_WIDTH`] wide (a copy when it already is). Colours are
/// averaged weighted by alpha, in integers, so the result is exact.
pub fn analysis_picture(picture: &Picture) -> Picture {
    if !picture.is_valid() || picture.width <= ANALYSIS_WIDTH {
        return picture.clone();
    }
    let k = picture.width.div_ceil(ANALYSIS_WIDTH);
    let (w, h) = (picture.width / k, (picture.height / k).max(1));
    let src_w = picture.width as usize;
    let mut rgba = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut sum = [0u64; 3];
            let mut alpha = 0u64;
            let mut n = 0u64;
            for sy in y * k as usize..((y + 1) * k as usize).min(picture.height as usize) {
                for sx in x * k as usize..(x + 1) * k as usize {
                    let i = (sy * src_w + sx) * 4;
                    let p = &picture.rgba[i..i + 4];
                    let a = u64::from(p[3]);
                    for c in 0..3 {
                        sum[c] += u64::from(p[c]) * a;
                    }
                    alpha += a;
                    n += 1;
                }
            }
            let o = (y * w as usize + x) * 4;
            for (c, total) in sum.iter().enumerate() {
                // A weighted mean of 8-bit values fits a u8.
                rgba[o + c] = (total + alpha / 2).checked_div(alpha).unwrap_or(0) as u8;
            }
            // A mean of 8-bit values fits a u8.
            rgba[o + 3] = ((alpha + n / 2) / n.max(1)) as u8;
        }
    }
    Picture {
        width: w,
        height: h,
        rotation: picture.rotation,
        mirrored: picture.mirrored,
        rgba,
    }
}

/// What a picture enters the stack's sliders as (CL-019's "stack input"):
/// the picture after the cleanup stages and stage 4, the colour match.
/// No cleanup stage exists yet, so this is the match alone; a stack
/// without an active match returns the picture as it is. Pixels are
/// evaluated one by one in `f32` and quantized once, like a grade.
pub fn stack_input(stack: &ColorStack, picture: &Picture) -> Picture {
    let mut out = picture.clone();
    let Some(stage) = MatchStage::new(stack).filter(|_| !stack.bypass && stack.amount > 0) else {
        return out;
    };
    if !picture.is_valid() {
        return out;
    }
    for px in out.rgba.as_chunks_mut::<4>().0 {
        if px[3] == 0 {
            continue;
        }
        let v = stage.apply([px[0], px[1], px[2]].map(|c| f32::from(c) / 255.0));
        px[0] = quantize(v[0]);
        px[1] = quantize(v[1]);
        px[2] = quantize(v[2]);
    }
    out
}

/// Linear-light channel sums over some pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct ChannelSums {
    sum: [f64; 3],
    count: u64,
}

impl ChannelSums {
    fn add(&mut self, rgb: [f64; 3]) {
        for (sum, v) in self.sum.iter_mut().zip(rgb) {
            *sum += v;
        }
        self.count += 1;
    }

    fn merge(&mut self, other: &ChannelSums) {
        for c in 0..3 {
            self.sum[c] += other.sum[c];
        }
        self.count += other.count;
    }
}

/// What auto color measures of one or more frames (CL-019): per-channel
/// and luma histograms of the opaque pixels, and the linear-light sums of
/// the pixels that are neither clipped nor too dark (the gray-world
/// average), with those of the near-grey ones among them apart.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameStats {
    /// Opaque pixels.
    pub count: u64,
    /// Red, green, blue and luma histograms of 8-bit code values.
    hist: Box<[[u64; 256]; 4]>,
    /// Pixels with no channel at 0.98 or above and luma at least 0.05.
    usable: ChannelSums,
    /// The usable pixels that are nearly grey: the spread of their
    /// channels is at most [`NEAR_GREY`] of the largest.
    greyish: ChannelSums,
}

impl Default for FrameStats {
    fn default() -> Self {
        FrameStats {
            count: 0,
            hist: Box::new([[0; 256]; 4]),
            usable: ChannelSums::default(),
            greyish: ChannelSums::default(),
        }
    }
}

/// How far from grey a pixel may be (channel spread over the largest
/// channel, code values) and still count as a grey for white balance. The
/// strongest cast Temperature and Tint remove (a linear gain of 2^0.6)
/// spreads a grey by about 0.16; skin (about 0.35) stays out.
const NEAR_GREY: f64 = 0.2;
/// At least one usable pixel in this many must be nearly grey before white
/// balance trusts the greys alone; with fewer, every usable pixel counts
/// (plain gray world).
const GREYISH_ONE_IN: u64 = 20;

impl FrameStats {
    /// These statistics and `other`'s together.
    pub fn merge(&mut self, other: &FrameStats) {
        self.count += other.count;
        for (a, b) in self.hist.iter_mut().zip(other.hist.iter()) {
            for (x, y) in a.iter_mut().zip(b.iter()) {
                *x += y;
            }
        }
        self.usable.merge(&other.usable);
        self.greyish.merge(&other.greyish);
    }

    /// The value (0–1) below which a share `q` of the pixels of channel
    /// `c` (0–2, 3 for luma) lie.
    fn percentile(&self, c: usize, q: f64) -> f64 {
        let want = (q * self.count as f64).ceil().max(1.0);
        let mut seen = 0u64;
        for (bin, n) in self.hist[c].iter().enumerate() {
            seen += n;
            if seen as f64 >= want {
                return bin as f64 / 255.0;
            }
        }
        1.0
    }
}

/// The statistics of `picture`'s opaque pixels (CL-019).
pub fn frame_stats(picture: &Picture) -> FrameStats {
    let mut out = FrameStats::default();
    if !picture.is_valid() {
        return out;
    }
    let lin = linear_table();
    let n = picture.width as usize * picture.height as usize * 4;
    for px in picture.rgba[..n].as_chunks::<4>().0 {
        if !opaque(px) {
            continue;
        }
        let [r, g, b, _] = *px;
        let y = luma_bin(r, g, b);
        out.count += 1;
        out.hist[0][usize::from(r)] += 1;
        out.hist[1][usize::from(g)] += 1;
        out.hist[2][usize::from(b)] += 1;
        out.hist[3][y] += 1;
        // 0.98 × 255 = 249.9 and 0.05 × 255 = 12.75.
        if r < 250 && g < 250 && b < 250 && y >= 13 {
            let rgb = [r, g, b].map(|v| lin[usize::from(v)]);
            out.usable.add(rgb);
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            if f64::from(max - min) <= NEAR_GREY * f64::from(max) {
                out.greyish.add(rgb);
            }
        }
    }
    out
}

/// White balance's log2 channel gain at ±100 (the evaluator's `WB_STOPS`).
const WB_STOPS: f64 = 0.3;
/// Exposure ±100 is ±3 EV.
const EXPOSURE_EV: f64 = 3.0;
/// How far Blacks and Whites move their end at ±100.
const RANGE_ENDS: f64 = 0.2;
/// Where the black and white points go (CL-019), and how near is near
/// enough to leave a point alone.
const BLACK_POINT: f64 = 0.02;
const WHITE_POINT: f64 = 0.98;
const POINT_TOLERANCE: f64 = 0.02;
/// The percentiles taken as the black and white points.
const LOW: f64 = 0.005;
const HIGH: f64 = 0.995;
/// A spread (white point minus black point) below this, in luma and every
/// channel, is a uniform frame (about 8 of 255 steps).
const UNIFORM_SPREAD: f64 = 8.0 / 255.0;
/// The luma spread auto contrast aims for, and the band it leaves alone.
const TARGET_STD: f64 = 0.18;
const FLAT_STD: f64 = 0.12;
const PUNCHY_STD: f64 = 0.28;
/// Auto's Contrast never goes past ±30.
const CONTRAST_LIMIT: f64 = 30.0;
/// The largest exposure change, in EV.
const EXPOSURE_LIMIT: f64 = 1.5;
/// Vivid's extra Vibrance and Saturation (FreeCut defaults, CL-019).
const VIVID_VIBRANCE: i16 = 20;
const VIVID_SATURATION: i16 = 5;

fn level(v: f64) -> i16 {
    // Rounded and limited to ±100, so the cast is exact.
    libm::round(v).clamp(-100.0, 100.0) as i16
}

/// The auto correction of `frames` in `style` (CL-019): Temperature and
/// Tint that bring the gray-world average to neutral, Exposure, Blacks
/// and Whites that put the black and white points (0.5 % and 99.5 % of
/// luma) near 0.02 and 0.98, and Contrast that brings a flat or harsh
/// luma spread toward the middle; Vivid adds Vibrance and Saturation.
/// Every offset is limited to its slider's ±100.
pub fn auto_color(frames: &[FrameStats], style: AutoStyle) -> Result<AutoOffsets, ColorStatsError> {
    let mut all = FrameStats::default();
    for f in frames {
        all.merge(f);
    }
    if all.count == 0 {
        return Err(ColorStatsError::NoPixels);
    }
    let lo = all.percentile(3, LOW);
    let hi = all.percentile(3, HIGH);
    let uniform = (0..4).all(|c| all.percentile(c, HIGH) - all.percentile(c, LOW) < UNIFORM_SPREAD);
    if uniform || hi <= 0.0 {
        return Err(ColorStatsError::NothingToCorrect);
    }

    // White balance: the gains that make the average of the near-grey
    // pixels (or, with too few of them, of every usable pixel) neutral.
    let (mut temperature, mut tint) = (0, 0);
    let greys = if all.greyish.count * GREYISH_ONE_IN >= all.usable.count {
        all.greyish
    } else {
        all.usable
    };
    if greys.count * 100 >= all.count && greys.sum.iter().all(|s| *s > 0.0) {
        let [r, g, b] = greys.sum;
        let t = libm::log2(b / r) / (2.0 * WB_STOPS);
        temperature = level(100.0 * t);
        let t_used = f64::from(temperature) / 100.0;
        tint = level(100.0 * (libm::log2(g / r) - WB_STOPS * t_used) / WB_STOPS);
    }

    // Exposure: lift a dim white point to 0.98, or bring down a clipped,
    // bright picture's middle toward 0.5.
    let median = all.percentile(3, 0.5);
    let ev = if hi < WHITE_POINT - POINT_TOLERANCE {
        libm::log2(to_linear(WHITE_POINT) / to_linear(hi)).clamp(0.0, EXPOSURE_LIMIT)
    } else if hi >= WHITE_POINT && median > 0.62 {
        libm::log2(to_linear(0.5) / to_linear(median)).clamp(-EXPOSURE_LIMIT, 0.0)
    } else {
        0.0
    };
    let exposure = level(100.0 * ev / EXPOSURE_EV);
    let gain = libm::exp2(f64::from(exposure) / 100.0 * EXPOSURE_EV);
    let exposed = |v: f64| to_code(to_linear(v) * gain);

    // Blacks and Whites: the ends of the range, after the exposure.
    let (lo_e, hi_e) = (exposed(lo), exposed(hi));
    // A point that is already near its target, or past it, stays: the
    // range is only ever stretched, never squeezed.
    let lo_t = if lo_e <= BLACK_POINT + POINT_TOLERANCE {
        lo_e
    } else {
        BLACK_POINT
    };
    let hi_t = if hi_e >= WHITE_POINT - POINT_TOLERANCE {
        hi_e
    } else {
        WHITE_POINT
    };
    let (mut blacks, mut whites) = (0, 0);
    if hi_e - lo_e > 1e-6 {
        let s = (hi_t - lo_t) / (hi_e - lo_e);
        let b0 = lo_t - lo_e * s;
        let w0 = b0 + s;
        blacks = level(100.0 * b0 / RANGE_ENDS);
        whites = level(100.0 * (w0 - 1.0) / RANGE_ENDS);
    }

    // Contrast: the luma spread after exposure and the ends, from the
    // histogram's bins in order.
    let (b_used, w_used) = (
        RANGE_ENDS * f64::from(blacks) / 100.0,
        1.0 + RANGE_ENDS * f64::from(whites) / 100.0,
    );
    let (mut sum, mut sum_sq) = (0.0, 0.0);
    for (bin, n) in all.hist[3].iter().enumerate() {
        if *n == 0 {
            continue;
        }
        let v = b_used + exposed(bin as f64 / 255.0) * (w_used - b_used);
        sum += v * *n as f64;
        sum_sq += v * v * *n as f64;
    }
    let mean = sum / all.count as f64;
    let std = (sum_sq / all.count as f64 - mean * mean).max(0.0).sqrt();
    let contrast = if std < FLAT_STD && std > 0.0 {
        level((50.0 * libm::log2(TARGET_STD / std)).min(CONTRAST_LIMIT))
    } else if std > PUNCHY_STD {
        level((-50.0 * libm::log2(std / TARGET_STD)).max(-CONTRAST_LIMIT))
    } else {
        0
    };

    let (vibrance, saturation) = match style {
        AutoStyle::Natural => (0, 0),
        AutoStyle::Vivid => (VIVID_VIBRANCE, VIVID_SATURATION),
    };
    Ok(AutoOffsets {
        exposure,
        contrast,
        whites,
        blacks,
        temperature,
        tint,
        vibrance,
        saturation,
    }
    .clamped())
}

/// The statistics a match compares (CL-020): sums and sums of squares of
/// Rec.709 `Y'`, `Cb` and `Cr` of code values (the space of stage 4, see
/// `matching`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct YccStats {
    pub count: u64,
    sum: [f64; 3],
    sum_sq: [f64; 3],
}

impl YccStats {
    /// These statistics and `other`'s together.
    pub fn merge(&mut self, other: &YccStats) {
        self.count += other.count;
        for c in 0..3 {
            self.sum[c] += other.sum[c];
            self.sum_sq[c] += other.sum_sq[c];
        }
    }

    /// The mean of `Y'`, `Cb` and `Cr` (0 without pixels).
    pub fn mean(&self) -> [f64; 3] {
        let n = self.count.max(1) as f64;
        self.sum.map(|s| s / n)
    }

    /// The standard deviation of `Y'`, `Cb` and `Cr`.
    pub fn std(&self) -> [f64; 3] {
        let n = self.count.max(1) as f64;
        let mean = self.mean();
        [0, 1, 2].map(|c| (self.sum_sq[c] / n - mean[c] * mean[c]).max(0.0).sqrt())
    }
}

/// `Y'`, `Cb` and `Cr` of 8-bit code values, in `f64` (the evaluator's
/// constants).
fn ycc64(px: [u8; 3]) -> [f64; 3] {
    let [r, g, b] = px.map(|v| f64::from(v) / 255.0);
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    [y, (b - y) / 1.8556, (r - y) / 1.5748]
}

/// The match statistics of `picture`'s opaque pixels (CL-020), in `f64`.
pub fn ycc_stats(picture: &Picture) -> YccStats {
    let mut out = YccStats::default();
    if !picture.is_valid() {
        return out;
    }
    let n = picture.width as usize * picture.height as usize * 4;
    for px in picture.rgba[..n].as_chunks::<4>().0 {
        if !opaque(px) {
            continue;
        }
        let v = ycc64([px[0], px[1], px[2]]);
        out.count += 1;
        for (c, v) in v.into_iter().enumerate() {
            out.sum[c] += v;
            out.sum_sq[c] += v * v;
        }
    }
    out
}

/// A match's gains and offsets per `Y'CbCr` channel (CL-020), before they
/// are rounded into a `MatchTransform`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchGains {
    pub gain: [f64; 3],
    pub offset: [f64; 3],
    /// Whether a limit held a gain or a shift back (CL-020's notice).
    pub limited: bool,
}

impl MatchGains {
    /// The gains and offsets ×10⁴, rounded half away from zero (the
    /// stored fixed point of `MatchTransform`).
    pub fn to_fixed(&self) -> ([i32; 3], [i32; 3]) {
        // Gains are within 0.5–2 and offsets within ±1.5, so the casts are
        // exact.
        let fix = |v: f64| libm::round(v * f64::from(MATCH_ONE)) as i32;
        (self.gain.map(fix), self.offset.map(fix))
    }
}

/// The smallest `Y'` spread a frame needs to be matched.
const MIN_DETAIL: f64 = 0.01;
/// CL-020's limits: gains 0.5–2×, and the colour-difference means move by
/// at most 0.1.
const GAIN_MIN: f64 = 0.5;
const GAIN_MAX: f64 = 2.0;
const AB_SHIFT: f64 = 0.1;

/// The transform that moves the target statistics (several frames,
/// pooled) onto the reference's (CL-020, Reinhard-style transfer):
/// per channel `gain = σ_ref / σ_target` and `offset` so the target mean
/// lands on the reference mean, `out = gain · in + offset`. Gains are
/// limited to 0.5–2× (all three channels) and the `Cb`/`Cr` mean shift to
/// ±0.1.
pub fn match_stats(
    target: &[YccStats],
    reference: &YccStats,
) -> Result<MatchGains, ColorStatsError> {
    let mut t = YccStats::default();
    for s in target {
        t.merge(s);
    }
    if t.count == 0 || reference.count == 0 {
        return Err(ColorStatsError::NoPixels);
    }
    let (mt, st) = (t.mean(), t.std());
    let (mr, sr) = (reference.mean(), reference.std());
    if st[0] < MIN_DETAIL || sr[0] < MIN_DETAIL {
        return Err(ColorStatsError::NotEnoughDetail);
    }
    let mut limited = false;
    let mut gain = [1.0; 3];
    let mut offset = [0.0; 3];
    for c in 0..3 {
        // Equal statistics give exactly 1 (x / x), so a self-match is the
        // identity.
        let raw = sr[c] / st[c].max(1e-6);
        let g = raw.clamp(GAIN_MIN, GAIN_MAX);
        limited |= g != raw;
        let shift_raw = mr[c] - mt[c];
        let shift = if c == 0 {
            shift_raw
        } else {
            shift_raw.clamp(-AB_SHIFT, AB_SHIFT)
        };
        limited |= shift != shift_raw;
        gain[c] = g;
        offset[c] = mt[c] + shift - g * mt[c];
    }
    Ok(MatchGains {
        gain,
        offset,
        limited,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(rgb: [u8; 3]) -> Picture {
        Picture::new(8, 8, [rgb[0], rgb[1], rgb[2], 255].repeat(64))
    }

    #[test]
    fn luma_bins_round_and_percentiles_walk_the_histogram() {
        assert_eq!(luma_bin(255, 255, 255), 255);
        assert_eq!(luma_bin(0, 0, 0), 0);
        let s = frame_stats(&solid([10, 200, 30]));
        assert_eq!(s.count, 64);
        assert_eq!(s.percentile(1, 0.5), 200.0 / 255.0);
    }

    #[test]
    fn analysis_pictures_are_at_most_512_wide() {
        let p = Picture::new(1030, 4, vec![100; 1030 * 4 * 4]);
        let a = analysis_picture(&p);
        assert_eq!((a.width, a.height), (343, 1));
        assert!(a.rgba.iter().all(|v| *v == 100));
        let small = solid([1, 2, 3]);
        assert_eq!(analysis_picture(&small), small);
    }
}
