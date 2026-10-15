//! Whole-layer animation effects (FX-030): blur, wipe and tint, as the
//! frame plan's `LayerMotion` asks for them.
//!
//! The wipe and the tint are per-pixel ([`super::draw::WipeMask`],
//! [`super::draw::Tint8`]) and drawn in the layer's own pass. The blur
//! needs the layer's neighbourhood: the layer (wipe and tint included) is
//! drawn alone into a premultiplied buffer in output pixels, every channel
//! is blurred with the text effects' separable Gaussian
//! ([`crate::text::effects::blur_reduced`]: power-of-two downsampling
//! above [`LAYER_BLUR_DOWNSAMPLE_ABOVE`] px), and the result replaces the
//! layer, placed 1:1. Rows and channels are split across threads, each
//! value computed by the same sequential arithmetic, so the bytes do not
//! depend on the thread count. Blurred layers are kept in a small cache
//! keyed by everything that shapes them (the source picture's identity,
//! the map, opacity, wipe, tint, radius and region), so a paused frame or
//! a layer whose motion has settled is blurred once.

use std::sync::Arc;

use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::model::Direction;
pub use freecut_timeline::plan::WIPE_FEATHER;
use freecut_timeline::plan::{LayerMotion, LayerPlan};

use super::Picture;
use super::draw::{Prepared, PreparedSource, Shader, Tint8, ToSource, WipeFrom, WipeMask};
use crate::text::effects::{blur_reach, blur_reduced};

/// Blur radii (output px) above this are blurred at a power-of-two
/// reduced resolution (a whole 1080p layer stays fast; the Gaussian is
/// wide enough there that the reduction is invisible).
pub const LAYER_BLUR_DOWNSAMPLE_ABOVE: f32 = 8.0;

/// Blur radii (output px) below this draw sharp.
const MIN_LAYER_BLUR: f64 = 0.5;

/// What a layer's animation asks of the whole layer besides its matrix
/// and opacity.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayerEffects {
    /// Gaussian blur radius in canvas px (the visible spread of an edge;
    /// σ is half of it, as for text blurs). 0: sharp.
    pub blur_px: f64,
    pub wipe: Option<Wipe>,
    pub tint: Option<Tint>,
}

/// A straight-edged reveal of the layer rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wipe {
    /// The share shown, 0..=1.
    pub shown: f64,
    /// The side the edge starts from (Left: from the left edge; Up: from
    /// the top). Rotations count as Left.
    pub from: Direction,
    /// The soft edge's width as a share of the layer's size along the
    /// wipe ([`WIPE_FEATHER`] for FX-030's preset; 0: a hard edge). The
    /// layer shows fully up to `shown·(1 + feather) − feather` and fades
    /// to nothing at `shown·(1 + feather)`, so `shown` 0 shows nothing and
    /// 1 everything.
    pub feather: f64,
}

/// The layer's colours moved toward `color` by `amount`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tint {
    /// Straight sRGB.
    pub color: [u8; 3],
    /// 0..=1: 0 leaves the layer as it is, 1 makes it `color` (coverage
    /// and alpha kept).
    pub amount: f64,
}

impl LayerEffects {
    /// Nothing to do.
    pub const NONE: LayerEffects = LayerEffects {
        blur_px: 0.0,
        wipe: None,
        tint: None,
    };

    /// The effects a frame plan's `LayerMotion` carries: its blur, its wipe
    /// with the plan's soft edge (`reveal_feather`, FX-030's 8 % while a
    /// wipe plays) and its white tint.
    pub fn from_motion(m: &LayerMotion) -> LayerEffects {
        let blur_px = if m.blur_px.is_finite() && m.blur_px > 0.0 {
            m.blur_px
        } else {
            0.0
        };
        let wipe = (m.reveal < 1.0 || m.reveal.is_nan()).then_some(Wipe {
            shown: if m.reveal.is_nan() {
                0.0
            } else {
                m.reveal.max(0.0)
            },
            from: m.reveal_from,
            feather: if m.reveal_feather.is_finite() {
                m.reveal_feather.clamp(0.0, 1.0)
            } else {
                0.0
            },
        });
        let tint = (m.tint > 0.0).then_some(Tint {
            color: [255, 255, 255],
            amount: m.tint.min(1.0),
        });
        LayerEffects {
            blur_px,
            wipe,
            tint,
        }
    }

    /// The effects of `layer` ([`LayerEffects::from_motion`], none
    /// without motion).
    pub fn of(layer: &LayerPlan) -> LayerEffects {
        layer
            .motion
            .as_deref()
            .map_or(LayerEffects::NONE, LayerEffects::from_motion)
    }

    /// Whether the effects change nothing.
    pub fn is_none(&self) -> bool {
        self.blur_px <= 0.0 && self.wipe.is_none() && self.tint.is_none()
    }
}

impl Tint {
    /// In 8-bit terms; `None` when it changes nothing.
    pub(super) fn quantized(&self) -> Option<Tint8> {
        if self.amount.is_nan() || self.amount <= 0.0 {
            return None;
        }
        let k = (self.amount.min(1.0) * 255.0).round() as u32;
        (k > 0).then_some(Tint8 {
            rgb: self.color.map(u32::from),
            k,
        })
    }
}

impl Prepared {
    /// Attaches the per-pixel effects (wipe and tint) of `effects` to a
    /// layer placed by `m` (layer → output) whose rectangle is `rect`
    /// (`(x0, y0, x1, y1)` in layer coordinates).
    pub(super) fn with_effects(
        mut self,
        m: DMat3,
        rect: (f64, f64, f64, f64),
        effects: &LayerEffects,
    ) -> Option<Prepared> {
        if let Some(wipe) = &effects.wipe {
            let to_layer = ToSource::inverse_of(m)?;
            let from = match wipe.from {
                Direction::Right => WipeFrom::Right,
                Direction::Up => WipeFrom::Top,
                Direction::Down => WipeFrom::Bottom,
                Direction::Left | Direction::Clockwise | Direction::CounterClockwise => {
                    WipeFrom::Left
                }
            };
            if wipe.shown.is_nan() || wipe.shown <= 0.0 {
                // Nothing shown yet.
                return None;
            }
            self.wipe = WipeMask::new(to_layer, rect, wipe.shown, from, wipe.feather);
        }
        self.tint = effects.tint.and_then(|t| t.quantized());
        Some(self)
    }
}

/// The output scale for canvas lengths: the geometric mean of the canvas
/// → output axis scales.
pub(super) fn output_length_scale(to_output: DMat3) -> f64 {
    let s = (to_output.x_axis.x * to_output.y_axis.y - to_output.y_axis.x * to_output.x_axis.y)
        .abs()
        .sqrt();
    if s.is_finite() { s } else { 0.0 }
}

/// What makes a blurred layer: equal keys give equal pixels.
struct BlurKey {
    /// The sampled picture (compared by identity; holding it keeps the
    /// identity from being reused).
    picture: Option<Arc<Picture>>,
    bits: Vec<u64>,
}

impl BlurKey {
    fn new(layer: &Prepared, radius: f64, rect: (i64, i64, i64, i64)) -> BlurKey {
        let mut bits = Vec::with_capacity(32);
        let m = layer.to_source.matrix();
        bits.extend(m.to_cols_array().iter().map(|v| v.to_bits()));
        bits.push(u64::from(layer.to_source.is_projective()));
        bits.push(layer.source_size.0.to_bits());
        bits.push(layer.source_size.1.to_bits());
        bits.push(u64::from(layer.opacity));
        let picture = match &layer.source {
            PreparedSource::Fill(rgb, alpha) => {
                bits.push(
                    0x1_0000_0000
                        | u64::from(rgb[0]) << 24
                        | u64::from(rgb[1]) << 16
                        | u64::from(rgb[2]) << 8
                        | u64::from(*alpha),
                );
                None
            }
            PreparedSource::Picture(p) => {
                bits.push(2);
                Some(Arc::clone(p))
            }
            PreparedSource::Premultiplied(p) => {
                bits.push(3);
                Some(Arc::clone(p))
            }
        };
        match &layer.wipe {
            Some(w) => {
                bits.push(1);
                bits.extend(w.bits());
            }
            None => bits.push(0),
        }
        match layer.tint {
            Some(t) => bits.push(
                1 << 40
                    | u64::from(t.rgb[0]) << 24
                    | u64::from(t.rgb[1]) << 16
                    | u64::from(t.rgb[2]) << 8
                    | u64::from(t.k),
            ),
            None => bits.push(0),
        }
        bits.push(radius.to_bits());
        bits.extend([rect.0, rect.1, rect.2, rect.3].map(|v| v as u64));
        BlurKey { picture, bits }
    }

    fn matches(&self, other: &BlurKey) -> bool {
        self.bits == other.bits
            && match (&self.picture, &other.picture) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

/// Blurred layers most recently used, newest last.
#[derive(Default)]
pub(super) struct BlurCache {
    entries: Vec<(BlurKey, Arc<Picture>)>,
}

/// Blurred layers kept.
const BLUR_CACHE_ENTRIES: usize = 4;

/// Byte budget of the kept blurred layers.
fn blur_cache_bytes() -> usize {
    freecut_media::resources::current().cache_bytes(96, 8)
}

impl BlurCache {
    fn get(&mut self, key: &BlurKey) -> Option<Arc<Picture>> {
        let i = self.entries.iter().position(|(k, _)| k.matches(key))?;
        let entry = self.entries.remove(i);
        let picture = Arc::clone(&entry.1);
        self.entries.push(entry);
        Some(picture)
    }

    fn insert(&mut self, key: BlurKey, picture: Arc<Picture>) {
        self.entries.push((key, picture));
        let bytes =
            |e: &[(BlurKey, Arc<Picture>)]| -> usize { e.iter().map(|(_, p)| p.rgba.len()).sum() };
        while self.entries.len() > BLUR_CACHE_ENTRIES || bytes(&self.entries) > blur_cache_bytes() {
            self.entries.remove(0);
        }
    }

    /// How many blurred layers are kept (tests and diagnostics).
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// `layer` blurred by a Gaussian of `radius` output px: drawn alone (wipe
/// and tint included) into a premultiplied buffer that covers the layer
/// and the blur's reach, clipped to the output (`size`) grown by that
/// reach so light from just outside the frame still bleeds in, blurred
/// per channel, and returned as a premultiplied layer placed 1:1. Radii
/// below half a pixel return the layer unchanged; `None` when nothing is
/// left to draw.
pub(super) fn blurred(
    layer: Prepared,
    radius: f64,
    size: (u32, u32),
    threads: usize,
    cache: &mut BlurCache,
) -> Option<Prepared> {
    if radius.is_nan() || radius < MIN_LAYER_BLUR {
        return Some(layer);
    }
    let radius = radius.min(f64::from(crate::text::effects::MAX_BLUR_RADIUS));
    let reach = f64::from(blur_reach(radius as f32)) + 1.0;
    let (bx0, by0, bx1, by1) = layer.bounds?;
    let x0 = (bx0 - reach).max(-reach).floor();
    let y0 = (by0 - reach).max(-reach).floor();
    let x1 = (bx1 + reach).min(f64::from(size.0) + reach).ceil();
    let y1 = (by1 + reach).min(f64::from(size.1) + reach).ceil();
    if !(x0 < x1 && y0 < y1) {
        return None;
    }
    let rect = (x0 as i64, y0 as i64, x1 as i64, y1 as i64);
    let (w, h) = ((rect.2 - rect.0) as usize, (rect.3 - rect.1) as usize);
    let key = BlurKey::new(&layer, radius, rect);
    let picture = match cache.get(&key) {
        Some(p) => p,
        None => {
            let alone = draw_alone(&layer, rect, threads);
            let picture = Arc::new(blur_premultiplied(&alone, w, h, radius as f32, threads));
            cache.insert(key, Arc::clone(&picture));
            picture
        }
    };
    let m = DMat3::from_translation(DVec2::new(x0, y0));
    let bounds = super::draw::bounds(m, (w as f64, h as f64));
    Some(Prepared {
        to_source: ToSource::inverse_of(m)?,
        source_size: (w as f64, h as f64),
        source: PreparedSource::Premultiplied(picture),
        opacity: 255,
        bounds,
        wipe: None,
        tint: None,
    })
}

/// `layer` alone over nothing, premultiplied RGBA8, for the output pixels
/// `rect` (`x0, y0, x1, y1`, may reach outside the frame).
fn draw_alone(layer: &Prepared, rect: (i64, i64, i64, i64), threads: usize) -> Vec<u8> {
    let (w, h) = ((rect.2 - rect.0) as usize, (rect.3 - rect.1) as usize);
    let mut out = vec![0u8; w * h * 4];
    let shader = Shader::new(layer);
    let rows = |band: &mut [u8], first: usize| {
        for (r, row) in band.chunks_mut(w * 4).enumerate() {
            let py = (rect.1 + (first + r) as i64) as f64 + 0.5;
            for (c, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let x = (rect.0 + c as i64) as f64 + 0.5;
                if let Some(s) = shader.shade(x, py) {
                    *px = s.premultiplied();
                }
            }
        }
    };
    let bands = threads
        .min(h.div_ceil(32))
        .min((w * h / crate::parallel::MIN_BAND_PIXELS).max(1))
        .max(1);
    if bands == 1 {
        rows(&mut out, 0);
        return out;
    }
    let per_band = h.div_ceil(bands);
    crate::parallel::bands(&mut out, per_band * w * 4, |i, band| {
        rows(band, i * per_band)
    });
    out
}

/// A premultiplied `w × h` RGBA8 buffer blurred per channel (outside reads
/// as nothing), as a premultiplied picture.
fn blur_premultiplied(src: &[u8], w: usize, h: usize, radius: f32, threads: usize) -> Picture {
    let channel = |c: usize| -> Vec<f32> {
        let plane: Vec<f32> = src
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| f32::from(p[c]))
            .collect();
        blur_reduced(&plane, w, h, radius, 0.0, LAYER_BLUR_DOWNSAMPLE_ABOVE)
    };
    let mut planes = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let per = if w * h < crate::parallel::MIN_BAND_PIXELS {
        4
    } else {
        4usize.div_ceil(threads.clamp(1, 4))
    };
    crate::parallel::bands(&mut planes, per, |i, band| {
        for (j, plane) in band.iter_mut().enumerate() {
            *plane = channel(i * per + j);
        }
    });
    let mut rgba = vec![0u8; w * h * 4];
    for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        for (c, v) in px.iter_mut().enumerate() {
            *v = planes[c][i].round().clamp(0.0, 255.0) as u8;
        }
    }
    Picture::new(w as u32, h as u32, rgba)
}
