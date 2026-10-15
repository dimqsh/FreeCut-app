//! Glyph rasterization with swash and the styled text layer (C46).
//!
//! **Glyphs.** Unhinted coverage at the exact output size, colour glyphs
//! (COLR, sbix/CBDT) where the font has them, synthetic bold (outline
//! embolden) and italic (skew) when the font lacks the face. Each glyph is
//! placed on its own: at its pen position, or on the curve's arc turned to
//! the tangent (TX-016).
//!
//! **Coverage, then colour.** Monochrome glyphs add their coverage into one
//! 8-bit mask (saturating) and decorations join it as a union, so
//! overlapping glyphs don't double the opacity of translucent text and
//! abutting antialiased edges leave no seams. Colour glyphs are composited
//! "over" that, in text order.
//!
//! **The layer stack** (TX-011), back to front: background box or bubble,
//! shadow, outward glow, the body (outline and fill), with the inward glow
//! between fill and an inside or centred outline. Everything is computed
//! from masks in layer space: the union of the glyph coverage gives one
//! silhouette, its signed distance field (an exact Euclidean distance
//! transform) gives the outline bands and the glow's source (the letter
//! shapes with their outline, grown), and the body's alpha casts the
//! shadow. The result is **premultiplied** RGBA; glow adds
//! light (colour without alpha), so colour may exceed alpha.

use std::collections::HashMap;
use std::sync::Arc;

use swash::FontRef;
use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use zeno::{Angle, Format, Transform, Vector};

use super::animate::{AnimUnit, CaretBar, GlyphAnim, MotionState, Shown, UnitMap};
use super::curve::Arc as Curve;
use super::effects::{blur, blur_reach, edt, grown, shifted, signed_distance};
use super::layout::{Built, GlyphItem, TextLayout};
use super::projective::{Homography, MAX_RESAMPLED_SIDE, resample};
use super::style::{
    BoxHeight, BoxShape, Fill, FillExtent, GlowDirection, GradientStop, StrokePosition, TextBox,
};
use super::{MAX_RASTER_SIDE, TextRaster, TextSpec};
use crate::cpu::Picture;
use crate::shape::bubble::{bubble_body, bubble_path};
use crate::shape::{Coverage, Path, Point, Rect, fill_union, rounded_rect};
use freecut_timeline::model::Rotation;

/// Horizontal sub-pixel positions per pixel.
const SUBPIXEL_STEPS: f32 = 4.0;

/// Synthetic bold strength, as a fraction of the em (TX-004: ≈ 2 % em).
const EMBOLDEN_EM: f32 = 0.02;

/// Most glyph cache entries kept.
const MAX_GLYPHS: usize = 8192;

/// What one cache entry costs besides its bitmap data.
const GLYPH_ENTRY_BYTES: usize = 160;

/// How far below the baseline a lower-half box reaches, in em.
const LOWER_HALF_DROP_EM: f32 = 0.1;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: u64,
    index: u32,
    glyph: u16,
    size: u32,
    subpixel: u8,
    embolden: bool,
    skew: u32,
    /// Clockwise rotation (curved text), radians as bits.
    angle: u32,
    coords: Arc<[i16]>,
    /// The fill colour: COLR layers that use the text colour (palette
    /// index 0xFFFF) are drawn with it.
    foreground: [u8; 3],
}

#[derive(Clone)]
struct GlyphBitmap {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    /// Coverage (1 byte per pixel) or RGBA (4 bytes per pixel).
    color: bool,
    /// Colour data is premultiplied (swash's COLR output, blended into a
    /// zeroed buffer); bitmap glyphs (sbix, CBDT) are straight alpha.
    premultiplied: bool,
    data: Vec<u8>,
}

/// A glyph to draw: its bitmap and pen position in image pixels (before
/// the origin shift), and how its rest pose maps to where it is drawn.
struct Placed {
    bitmap: Arc<GlyphBitmap>,
    x: i32,
    y: i32,
    line: u32,
    /// Drawn layout px → rest layout px (gradients are evaluated at rest).
    to_rest: ToRest,
    /// Opacity (per-unit animation), 0..=1.
    alpha: f32,
    /// How far the fill moves toward the motion's tint colour (per-unit
    /// style channels), 0..=1.
    tint: f32,
    /// Hue turn of fill and outline in degrees (per-unit style channels).
    hue: f32,
    /// Factor on the glow intensity around the piece (per-unit style
    /// channels); 1 as set.
    glow: f32,
}

/// How a drawn glyph maps back to its rest pose.
#[derive(Debug, Clone, Copy)]
enum ToRest {
    /// `rest + R(−angle)·(l − pen)`: a glyph moved and turned rigidly
    /// (curve, offset), with the rest and drawn pens in layout px and
    /// `(sin, cos)` of `−angle`.
    Rigid {
        rest: (f32, f32),
        pen: (f32, f32),
        sin_cos: (f32, f32),
    },
    /// Any projective map.
    Map(Homography),
}

impl ToRest {
    fn rigid(rest: (f32, f32), pen: (f32, f32), angle: f32) -> ToRest {
        ToRest::Rigid {
            rest,
            pen,
            sin_cos: (-angle).sin_cos(),
        }
    }

    fn at(&self, l: (f32, f32)) -> (f32, f32) {
        match *self {
            ToRest::Rigid { rest, pen, sin_cos } => {
                let (s, c) = sin_cos;
                let (lx, ly) = (l.0 - pen.0, l.1 - pen.1);
                (rest.0 + c * lx - s * ly, rest.1 + s * lx + c * ly)
            }
            ToRest::Map(h) => h
                .apply((f64::from(l.0), f64::from(l.1)))
                .map_or((f32::NAN, f32::NAN), |(x, y)| (x as f32, y as f32)),
        }
    }
}

/// Rasterizes and caches glyphs. The cache holds at most
/// [`MAX_GLYPHS`] entries and about the configured byte budget; past either,
/// the entries least recently used are dropped (never those of the render
/// in progress).
pub(super) struct GlyphRasterizer {
    context: ScaleContext,
    glyphs: HashMap<GlyphKey, CachedGlyph>,
    /// Bytes held by `glyphs`.
    bytes: usize,
    budget: usize,
    /// Counts renders; each entry remembers the last one that used it.
    clock: u64,
}

struct CachedGlyph {
    bitmap: Option<Arc<GlyphBitmap>>,
    bytes: usize,
    last_use: u64,
}

impl Default for GlyphRasterizer {
    fn default() -> GlyphRasterizer {
        GlyphRasterizer {
            context: ScaleContext::default(),
            glyphs: HashMap::new(),
            bytes: 0,
            budget: freecut_media::resources::current().cache_bytes(32, 4),
            clock: 0,
        }
    }
}

/// Premultiplied colour, 0..=1 per channel.
type Rgba = [f32; 4];

fn premul(c: [u8; 4], opacity: f32) -> Rgba {
    let a = f32::from(c[3]) / 255.0 * opacity.clamp(0.0, 1.0);
    [
        f32::from(c[0]) / 255.0 * a,
        f32::from(c[1]) / 255.0 * a,
        f32::from(c[2]) / 255.0 * a,
        a,
    ]
}

/// `src` over `dst`, premultiplied.
#[inline]
fn over(src: Rgba, dst: Rgba) -> Rgba {
    let k = 1.0 - src[3];
    [
        src[0] + dst[0] * k,
        src[1] + dst[1] * k,
        src[2] + dst[2] * k,
        src[3] + dst[3] * k,
    ]
}

#[inline]
fn scaled(c: Rgba, k: f32) -> Rgba {
    [c[0] * k, c[1] * k, c[2] * k, c[3] * k]
}

#[inline]
fn add(a: Rgba, b: Rgba) -> Rgba {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

/// A gradient ready to evaluate in rest layout px.
struct Paint {
    /// Sorted stops, premultiplied colours.
    stops: Vec<(f32, Rgba)>,
    kind: PaintKind,
    extent: FillExtent,
}

enum PaintKind {
    Linear { dir: (f32, f32) },
    Radial { center: (f32, f32), spread: f32 },
}

impl Paint {
    fn new(fill: &Fill) -> Option<Paint> {
        let (stops, kind, extent) = match fill {
            Fill::Solid(_) => return None,
            Fill::Linear(g) => {
                let a = finite(g.angle_deg, 90.0).to_radians();
                (
                    &g.stops,
                    PaintKind::Linear {
                        dir: (a.cos(), a.sin()),
                    },
                    g.extent,
                )
            }
            Fill::Radial(g) => (
                &g.stops,
                PaintKind::Radial {
                    center: (finite(g.center.0, 0.5), finite(g.center.1, 0.5)),
                    spread: finite(g.spread, 1.0).clamp(0.1, 2.0),
                },
                g.extent,
            ),
        };
        let mut sorted: Vec<GradientStop> = stops.clone();
        sorted.sort_by(|a, b| finite(a.at, 0.0).total_cmp(&finite(b.at, 0.0)));
        let stops: Vec<(f32, Rgba)> = sorted
            .iter()
            .map(|s| (finite(s.at, 0.0).clamp(0.0, 1.0), premul(s.color, 1.0)))
            .collect();
        if stops.is_empty() {
            return None;
        }
        Some(Paint {
            stops,
            kind,
            extent,
        })
    }

    /// The colour at rest point `q` of a glyph on `line`, whose extent is
    /// `rect`.
    fn at(&self, q: (f32, f32), rect: Rect) -> Rgba {
        let t = match self.kind {
            PaintKind::Linear { dir } => {
                let corners = [
                    (rect.x0, rect.y0),
                    (rect.x1, rect.y0),
                    (rect.x0, rect.y1),
                    (rect.x1, rect.y1),
                ];
                let proj = |p: (f32, f32)| p.0 * dir.0 + p.1 * dir.1;
                let lo = corners
                    .iter()
                    .map(|&c| proj(c))
                    .fold(f32::INFINITY, f32::min);
                let hi = corners
                    .iter()
                    .map(|&c| proj(c))
                    .fold(f32::NEG_INFINITY, f32::max);
                if hi - lo > 1e-6 {
                    (proj(q) - lo) / (hi - lo)
                } else {
                    0.0
                }
            }
            PaintKind::Radial { center, spread } => {
                let c = (
                    rect.x0 + center.0 * rect.width(),
                    rect.y0 + center.1 * rect.height(),
                );
                let radius = spread * rect.width().hypot(rect.height()) / 2.0;
                if radius > 1e-6 {
                    (q.0 - c.0).hypot(q.1 - c.1) / radius
                } else {
                    0.0
                }
            }
        };
        self.sample(t)
    }

    fn sample(&self, t: f32) -> Rgba {
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let first = self.stops[0];
        if t <= first.0 {
            return first.1;
        }
        for pair in self.stops.windows(2) {
            let ((a, ca), (b, cb)) = (pair[0], pair[1]);
            if t <= b {
                if b - a <= 1e-6 {
                    return cb;
                }
                let k = (t - a) / (b - a);
                return add(scaled(ca, 1.0 - k), scaled(cb, k));
            }
        }
        self.stops[self.stops.len() - 1].1
    }
}

fn finite(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

/// The box or bubble behind the text, in layout px.
struct Backdrop {
    path: Path,
    /// Per-character boxes under motion with some piece faded: `path`'s
    /// pieces grouped by their glyph's opacity, each drawn at it (TX-013).
    /// Empty: `path` at full strength.
    faded: Vec<(Path, f32)>,
    color: [u8; 4],
    opacity: f32,
    /// Border colour and width (layout px).
    border: Option<([u8; 4], f32)>,
}

impl GlyphRasterizer {
    pub fn render(
        &mut self,
        built: &Built,
        spec: &TextSpec,
        scale: f32,
        info: TextLayout,
        motion: Option<&Animated<'_>>,
    ) -> TextRaster {
        let em = spec.size();
        let curve = Curve::new(built, spec.curve_amount());
        let (fg_rgb, fg_alpha) = match &spec.fill {
            Fill::Solid(c) => ([c[0], c[1], c[2]], c[3]),
            Fill::Linear(g) => (stop_rgb(&g.stops), 255),
            Fill::Radial(g) => (stop_rgb(&g.stops), 255),
        };
        // Under motion, decorations are cut per glyph and placed with the
        // glyphs; boxes follow the revealed glyphs (TX-013).
        let (placed, decorations, backdrop) = match motion {
            None => (
                self.place(built, curve.as_ref(), scale, fg_rgb),
                decoration_paths(built, spec, curve.as_ref()),
                if curve.is_some() {
                    None
                } else {
                    backdrop(built, spec, &info, em)
                },
            ),
            Some(m) => {
                let (placed, poses) =
                    self.place_animated(built, spec, curve.as_ref(), scale, fg_rgb, m);
                let backdrop = if curve.is_some() {
                    None
                } else {
                    animated_backdrop(built, spec, &info, em, &poses)
                };
                (placed, Vec::new(), backdrop)
            }
        };
        let empty = || TextRaster {
            image: Arc::new(Picture::empty()),
            scale,
            origin: (0, 0),
            layout: info.clone(),
        };

        // The image covers the box, all ink and every effect.
        let (bw, bh) = (info.size.0 * scale, info.size.1 * scale);
        let mut bounds = Rect::new(0.0, 0.0, bw.max(0.0), bh.max(0.0));
        let mut inked = false;
        for p in &placed {
            let b = &p.bitmap;
            if b.width == 0 || b.height == 0 {
                continue;
            }
            inked = true;
            let x0 = (p.x + b.left) as f32;
            let y0 = (p.y - b.top) as f32;
            bounds = bounds.union(&Rect::new(
                x0,
                y0,
                x0 + b.width as f32,
                y0 + b.height as f32,
            ));
        }
        if !inked {
            return empty();
        }
        let to_image = |p: Point| Point::new(p.x * scale, p.y * scale);
        for (_, path) in &decorations {
            if let Some(b) = path.map(to_image).bounds() {
                bounds = bounds.union(&b);
            }
        }
        if let Some(b) = backdrop
            .as_ref()
            .and_then(|d| d.path.map(to_image).bounds())
        {
            bounds = bounds.union(&b);
        }
        let margin = effect_margin(spec, em * scale) + 2.0;
        let bounds = bounds.outset(margin, margin);
        let x0 = bounds.x0.floor() as i32;
        let y0 = bounds.y0.floor() as i32;
        let max = MAX_RASTER_SIDE as i32;
        let w = (bounds.x1.ceil() as i32 - x0).clamp(1, max) as usize;
        let h = (bounds.y1.ceil() as i32 - y0).clamp(1, max) as usize;
        let n = w * h;
        // Layout px → image px.
        let to_pixels = |p: Point| Point::new(p.x * scale - x0 as f32, p.y * scale - y0 as f32);

        // Fill coverage: monochrome glyphs (added), decorations (union).
        let mut coverage = Coverage::new(w, h);
        for p in placed.iter().filter(|p| !p.bitmap.color) {
            let b = &p.bitmap;
            add_coverage(
                &mut coverage.data,
                w,
                h,
                b,
                p.x + b.left - x0,
                p.y - b.top - y0,
            );
        }
        let paint = Paint::new(&spec.fill);
        // Gradient colour, accumulated per glyph from its rest pose:
        // premultiplied colour × coverage, and the coverage.
        let mut tint: Option<(Vec<Rgba>, Vec<f32>)> =
            paint.as_ref().map(|_| (vec![[0.0; 4]; n], vec![0.0; n]));
        if let (Some(paint), Some((acc, weight))) = (&paint, tint.as_mut()) {
            for p in placed.iter().filter(|p| !p.bitmap.color) {
                let rect = extent_rect(paint.extent, &info, p.line);
                let b = &p.bitmap;
                let (gx, gy) = (p.x + b.left - x0, p.y - b.top - y0);
                for row in 0..b.height as i32 {
                    for col in 0..b.width as i32 {
                        let (x, y) = (gx + col, gy + row);
                        if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
                            continue;
                        }
                        let cov = b.data[(row as u32 * b.width + col as u32) as usize];
                        if cov == 0 {
                            continue;
                        }
                        // Pixel centre → layout px → the glyph's rest frame.
                        let q = p.to_rest.at((
                            (x as f32 + 0.5 + x0 as f32) / scale,
                            (y as f32 + 0.5 + y0 as f32) / scale,
                        ));
                        if !(q.0.is_finite() && q.1.is_finite()) {
                            continue;
                        }
                        // A faint piece barely colours what it overlaps.
                        let k = f32::from(cov) / 255.0 * p.alpha.clamp(0.0, 1.0);
                        let i = y as usize * w + x as usize;
                        acc[i] = add(acc[i], scaled(paint.at(q, rect), k));
                        weight[i] += k;
                    }
                }
            }
        }
        for (line, path) in &decorations {
            let mut local = Coverage::new(w, h);
            fill_union(&mut local, &path.map(to_pixels));
            if let (Some(paint), Some((acc, weight))) = (&paint, tint.as_mut()) {
                let rect = extent_rect(paint.extent, &info, *line);
                for (i, &cov) in local.data.iter().enumerate() {
                    if cov == 0 {
                        continue;
                    }
                    let lp = (
                        ((i % w) as f32 + 0.5 + x0 as f32) / scale,
                        ((i / w) as f32 + 0.5 + y0 as f32) / scale,
                    );
                    let q = curve.as_ref().map_or(lp, |arc| arc.rest(*line, lp));
                    let k = f32::from(cov) / 255.0;
                    acc[i] = add(acc[i], scaled(paint.at(q, rect), k));
                    weight[i] += k;
                }
            }
            for (d, &s) in coverage.data.iter_mut().zip(&local.data) {
                *d = (*d).max(s);
            }
        }
        // Colour glyphs, premultiplied, in text order.
        let fill_alpha = u32::from(fg_alpha);
        let mut colour = vec![
            0u8;
            if placed.iter().any(|p| p.bitmap.color) {
                n * 4
            } else {
                0
            }
        ];
        for p in placed.iter().filter(|p| p.bitmap.color) {
            let b = &p.bitmap;
            composite_glyph(
                &mut colour,
                w,
                h,
                b,
                p.x + b.left - x0,
                p.y - b.top - y0,
                fill_alpha,
            );
        }

        // Per-glyph style channels: the fill (and outline) hue turned, then
        // the fill moved toward the motion's tint colour, at its own alpha;
        // the glow around each glyph scaled.
        let glyph_tint = tint_plane(&placed, w, h, x0, y0);
        let glyph_hue = hue_plane(&placed, w, h, x0, y0);
        let glyph_glow = glow_plane(&placed, w, h, x0, y0);
        let tint_rgb = motion.map_or([255; 3], |m| m.state.tint_color);
        let tint_rgb = tint_rgb.map(|c| f32::from(c) / 255.0);

        // The fill layer P and the silhouette.
        let solid = match &spec.fill {
            Fill::Solid(c) => premul(*c, 1.0),
            _ => [0.0; 4],
        };
        let mut body: Vec<Rgba> = Vec::with_capacity(n);
        let mut sil: Vec<f32> = Vec::with_capacity(n);
        for i in 0..n {
            let cov = f32::from(coverage.data[i]) / 255.0;
            let mut unit = match &tint {
                Some((acc, weight)) if weight[i] > 0.0 => scaled(acc[i], 1.0 / weight[i]),
                Some(_) => [0.0; 4],
                None => solid,
            };
            if let Some(deg) = glyph_hue.as_ref().map(|t| t[i]).filter(|&d| d != 0.0) {
                unit = turned(unit, deg);
            }
            if let Some(k) = glyph_tint.as_ref().map(|t| t[i]).filter(|&k| k > 0.0) {
                let a = unit[3];
                for c in 0..3 {
                    unit[c] += (tint_rgb[c] * a - unit[c]) * k;
                }
            }
            let mut px = scaled(unit, cov);
            let mut s = cov;
            if !colour.is_empty() {
                let c = &colour[i * 4..i * 4 + 4];
                let g = [
                    f32::from(c[0]) / 255.0,
                    f32::from(c[1]) / 255.0,
                    f32::from(c[2]) / 255.0,
                    f32::from(c[3]) / 255.0,
                ];
                px = over(g, px);
                s = s.max(g[3]);
            }
            body.push(px);
            sil.push(s);
        }

        // Per-glyph opacity (per-unit animation), spread to the outline,
        // glow and shadow around each glyph from its nearest ink.
        let fade = opacity_plane(&placed, w, h, x0, y0);

        let px_em = em * scale;
        // Inward glow: light from the edges inwards, clipped to the
        // silhouette, under an inside or centred outline.
        if let Some(glow) = spec.glow.filter(|g| g.intensity > 0.0 && g.radius > 0.0)
            && matches!(glow.direction, GlowDirection::Inward | GlowDirection::Both)
        {
            let radius = glow.radius.clamp(0.0, 1.0) * px_em;
            let holes: Vec<f32> = sil.iter().map(|s| 1.0 - s).collect();
            let lit = blur(&holes, w, h, radius, 1.0);
            let (dx, dy) = offset(glow.offset.clamp(0.0, 0.5) * px_em, glow.angle_deg);
            let lit = shifted(&lit, w, h, dx, dy);
            let light = premul(glow.color, glow.intensity.clamp(0.0, 1.0));
            let factor = glow_factors(glyph_glow.as_deref(), glow.intensity);
            for i in 0..n {
                let k = sil[i] * (2.0 * lit[i]).min(1.0) * factor.as_ref().map_or(1.0, |f| f[i]);
                if k > 0.0 {
                    let p = &mut body[i];
                    p[0] += light[0] * k;
                    p[1] += light[1] * k;
                    p[2] += light[2] * k;
                }
            }
        }

        // The outline, conflation-free: inside the silhouette the outline
        // and the fill share each pixel's covered area exactly.
        // The silhouette's signed distance field, made when an outline or a
        // glow needs it, and how far an outline reaches outside it.
        let mut distance: Option<Vec<f32>> = None;
        let mut outline_reach = 0.0;
        if let Some(stroke) = spec.stroke.filter(|s| s.width > 0.0 && s.opacity > 0.0) {
            let width = stroke.width.clamp(0.0, 0.3) * px_em;
            outline_reach = match stroke.position {
                StrokePosition::Outside => width,
                StrokePosition::Center => width / 2.0,
                StrokePosition::Inside => 0.0,
            };
            let sd = distance.insert(signed_distance(&sil, w, h));
            let outline = premul(stroke.color, stroke.opacity);
            for i in 0..n {
                let o = match glyph_hue.as_ref().map(|t| t[i]).filter(|&d| d != 0.0) {
                    Some(deg) => turned(outline, deg),
                    None => outline,
                };
                let s = sil[i];
                let unit = if s > 0.0 {
                    scaled(body[i], 1.0 / s)
                } else {
                    [0.0; 4]
                };
                let (outer, inner) = match stroke.position {
                    StrokePosition::Outside => (grown(sd[i], width), s),
                    StrokePosition::Center => (
                        grown(sd[i], width / 2.0).max(s),
                        grown(sd[i], -width / 2.0).min(s),
                    ),
                    StrokePosition::Inside => (s, grown(sd[i], -width).min(s)),
                };
                let ring = (outer - s).max(0.0);
                let band = (s - inner).max(0.0);
                body[i] = add(
                    add(scaled(o, ring), scaled(over(o, unit), band)),
                    scaled(unit, inner),
                );
            }
        }

        if let Some(fade) = &fade {
            for (p, &k) in body.iter_mut().zip(fade) {
                *p = scaled(*p, k);
            }
        }

        // The box or bubble: the border composed over the fill at full
        // strength, then the whole box at its opacity, once (TX-013).
        let mut out: Vec<Rgba> = vec![[0.0; 4]; n];
        let mut box_alpha: Option<Vec<f32>> = None;
        if let Some(back) = &backdrop {
            let mask_of = |path: &Path| {
                let mut mask = Coverage::new(w, h);
                fill_union(&mut mask, &path.map(to_pixels));
                mask.data
            };
            let m: Vec<f32> = if back.faded.is_empty() {
                mask_of(&back.path)
                    .iter()
                    .map(|&v| f32::from(v) / 255.0)
                    .collect()
            } else {
                // Each group at its glyphs' opacity; where boxes of
                // different opacities meet, the stronger shows.
                let mut m = vec![0.0f32; n];
                for (path, alpha) in &back.faded {
                    for (v, &c) in m.iter_mut().zip(&mask_of(path)) {
                        *v = v.max(f32::from(c) / 255.0 * alpha);
                    }
                }
                m
            };
            let fill = premul(back.color, 1.0);
            let opacity = back.opacity.clamp(0.0, 1.0);
            match back.border.filter(|b| b.1 > 0.0) {
                Some((color, width)) => {
                    let sd = signed_distance(&m, w, h);
                    let rim = over(premul(color, 1.0), fill);
                    let width = width * scale;
                    for i in 0..n {
                        let inner = grown(sd[i], -width).min(m[i]);
                        let px = add(scaled(rim, m[i] - inner), scaled(fill, inner));
                        out[i] = scaled(px, opacity);
                    }
                }
                None => {
                    for i in 0..n {
                        out[i] = scaled(fill, m[i] * opacity);
                    }
                }
            }
            box_alpha = Some(out.iter().map(|p| p[3]).collect());
        }

        // Shadow, cast by the body (and the box when asked).
        let body_alpha: Vec<f32> = body.iter().map(|p| p[3].clamp(0.0, 1.0)).collect();
        if let Some(shadow) = spec.shadow.filter(|s| s.opacity > 0.0) {
            let source: Vec<f32> = match (&box_alpha, shadow.include_box) {
                (Some(b), true) => body_alpha
                    .iter()
                    .zip(b)
                    .map(|(a, b)| a + b - a * b)
                    .collect(),
                _ => body_alpha.clone(),
            };
            let radius = shadow.blur.clamp(0.0, 0.5) * px_em;
            let soft = blur(&source, w, h, radius, 0.0);
            let (dx, dy) = offset(shadow.distance.clamp(0.0, 1.0) * px_em, shadow.angle_deg);
            let cast = shifted(&soft, w, h, dx, dy);
            let colour = premul(shadow.color, shadow.opacity);
            for i in 0..n {
                if cast[i] > 0.0 {
                    out[i] = over(scaled(colour, cast[i].min(1.0)), out[i]);
                }
            }
        }

        // Outward glow: light added behind outline and fill, radiating from
        // the letter shapes (so hollow or transparent text glows too).
        if let Some(glow) = spec.glow.filter(|g| g.intensity > 0.0 && g.radius > 0.0)
            && matches!(glow.direction, GlowDirection::Outward | GlowDirection::Both)
        {
            // The shape grown by a third of the radius, blurred over the rest:
            // thin strokes glow as brightly as thick ones, and the light
            // fades out at about the radius.
            let radius = glow.radius.clamp(0.0, 1.0) * px_em;
            let sd = distance.get_or_insert_with(|| signed_distance(&sil, w, h));
            let mut source: Vec<f32> = sd
                .iter()
                .map(|&d| grown(d - outline_reach, radius / 3.0))
                .collect();
            if let Some(fade) = &fade {
                for (v, &k) in source.iter_mut().zip(fade) {
                    *v *= k;
                }
            }
            if let Some(factor) = glow_factors(glyph_glow.as_deref(), glow.intensity) {
                for (v, &k) in source.iter_mut().zip(&factor) {
                    *v *= k;
                }
            }
            let soft = blur(&source, w, h, radius * 2.0 / 3.0, 0.0);
            let (dx, dy) = offset(glow.offset.clamp(0.0, 0.5) * px_em, glow.angle_deg);
            let lit = shifted(&soft, w, h, dx, dy);
            let light = premul(glow.color, glow.intensity.clamp(0.0, 1.0));
            for i in 0..n {
                let k = lit[i].min(1.0);
                if k > 0.0 {
                    let p = &mut out[i];
                    p[0] += light[0] * k;
                    p[1] += light[1] * k;
                    p[2] += light[2] * k;
                }
            }
        }

        let mut rgba = vec![0u8; n * 4];
        let mut any = false;
        for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let p = over(body[i], out[i]);
            for (c, v) in px.iter_mut().zip(p) {
                *c = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            any |= *px != [0; 4];
        }
        if !any {
            return empty();
        }
        TextRaster {
            image: Arc::new(Picture {
                width: w as u32,
                height: h as u32,
                rotation: Rotation::None,
                mirrored: false,
                rgba,
            }),
            scale,
            origin: (-x0, -y0),
            layout: info,
        }
    }

    fn place(
        &mut self,
        built: &Built,
        curve: Option<&Curve>,
        scale: f32,
        foreground: [u8; 3],
    ) -> Vec<Placed> {
        self.clock += 1;
        let mut placed = Vec::with_capacity(built.items.len());
        for g in &built.items {
            let (pen, angle) = match curve {
                Some(arc) => arc.place(g.unit.line, g.x, g.advance),
                None => ((g.x, g.y), 0.0),
            };
            let (x, sub) = subpixel(pen.0 * scale);
            if let Some(bitmap) = self.bitmap(g, scale, sub, angle, foreground) {
                placed.push(Placed {
                    bitmap,
                    x,
                    y: (pen.1 * scale).round() as i32,
                    line: g.unit.line,
                    to_rest: ToRest::rigid((g.x, g.y), pen, angle),
                    alpha: 1.0,
                    tint: 0.0,
                    hue: 0.0,
                    glow: 1.0,
                });
            }
        }
        self.trim();
        placed
    }

    fn bitmap(
        &mut self,
        g: &GlyphItem,
        scale: f32,
        subpixel: u8,
        angle: f32,
        foreground: [u8; 3],
    ) -> Option<Arc<GlyphBitmap>> {
        let size = g.size * scale;
        let key = GlyphKey {
            font: g.font.data.id(),
            index: g.font.index,
            glyph: g.id,
            size: size.to_bits(),
            subpixel,
            embolden: g.embolden,
            skew: g.skew.to_bits(),
            angle: angle.to_bits(),
            coords: Arc::clone(&g.coords),
            foreground,
        };
        if let Some(entry) = self.glyphs.get_mut(&key) {
            entry.last_use = self.clock;
            return entry.bitmap.clone();
        }
        let b = rasterize_glyph(
            &mut self.context,
            g.font.data.data(),
            &key,
            size,
            f32::from(subpixel) / SUBPIXEL_STEPS,
            g.skew,
            angle,
        )
        .map(Arc::new);
        let bytes =
            GLYPH_ENTRY_BYTES + key.coords.len() * 2 + b.as_ref().map_or(0, |b| b.data.len());
        self.bytes += bytes;
        self.glyphs.insert(
            key,
            CachedGlyph {
                bitmap: b.clone(),
                bytes,
                last_use: self.clock,
            },
        );
        b
    }

    /// Drops the least recently used entries, down to three quarters of
    /// the limits, when the cache is over one of them. Entries used by the
    /// current render stay.
    fn trim(&mut self) {
        if self.bytes <= self.budget && self.glyphs.len() <= MAX_GLYPHS {
            return;
        }
        let mut ages: Vec<(u64, GlyphKey)> = self
            .glyphs
            .iter()
            .filter(|(_, e)| e.last_use < self.clock)
            .map(|(k, e)| (e.last_use, k.clone()))
            .collect();
        ages.sort_unstable_by_key(|(age, _)| *age);
        for (_, key) in ages {
            if self.bytes <= self.budget / 4 * 3 && self.glyphs.len() <= MAX_GLYPHS / 4 * 3 {
                break;
            }
            if let Some(e) = self.glyphs.remove(&key) {
                self.bytes -= e.bytes;
            }
        }
    }
}

/// The first stop's colour, for colour-glyph layers that use the text
/// colour.
fn stop_rgb(stops: &[GradientStop]) -> [u8; 3] {
    stops
        .first()
        .map_or([255; 3], |s| [s.color[0], s.color[1], s.color[2]])
}

/// The rest-pose rectangle a gradient spans for a glyph on `line`.
fn extent_rect(extent: FillExtent, info: &TextLayout, line: u32) -> Rect {
    let block = Rect::new(0.0, 0.0, info.size.0, info.size.1);
    match extent {
        FillExtent::Block => block,
        FillExtent::Line => info.lines.get(line as usize).map_or(block, |l| {
            Rect::new(l.left, l.top, l.left + l.width, l.bottom)
        }),
    }
}

/// (dx, dy) of an offset of `distance` px at `angle_deg` clockwise from the
/// right.
fn offset(distance: f32, angle_deg: f32) -> (f32, f32) {
    let a = finite(angle_deg, 45.0).to_radians();
    let d = finite(distance, 0.0);
    (d * a.cos(), d * a.sin())
}

/// How far (image px) effects reach beyond the shapes that cast them.
fn effect_margin(spec: &TextSpec, px_em: f32) -> f32 {
    let outline = spec
        .stroke
        .filter(|s| s.width > 0.0 && s.opacity > 0.0)
        .map_or(0.0, |s| {
            let w = s.width.clamp(0.0, 0.3) * px_em;
            match s.position {
                StrokePosition::Outside => w,
                StrokePosition::Center => w / 2.0,
                StrokePosition::Inside => 0.0,
            }
        });
    let shadow = spec.shadow.filter(|s| s.opacity > 0.0).map_or(0.0, |s| {
        s.distance.clamp(0.0, 1.0) * px_em + blur_reach(s.blur.clamp(0.0, 0.5) * px_em) + outline
    });
    // As the outward glow is made: the shapes grown by a third of the
    // radius (not capped), then blurred over two thirds (capped).
    let glow = spec.glow.filter(|g| g.intensity > 0.0).map_or(0.0, |g| {
        let radius = g.radius.clamp(0.0, 1.0) * px_em;
        g.offset.clamp(0.0, 0.5) * px_em + radius / 3.0 + blur_reach(radius * 2.0 / 3.0) + outline
    });
    let m = outline.max(shadow).max(glow);
    if m.is_finite() { m } else { 0.0 }
}

/// What [`GlyphRasterizer::render`] draws, in layout px, estimated without
/// rasterizing a glyph: the text box, each glyph's em box around its pen
/// (on a curve, a square that holds it at any angle), the decorations and
/// the box or bubble with its tail. Effects are added by [`image_extent`].
pub(super) fn drawn_bounds(built: &Built, spec: &TextSpec, info: &TextLayout) -> Rect {
    let curve = Curve::new(built, spec.curve_amount());
    let mut bounds = Rect::new(0.0, 0.0, info.size.0.max(0.0), info.size.1.max(0.0));
    let mut add = |r: Rect| {
        if [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) {
            bounds = bounds.union(&r);
        }
    };
    for g in &built.items {
        let size = g.size.max(0.0);
        let advance = g.advance.max(0.0);
        add(match &curve {
            Some(arc) => {
                let ((x, y), _) = arc.place(g.unit.line, g.x, g.advance);
                let half = advance + 1.5 * size;
                Rect::new(x - half, y - half, x + half, y + half)
            }
            None => Rect::new(
                g.x - 0.5 * size,
                g.y - 1.5 * size,
                g.x + advance + 0.5 * size,
                g.y + size,
            ),
        });
    }
    for (_, path) in decoration_paths(built, spec, curve.as_ref()) {
        if let Some(b) = path.bounds() {
            add(b);
        }
    }
    if curve.is_none()
        && let Some(b) = backdrop(built, spec, info, spec.size()).and_then(|d| d.path.bounds())
    {
        add(b);
    }
    bounds
}

/// The image size (px) [`GlyphRasterizer::render`] makes at `scale` for
/// `bounds` from [`drawn_bounds`]: the bounds scaled, plus the effects'
/// margin (blurs capped in image px) on every side.
pub(super) fn image_extent(spec: &TextSpec, bounds: Rect, scale: f32) -> (f32, f32) {
    let margin = 2.0 * (effect_margin(spec, spec.size() * scale) + 2.0) + 2.0;
    (
        bounds.width() * scale + margin,
        bounds.height() * scale + margin,
    )
}

/// Underline and strike paths in layout px, with the line they belong to.
fn decoration_paths(built: &Built, spec: &TextSpec, curve: Option<&Curve>) -> Vec<(u32, Path)> {
    let mut out = Vec::new();
    for d in &built.decorations {
        let strokes = [
            (spec.decoration.underline, d.underline),
            (spec.decoration.strike, d.strike),
        ];
        for (on, (top, thick)) in strokes {
            if !on || thick <= 0.0 {
                continue;
            }
            let mut path = Path::new();
            match curve {
                None => path.push(rounded_rect(Rect::new(d.x0, top, d.x1, top + thick), 0.0)),
                Some(arc) => {
                    let (dy0, dy1) = (top - d.baseline, top + thick - d.baseline);
                    let radius = arc.radius(d.line);
                    // About one segment per degree of arc.
                    let steps =
                        (((d.x1 - d.x0) / radius).to_degrees().ceil() as usize).clamp(2, 720);
                    let mut contour = Vec::with_capacity(2 * steps + 2);
                    for k in 0..=steps {
                        let x = d.x0 + (d.x1 - d.x0) * k as f32 / steps as f32;
                        let (px, py) = arc.point(d.line, x, dy0);
                        contour.push(Point::new(px, py));
                    }
                    for k in (0..=steps).rev() {
                        let x = d.x0 + (d.x1 - d.x0) * k as f32 / steps as f32;
                        let (px, py) = arc.point(d.line, x, dy1);
                        contour.push(Point::new(px, py));
                    }
                    path.push(contour);
                }
            }
            out.push((d.line, path));
        }
    }
    out
}

/// The background box or bubble in layout px (TX-013, TX-055). A bubble
/// wins when both are set (TX-054). `None` when neither applies.
fn backdrop(built: &Built, spec: &TextSpec, info: &TextLayout, em: f32) -> Option<Backdrop> {
    let text = Rect::new(0.0, 0.0, info.size.0, info.size.1);
    if let Some(b) = spec.bubble.filter(|b| b.opacity > 0.0) {
        let pad = (b.pad_x.clamp(0.0, 1.0) * em, b.pad_y.clamp(0.0, 1.0) * em);
        let body = bubble_body(b.spec.kind, text, pad, em);
        let path = bubble_path(&b.spec, body, b.roundness);
        return Some(Backdrop {
            path,
            faded: Vec::new(),
            color: b.fill,
            opacity: b.opacity.clamp(0.0, 1.0),
            border: b.border.map(|br| (br.color, br.width.clamp(0.0, 0.2) * em)),
        });
    }
    let bx = spec.background.filter(|b| b.opacity > 0.0)?;
    let (px, py) = (bx.pad_x.clamp(0.0, 1.0) * em, bx.pad_y.clamp(0.0, 1.0) * em);
    let roundness = bx.roundness.clamp(0.0, 1.0);
    let corner = |r: Rect| roundness * r.width().min(r.height()) / 2.0;
    let band = |line: usize, x0: f32, x1: f32| band(built, info, &bx, line, x0, x1, (px, py), em);
    let mut path = Path::new();
    match bx.shape {
        BoxShape::Block => {
            let r = text.outset(px, py);
            path.push(rounded_rect(r, corner(r)));
        }
        BoxShape::PerLine => {
            for (i, l) in info.lines.iter().enumerate() {
                if l.width > 0.0
                    && let Some(r) = band(i, l.left, l.left + l.width)
                {
                    path.push(rounded_rect(r, corner(r)));
                }
            }
        }
        BoxShape::PerChar => {
            for c in &built.clusters {
                if c.x1 > c.x0
                    && let Some(r) = band(c.line as usize, c.x0, c.x1)
                {
                    path.push(rounded_rect(r, corner(r)));
                }
            }
        }
    }
    (!path.is_empty()).then_some(Backdrop {
        path,
        faded: Vec::new(),
        color: bx.color,
        opacity: bx.opacity.clamp(0.0, 1.0),
        border: bx.border.map(|b| (b.color, b.width.clamp(0.0, 0.2) * em)),
    })
}

/// The box of `line` from `x0` to `x1` (layout px) with the padding
/// `(px, py)`, `None` when empty.
#[allow(clippy::too_many_arguments)]
fn band(
    built: &Built,
    info: &TextLayout,
    bx: &TextBox,
    line: usize,
    x0: f32,
    x1: f32,
    (px, py): (f32, f32),
    em: f32,
) -> Option<Rect> {
    let l = info.lines.get(line)?;
    let r = match bx.height {
        BoxHeight::Full => Rect::new(x0, l.top, x1, l.bottom).outset(px, py),
        BoxHeight::LowerHalf => {
            let xh = built.x_heights.get(line).copied().unwrap_or(0.5 * em);
            Rect::new(
                x0,
                l.baseline - xh / 2.0,
                x1,
                l.baseline + LOWER_HALF_DROP_EM * em,
            )
            .outset(px, 0.0)
        }
    };
    (!r.is_empty()).then_some(r)
}

fn rasterize_glyph(
    context: &mut ScaleContext,
    data: &[u8],
    key: &GlyphKey,
    size: f32,
    offset_x: f32,
    skew: f32,
    angle: f32,
) -> Option<GlyphBitmap> {
    let font = FontRef::from_index(data, key.index as usize)?;
    let mut scaler = context
        .builder(font)
        .size(size)
        .hint(false)
        .normalized_coords(key.coords.iter())
        .build();
    let sources = [
        Source::ColorOutline(0),
        Source::ColorBitmap(StrikeWith::BestFit),
        Source::Outline,
    ];
    let mut render = Render::new(&sources);
    let [r, g, b] = key.foreground;
    render
        .format(Format::Alpha)
        .offset(Vector::new(offset_x, 0.0))
        .default_color([r, g, b, 255]);
    if key.embolden {
        render.embolden(size * EMBOLDEN_EM);
    }
    let mut transform: Option<Transform> = None;
    if skew != 0.0 {
        transform = Some(Transform::skew(
            Angle::from_degrees(skew),
            Angle::from_degrees(0.0),
        ));
    }
    if angle != 0.0 {
        // Outlines are y up: a clockwise turn on screen is negative there.
        let turn = Transform::rotation(Angle::from_radians(-angle));
        transform = Some(transform.map_or(turn, |t| t.then(&turn)));
    }
    if transform.is_some() {
        render.transform(transform);
    }
    let image = render.render(&mut scaler, key.glyph)?;
    let premultiplied = matches!(image.source, Source::ColorOutline(_));
    let color = match image.content {
        Content::Mask => false,
        Content::Color => true,
        // Only produced for `Format::Subpixel`, which isn't requested.
        Content::SubpixelMask => return None,
    };
    let expected = image.placement.width as usize
        * image.placement.height as usize
        * if color { 4 } else { 1 };
    if image.data.len() < expected {
        return None;
    }
    Some(GlyphBitmap {
        left: image.placement.left,
        top: image.placement.top,
        width: image.placement.width,
        height: image.placement.height,
        color,
        premultiplied: color && premultiplied,
        data: image.data,
    })
}

/// Rounded `a * b / 255` for bytes.
fn mul255(a: u32, b: u32) -> u32 {
    let x = a * b + 128;
    (x + (x >> 8)) >> 8
}

/// Adds a monochrome glyph's coverage into `dst` (saturating).
fn add_coverage(dst: &mut [u8], w: usize, h: usize, glyph: &GlyphBitmap, gx: i32, gy: i32) {
    for row in 0..glyph.height as i32 {
        let y = gy + row;
        if y < 0 || y as usize >= h {
            continue;
        }
        for col in 0..glyph.width as i32 {
            let x = gx + col;
            if x < 0 || x as usize >= w {
                continue;
            }
            let s = (row as usize) * glyph.width as usize + col as usize;
            let d = y as usize * w + x as usize;
            dst[d] = dst[d].saturating_add(glyph.data[s]);
        }
    }
}

/// Draws one colour glyph (premultiplied "over") into the premultiplied
/// buffer, at the fill's alpha `fill_a` (0..=255).
fn composite_glyph(
    dst: &mut [u8],
    w: usize,
    h: usize,
    glyph: &GlyphBitmap,
    gx: i32,
    gy: i32,
    fill_a: u32,
) {
    for row in 0..glyph.height as i32 {
        let y = gy + row;
        if y < 0 || y as usize >= h {
            continue;
        }
        for col in 0..glyph.width as i32 {
            let x = gx + col;
            if x < 0 || x as usize >= w {
                continue;
            }
            let s = (row as usize) * glyph.width as usize + col as usize;
            let px = &glyph.data[s * 4..s * 4 + 4];
            let sa = mul255(u32::from(px[3]), fill_a);
            // Premultiplied data already carries its alpha.
            let weight = if glyph.premultiplied { fill_a } else { sa };
            let sc = [
                mul255(u32::from(px[0]), weight).min(sa),
                mul255(u32::from(px[1]), weight).min(sa),
                mul255(u32::from(px[2]), weight).min(sa),
            ];
            if sa == 0 {
                continue;
            }
            let d = (y as usize * w + x as usize) * 4;
            let inv = 255 - sa;
            for c in 0..3 {
                dst[d + c] = (sc[c] + mul255(u32::from(dst[d + c]), inv)).min(255) as u8;
            }
            dst[d + 3] = (sa + mul255(u32::from(dst[d + 3]), inv)).min(255) as u8;
        }
    }
}

/// A per-unit animation evaluated for the layout being drawn.
pub(super) struct Animated<'a> {
    pub state: &'a MotionState,
    pub map: &'a UnitMap,
}

/// Where a glyph is drawn under motion: rest layout px → drawn layout px,
/// its opacity and its revealed part.
#[derive(Debug, Clone, Copy)]
pub(super) struct Pose {
    pub map: Homography,
    pub alpha: f32,
    pub shown: Shown,
    /// Word rise's slot: the line box the glyph is clipped to.
    pub slot: Option<LineSlot>,
}

/// A line's box at rest that a moving glyph is clipped to (TX-056's Word
/// rise, [`GlyphAnim::line_clip`]): drawn layout px map back into the
/// text's flat frame (before a curve bends it, TX-016) by `to_flat`, where
/// ink between `top` and `bottom` shows.
#[derive(Debug, Clone, Copy)]
pub(super) struct LineSlot {
    top: f32,
    bottom: f32,
    to_flat: ToRest,
}

impl LineSlot {
    /// The slot of line `line` for a glyph whose rest pen is `pen`, placed
    /// on a curve by `frame` (`None` when straight).
    fn new(
        built: &Built,
        line: u32,
        pen: (f32, f32),
        frame: Option<((f32, f32), f32)>,
    ) -> Option<LineSlot> {
        let l = built.lines.get(line as usize)?;
        Some(LineSlot {
            top: l.top,
            bottom: l.bottom,
            to_flat: match frame {
                // The curve's frame is rigid: drawn = at + R(angle)·(flat −
                // pen), so flat = pen + R(−angle)·(drawn − at).
                Some((at, angle)) => ToRest::rigid(pen, at, angle),
                None => ToRest::Map(Homography::IDENTITY),
            },
        })
    }
}

/// The point a unit moves about (TX-023: a letter's ink-box centre, a
/// word's or line's box centre) and the eye distance of its 3D turns (1.5 ×
/// its width, FX-030), layout px.
#[derive(Debug, Clone, Copy)]
struct Pivot {
    centre: (f64, f64),
    eye: f64,
}

/// Source raster scales for resampled glyphs are rounded up to this step
/// (of the output scale), so nearby frames share glyph bitmaps.
const RESAMPLE_SCALE_STEP: f32 = 8.0;

/// Blur radii (image px) below this draw sharp.
const MIN_GLYPH_BLUR: f32 = 0.5;

impl GlyphRasterizer {
    /// The glyphs (and, under motion, decoration pieces and the caret)
    /// placed for `motion`, and each layout glyph's pose (`None` when it
    /// is hidden by a reveal).
    fn place_animated(
        &mut self,
        built: &Built,
        spec: &TextSpec,
        curve: Option<&Curve>,
        scale: f32,
        foreground: [u8; 3],
        motion: &Animated<'_>,
    ) -> (Vec<Placed>, Vec<Option<Pose>>) {
        self.clock += 1;
        let em = spec.size();
        let state = motion.state;
        let mut pivots: Vec<Option<Pivot>> = vec![None; motion.map.count as usize];
        let mut placed = Vec::with_capacity(built.items.len());
        let mut poses = Vec::with_capacity(built.items.len());
        for (i, g) in built.items.iter().enumerate() {
            let Some(st) = state.glyphs.get(i).copied() else {
                poses.push(None);
                continue;
            };
            if st.shown.is_none() {
                poses.push(None);
                continue;
            }
            let anim = st.anim;
            let pivot = if anim.is_translation() {
                None
            } else {
                let u = st.unit as usize;
                match pivots.get(u).copied().flatten() {
                    Some(p) => Some(p),
                    None => {
                        let p = self.pivot(built, motion.map, st.unit, scale, foreground, em);
                        if let Some(slot) = pivots.get_mut(u) {
                            *slot = Some(p);
                        }
                        Some(p)
                    }
                }
            };
            let frame = curve.map(|arc| arc.place(g.unit.line, g.x, g.advance));
            let map = glyph_map(&anim, pivot, em, (g.x, g.y), frame);
            let slot = if anim.line_clip {
                LineSlot::new(built, g.unit.line, (g.x, g.y), frame)
            } else {
                None
            };
            poses.push(Some(Pose {
                map,
                alpha: anim.opacity,
                shown: st.shown,
                slot,
            }));
            if anim.opacity <= 0.0 {
                continue;
            }
            // Decode draws another letter of the text centred in this
            // glyph's advance.
            let (item, dx) = match st.stand_in.and_then(|j| built.items.get(j as usize)) {
                Some(other) => (other, (g.advance - other.advance) / 2.0),
                None => (g, 0.0),
            };
            let rest = (g.x + dx, g.y);
            let drawn = if anim.is_translation() {
                let Some(pen) = map.apply((f64::from(rest.0), f64::from(rest.1))) else {
                    continue;
                };
                let pen = (pen.0 as f32, pen.1 as f32);
                let angle = frame.map_or(0.0, |(_, a)| a);
                let (x, sub) = subpixel(pen.0 * scale);
                let Some(bitmap) = self.bitmap(item, scale, sub, angle, foreground) else {
                    continue;
                };
                Placed {
                    bitmap,
                    x,
                    y: (pen.1 * scale).round() as i32,
                    line: g.unit.line,
                    to_rest: ToRest::rigid(rest, pen, angle),
                    alpha: anim.opacity,
                    tint: anim.tint,
                    hue: anim.hue,
                    glow: anim.glow,
                }
            } else {
                let Some(p) =
                    self.resampled(item, rest, &map, &anim, scale, foreground, g.unit.line)
                else {
                    continue;
                };
                p
            };
            if let Some(p) = finish(drawn, &anim, st.shown, slot.as_ref(), em, scale) {
                placed.push(p);
            }
        }
        placed.extend(decoration_pieces(built, spec, &poses, scale, em, state));
        if let Some(caret) = state.caret
            && let Some(p) = caret_piece(built, curve, caret, scale)
        {
            placed.push(p);
        }
        self.trim();
        (placed, poses)
    }

    /// The pivot of timed unit `u`.
    fn pivot(
        &mut self,
        built: &Built,
        map: &UnitMap,
        u: u32,
        scale: f32,
        foreground: [u8; 3],
        em: f32,
    ) -> Pivot {
        let advance = map.boxes.get(u as usize).copied().flatten();
        let mut ink: Option<Rect> = None;
        if map.unit == AnimUnit::Grapheme {
            for i in map.glyphs_of(u) {
                let (Some(g), Some(false)) = (built.items.get(i), map.blank.get(i).copied()) else {
                    continue;
                };
                let Some(b) = self.bitmap(g, scale, 0, 0.0, foreground) else {
                    continue;
                };
                if b.width == 0 || b.height == 0 {
                    continue;
                }
                let x0 = g.x + b.left as f32 / scale;
                let y0 = g.y - b.top as f32 / scale;
                let r = Rect::new(
                    x0,
                    y0,
                    x0 + b.width as f32 / scale,
                    y0 + b.height as f32 / scale,
                );
                ink = Some(ink.map_or(r, |i| i.union(&r)));
            }
        }
        let first = map.glyphs_of(u).next().and_then(|i| built.items.get(i));
        let centre = match (ink, advance) {
            (Some(r), _) | (None, Some(r)) => r.center(),
            (None, None) => first.map_or(Point::new(0.0, 0.0), |g| Point::new(g.x, g.y)),
        };
        let width = advance.or(ink).map_or(0.0, |r| r.width()).max(0.5 * em);
        Pivot {
            centre: (f64::from(centre.x), f64::from(centre.y)),
            eye: 1.5 * f64::from(width),
        }
    }

    /// A glyph rasterized flat and resampled through `map` (scale,
    /// rotation, 3D turn).
    #[allow(clippy::too_many_arguments)]
    fn resampled(
        &mut self,
        item: &GlyphItem,
        rest: (f32, f32),
        map: &Homography,
        anim: &GlyphAnim,
        scale: f32,
        foreground: [u8; 3],
        line: u32,
    ) -> Option<Placed> {
        let grow = (anim.scale * anim.scale_xy.0)
            .abs()
            .max((anim.scale * anim.scale_xy.1).abs());
        let step =
            ((grow.clamp(1.0 / 16.0, 16.0) * RESAMPLE_SCALE_STEP).ceil()) / RESAMPLE_SCALE_STEP;
        let source_scale = scale * step;
        let flat = self.bitmap(item, source_scale, 0, 0.0, foreground)?;
        if flat.width == 0 || flat.height == 0 {
            return None;
        }
        let ss = f64::from(source_scale);
        // Source px → glyph-local layout px → rest layout px → drawn layout
        // px → image px.
        let to_image = Homography::scale(f64::from(scale), f64::from(scale))
            .mul(map)
            .mul(&Homography::translation(
                f64::from(rest.0),
                f64::from(rest.1),
            ))
            .mul(&Homography::scale(1.0 / ss, 1.0 / ss))
            .mul(&Homography::translation(
                f64::from(flat.left),
                -f64::from(flat.top),
            ));
        let channels = if flat.color { 4 } else { 1 };
        let data: std::borrow::Cow<'_, [u8]> = if flat.color && !flat.premultiplied {
            std::borrow::Cow::Owned(premultiply(&flat.data))
        } else {
            std::borrow::Cow::Borrowed(&flat.data)
        };
        let r = resample(&data, flat.width, flat.height, channels, &to_image)?;
        Some(Placed {
            bitmap: Arc::new(GlyphBitmap {
                left: r.x0,
                top: -r.y0,
                width: r.width,
                height: r.height,
                color: flat.color,
                premultiplied: flat.color,
                data: r.data,
            }),
            x: 0,
            y: 0,
            line,
            to_rest: ToRest::Map(map.inverse()?),
            alpha: anim.opacity,
            tint: anim.tint,
            hue: anim.hue,
            glow: anim.glow,
        })
    }
}

/// Image x of a pen at `px` image px: the whole pixel and the sub-pixel
/// step.
fn subpixel(px: f32) -> (i32, u8) {
    let mut x = px.floor();
    let mut sub = ((px - x) * SUBPIXEL_STEPS).round();
    if sub >= SUBPIXEL_STEPS {
        x += 1.0;
        sub = 0.0;
    }
    (x as i32, sub as u8)
}

/// Rest layout px → drawn layout px for a glyph whose rest pen is `pen`:
/// the unit's motion about its pivot (TX-023: offset, then rotation and
/// scale about the pivot, the 3D turn applied first, FX-030), then the
/// curve's frame for the glyph (`(pen, angle)` on the arc, TX-016).
fn glyph_map(
    anim: &GlyphAnim,
    pivot: Option<Pivot>,
    em: f32,
    pen: (f32, f32),
    frame: Option<((f32, f32), f32)>,
) -> Homography {
    let off = (f64::from(anim.offset.0 * em), f64::from(anim.offset.1 * em));
    let motion = match pivot {
        Some(p) if !anim.is_translation() => {
            let (cx, cy) = p.centre;
            let s = f64::from(anim.scale);
            Homography::translation(cx + off.0, cy + off.1)
                .mul(&Homography::rotation(f64::from(anim.rotation).to_radians()))
                .mul(&Homography::scale(
                    s * f64::from(anim.scale_xy.0),
                    s * f64::from(anim.scale_xy.1),
                ))
                .mul(&Homography::turn(
                    f64::from(anim.turn.0),
                    f64::from(anim.turn.1),
                    p.eye,
                ))
                .mul(&Homography::translation(-cx, -cy))
        }
        _ => Homography::translation(off.0, off.1),
    };
    match frame {
        Some((at, angle)) => Homography::translation(f64::from(at.0), f64::from(at.1))
            .mul(&Homography::rotation(f64::from(angle)))
            .mul(&Homography::translation(
                -f64::from(pen.0),
                -f64::from(pen.1),
            ))
            .mul(&motion),
        None => motion,
    }
}

/// A placed piece blurred, clipped to its revealed part and, in Word
/// rise's slot, to its line's box.
fn finish(
    mut p: Placed,
    anim: &GlyphAnim,
    shown: Shown,
    slot: Option<&LineSlot>,
    em: f32,
    scale: f32,
) -> Option<Placed> {
    let radius = anim.blur * em * scale;
    if radius >= MIN_GLYPH_BLUR {
        p.bitmap = Arc::new(blurred(&p.bitmap, radius));
    }
    if let Shown::Range(lo, hi) = shown {
        let mut b = (*p.bitmap).clone();
        clip_to(&mut b, p.x, p.y, &p.to_rest, (lo, hi), scale);
        p.bitmap = Arc::new(b);
    }
    if let Some(slot) = slot {
        let mut b = (*p.bitmap).clone();
        clip_to_line(&mut b, p.x, p.y, slot, scale);
        p.bitmap = Arc::new(b);
    }
    (p.bitmap.width > 0 && p.bitmap.height > 0).then_some(p)
}

/// Keeps the ink of `b` (placed at `(x, y)`) inside `slot`'s line box,
/// with a one-pixel soft edge.
fn clip_to_line(b: &mut GlyphBitmap, x: i32, y: i32, slot: &LineSlot, scale: f32) {
    let channels = if b.color { 4 } else { 1 };
    let straight = b.color && !b.premultiplied;
    for row in 0..b.height as usize {
        for col in 0..b.width as usize {
            let l = (
                (x + b.left + col as i32) as f32 + 0.5,
                (y - b.top + row as i32) as f32 + 0.5,
            );
            let q = slot.to_flat.at((l.0 / scale, l.1 / scale));
            let keep = if q.1.is_finite() {
                ((slot.bottom - q.1) * scale + 0.5).clamp(0.0, 1.0)
                    * ((q.1 - slot.top) * scale + 0.5).clamp(0.0, 1.0)
            } else {
                0.0
            };
            if keep >= 1.0 {
                continue;
            }
            let i = (row * b.width as usize + col) * channels;
            let first = if straight { 3 } else { 0 };
            for v in &mut b.data[i + first..i + channels] {
                *v = (f32::from(*v) * keep).round() as u8;
            }
        }
    }
}

/// Straight RGBA → premultiplied.
fn premultiply(data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    for px in out.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        for c in &mut px[..3] {
            *c = mul255(u32::from(*c), a) as u8;
        }
    }
    out
}

/// `b` blurred by a Gaussian of `radius` image px (premultiplied for
/// colour), grown by the blur's reach.
fn blurred(b: &GlyphBitmap, radius: f32) -> GlyphBitmap {
    let pad = blur_reach(radius) as usize;
    let (w, h) = (b.width as usize, b.height as usize);
    let (bw, bh) = (w + 2 * pad, h + 2 * pad);
    let channels = if b.color { 4 } else { 1 };
    let source: std::borrow::Cow<'_, [u8]> = if b.color && !b.premultiplied {
        std::borrow::Cow::Owned(premultiply(&b.data))
    } else {
        std::borrow::Cow::Borrowed(&b.data)
    };
    let mut data = vec![0u8; bw * bh * channels];
    for k in 0..channels {
        let mut plane = vec![0.0f32; bw * bh];
        for y in 0..h {
            for x in 0..w {
                plane[(y + pad) * bw + x + pad] = f32::from(source[(y * w + x) * channels + k]);
            }
        }
        let soft = blur(&plane, bw, bh, radius, 0.0);
        for (i, v) in soft.iter().enumerate() {
            data[i * channels + k] = v.round().clamp(0.0, 255.0) as u8;
        }
    }
    GlyphBitmap {
        left: b.left - pad as i32,
        top: b.top + pad as i32,
        width: bw as u32,
        height: bh as u32,
        color: b.color,
        premultiplied: b.color,
        data,
    }
}

/// Keeps the ink of `b` (placed at `(x, y)`) whose rest x lies in `range`,
/// with a one-pixel soft edge.
fn clip_to(b: &mut GlyphBitmap, x: i32, y: i32, to_rest: &ToRest, range: (f32, f32), scale: f32) {
    let channels = if b.color { 4 } else { 1 };
    let straight = b.color && !b.premultiplied;
    for row in 0..b.height as usize {
        for col in 0..b.width as usize {
            let l = (
                (x + b.left + col as i32) as f32 + 0.5,
                (y - b.top + row as i32) as f32 + 0.5,
            );
            let q = to_rest.at((l.0 / scale, l.1 / scale));
            let keep = if q.0.is_finite() {
                ((range.1 - q.0) * scale + 0.5).clamp(0.0, 1.0)
                    * ((q.0 - range.0) * scale + 0.5).clamp(0.0, 1.0)
            } else {
                0.0
            };
            if keep >= 1.0 {
                continue;
            }
            let i = (row * b.width as usize + col) * channels;
            let first = if straight { 3 } else { 0 };
            for v in &mut b.data[i + first..i + channels] {
                *v = (f32::from(*v) * keep).round() as u8;
            }
        }
    }
}

/// A coverage bitmap of the polygon `points` (image px), positioned so a
/// [`Placed`] at (0, 0) draws it there.
fn polygon_bitmap(points: &[Point]) -> Option<GlyphBitmap> {
    let mut path = Path::new();
    path.push(points.to_vec());
    let b = path.bounds()?;
    if !(b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()) {
        return None;
    }
    let x0 = b.x0.floor() as i32;
    let y0 = b.y0.floor() as i32;
    let w = (b.x1.ceil() as i32 - x0).clamp(0, MAX_RESAMPLED_SIDE as i32) as usize;
    let h = (b.y1.ceil() as i32 - y0).clamp(0, MAX_RESAMPLED_SIDE as i32) as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let mut mask = Coverage::new(w, h);
    fill_union(
        &mut mask,
        &path.map(|p| Point::new(p.x - x0 as f32, p.y - y0 as f32)),
    );
    Some(GlyphBitmap {
        left: x0,
        top: -y0,
        width: w as u32,
        height: h as u32,
        color: false,
        premultiplied: false,
        data: mask.data,
    })
}

/// The rest rectangle `r` (layout px) through `map`, in image px.
fn mapped_quad(r: Rect, map: &Homography, scale: f32) -> Option<Vec<Point>> {
    let s = f64::from(scale);
    [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)]
        .iter()
        .map(|&(x, y)| {
            map.apply((f64::from(x), f64::from(y)))
                .map(|(x, y)| Point::new((x * s) as f32, (y * s) as f32))
        })
        .collect()
}

/// Underline and strike under motion: one piece per glyph advance, moved,
/// faded, blurred and revealed with its glyph.
fn decoration_pieces(
    built: &Built,
    spec: &TextSpec,
    poses: &[Option<Pose>],
    scale: f32,
    em: f32,
    state: &MotionState,
) -> Vec<Placed> {
    let mut out = Vec::new();
    for d in &built.decorations {
        let strokes = [
            (spec.decoration.underline, d.underline),
            (spec.decoration.strike, d.strike),
        ];
        for (on, (top, thick)) in strokes {
            if !on || thick <= 0.0 {
                continue;
            }
            for (i, g) in built.items.iter().enumerate() {
                if g.unit.line != d.line {
                    continue;
                }
                let (Some(Some(pose)), Some(st)) = (poses.get(i), state.glyphs.get(i)) else {
                    continue;
                };
                if pose.alpha <= 0.0 {
                    continue;
                }
                let (mut a, mut b) = (d.x0.max(g.x), d.x1.min(g.x + g.advance));
                if let Shown::Range(lo, hi) = pose.shown {
                    a = a.max(lo);
                    b = b.min(hi);
                }
                if b <= a {
                    continue;
                }
                let Some(quad) = mapped_quad(Rect::new(a, top, b, top + thick), &pose.map, scale)
                else {
                    continue;
                };
                let (Some(bitmap), Some(to_rest)) = (polygon_bitmap(&quad), pose.map.inverse())
                else {
                    continue;
                };
                let piece = Placed {
                    bitmap: Arc::new(bitmap),
                    x: 0,
                    y: 0,
                    line: d.line,
                    to_rest: ToRest::Map(to_rest),
                    alpha: pose.alpha,
                    tint: st.anim.tint,
                    hue: st.anim.hue,
                    glow: st.anim.glow,
                };
                // Already cut to the revealed part.
                if let Some(p) = finish(piece, &st.anim, Shown::All, pose.slot.as_ref(), em, scale)
                {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Typewriter's caret bar as a placed piece (on the arc when curved).
fn caret_piece(
    built: &Built,
    curve: Option<&Curve>,
    caret: CaretBar,
    scale: f32,
) -> Option<Placed> {
    let line = built.lines.get(caret.line as usize)?;
    let (x0, x1) = (caret.x, caret.x + caret.width);
    let corners = [
        (x0, line.top),
        (x1, line.top),
        (x1, line.bottom),
        (x0, line.bottom),
    ];
    let quad: Vec<Point> = corners
        .iter()
        .map(|&(x, y)| {
            let (x, y) = match curve {
                Some(arc) => arc.point(caret.line, x, y - line.baseline),
                None => (x, y),
            };
            Point::new(x * scale, y * scale)
        })
        .collect();
    let to_rest = match curve {
        // The caret is thin: its pixels map back through the arc's frame
        // at its middle.
        Some(arc) => {
            let ((px, py), angle) = arc.place(caret.line, x0, caret.width);
            ToRest::rigid((x0, line.baseline), (px, py), angle)
        }
        None => ToRest::Map(Homography::IDENTITY),
    };
    Some(Placed {
        bitmap: Arc::new(polygon_bitmap(&quad)?),
        x: 0,
        y: 0,
        line: caret.line,
        to_rest,
        alpha: 1.0,
        tint: 0.0,
        hue: 0.0,
        glow: 1.0,
    })
}

/// Calls `f(pixel, coverage)` for every pixel of image `w × h` (origin
/// `(x0, y0)`) that piece `p` covers: its monochrome coverage, or a colour
/// glyph's alpha, 0..=1.
fn each_covered(p: &Placed, w: usize, h: usize, x0: i32, y0: i32, mut f: impl FnMut(usize, f32)) {
    let b = &p.bitmap;
    let (gx, gy) = (p.x + b.left - x0, p.y - b.top - y0);
    for row in 0..b.height as i32 {
        let y = gy + row;
        if y < 0 || y as usize >= h {
            continue;
        }
        for col in 0..b.width as i32 {
            let x = gx + col;
            if x < 0 || x as usize >= w {
                continue;
            }
            let s = row as usize * b.width as usize + col as usize;
            let cov = if b.color {
                b.data[s * 4 + 3]
            } else {
                b.data[s]
            };
            if cov > 0 {
                f(y as usize * w + x as usize, f32::from(cov) / 255.0);
            }
        }
    }
}

/// How [`spread`] sums up a block of inked pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reduce {
    /// The weighted mean.
    Mean,
    /// The value of its most weighted pixel (values that must not be
    /// averaged, such as angles).
    Dominant,
}

/// `own`, valid where `weight` is positive (inked pixels), spread to the
/// other pixels (where outline, glow and shadow reach): each takes the
/// value of the nearest inked block of 4 × 4 pixels. Values change only
/// from glyph to glyph, so blocks are fine and the distance transform runs
/// on a sixteenth of the pixels. `None` without ink.
fn spread(own: &[f32], weight: &[f32], w: usize, h: usize, reduce: Reduce) -> Option<Vec<f32>> {
    const BLOCK: usize = 4;
    if !weight.iter().any(|&k| k > 0.0) {
        return None;
    }
    let (bw, bh) = (w.div_ceil(BLOCK), h.div_ceil(BLOCK));
    let mut acc = vec![0.0f32; bw * bh];
    let mut block_weight = vec![0.0f32; bw * bh];
    for y in 0..h {
        for x in 0..w {
            let (i, b) = (y * w + x, (y / BLOCK) * bw + x / BLOCK);
            let k = weight[i];
            if k <= 0.0 {
                continue;
            }
            match reduce {
                Reduce::Mean => {
                    acc[b] += own[i] * k;
                    block_weight[b] += k;
                }
                Reduce::Dominant if k > block_weight[b] => {
                    acc[b] = own[i];
                    block_weight[b] = k;
                }
                Reduce::Dominant => {}
            }
        }
    }
    let inked: Vec<bool> = block_weight.iter().map(|&v| v > 0.0).collect();
    let (_, nearest) = edt(&inked, bw, bh);
    let value: Vec<f32> = acc
        .iter()
        .zip(&block_weight)
        .map(|(&a, &k)| match reduce {
            Reduce::Mean if k > 0.0 => a / k,
            Reduce::Mean => 0.0,
            Reduce::Dominant => a,
        })
        .collect();
    Some(
        (0..w * h)
            .map(|i| {
                if weight[i] > 0.0 {
                    own[i]
                } else {
                    value[nearest[(i / w / BLOCK) * bw + (i % w) / BLOCK] as usize]
                }
            })
            .collect(),
    )
}

/// Per-pixel opacity of the drawn glyphs (image `w × h`, origin `(x0,
/// y0)`) as a factor on their merged coverage, `None` when every piece is
/// opaque. Pieces of one opacity add up as the coverage does (a unit's
/// own glyphs, joins); pieces of different opacities composite one over
/// the other, so a faint letter passing over an opaque one leaves it
/// opaque. Pixels without ink take the opacity of the nearest ink.
fn opacity_plane(placed: &[Placed], w: usize, h: usize, x0: i32, y0: i32) -> Option<Vec<f32>> {
    if placed.iter().all(|p| p.alpha >= 1.0) {
        return None;
    }
    let n = w * h;
    let alpha = |p: &Placed| p.alpha.clamp(0.0, 1.0);
    let mut alphas: Vec<f32> = placed.iter().map(alpha).collect();
    alphas.sort_by(f32::total_cmp);
    alphas.dedup();
    // Π(1 − U·α) and Π(1 − U) over the groups, U a group's summed
    // coverage (at most 1).
    let mut clear_shown = vec![1.0f32; n];
    let mut clear = vec![1.0f32; n];
    let mut group = vec![0.0f32; n];
    for a in alphas {
        let mut touched = Vec::new();
        for p in placed.iter().filter(|p| alpha(p) == a) {
            each_covered(p, w, h, x0, y0, |i, k| {
                if group[i] == 0.0 {
                    touched.push(i);
                }
                group[i] += k;
            });
        }
        for i in touched {
            let u = group[i].min(1.0);
            clear_shown[i] *= 1.0 - u * a;
            clear[i] *= 1.0 - u;
            group[i] = 0.0;
        }
    }
    let weight: Vec<f32> = clear.iter().map(|c| 1.0 - c).collect();
    let own: Vec<f32> = clear_shown
        .iter()
        .zip(&weight)
        .map(|(c, &k)| {
            if k > 0.0 {
                ((1.0 - c) / k).min(1.0)
            } else {
                0.0
            }
        })
        .collect();
    spread(&own, &weight, w, h, Reduce::Mean)
}

/// Per-pixel tint of the drawn glyphs' fill (image `w × h`, origin `(x0,
/// y0)`): the tint of the monochrome pieces over each pixel, weighted by
/// coverage and opacity. `None` when no piece is tinted.
fn tint_plane(placed: &[Placed], w: usize, h: usize, x0: i32, y0: i32) -> Option<Vec<f32>> {
    if placed.iter().all(|p| p.tint.is_nan() || p.tint <= 0.0) {
        return None;
    }
    let n = w * h;
    let mut acc = vec![0.0f32; n];
    let mut weight = vec![0.0f32; n];
    for p in placed.iter().filter(|p| !p.bitmap.color) {
        let (tint, a) = (p.tint.clamp(0.0, 1.0), p.alpha.clamp(0.0, 1.0));
        each_covered(p, w, h, x0, y0, |i, k| {
            acc[i] += k * a * tint;
            weight[i] += k * a;
        });
    }
    Some(
        acc.iter()
            .zip(&weight)
            .map(|(&a, &k)| if k > 0.0 { a / k } else { 0.0 })
            .collect(),
    )
}

/// Per-pixel hue turn (degrees) of fill and outline (image `w × h`, origin
/// `(x0, y0)`): that of the piece covering each pixel most (by coverage
/// and opacity; angles are not averaged), spread to the outline around
/// the ink. `None` when no piece turns.
fn hue_plane(placed: &[Placed], w: usize, h: usize, x0: i32, y0: i32) -> Option<Vec<f32>> {
    if placed.iter().all(|p| !p.hue.is_finite() || p.hue == 0.0) {
        return None;
    }
    let n = w * h;
    let mut own = vec![0.0f32; n];
    let mut weight = vec![0.0f32; n];
    for p in placed {
        let (hue, a) = (finite(p.hue, 0.0), p.alpha.clamp(0.0, 1.0));
        each_covered(p, w, h, x0, y0, |i, k| {
            if k * a > weight[i] {
                own[i] = hue;
                weight[i] = k * a;
            }
        });
    }
    spread(&own, &weight, w, h, Reduce::Dominant)
}

/// Per-pixel glow factor (image `w × h`, origin `(x0, y0)`): the factors
/// of the pieces over each pixel weighted by coverage and opacity, spread
/// to where the glow reaches. `None` when every piece keeps the glow as
/// set.
fn glow_plane(placed: &[Placed], w: usize, h: usize, x0: i32, y0: i32) -> Option<Vec<f32>> {
    if placed.iter().all(|p| p.glow == 1.0) {
        return None;
    }
    let n = w * h;
    let mut acc = vec![0.0f32; n];
    let mut weight = vec![0.0f32; n];
    for p in placed {
        let (g, a) = (finite(p.glow, 1.0).max(0.0), p.alpha.clamp(0.0, 1.0));
        each_covered(p, w, h, x0, y0, |i, k| {
            acc[i] += k * a * g;
            weight[i] += k * a;
        });
    }
    let own: Vec<f32> = acc
        .iter()
        .zip(&weight)
        .map(|(&a, &k)| if k > 0.0 { a / k } else { 1.0 })
        .collect();
    spread(&own, &weight, w, h, Reduce::Mean)
}

/// The glow plane as factors on a glow of `intensity`: the glow's
/// intensity times the factor, at most full (as the whole-text loop's
/// modulated intensity is), over the set intensity.
fn glow_factors(plane: Option<&[f32]>, intensity: f32) -> Option<Vec<f32>> {
    let set = intensity.clamp(0.0, 1.0);
    let plane = plane.filter(|_| set > 0.0)?;
    Some(plane.iter().map(|&g| g.min(1.0 / set)).collect())
}

/// Premultiplied `c` with its HSL hue turned by `deg`, lightness,
/// saturation and alpha kept (Hue cycle, per glyph).
fn turned(c: Rgba, deg: f32) -> Rgba {
    let a = c[3];
    if a <= 0.0 {
        return c;
    }
    let (r, g, b) = (c[0] / a, c[1] / a, c[2] / a);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    if d <= 1e-6 {
        return c;
    }
    let l = (max + min) / 2.0;
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    let h = (h + deg).rem_euclid(360.0);
    // The chroma is kept: d = (1 − |2l − 1|)·s.
    let x = d * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - d / 2.0;
    let (r1, g1, b1) = match (h / 60.0) as u8 {
        0 => (d, x, 0.0),
        1 => (x, d, 0.0),
        2 => (0.0, d, x),
        3 => (0.0, x, d),
        4 => (x, 0.0, d),
        _ => (d, 0.0, x),
    };
    let ch = |v: f32| ((v + m).clamp(0.0, 1.0)) * a;
    [ch(r1), ch(g1), ch(b1), a]
}

/// The background box under motion (TX-013): per-character boxes move and
/// fade with their glyph; per-line and block boxes cover the glyphs
/// revealed so far, line by line, at rest. A glyph not yet visible
/// (opacity 0, as Typewriter types) is not revealed. A bubble is never
/// revealed per unit (TX-023).
fn animated_backdrop(
    built: &Built,
    spec: &TextSpec,
    info: &TextLayout,
    em: f32,
    poses: &[Option<Pose>],
) -> Option<Backdrop> {
    if spec.bubble.is_some_and(|b| b.opacity > 0.0) {
        return backdrop(built, spec, info, em);
    }
    let bx = spec.background.filter(|b| b.opacity > 0.0)?;
    // Each non-blank cluster's revealed advance span and its glyph's pose.
    let mut first_glyph: HashMap<u32, usize> = HashMap::new();
    for (i, g) in built.items.iter().enumerate() {
        first_glyph.entry(g.cluster).or_insert(i);
    }
    let mut complete = true;
    let mut spans: Vec<(u32, f32, f32, Homography, f32)> = Vec::new();
    for c in &built.clusters {
        let pose = first_glyph
            .get(&c.cluster)
            .and_then(|&i| poses.get(i).copied().flatten())
            .filter(|p| p.alpha > 0.0);
        let Some(pose) = pose else {
            complete = false;
            continue;
        };
        let (mut a, mut b) = (c.x0, c.x1);
        if let Shown::Range(lo, hi) = pose.shown {
            a = a.max(lo);
            b = b.min(hi);
            complete &= a <= c.x0 && b >= c.x1;
        }
        if b > a {
            spans.push((c.line, a, b, pose.map, pose.alpha.min(1.0)));
        } else {
            complete = false;
        }
    }
    let pad = (bx.pad_x.clamp(0.0, 1.0) * em, bx.pad_y.clamp(0.0, 1.0) * em);
    let roundness = bx.roundness.clamp(0.0, 1.0);
    let corner = |r: Rect| roundness * r.width().min(r.height()) / 2.0;
    let mut path = Path::new();
    let mut faded: Vec<(Path, f32)> = Vec::new();
    match bx.shape {
        BoxShape::Block if complete => return backdrop(built, spec, info, em),
        BoxShape::Block => {
            let mut rect: Option<Rect> = None;
            for &(line, a, b, _, _) in &spans {
                let Some(l) = info.lines.get(line as usize) else {
                    continue;
                };
                let r = Rect::new(a, l.top, b, l.bottom);
                rect = Some(rect.map_or(r, |q| q.union(&r)));
            }
            let r = rect?.outset(pad.0, pad.1);
            path.push(rounded_rect(r, corner(r)));
        }
        BoxShape::PerLine => {
            let mut lines: Vec<Option<(f32, f32)>> = vec![None; info.lines.len()];
            for &(line, a, b, _, _) in &spans {
                if let Some(slot) = lines.get_mut(line as usize) {
                    *slot = Some(slot.map_or((a, b), |(x, y)| (x.min(a), y.max(b))));
                }
            }
            for (i, span) in lines.iter().enumerate() {
                if let Some((a, b)) = span
                    && let Some(r) = band(built, info, &bx, i, *a, *b, pad, em)
                {
                    path.push(rounded_rect(r, corner(r)));
                }
            }
        }
        BoxShape::PerChar => {
            for &(line, a, b, map, alpha) in &spans {
                let Some(r) = band(built, info, &bx, line as usize, a, b, pad, em) else {
                    continue;
                };
                let moved: Option<Vec<Point>> = rounded_rect(r, corner(r))
                    .iter()
                    .map(|p| {
                        map.apply((f64::from(p.x), f64::from(p.y)))
                            .map(|(x, y)| Point::new(x as f32, y as f32))
                    })
                    .collect();
                if let Some(moved) = moved {
                    match faded.iter_mut().find(|(_, a)| *a == alpha) {
                        Some((group, _)) => group.push(moved.clone()),
                        None => {
                            let mut group = Path::new();
                            group.push(moved.clone());
                            faded.push((group, alpha));
                        }
                    }
                    path.push(moved);
                }
            }
            if faded.iter().all(|(_, a)| *a >= 1.0) {
                faded.clear();
            }
        }
    }
    (!path.is_empty()).then_some(Backdrop {
        path,
        faded,
        color: bx.color,
        opacity: bx.opacity.clamp(0.0, 1.0),
        border: bx.border.map(|b| (b.color, b.width.clamp(0.0, 0.2) * em)),
    })
}

/// Rest layout px that motion may move ink into, beyond the rest layout:
/// each glyph's em box around its moved pen, grown by its scale (and
/// doubled for 3D turns).
pub(super) fn motion_bounds(built: &Built, state: &MotionState, em: f32) -> Option<Rect> {
    let mut out: Option<Rect> = None;
    for (g, st) in built.items.iter().zip(&state.glyphs) {
        if st.shown.is_none() {
            continue;
        }
        let a = &st.anim;
        let size = g.size.max(0.0);
        let half = 0.5 * (g.advance.max(0.0) + size) + 1.5 * size;
        let grow = (a.scale * a.scale_xy.0)
            .abs()
            .max((a.scale * a.scale_xy.1).abs())
            .max(1.0)
            * if a.turn == (0.0, 0.0) { 1.0 } else { 2.0 };
        let reach = half * grow + a.blur * em * 3.0;
        let cx = g.x + g.advance / 2.0 + a.offset.0 * em;
        let cy = g.y - 0.25 * size + a.offset.1 * em;
        let r = Rect::new(cx - reach, cy - reach, cx + reach, cy + reach);
        if [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) {
            out = Some(out.map_or(r, |o| o.union(&r)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(premultiplied: bool, data: Vec<u8>) -> GlyphBitmap {
        GlyphBitmap {
            left: 0,
            top: 0,
            width: 1,
            height: 1,
            color: true,
            premultiplied,
            data,
        }
    }

    #[test]
    fn colr_pixels_are_premultiplied_bitmap_pixels_straight() {
        // Half-covered red: premultiplied (COLR) and straight (sbix) give
        // the same premultiplied pixel.
        for (g, label) in [
            (glyph(true, vec![128, 0, 0, 128]), "COLR"),
            (glyph(false, vec![255, 0, 0, 128]), "bitmap"),
        ] {
            let mut px = vec![0u8; 4];
            composite_glyph(&mut px, 1, 1, &g, 0, 0, 255);
            assert_eq!(px, [128, 0, 0, 128], "{label}");
        }
    }

    #[test]
    fn the_glyph_cache_stays_within_its_byte_budget_during_a_zoom() {
        use super::super::FontRegistry;
        use super::super::layout::Engine;
        let (_, collection) = FontRegistry::bundled().snapshot();
        let mut engine = Engine::new(collection);
        let spec = TextSpec {
            text: "abcdefghijklmnopqrstuvwxyz".into(),
            size_px: 100.0,
            ..TextSpec::new("")
        };
        let built = engine.build(&spec);
        let info = engine.describe(&built, &spec);
        let budget = 2 << 20;
        let mut glyphs = GlyphRasterizer {
            budget,
            ..GlyphRasterizer::default()
        };
        let held = |g: &GlyphRasterizer| -> usize {
            g.glyphs
                .values()
                .filter_map(|e| e.bitmap.as_ref())
                .map(|b| b.data.len())
                .sum()
        };
        // A zoom: a new raster scale, and new glyph bitmaps, every frame.
        for frame in 0..40 {
            let scale = 1.0 + frame as f32 / 40.0;
            glyphs.render(&built, &spec, scale, info.clone(), None);
            assert!(held(&glyphs) <= budget, "frame {frame}: {}", held(&glyphs));
            assert_eq!(
                glyphs.bytes,
                glyphs.glyphs.values().map(|e| e.bytes).sum::<usize>()
            );
        }
        // The last frame's glyphs are still cached.
        let entries = glyphs.glyphs.len();
        glyphs.render(&built, &spec, 1.0 + 39.0 / 40.0, info.clone(), None);
        assert_eq!(glyphs.glyphs.len(), entries);
    }

    #[test]
    fn coverage_adds_and_saturates() {
        let mut cov = vec![200u8, 10];
        let g = GlyphBitmap {
            left: 0,
            top: 0,
            width: 2,
            height: 1,
            color: false,
            premultiplied: false,
            data: vec![100, 20],
        };
        add_coverage(&mut cov, 2, 1, &g, 0, 0);
        assert_eq!(cov, [255, 30]);
    }

    #[test]
    fn gradients_interpolate_premultiplied_and_clamp() {
        let paint = Paint::new(&Fill::Linear(super::super::style::LinearGradient {
            angle_deg: 0.0,
            stops: vec![
                GradientStop {
                    at: 1.0,
                    color: [0, 0, 255, 255],
                },
                GradientStop {
                    at: 0.0,
                    color: [255, 0, 0, 255],
                },
            ],
            extent: FillExtent::Block,
        }))
        .unwrap();
        let r = Rect::new(0.0, 0.0, 100.0, 10.0);
        assert_eq!(paint.at((-5.0, 5.0), r), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(paint.at((200.0, 5.0), r), [0.0, 0.0, 1.0, 1.0]);
        let mid = paint.at((50.0, 5.0), r);
        assert!((mid[0] - 0.5).abs() < 1e-6 && (mid[2] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn offsets_follow_the_clockwise_angle() {
        let (dx, dy) = offset(10.0, 45.0);
        assert!(dx > 7.0 && dy > 7.0);
        let (dx, dy) = offset(10.0, 270.0);
        assert!(dx.abs() < 1e-4 && dy < -9.9);
    }
}
