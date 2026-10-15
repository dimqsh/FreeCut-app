//! Text on the CPU: parley lays text out, swash rasterizes the glyphs.
//!
//! This is the CPU reference text path (TX-003…TX-016, TX-055; README
//! C44, C46), decoupled from the timeline's text model: callers describe one
//! block of uniformly styled text with a [`TextSpec`] and its [`style`];
//! [`TextSpec::from_clip`] maps the model's text clips onto it.
//!
//! **Layers.** Outline, shadow and glow are computed from the glyph
//! coverage in layer space (a distance transform of the union silhouette
//! and separable Gaussian blurs); boxes, bubbles ([`crate::shape`]) and
//! decorations are vector paths filled as unions. The raster is
//! premultiplied, so a glow adds light when the compositor draws it.
//! Layout reports every glyph ([`PlacedGlyph`]) with its animation units.
//!
//! **Units.** A spec is laid out in *layout px* (whatever unit the caller
//! uses for `size_px` and `box_width_px`). Layout never depends on the
//! raster scale: lines break at the same places at every preview size and
//! at export (no hinting, no pixel-quantized metrics). The raster scale only
//! multiplies glyph sizes and positions.
//!
//! **The box.** The text box starts at (0, 0), top left, y down. Its width
//! is `box_width_px` when set (text wraps inside it) and the widest line
//! otherwise; its height is the sum of the line heights. Glyphs may ink
//! outside the box (italic overhangs, tall marks); rasters include that ink
//! and say where the box origin is.

pub mod animate;
mod clip;
mod curve;
pub(crate) mod effects;
mod fonts;
mod layout;
pub mod projective;
mod raster;
pub mod style;

use std::collections::HashMap;
use std::sync::Arc;

pub use fonts::{BUNDLED_FONTS, BundledFont, DEFAULT_FAMILY, FontRegistry, ScanState};
pub use layout::{FallbackReport, GlyphFont, LineInfo, PlacedGlyph, TextLayout, UnitIds};
pub use style::{
    Border, BoxHeight, BoxShape, Decoration, Fill, FillExtent, Glow, GlowDirection, GradientStop,
    LinearGradient, RadialGradient, Shadow, Stroke, StrokePosition, TextBox, TextBubble,
};

use crate::cpu::Picture;
use crate::shape::bubble::{BubbleKind, BubbleSpec, Side};
use animate::{TextMotion, UnitMap};

/// Horizontal alignment of lines inside the box (TX-005).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    /// Lines that wrap are spread over the fixed box width; the last line
    /// of each paragraph, and text without a fixed width, is set left.
    Justify,
}

/// One block of uniformly styled text.
///
/// Text is expected with LF line breaks (CRLF is normalized upstream); a
/// stray `\r` is treated as whitespace. It is drawn as given: the case
/// transform (TX-008) is applied by whoever builds the spec.
///
/// The style fields are described in [`style`]; lengths relative to the
/// font size are fractions of `size_px`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpec {
    /// The display text (case transform already applied).
    pub text: String,
    /// Font family name; a missing family renders with Noto Sans and is
    /// reported in [`FallbackReport::family_missing`].
    pub family: String,
    /// Font size (em) in layout px.
    pub size_px: f32,
    pub fill: Fill,
    pub bold: bool,
    pub italic: bool,
    pub align: TextAlign,
    /// `Some(w)`: fixed-width box, lines wrap at `w` (at least 1 em).
    /// `None`: the box grows with the text; only explicit breaks wrap.
    /// Ignored while the text is curved (TX-005).
    pub box_width_px: Option<f32>,
    /// Multiple of the font's natural line height.
    pub line_spacing: f32,
    /// Added after each grapheme cluster but the last of a line, as a
    /// fraction of the font size (−0.2..=1, TX-009).
    pub letter_spacing: f32,
    pub decoration: Decoration,
    pub stroke: Option<Stroke>,
    pub shadow: Option<Shadow>,
    pub glow: Option<Glow>,
    /// The background box; paused while curved. A bubble wins when both
    /// are set (TX-054).
    pub background: Option<TextBox>,
    /// The bubble; paused while curved.
    pub bubble: Option<TextBubble>,
    /// Curve amount −1..=1 (TX-016): 0 is straight, ±1 a full circle.
    pub curve: f32,
    /// Optional ligatures (`liga`, `clig`, `dlig`) may form. The frame
    /// plan turns them off for a clip that moves its letters one by one,
    /// and they are off whenever the letter spacing isn't 0 (CSS's rule),
    /// so each letter is its own glyph (the text architecture's ligature
    /// policy). Required ligatures and the shaping of complex scripts are
    /// never touched.
    pub ligatures: bool,
}

impl TextSpec {
    /// `text` in white 64 px Noto Sans, left aligned, auto width, no
    /// decoration, effects or curve.
    pub fn new(text: impl Into<String>) -> TextSpec {
        TextSpec {
            text: text.into(),
            family: DEFAULT_FAMILY.to_owned(),
            size_px: 64.0,
            fill: Fill::Solid([255, 255, 255, 255]),
            bold: false,
            italic: false,
            align: TextAlign::Left,
            box_width_px: None,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            decoration: Decoration::default(),
            stroke: None,
            shadow: None,
            glow: None,
            background: None,
            bubble: None,
            curve: 0.0,
            ligatures: true,
        }
    }

    /// The font size actually used: finite and within 0.5..=10,000 px.
    fn size(&self) -> f32 {
        if self.size_px.is_finite() {
            self.size_px.clamp(0.5, 10_000.0)
        } else {
            64.0
        }
    }

    fn spacing(&self) -> f32 {
        if self.line_spacing.is_finite() {
            self.line_spacing.clamp(0.1, 10.0)
        } else {
            1.0
        }
    }

    /// Letter spacing in layout px.
    fn letter_spacing_px(&self) -> f32 {
        let f = if self.letter_spacing.is_finite() {
            self.letter_spacing.clamp(-0.2, 1.0)
        } else {
            0.0
        };
        f * self.size()
    }

    /// The curve amount used: finite, −1..=1.
    fn curve_amount(&self) -> f32 {
        if self.curve.is_finite() {
            self.curve.clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }

    /// Whether the text is drawn on an arc.
    pub fn is_curved(&self) -> bool {
        self.curve_amount().abs() >= 1e-3
    }

    /// The wrap width actually used: at least 1 em; none while curved.
    /// Whether optional ligatures form: asked for, and no letter spacing.
    fn forms_ligatures(&self) -> bool {
        self.ligatures && self.letter_spacing_px() == 0.0
    }

    fn wrap_width(&self) -> Option<f32> {
        if self.is_curved() {
            return None;
        }
        self.box_width_px
            .filter(|w| w.is_finite())
            .map(|w| w.max(self.size()))
    }

    fn key(&self, scale: f32) -> SpecKey {
        // Destructured in full, so a new field can't be left out of the
        // cache key.
        let TextSpec {
            text,
            family,
            size_px: _,
            fill,
            bold,
            italic,
            align,
            box_width_px: _,
            line_spacing: _,
            letter_spacing: _,
            decoration,
            stroke,
            shadow,
            glow,
            background,
            bubble,
            curve: _,
            ligatures: _,
        } = self;
        let mut bits = KeyBits::default();
        bits.f(self.size());
        bits.u(u32::from(*bold) | u32::from(*italic) << 1);
        bits.u(match align {
            TextAlign::Left => 0,
            TextAlign::Center => 1,
            TextAlign::Right => 2,
            TextAlign::Justify => 3,
        });
        match self.wrap_width() {
            Some(w) => bits.f(w),
            None => bits.u(u32::MAX),
        }
        bits.f(self.spacing());
        bits.f(self.letter_spacing_px());
        bits.f(self.curve_amount());
        bits.u(u32::from(self.forms_ligatures()));
        bits.u(u32::from(decoration.underline) | u32::from(decoration.strike) << 1);
        bits.fill(fill);
        match stroke {
            None => bits.u(0),
            Some(s) => {
                bits.u(1 + s.position as u32);
                bits.color(s.color);
                bits.f(s.width);
                bits.f(s.opacity);
            }
        }
        match shadow {
            None => bits.u(0),
            Some(s) => {
                bits.u(1 + u32::from(s.include_box));
                bits.color(s.color);
                for v in [s.opacity, s.blur, s.distance, s.angle_deg] {
                    bits.f(v);
                }
            }
        }
        match glow {
            None => bits.u(0),
            Some(g) => {
                bits.u(1 + g.direction as u32);
                bits.color(g.color);
                for v in [g.intensity, g.radius, g.offset, g.angle_deg] {
                    bits.f(v);
                }
            }
        }
        match background {
            None => bits.u(0),
            Some(b) => {
                bits.u(1 + b.shape as u32 * 2 + b.height as u32);
                bits.color(b.color);
                for v in [b.opacity, b.pad_x, b.pad_y, b.roundness] {
                    bits.f(v);
                }
                bits.border(b.border);
            }
        }
        match bubble {
            None => bits.u(0),
            Some(b) => {
                bits.u(1);
                bits.bubble(&b.spec);
                bits.color(b.fill);
                for v in [b.opacity, b.pad_x, b.pad_y, b.roundness] {
                    bits.f(v);
                }
                bits.border(b.border);
            }
        }
        bits.f(scale);
        SpecKey {
            text: text.clone(),
            family: family.clone(),
            bits: bits.0,
            motion: Vec::new(),
        }
    }
}

/// The non-text part of a cache key: every style value as bits.
#[derive(Default)]
struct KeyBits(Vec<u32>);

impl KeyBits {
    fn u(&mut self, v: u32) {
        self.0.push(v);
    }

    fn f(&mut self, v: f32) {
        // -0.0 and 0.0 draw the same.
        self.0.push(if v == 0.0 { 0 } else { v.to_bits() });
    }

    fn color(&mut self, c: [u8; 4]) {
        self.0.push(u32::from_le_bytes(c));
    }

    fn stops(&mut self, stops: &[GradientStop]) {
        self.u(stops.len() as u32);
        for s in stops {
            self.f(s.at);
            self.color(s.color);
        }
    }

    fn fill(&mut self, fill: &Fill) {
        match fill {
            Fill::Solid(c) => {
                self.u(0);
                self.color(*c);
            }
            Fill::Linear(g) => {
                self.u(1 + g.extent as u32 * 8);
                self.f(g.angle_deg);
                self.stops(&g.stops);
            }
            Fill::Radial(g) => {
                self.u(2 + g.extent as u32 * 8);
                self.f(g.center.0);
                self.f(g.center.1);
                self.f(g.spread);
                self.stops(&g.stops);
            }
        }
    }

    fn border(&mut self, border: Option<Border>) {
        match border {
            None => self.u(0),
            Some(b) => {
                self.u(1);
                self.color(b.color);
                self.f(b.width);
            }
        }
    }

    fn bubble(&mut self, spec: &BubbleSpec) {
        self.u(match spec.kind {
            BubbleKind::Rounded => 0,
            BubbleKind::Oval => 1,
            BubbleKind::Thought => 2,
            BubbleKind::Shout => 3,
            BubbleKind::Square => 4,
            BubbleKind::Tag => 5,
            BubbleKind::ArrowTag { point: Side::Left } => 6,
            BubbleKind::ArrowTag { point: Side::Right } => 7,
            BubbleKind::Ticket => 8,
        });
        match spec.tail {
            None => self.u(0),
            Some(t) => {
                self.u(1);
                for v in [t.tip.x, t.tip.y, t.base, t.bend] {
                    self.f(v);
                }
            }
        }
        self.u(u32::from(spec.spikes) | u32::from(spec.trail) << 8);
        self.u(spec.seed);
        for v in [spec.spike_depth, spec.jitter, spec.lobe] {
            self.f(v);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SpecKey {
    text: String,
    family: String,
    bits: Vec<u32>,
    /// The digest of the per-glyph motion state ([`animate::MotionState::digest`]);
    /// empty for text at rest.
    motion: Vec<u32>,
}

/// A rasterized text block.
#[derive(Debug, Clone)]
pub struct TextRaster {
    /// **Premultiplied** RGBA (C46): every layer of the style composed in
    /// order. Where a glow adds light, colour exceeds alpha (alpha 0 with
    /// colour adds light when drawn). Empty (0 × 0) when nothing is inked.
    pub image: Arc<Picture>,
    /// Image pixels per layout px.
    pub scale: f32,
    /// Where the box origin (layout (0, 0)) lies in image pixels.
    pub origin: (i32, i32),
    pub layout: TextLayout,
}

/// Rasters larger than this on a side are made at a lower scale.
pub const MAX_RASTER_SIDE: u32 = 8192;

/// Rasters estimated to cover more pixels than this (about 4096 × 2048)
/// are made at a lower scale.
pub const MAX_RASTER_PIXELS: u32 = 1 << 23;

/// Byte budget of the raster cache.
fn raster_cache_bytes() -> usize {
    freecut_media::resources::current().cache_bytes(64, 8)
}

/// The part of [`raster_cache_bytes`] that rasters of moving text may
/// hold, so playing an animation never pushes still text out.
fn animated_cache_bytes() -> usize {
    freecut_media::resources::current().cache_bytes(16, 2)
}

/// Layouts kept for animated text (each frame of a moving title needs its
/// layout to evaluate the motion, even when its raster is cached).
const LAYOUT_CACHE_ENTRIES: usize = 8;

/// Lays out and rasterizes text; one per thread (parley's and swash's
/// contexts are single-threaded). Results are cached by spec and scale.
pub struct TextRenderer {
    registry: FontRegistry,
    generation: u64,
    engine: layout::Engine,
    glyphs: raster::GlyphRasterizer,
    cache: HashMap<SpecKey, CacheEntry>,
    cache_bytes: usize,
    animated_bytes: usize,
    /// Built layouts of animated text, by spec (scale 0), most recent last.
    layouts: Vec<(SpecKey, layout::Built, TextLayout)>,
    clock: u64,
}

struct CacheEntry {
    raster: Arc<TextRaster>,
    last_use: u64,
    bytes: usize,
    /// Made for moving text (counts toward [`animated_cache_bytes`]).
    animated: bool,
}

impl std::fmt::Debug for TextRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextRenderer")
            .field("generation", &self.generation)
            .field("cached", &self.cache.len())
            .field("cache_bytes", &self.cache_bytes)
            .finish_non_exhaustive()
    }
}

impl TextRenderer {
    pub fn new(registry: FontRegistry) -> TextRenderer {
        let (generation, collection) = registry.snapshot();
        TextRenderer {
            registry,
            generation,
            engine: layout::Engine::new(collection),
            glyphs: raster::GlyphRasterizer::default(),
            cache: HashMap::new(),
            cache_bytes: 0,
            animated_bytes: 0,
            layouts: Vec::new(),
            clock: 0,
        }
    }

    pub fn registry(&self) -> &FontRegistry {
        &self.registry
    }

    /// Picks up a newly published font set, dropping everything made with
    /// the old one.
    fn sync_fonts(&mut self) {
        if self.registry.generation() != self.generation {
            let (generation, collection) = self.registry.snapshot();
            self.generation = generation;
            self.engine = layout::Engine::new(collection);
            self.glyphs = raster::GlyphRasterizer::default();
            self.cache.clear();
            self.cache_bytes = 0;
            self.animated_bytes = 0;
            self.layouts.clear();
        }
    }

    /// Measures `spec` without rasterizing it.
    pub fn layout(&mut self, spec: &TextSpec) -> TextLayout {
        self.sync_fonts();
        let built = self.engine.build(spec);
        self.engine.describe(&built, spec)
    }

    /// `spec` rasterized at `scale` image px per layout px (clamped so the
    /// image stays within [`MAX_RASTER_SIDE`] and about
    /// [`MAX_RASTER_PIXELS`]; the returned
    /// [`TextRaster::scale`] is the one used).
    pub fn rasterize(&mut self, spec: &TextSpec, scale: f32) -> Arc<TextRaster> {
        self.sync_fonts();
        self.clock += 1;
        let scale = sane_scale(scale);
        let key = spec.key(scale);
        if let Some(entry) = self.cache.get_mut(&key) {
            entry.last_use = self.clock;
            return Arc::clone(&entry.raster);
        }
        let built = self.engine.build(spec);
        let layout = self.engine.describe(&built, spec);
        let bounds = raster::drawn_bounds(&built, spec, &layout);
        let scale = limited_scale(spec, bounds, scale);
        let raster = Arc::new(self.glyphs.render(&built, spec, scale, layout, None));
        self.insert(key, Arc::clone(&raster), false);
        raster
    }

    /// `spec` with every glyph posed by `motion` at its time (TX-022 steps
    /// 3 and 4), rasterized at `scale` like [`TextRenderer::rasterize`].
    /// The layout is the rest layout (animation never reflows text), so
    /// [`TextRaster::layout`] and the box origin are those of the text at
    /// rest. A state that looks as the rest does returns the still
    /// raster; otherwise rasters are cached by the spec and a digest of
    /// the per-glyph state ([`animate::MotionState::digest`]), so frames whose
    /// glyphs stand still (before a unit's window, after the In) reuse
    /// their raster.
    pub fn rasterize_animated(
        &mut self,
        spec: &TextSpec,
        motion: &TextMotion,
        scale: f32,
    ) -> Arc<TextRaster> {
        self.sync_fonts();
        let scale = sane_scale(scale);
        let slot = self.built(spec);
        let (_, _, layout) = &self.layouts[slot];
        let map = UnitMap::new(layout, &spec.text, motion.unit);
        let state = motion.evaluate_with(&map, layout, &spec.text);
        if state.is_rest() {
            return self.rasterize(spec, scale);
        }
        self.clock += 1;
        let mut key = spec.key(scale);
        key.motion = state.digest();
        if let Some(entry) = self.cache.get_mut(&key) {
            entry.last_use = self.clock;
            return Arc::clone(&entry.raster);
        }
        let (_, built, layout) = &self.layouts[slot];
        let mut bounds = raster::drawn_bounds(built, spec, layout);
        if let Some(b) = raster::motion_bounds(built, &state, spec.size()) {
            bounds = bounds.union(&b);
        }
        let scale = limited_scale(spec, bounds, scale);
        let animated = raster::Animated {
            state: &state,
            map: &map,
        };
        let raster =
            Arc::new(
                self.glyphs
                    .render(built, spec, scale, layout.clone(), Some(&animated)),
            );
        self.insert(key, Arc::clone(&raster), true);
        raster
    }

    /// The size of `spec`'s box at rest in layout px, from a kept layout
    /// (the box FX-030's distances and pivots measure from).
    pub fn box_size(&mut self, spec: &TextSpec) -> (f32, f32) {
        self.sync_fonts();
        let slot = self.built(spec);
        self.layouts[slot].2.size
    }

    /// The index in `self.layouts` of `spec`'s built layout, building it
    /// when it isn't kept.
    fn built(&mut self, spec: &TextSpec) -> usize {
        let key = spec.key(0.0);
        if let Some(i) = self.layouts.iter().position(|(k, _, _)| *k == key) {
            let entry = self.layouts.remove(i);
            self.layouts.push(entry);
        } else {
            let built = self.engine.build(spec);
            let layout = self.engine.describe(&built, spec);
            if self.layouts.len() >= LAYOUT_CACHE_ENTRIES {
                self.layouts.remove(0);
            }
            self.layouts.push((key, built, layout));
        }
        self.layouts.len() - 1
    }

    fn insert(&mut self, key: SpecKey, raster: Arc<TextRaster>, animated: bool) {
        let bytes = raster.image.rgba.len() + key.text.len() + 4 * key.motion.len() + 256;
        if bytes > raster_cache_bytes() || animated && bytes > animated_cache_bytes() {
            return;
        }
        while !self.cache.is_empty()
            && (self.cache_bytes + bytes > raster_cache_bytes()
                || animated && self.animated_bytes + bytes > animated_cache_bytes())
        {
            // Over the animated share, the oldest animated raster goes.
            let only_animated = animated && self.animated_bytes + bytes > animated_cache_bytes();
            let oldest = self
                .cache
                .iter()
                .filter(|(_, e)| !only_animated || e.animated)
                .min_by_key(|(_, e)| e.last_use)
                .map(|(k, _)| k.clone());
            match oldest.and_then(|k| self.cache.remove(&k)) {
                Some(e) => {
                    self.cache_bytes -= e.bytes;
                    if e.animated {
                        self.animated_bytes -= e.bytes;
                    }
                }
                None => break,
            }
        }
        self.cache_bytes += bytes;
        if animated {
            self.animated_bytes += bytes;
        }
        self.cache.insert(
            key,
            CacheEntry {
                raster,
                last_use: self.clock,
                bytes,
                animated,
            },
        );
    }

    /// How many rasters are cached (for tests and diagnostics).
    pub fn cached_rasters(&self) -> usize {
        self.cache.len()
    }
}

/// A raster scale that is a positive finite number within 1/256..=256.
fn sane_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale.clamp(1.0 / 256.0, 256.0)
    } else {
        1.0
    }
}

/// `scale` lowered so the image of `bounds` stays within
/// [`MAX_RASTER_SIDE`] and about [`MAX_RASTER_PIXELS`]. The styled pipeline
/// keeps several float planes per pixel; the extent grows at most linearly
/// with the scale (blurs are capped in image px), so proportional steps
/// settle within the budget in a few rounds.
fn limited_scale(spec: &TextSpec, bounds: crate::shape::Rect, scale: f32) -> f32 {
    let mut scale = scale;
    for _ in 0..4 {
        let (w, h) = raster::image_extent(spec, bounds, scale);
        let side = MAX_RASTER_SIDE as f32 / w.max(h);
        let area = (MAX_RASTER_PIXELS as f32 / (w * h)).sqrt();
        let k = side.min(area);
        if !(k.is_finite() && k < 1.0) {
            break;
        }
        scale *= k;
    }
    scale
}
