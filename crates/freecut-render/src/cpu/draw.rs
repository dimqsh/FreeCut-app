//! Per-pixel work: inverse mapping (affine, or projective for layers
//! turned in 3D), edge coverage, bilinear sampling, the whole-layer wipe
//! and tint, and "over" blending, one row at a time, without allocating.

use std::sync::Arc;

use freecut_timeline::glam::{DMat3, DVec2};

use super::Picture;
use crate::text::projective::{Homography, MIN_W};

/// Whether `m` has no perspective: its bottom row is exactly `[0 0 1]`.
pub(super) fn is_affine(m: DMat3) -> bool {
    m.x_axis.z == 0.0 && m.y_axis.z == 0.0 && m.z_axis.z == 1.0
}

/// `m` as a row-major [`Homography`].
pub(super) fn homography(m: DMat3) -> Homography {
    Homography([
        m.x_axis.x, m.y_axis.x, m.z_axis.x, m.x_axis.y, m.y_axis.y, m.z_axis.y, m.x_axis.z,
        m.y_axis.z, m.z_axis.z,
    ])
}

/// A row-major [`Homography`] as a column-major matrix.
pub(super) fn matrix_of(h: &Homography) -> DMat3 {
    let m = &h.0;
    DMat3::from_cols_array(&[m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]])
}

/// The image of `p` under `m` (dividing by the third coordinate); `None`
/// at or behind the eye ([`MIN_W`]).
pub(super) fn project(m: DMat3, p: DVec2) -> Option<DVec2> {
    if is_affine(m) {
        let q = m.transform_point2(p);
        return q.is_finite().then_some(q);
    }
    homography(m)
        .apply((p.x, p.y))
        .map(|(x, y)| DVec2::new(x, y))
}

/// How much `m` magnifies around `p`: the longer of the two columns of
/// its Jacobian there (for an affine map, the longer axis). `None` at or
/// behind the eye.
pub(super) fn local_scale(m: DMat3, p: DVec2) -> Option<f64> {
    let (a, b, c) = (m.x_axis.x, m.y_axis.x, m.z_axis.x);
    let (d, e, f) = (m.x_axis.y, m.y_axis.y, m.z_axis.y);
    let (g, h, i) = (m.x_axis.z, m.y_axis.z, m.z_axis.z);
    let w = g * p.x + h * p.y + i;
    if w.is_nan() || w <= MIN_W {
        return None;
    }
    let x = (a * p.x + b * p.y + c) / w;
    let y = (d * p.x + e * p.y + f) / w;
    let col_x = ((a - x * g) / w).hypot((d - y * g) / w);
    let col_y = ((b - x * h) / w).hypot((e - y * h) / w);
    let s = col_x.max(col_y);
    s.is_finite().then_some(s)
}

/// A map from output pixel coordinates to source coordinates: affine, or
/// projective for a layer turned in 3D (FX-030), whose source point is
/// `(X / W, Y / W)` with `W` positive in front of the eye.
#[derive(Debug, Clone, Copy)]
pub(super) struct ToSource {
    m: DMat3,
    projective: bool,
}

impl ToSource {
    /// The inverse of the layer → output map `m`, when it is invertible.
    /// A projective `m` is inverted as a [`Homography`], so preimages in
    /// front of the eye get a positive `W`.
    pub fn inverse_of(m: DMat3) -> Option<ToSource> {
        if is_affine(m) {
            let det = m.determinant();
            return (det.is_finite() && det.abs() > 1e-12).then(|| ToSource {
                m: m.inverse(),
                projective: false,
            });
        }
        let inverse = homography(m).inverse()?;
        Some(ToSource {
            m: matrix_of(&inverse),
            projective: true,
        })
    }

    /// The map as a matrix (its bottom row is meaningful only when
    /// [`ToSource::is_projective`]).
    pub fn matrix(self) -> DMat3 {
        self.m
    }

    pub fn is_projective(self) -> bool {
        self.projective
    }

    /// Applies the affine `next` after this map.
    pub fn then(self, next: DMat3) -> ToSource {
        ToSource {
            m: next * self.m,
            ..self
        }
    }
}

pub(super) enum PreparedSource {
    /// Straight RGB and an alpha that already includes the layer opacity.
    Fill([u8; 3], u32),
    Picture(Arc<Picture>),
    /// A premultiplied picture (text layers, C46), drawn with premultiplied
    /// "over": colour without alpha adds light.
    Premultiplied(Arc<Picture>),
}

/// A whole-layer wipe (FX-030's Wipe reveal) in output pixels: the share
/// `u` (0 at the side the wipe starts from, 1 at the far side of the
/// layer rectangle, clamped to that range) is `num · p / den · p` for the
/// output point `p = (x, y, 1)`, and the layer shows where `u` is before
/// the edge, fading over the feather.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct WipeMask {
    num: [f64; 3],
    den: [f64; 3],
    /// The edge's position in `u`.
    edge: f64,
    /// 1 / feather (in `u`); infinite for a hard edge.
    inv_feather: f64,
}

impl WipeMask {
    /// The mask of a wipe showing `shown` (0..=1) of the layer rectangle
    /// `rect` (`(x0, y0, x1, y1)` in layer coordinates) from `from`'s side,
    /// with a soft edge `feather` (a share of the rectangle's size along
    /// the wipe) wide; `to_layer` maps output pixels to layer coordinates.
    /// The edge enters at `shown = 0` with the whole feather outside the
    /// layer and leaves at `shown = 1` with all of it past the far side,
    /// so both ends are exact. `None` when nothing is hidden.
    pub fn new(
        to_layer: ToSource,
        rect: (f64, f64, f64, f64),
        shown: f64,
        from: WipeFrom,
        feather: f64,
    ) -> Option<WipeMask> {
        let shown = if shown.is_nan() { 1.0 } else { shown };
        if shown >= 1.0 {
            return None;
        }
        let shown = shown.max(0.0);
        let feather = if feather.is_finite() {
            feather.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (x0, y0, x1, y1) = rect;
        let (w, h) = (x1 - x0, y1 - y0);
        // u = α·x + β·y + γ in layer coordinates.
        let (alpha, beta, gamma) = match from {
            WipeFrom::Left => (1.0 / w, 0.0, -x0 / w),
            WipeFrom::Right => (-1.0 / w, 0.0, x1 / w),
            WipeFrom::Top => (0.0, 1.0 / h, -y0 / h),
            WipeFrom::Bottom => (0.0, -1.0 / h, y1 / h),
        };
        if !(alpha.is_finite() && beta.is_finite() && gamma.is_finite()) {
            return None;
        }
        let m = to_layer.matrix();
        let rows = [
            [m.x_axis.x, m.y_axis.x, m.z_axis.x],
            [m.x_axis.y, m.y_axis.y, m.z_axis.y],
            if to_layer.is_projective() {
                [m.x_axis.z, m.y_axis.z, m.z_axis.z]
            } else {
                [0.0, 0.0, 1.0]
            },
        ];
        let num = [0, 1, 2].map(|k| alpha * rows[0][k] + beta * rows[1][k] + gamma * rows[2][k]);
        Some(WipeMask {
            num,
            den: rows[2],
            edge: shown * (1.0 + feather) - feather / 2.0,
            inv_feather: if feather > 0.0 {
                1.0 / feather
            } else {
                f64::INFINITY
            },
        })
    }

    /// Every value as bits (a cache key).
    pub fn bits(&self) -> [u64; 8] {
        let (n, d) = (&self.num, &self.den);
        [
            n[0],
            n[1],
            n[2],
            d[0],
            d[1],
            d[2],
            self.edge,
            self.inv_feather,
        ]
        .map(f64::to_bits)
    }

    /// The shown fraction (0..=255) at the output point `(px, py)`.
    #[inline]
    pub fn factor(&self, px: f64, py: f64) -> u32 {
        let (n, d) = (&self.num, &self.den);
        let w = d[0] * px + d[1] * py + d[2];
        if w.is_nan() || w <= 0.0 {
            return 0;
        }
        let u = ((n[0] * px + n[1] * py + n[2]) / w).clamp(0.0, 1.0);
        let f = if self.inv_feather.is_finite() {
            ((self.edge - u) * self.inv_feather + 0.5).clamp(0.0, 1.0)
        } else if u < self.edge {
            1.0
        } else {
            0.0
        };
        // NaN shows nothing.
        let v = (f * 255.0).round();
        if v >= 1.0 { v.min(255.0) as u32 } else { 0 }
    }
}

/// The side a wipe starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WipeFrom {
    Left,
    Right,
    Top,
    Bottom,
}

/// A tint in 8-bit terms: colours move `k` / 255 of the way to `rgb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Tint8 {
    pub rgb: [u32; 3],
    /// 1..=255.
    pub k: u32,
}

impl Tint8 {
    /// Straight colour mixed toward the tint.
    #[inline]
    pub fn straight(&self, rgb: [u32; 3]) -> [u32; 3] {
        let inv = 255 - self.k;
        [0, 1, 2].map(|c| div255(rgb[c] * inv + self.rgb[c] * self.k))
    }

    /// Premultiplied colour mixed toward the tint at the pixel's own
    /// alpha (`c + (T·α − c)·k`), so coverage and alpha are kept and a
    /// full tint is exactly the tint colour at that alpha. Added light
    /// (colour above alpha) moves toward the tint too.
    #[inline]
    pub fn premultiplied(&self, src: [u32; 4]) -> [u32; 4] {
        let inv = 255 - self.k;
        let a = src[3].min(255);
        let mut out = src;
        for c in 0..3 {
            out[c] = div255(src[c].min(255) * inv + mul255(self.rgb[c], a) * self.k);
        }
        out
    }
}

/// A layer ready to draw.
pub(super) struct Prepared {
    /// Output pixel → source coordinates (continuous; pixel centres at
    /// `n + 0.5`).
    pub to_source: ToSource,
    /// The source rectangle: pixels are drawn with the fraction of them it
    /// covers (edge antialiasing).
    pub source_size: (f64, f64),
    pub source: PreparedSource,
    /// 0..=255.
    pub opacity: u32,
    /// Output-space bounding box (x0, y0, x1, y1) before clipping.
    pub bounds: Option<(f64, f64, f64, f64)>,
    /// The wipe reveal, when part of the layer is hidden.
    pub wipe: Option<WipeMask>,
    pub tint: Option<Tint8>,
}

impl Prepared {
    /// Whether the layer takes the plain path: affine, no wipe, no tint.
    fn is_plain(&self) -> bool {
        !self.to_source.is_projective() && self.wipe.is_none() && self.tint.is_none()
    }
}

/// A layer clipped to the output: integer pixel bounds included.
pub(super) struct Clipped {
    layer: Prepared,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl Prepared {
    pub fn clip(self, width: u32, height: u32) -> Option<Clipped> {
        let (min_x, min_y, max_x, max_y) = self.bounds?;
        let x0 = min_x.floor().max(0.0);
        let y0 = min_y.floor().max(0.0);
        let x1 = max_x.ceil().min(f64::from(width));
        let y1 = max_y.ceil().min(f64::from(height));
        (x0 < x1 && y0 < y1).then_some(Clipped {
            layer: self,
            x0: x0 as u32,
            y0: y0 as u32,
            x1: x1 as u32,
            y1: y1 as u32,
        })
    }
}

/// Bounds of a quad with a corner at or behind the eye: everything.
const UNBOUNDED: (f64, f64, f64, f64) = (
    f64::NEG_INFINITY,
    f64::NEG_INFINITY,
    f64::INFINITY,
    f64::INFINITY,
);

/// The output bounding box of the rectangle `(0, 0)..size` under `m`
/// (projected when `m` is projective; unbounded when a corner lies at or
/// behind the eye, where only the pixels' own test decides).
pub(super) fn bounds(m: DMat3, size: (f64, f64)) -> Option<(f64, f64, f64, f64)> {
    let (w, h) = size;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let affine = is_affine(m);
    let mut b = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for corner in [
        DVec2::new(0.0, 0.0),
        DVec2::new(w, 0.0),
        DVec2::new(0.0, h),
        DVec2::new(w, h),
    ] {
        let c = if affine {
            m.transform_point2(corner)
        } else {
            match project(m, corner) {
                Some(c) => c,
                None => return Some(UNBOUNDED),
            }
        };
        if !c.is_finite() {
            return None;
        }
        b = (b.0.min(c.x), b.1.min(c.y), b.2.max(c.x), b.3.max(c.y));
    }
    Some(b)
}

/// Rounded `a * b / 255` for values up to 255.
#[inline]
pub(super) fn mul255(a: u32, b: u32) -> u32 {
    div255(a * b)
}

/// Rounded `v / 255` for `v <= 255 * 255 * 2`.
#[inline]
pub(super) fn div255(v: u32) -> u32 {
    let x = v + 128;
    (x + (x >> 8)) >> 8
}

/// Straight-alpha "over": `src` with coverage `alpha` (0..=255) onto `dst`.
#[inline]
fn blend(dst: &mut [u8; 4], src: [u32; 3], alpha: u32) {
    if alpha == 0 {
        return;
    }
    let da = u32::from(dst[3]);
    if alpha >= 255 {
        *dst = [src[0] as u8, src[1] as u8, src[2] as u8, 255];
        return;
    }
    let inv = 255 - alpha;
    if da == 255 {
        for c in 0..3 {
            dst[c] = div255(src[c] * alpha + u32::from(dst[c]) * inv) as u8;
        }
        return;
    }
    // Translucent destination: out_a = a + da (1 - a); colours weighted by
    // their contribution.
    let out_a = alpha + mul255(da, inv);
    if out_a == 0 {
        return;
    }
    let dw = da * inv; // destination weight, ×255
    let sw = alpha * 255; // source weight, ×255
    let total = out_a * 255;
    for c in 0..3 {
        let v = src[c] * sw + u32::from(dst[c]) * dw;
        dst[c] = ((v + total / 2) / total).min(255) as u8;
    }
    dst[3] = out_a.min(255) as u8;
}

/// Premultiplied "over" of `src` (premultiplied, already scaled by opacity
/// and coverage) onto the straight-alpha `dst`. A source pixel with alpha 0
/// and colour adds that colour (glow light); one with nothing never
/// changes `dst`. Light falling on a fully transparent pixel has nowhere to
/// go and is dropped.
#[inline]
fn blend_premultiplied(dst: &mut [u8; 4], src: [u32; 4]) {
    if src == [0; 4] {
        return;
    }
    let sa = src[3].min(255);
    let inv = 255 - sa;
    let da = u32::from(dst[3]);
    if da == 255 {
        for c in 0..3 {
            dst[c] = (src[c] + mul255(u32::from(dst[c]), inv)).min(255) as u8;
        }
        return;
    }
    let out_a = sa + mul255(da, inv);
    if out_a == 0 {
        return;
    }
    for c in 0..3 {
        let premul = src[c] + mul255(mul255(u32::from(dst[c]), da), inv);
        dst[c] = ((premul * 255 + out_a / 2) / out_a).min(255) as u8;
    }
    dst[3] = out_a.min(255) as u8;
}

/// Bilinear sample of a premultiplied picture: all four channels weighted
/// alike (premultiplied colour needs no alpha weighting).
#[inline]
fn sample_premultiplied(pic: &Picture, sx: f64, sy: f64) -> [u32; 4] {
    let (w, h) = (i64::from(pic.width), i64::from(pic.height));
    let fx = ((sx - 0.5) * 256.0).floor() as i64;
    let fy = ((sy - 0.5) * 256.0).floor() as i64;
    let (mut x0, mut wx) = (fx >> 8, (fx & 255) as u32);
    let (mut y0, mut wy) = (fy >> 8, (fy & 255) as u32);
    if x0 < 0 {
        x0 = 0;
        wx = 0;
    }
    if y0 < 0 {
        y0 = 0;
        wy = 0;
    }
    if x0 >= w - 1 {
        x0 = w - 1;
        wx = 0;
    }
    if y0 >= h - 1 {
        y0 = h - 1;
        wy = 0;
    }
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let px = |x: i64, y: i64| -> &[u8] {
        let i = ((y * w + x) * 4) as usize;
        &pic.rgba[i..i + 4]
    };
    if wx == 0 && wy == 0 {
        let p = px(x0, y0);
        return [
            u32::from(p[0]),
            u32::from(p[1]),
            u32::from(p[2]),
            u32::from(p[3]),
        ];
    }
    let taps = [
        (px(x0, y0), (256 - wx) * (256 - wy)),
        (px(x1, y0), wx * (256 - wy)),
        (px(x0, y1), (256 - wx) * wy),
        (px(x1, y1), wx * wy),
    ];
    let mut c = [0u32; 4];
    for (q, wgt) in taps {
        for k in 0..4 {
            c[k] += u32::from(q[k]) * wgt;
        }
    }
    c.map(|v| (v + 32768) >> 16)
}

/// Bilinear sample of `pic` at continuous coordinates (pixel centres at
/// `n + 0.5`), edges clamped. Returns straight RGB and alpha; colours are
/// weighted by alpha so transparent texels don't bleed.
#[inline]
fn sample(pic: &Picture, sx: f64, sy: f64) -> ([u32; 3], u32) {
    let (w, h) = (i64::from(pic.width), i64::from(pic.height));
    let fx = ((sx - 0.5) * 256.0).floor() as i64;
    let fy = ((sy - 0.5) * 256.0).floor() as i64;
    let (mut x0, mut wx) = (fx >> 8, (fx & 255) as u32);
    let (mut y0, mut wy) = (fy >> 8, (fy & 255) as u32);
    if x0 < 0 {
        x0 = 0;
        wx = 0;
    }
    if y0 < 0 {
        y0 = 0;
        wy = 0;
    }
    if x0 >= w - 1 {
        x0 = w - 1;
        wx = 0;
    }
    if y0 >= h - 1 {
        y0 = h - 1;
        wy = 0;
    }
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let px = |x: i64, y: i64| -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [
            pic.rgba[i],
            pic.rgba[i + 1],
            pic.rgba[i + 2],
            pic.rgba[i + 3],
        ]
    };
    if wx == 0 && wy == 0 {
        let p = px(x0, y0);
        return (
            [u32::from(p[0]), u32::from(p[1]), u32::from(p[2])],
            u32::from(p[3]),
        );
    }
    let p = [px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1)];
    let weights = [
        (256 - wx) * (256 - wy),
        wx * (256 - wy),
        (256 - wx) * wy,
        wx * wy,
    ];
    if p.iter().all(|q| q[3] == 255) {
        let mut c = [0u32; 3];
        for (q, wgt) in p.iter().zip(weights) {
            for k in 0..3 {
                c[k] += u32::from(q[k]) * wgt;
            }
        }
        return (c.map(|v| (v + 32768) >> 16), 255);
    }
    let mut sum_a = 0u64;
    let mut c = [0u64; 3];
    for (q, wgt) in p.iter().zip(weights) {
        let wa = u64::from(wgt) * u64::from(q[3]);
        sum_a += wa;
        for k in 0..3 {
            c[k] += wa * u64::from(q[k]);
        }
    }
    if sum_a == 0 {
        return ([0; 3], 0);
    }
    let a = ((sum_a + 32768) >> 16) as u32;
    let rgb = c.map(|v| ((v + sum_a / 2) / sum_a).min(255) as u32);
    (rgb, a)
}

/// Draws every layer, bottom first, into `band`: whole output rows starting
/// at row `y0` of a frame `width` pixels wide.
///
/// Edges are antialiased by coverage: each pixel is treated as a 1 × 1 box
/// and the fraction of it inside the layer rectangle (measured
/// perpendicular to each pair of edges, in output pixels) scales the
/// layer's alpha. Pixels near an edge sample the source clamped to its
/// border. Integer-aligned edges stay exact (coverage 0 or 1).
pub(super) fn draw_rows(band: &mut [u8], y0: u32, width: u32, layers: &[Clipped]) {
    let row_bytes = width as usize * 4;
    let rows = (band.len() / row_bytes.max(1)) as u32;
    let y_end = y0 + rows;
    for clipped in layers {
        let layer = &clipped.layer;
        if !layer.is_plain() {
            let shader = Shader::new(layer);
            for y in clipped.y0.max(y0)..clipped.y1.min(y_end) {
                let py = f64::from(y) + 0.5;
                let start = (y - y0) as usize * row_bytes;
                let row = &mut band[start..start + row_bytes];
                let pixels =
                    &mut row.as_chunks_mut::<4>().0[clipped.x0 as usize..clipped.x1 as usize];
                for (dst, x) in pixels.iter_mut().zip(clipped.x0..) {
                    match shader.shade(f64::from(x) + 0.5, py) {
                        Some(Shaded::Straight(rgb, alpha)) => blend(dst, rgb, alpha),
                        Some(Shaded::Premultiplied(src)) => blend_premultiplied(dst, src),
                        None => {}
                    }
                }
            }
            continue;
        }
        let m = layer.to_source.m;
        let (a, b, c) = (m.x_axis.x, m.y_axis.x, m.z_axis.x);
        let (d, e, f) = (m.x_axis.y, m.y_axis.y, m.z_axis.y);
        let (sw, sh) = layer.source_size;
        let edges = Edges::new(a.hypot(b), d.hypot(e), sw, sh);
        for y in clipped.y0.max(y0)..clipped.y1.min(y_end) {
            let py = f64::from(y) + 0.5;
            let (row_x, row_y) = (b * py + c, e * py + f);
            let start = (y - y0) as usize * row_bytes;
            let row = &mut band[start..start + row_bytes];
            let pixels = row.as_chunks_mut::<4>().0;
            let Some((xs, xe)) = row_span(
                a,
                row_x,
                (-edges.margin_x, sw + edges.margin_x),
                d,
                row_y,
                (-edges.margin_y, sh + edges.margin_y),
                clipped.x0,
                clipped.x1,
            ) else {
                continue;
            };
            let pixels = &mut pixels[xs as usize..xe as usize];
            // The exact coverage stays per pixel; the span only skips pixels
            // that are certainly outside.
            let covered = |px: f64| {
                let (sx, sy) = (a * px + row_x, d * px + row_y);
                let cov = edges.coverage(sx, sy);
                (cov > 0).then_some((sx, sy, cov))
            };
            match &layer.source {
                PreparedSource::Fill(rgb, alpha) => {
                    let rgb = rgb.map(u32::from);
                    for (dst, x) in pixels.iter_mut().zip(xs..) {
                        if let Some((_, _, cov)) = covered(f64::from(x) + 0.5) {
                            blend(dst, rgb, mul255(*alpha, cov));
                        }
                    }
                }
                PreparedSource::Picture(pic) => {
                    for (dst, x) in pixels.iter_mut().zip(xs..) {
                        if let Some((sx, sy, cov)) = covered(f64::from(x) + 0.5) {
                            let (rgb, alpha) = sample(pic, sx, sy);
                            blend(dst, rgb, mul255(mul255(alpha, layer.opacity), cov));
                        }
                    }
                }
                PreparedSource::Premultiplied(pic) => {
                    for (dst, x) in pixels.iter_mut().zip(xs..) {
                        if let Some((sx, sy, cov)) = covered(f64::from(x) + 0.5) {
                            let k = mul255(layer.opacity, cov);
                            let src = sample_premultiplied(pic, sx, sy);
                            blend_premultiplied(dst, src.map(|v| mul255(v, k)));
                        }
                    }
                }
            }
        }
    }
}

/// What one layer puts at one output pixel.
pub(super) enum Shaded {
    /// Straight colour and alpha (coverage, opacity and wipe included).
    Straight([u32; 3], u32),
    /// Premultiplied colour and alpha, already scaled the same way.
    Premultiplied([u32; 4]),
}

impl Shaded {
    /// As premultiplied RGBA8.
    #[inline]
    pub fn premultiplied(self) -> [u8; 4] {
        match self {
            Shaded::Straight(rgb, a) => {
                let a = a.min(255);
                [
                    mul255(rgb[0].min(255), a) as u8,
                    mul255(rgb[1].min(255), a) as u8,
                    mul255(rgb[2].min(255), a) as u8,
                    a as u8,
                ]
            }
            Shaded::Premultiplied(p) => p.map(|v| v.min(255) as u8),
        }
    }
}

/// Evaluates one layer at any output pixel: the general path, for layers
/// with a projective map, a wipe or a tint, and for drawing a layer alone
/// (whole-layer blur). Plain layers give the same values as the plain
/// path in [`draw_rows`] apart from its span culling.
pub(super) struct Shader<'a> {
    layer: &'a Prepared,
    m: [f64; 9],
    /// Coverage gradients for an affine map (constant).
    edges: Edges,
}

impl<'a> Shader<'a> {
    pub fn new(layer: &'a Prepared) -> Shader<'a> {
        let m = layer.to_source.m;
        let (a, b) = (m.x_axis.x, m.y_axis.x);
        let (d, e) = (m.x_axis.y, m.y_axis.y);
        let (sw, sh) = layer.source_size;
        Shader {
            layer,
            m: [
                a, b, m.z_axis.x, d, e, m.z_axis.y, m.x_axis.z, m.y_axis.z, m.z_axis.z,
            ],
            edges: Edges::new(a.hypot(b), d.hypot(e), sw, sh),
        }
    }

    /// The source point and the coverage (1..=255) of the output pixel
    /// centred at `(px, py)`; `None` when it misses the layer or its
    /// preimage lies at or behind the eye. A projective map measures
    /// coverage with its gradient at the pixel (the map is close to affine
    /// across one pixel).
    #[inline]
    fn locate(&self, px: f64, py: f64) -> Option<(f64, f64, u32)> {
        let [a, b, c, d, e, f, g, h, i] = self.m;
        if !self.layer.to_source.is_projective() {
            let (sx, sy) = (a * px + (b * py + c), d * px + (e * py + f));
            let cov = self.edges.coverage(sx, sy);
            return (cov > 0).then_some((sx, sy, cov));
        }
        let w = g * px + h * py + i;
        if w.is_nan() || w <= 1e-12 {
            return None;
        }
        let (sx, sy) = ((a * px + b * py + c) / w, (d * px + e * py + f) / w);
        let gx = ((a - sx * g) / w).hypot((b - sx * h) / w);
        let gy = ((d - sy * g) / w).hypot((e - sy * h) / w);
        let (sw, sh) = self.layer.source_size;
        let cov = Edges::new(gx, gy, sw, sh).coverage(sx, sy);
        (cov > 0).then_some((sx, sy, cov))
    }

    /// The layer's contribution at the output pixel centred at `(px, py)`.
    #[inline]
    pub fn shade(&self, px: f64, py: f64) -> Option<Shaded> {
        let layer = self.layer;
        let (sx, sy, cov) = self.locate(px, py)?;
        let cov = match &layer.wipe {
            Some(w) => mul255(cov, w.factor(px, py)),
            None => cov,
        };
        if cov == 0 {
            return None;
        }
        Some(match &layer.source {
            PreparedSource::Fill(rgb, alpha) => {
                let rgb = rgb.map(u32::from);
                let rgb = layer.tint.map_or(rgb, |t| t.straight(rgb));
                Shaded::Straight(rgb, mul255(*alpha, cov))
            }
            PreparedSource::Picture(pic) => {
                let (rgb, alpha) = sample(pic, sx, sy);
                let rgb = layer.tint.map_or(rgb, |t| t.straight(rgb));
                Shaded::Straight(rgb, mul255(mul255(alpha, layer.opacity), cov))
            }
            PreparedSource::Premultiplied(pic) => {
                let src = sample_premultiplied(pic, sx, sy);
                let src = layer.tint.map_or(src, |t| t.premultiplied(src));
                let k = mul255(layer.opacity, cov);
                Shaded::Premultiplied(src.map(|v| mul255(v.min(255), k)))
            }
        })
    }
}

/// Coverage of output pixels by a layer rectangle `(0, 0)..(sw, sh)` in
/// source units.
struct Edges {
    /// Source units per output pixel across the x and y edges (the length
    /// of the gradient of `sx` and `sy`).
    gx: f64,
    gy: f64,
    /// The rectangle's size in output pixels across each pair of edges.
    wx: f64,
    wy: f64,
    /// Half an output pixel in source units: how far outside the rectangle
    /// a pixel centre can still be partly covered.
    margin_x: f64,
    margin_y: f64,
}

impl Edges {
    fn new(gx: f64, gy: f64, sw: f64, sh: f64) -> Edges {
        Edges {
            gx,
            gy,
            wx: sw / gx,
            wy: sh / gy,
            margin_x: 0.5 * gx,
            margin_y: 0.5 * gy,
        }
    }

    /// The covered fraction (0..=255) of the output pixel whose centre maps
    /// to `(sx, sy)`: a 1 px box against each pair of edges, multiplied.
    #[inline]
    fn coverage(&self, sx: f64, sy: f64) -> u32 {
        let span = |s: f64, g: f64, w: f64| {
            let u = s / g;
            ((u + 0.5).min(w) - (u - 0.5).max(0.0)).clamp(0.0, 1.0)
        };
        let cx = span(sx, self.gx, self.wx);
        if cx <= 0.0 {
            return 0;
        }
        let cy = span(sy, self.gy, self.wy);
        // NaN (a degenerate map) draws nothing.
        let cov = (cx * cy * 255.0).round();
        if cov >= 1.0 { cov.min(255.0) as u32 } else { 0 }
    }
}

/// The columns `x0..x1` of a row whose pixel centres may map inside the
/// source ranges `sx ∈ x_range` and `sy ∈ y_range`, given `sx = a·px + rx`
/// and `sy = d·px + ry` (conservative by one pixel on each side).
#[allow(clippy::too_many_arguments)]
fn row_span(
    a: f64,
    rx: f64,
    x_range: (f64, f64),
    d: f64,
    ry: f64,
    y_range: (f64, f64),
    x0: u32,
    x1: u32,
) -> Option<(u32, u32)> {
    let (mut lo, mut hi) = (f64::from(x0), f64::from(x1));
    for (k, r, (min, max)) in [(a, rx, x_range), (d, ry, y_range)] {
        if k == 0.0 {
            if !(r > min && r < max) {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((min - r) / k, (max - r) / k);
        // px = x + 0.5 lies in [min, max].
        lo = lo.max(t0.min(t1).floor() - 1.0);
        hi = hi.min(t0.max(t1).ceil() + 1.0);
    }
    // NaN-safe: an empty or undefined span draws nothing.
    if lo.partial_cmp(&hi) != Some(std::cmp::Ordering::Less) {
        return None;
    }
    Some((lo.max(f64::from(x0)) as u32, hi.min(f64::from(x1)) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul255_rounds() {
        assert_eq!(mul255(255, 255), 255);
        assert_eq!(mul255(0, 255), 0);
        assert_eq!(mul255(128, 255), 128);
        assert_eq!(mul255(255, 128), 128);
        for a in 0..=255 {
            for b in 0..=255 {
                let exact = (f64::from(a) * f64::from(b) / 255.0).round() as u32;
                assert_eq!(mul255(a, b), exact, "{a} * {b}");
            }
        }
    }

    #[test]
    fn blend_over_opaque_matches_the_formula() {
        let mut dst = [0, 100, 200, 255];
        blend(&mut dst, [255, 255, 255], 128);
        assert_eq!(dst, [128, 178, 228, 255]);
    }

    #[test]
    fn blend_onto_transparent_keeps_the_source() {
        let mut dst = [0, 0, 0, 0];
        blend(&mut dst, [10, 20, 30], 77);
        assert_eq!(dst, [10, 20, 30, 77]);
    }

    #[test]
    fn sampling_at_pixel_centres_is_exact() {
        let pic = Picture::new(2, 1, vec![10, 20, 30, 255, 200, 100, 50, 255]);
        assert_eq!(sample(&pic, 0.5, 0.5), ([10, 20, 30], 255));
        assert_eq!(sample(&pic, 1.5, 0.5), ([200, 100, 50], 255));
        // Halfway between the two centres.
        assert_eq!(sample(&pic, 1.0, 0.5), ([105, 60, 40], 255));
        // Clamped at the edges.
        assert_eq!(sample(&pic, 0.1, 0.9), ([10, 20, 30], 255));
    }

    #[test]
    fn premultiplied_light_adds_over_black_and_saturates_over_white() {
        // Glow light: colour with alpha 0.
        let light = [100, 60, 20, 0];
        let mut black = [0, 0, 0, 255];
        blend_premultiplied(&mut black, light);
        assert_eq!(black, [100, 60, 20, 255]);
        let mut white = [255, 255, 255, 255];
        blend_premultiplied(&mut white, light);
        assert_eq!(white, [255, 255, 255, 255]);
        let mut grey = [200, 200, 200, 255];
        blend_premultiplied(&mut grey, light);
        assert_eq!(grey, [255, 255, 220, 255]);
    }

    #[test]
    fn premultiplied_nothing_never_darkens() {
        for dst in [
            [0, 0, 0, 255],
            [255, 255, 255, 255],
            [10, 200, 30, 90],
            [0; 4],
        ] {
            let mut d = dst;
            blend_premultiplied(&mut d, [0, 0, 0, 0]);
            assert_eq!(d, dst);
        }
    }

    #[test]
    fn premultiplied_over_matches_straight_over() {
        // Half-transparent red, premultiplied, over opaque blue and over a
        // translucent destination.
        for dst in [[0, 0, 255, 255], [0, 0, 255, 100], [0; 4]] {
            let mut a = dst;
            let mut b = dst;
            blend(&mut a, [255, 0, 0], 128);
            blend_premultiplied(&mut b, [128, 0, 0, 128]);
            for c in 0..4 {
                assert!(a[c].abs_diff(b[c]) <= 1, "{dst:?}: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn premultiplied_sampling_weights_all_channels_alike() {
        let pic = Picture::new(2, 1, vec![200, 100, 0, 200, 0, 0, 0, 0]);
        assert_eq!(sample_premultiplied(&pic, 1.0, 0.5), [100, 50, 0, 100]);
        assert_eq!(sample_premultiplied(&pic, 0.5, 0.5), [200, 100, 0, 200]);
    }

    #[test]
    fn premultiplied_tint_endpoints_keep_alpha() {
        let white = Tint8 {
            rgb: [255, 255, 255],
            k: 255,
        };
        // Full tint: the tint colour at the pixel's alpha.
        assert_eq!(white.premultiplied([10, 20, 30, 128]), [128, 128, 128, 128]);
        assert_eq!(white.premultiplied([0, 0, 0, 0]), [0, 0, 0, 0]);
        let none = Tint8 { k: 0, ..white };
        assert_eq!(none.premultiplied([10, 20, 30, 128]), [10, 20, 30, 128]);
        // Straight colour, half way.
        let half = Tint8 {
            rgb: [0, 0, 255],
            k: 128,
        };
        assert_eq!(half.straight([200, 100, 0]), [100, 50, 128]);
    }

    #[test]
    fn a_wipe_through_a_projective_map_measures_in_layer_coordinates() {
        // A layer 100 px wide, turned so that its right half is smaller.
        let m = DMat3::from_cols(
            freecut_timeline::glam::DVec3::new(0.8, 0.0, 0.002),
            freecut_timeline::glam::DVec3::Y,
            freecut_timeline::glam::DVec3::Z,
        );
        let to_layer = ToSource::inverse_of(m).unwrap();
        assert!(to_layer.is_projective());
        let wipe =
            WipeMask::new(to_layer, (0.0, 0.0, 100.0, 50.0), 0.5, WipeFrom::Left, 0.0).unwrap();
        // The layer's x = 50 lands at 40 / 1.1 in output.
        let edge = project(m, DVec2::new(50.0, 10.0)).unwrap().x;
        assert_eq!(wipe.factor(edge - 0.01, 10.0), 255);
        assert_eq!(wipe.factor(edge + 0.01, 10.0), 0);
        assert!(
            WipeMask::new(to_layer, (0.0, 0.0, 100.0, 50.0), 1.0, WipeFrom::Left, 0.1).is_none()
        );
    }

    #[test]
    fn transparent_texels_do_not_darken_the_sample() {
        let pic = Picture::new(2, 1, vec![255, 0, 0, 255, 0, 0, 0, 0]);
        let (rgb, a) = sample(&pic, 1.0, 0.5);
        assert_eq!(rgb, [255, 0, 0]);
        assert_eq!(a, 128);
    }
}
