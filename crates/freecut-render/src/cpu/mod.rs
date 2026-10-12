//! The CPU compositor: the shared preview and export renderer, and the
//! reference the GPU compositor will be tested against.
//!
//! It renders a [`FramePlan`] into straight-alpha sRGB RGBA8 at any output
//! size. Layers are drawn bottom to top through their `layer_to_canvas`
//! transform (then canvas → output scaling), sampled bilinearly, and
//! blended "over" in 8-bit integer arithmetic. Layer edges are antialiased
//! by coverage (a 1 px box against each pair of edges), so rotated and
//! sub-pixel-placed layers get smooth, stable edges; integer-aligned edges
//! stay exact. Pictures shown at less than about 70 % of their size are
//! sampled from a box-filtered mip level so fine detail doesn't alias.
//!
//! Decoding is not done here: pictures come from a [`LayerImages`]
//! implementation supplied by the caller (see `crate::media` for one backed
//! by `freecut-media` decoders). Text layers are converted from the plan's
//! text data ([`TextLayer::from_plan`], the default of
//! [`LayerImages::text`]) and rasterized by the compositor's own
//! [`TextRenderer`] at the final output size.
//!
//! **Animation** (FX-030, TX-022): a projective `layer_to_canvas` (a layer
//! turned in 3D, with the plan's perspective) is drawn projectively, each
//! pixel's preimage found by the inverse homography; pixels whose preimage
//! lies at or behind the eye are not drawn. A layer turned past 90° shows
//! its back mirrored, like a transparent sheet (from the matrix alone a
//! back face can't be told from a mirrored layer; no preset turns a whole
//! layer that far). The plan's `LayerMotion` adds a wipe, a tint and a
//! blur ([`LayerEffects`], the `effects` module docs).
//!
//! **Determinism.** The same plan and inputs give identical bytes, whatever
//! the thread count: rows are split into bands and every pixel is computed
//! independently with the same arithmetic.

mod draw;
mod effects;
mod mip;

use std::sync::Arc;

use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::ids::{MediaId, SeqId};
use freecut_timeline::model::{Document, Rotation, VAnchor};
use freecut_timeline::motion::PresetCatalog;
use freecut_timeline::plan::{FramePlan, LayerPlan, LayerSource, PlanError, PlanInputs, PlanNode};
use freecut_timeline::time::Tick;

use crate::motion::TextMotionOptions;
use crate::text::animate::TextMotion;
use crate::text::{FontRegistry, TextRenderer, TextSpec};
use draw::{Prepared, PreparedSource, ToSource};
pub use effects::{LAYER_BLUR_DOWNSAMPLE_ABOVE, LayerEffects, Tint, WIPE_FEATHER, Wipe};

/// A decoded picture: straight-alpha RGBA8 at its stored size, plus how it
/// is turned for display (mirror first, then clockwise rotation; the same
/// convention as the media probe).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rotation: Rotation,
    pub mirrored: bool,
    /// `width * height * 4` bytes, rows top to bottom.
    pub rgba: Vec<u8>,
}

impl Picture {
    /// An upright picture.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Picture {
        Picture {
            width,
            height,
            rotation: Rotation::None,
            mirrored: false,
            rgba,
        }
    }

    /// 0 × 0, nothing to draw.
    pub fn empty() -> Picture {
        Picture::new(0, 0, Vec::new())
    }

    /// Whether the buffer holds `width × height` pixels.
    pub fn is_valid(&self) -> bool {
        let need = u64::from(self.width) * u64::from(self.height) * 4;
        usize::try_from(need).is_ok_and(|n| self.rgba.len() >= n)
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Which point of the text box sits at the origin of a [`TextLayer`]'s
/// coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TextAnchor {
    /// The box's top-left corner.
    TopLeft,
    /// The middle of the box's top edge: the timeline's convention for
    /// `VAnchor::Top` (TX-005).
    TopCenter,
    /// The box's centre: the timeline's convention for `VAnchor::Middle`,
    /// the default (FX-001 plan docs), so a plan's `layer_to_canvas` can be
    /// passed through as is.
    #[default]
    Center,
    /// The middle of the box's bottom edge: the timeline's convention for
    /// `VAnchor::Bottom` (TX-005).
    BottomCenter,
}

impl TextAnchor {
    /// The anchor the plan's text layer origin follows for `v` (the
    /// `freecut_timeline::plan` module docs): the box's horizontal centre
    /// and the edge `v` names, so that edge stays put as lines come and go.
    pub fn for_v_anchor(v: VAnchor) -> TextAnchor {
        match v {
            VAnchor::Top => TextAnchor::TopCenter,
            VAnchor::Middle => TextAnchor::Center,
            VAnchor::Bottom => TextAnchor::BottomCenter,
        }
    }

    /// The anchor's point in a box of `size` (layout px, origin top left).
    pub fn point(self, size: (f32, f32)) -> (f32, f32) {
        let (w, h) = size;
        match self {
            TextAnchor::TopLeft => (0.0, 0.0),
            TextAnchor::TopCenter => (w / 2.0, 0.0),
            TextAnchor::Center => (w / 2.0, h / 2.0),
            TextAnchor::BottomCenter => (w / 2.0, h),
        }
    }
}

/// A text layer ready to rasterize: what to draw and where.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayer {
    /// Lengths in canvas pixels at 100 % scale.
    pub spec: TextSpec,
    /// Maps layer coordinates (layout px, origin at `anchor`) to canvas
    /// pixels.
    pub layer_to_canvas: DMat3,
    pub anchor: TextAnchor,
    /// Per-unit motion at the frame's time (TX-022 step 3, TX-023), drawn
    /// in the text's local frame before `layer_to_canvas` (which carries
    /// the clip's keyframed transform with the slot's block offsets
    /// folded in, step 5). `None`: the text at rest.
    pub motion: Option<TextMotion>,
}

impl TextLayer {
    /// The text a [`LayerSource::Text`] layer shows, following the plan's
    /// convention (`freecut_timeline::plan` module docs): lengths are the
    /// clip's ref px times `scale_px` ([`TextSpec::from_clip`]), the layer
    /// origin is the text box's horizontal centre and the edge its
    /// `v_anchor` names ([`TextAnchor::for_v_anchor`]), and
    /// `layer_to_canvas` is passed through unchanged.
    ///
    /// The layer's animation work becomes [`TextLayer::motion`]
    /// ([`crate::motion::layer_text_motion`]): its playing per-unit preset
    /// and its Reveal key. Block offsets and opacity are already in the
    /// plan's matrix and opacity.
    ///
    /// `None` for other layers, for empty text (TX-002: an empty clip
    /// renders nothing) and for a scale that isn't a positive finite number.
    pub fn from_plan(layer: &LayerPlan) -> Option<TextLayer> {
        let LayerSource::Text { data, scale_px } = &layer.source else {
            return None;
        };
        if data.content.is_empty() || !scale_px.is_finite() || *scale_px <= 0.0 {
            return None;
        }
        Some(TextLayer {
            spec: TextSpec {
                ligatures: layer.ligatures,
                ..TextSpec::from_clip(data, *scale_px)
            },
            layer_to_canvas: layer.layer_to_canvas,
            anchor: TextAnchor::for_v_anchor(data.layout.v_anchor),
            motion: crate::motion::layer_text_motion(layer),
        })
    }
}

/// A picture a [`LayerImages`] provider could not supply because its file
/// couldn't be opened or a frame couldn't be decoded. (Offline media, which
/// simply has no file, is not a failure.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageFailure {
    pub asset: MediaId,
    /// The media time asked for; `None` for a still.
    pub source_time: Option<Tick>,
    pub reason: String,
}

/// Supplies what the compositor can't make itself. Implemented by preview
/// and export. A missing picture draws the offline slate; missing text
/// draws nothing.
pub trait LayerImages {
    /// The video frame of `asset` displayed at media time `source_time`.
    fn video(&mut self, asset: MediaId, source_time: Tick) -> Option<Arc<Picture>>;

    /// Like [`LayerImages::video`] for a layer drawn at only `fraction` of
    /// its display size along each display axis (asked only when both are
    /// below about 0.7, as for mip levels). Providers that can decode smaller
    /// pictures cheaply may return one at least that large; the compositor
    /// draws any size. The default ignores the hint.
    fn video_scaled(
        &mut self,
        asset: MediaId,
        source_time: Tick,
        fraction: (f64, f64),
    ) -> Option<Arc<Picture>> {
        let _ = fraction;
        self.video(asset, source_time)
    }

    /// The still picture `asset`.
    fn still(&mut self, asset: MediaId) -> Option<Arc<Picture>>;

    /// The text shown by a [`LayerSource::Text`] layer. The default converts
    /// the layer's own text data ([`TextLayer::from_plan`]); override it
    /// only to substitute text (tests, an on-canvas editor's live draft).
    fn text(&mut self, layer: &LayerPlan) -> Option<TextLayer> {
        TextLayer::from_plan(layer)
    }

    /// The failures since the last call, oldest first (export stops on
    /// them: EX-022 `decode-error`). The default has none.
    fn take_failures(&mut self) -> Vec<ImageFailure> {
        Vec::new()
    }
}

/// Supplies no pictures: media layers draw slates. Text layers draw their
/// own text (the default [`LayerImages::text`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoImages;

impl LayerImages for NoImages {
    fn video(&mut self, _: MediaId, _: Tick) -> Option<Arc<Picture>> {
        None
    }

    fn still(&mut self, _: MediaId) -> Option<Arc<Picture>> {
        None
    }
}

/// A rendered frame: straight-alpha sRGB RGBA8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Frame {
    /// A transparent frame.
    pub fn new(width: u32, height: u32) -> Frame {
        Frame {
            width,
            height,
            rgba: vec![0; width as usize * height as usize * 4],
        }
    }

    /// The pixel at (x, y); `None` outside the frame.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.rgba.get(i..i + 4)?.try_into().ok()
    }
}

/// The colour of the "media offline" slate (EX-005).
pub const SLATE: [u8; 4] = [70, 30, 30, 255];

/// Where generated content the renderer can't draw yet sits.
pub const PLACEHOLDER: [u8; 4] = [140, 90, 200, 60];

/// The output size for showing `canvas` within `max`: aspect kept, never
/// upscaled, at least 1 × 1 (the preview's rule).
pub fn fit_size(canvas: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let (cw, ch) = (f64::from(canvas.0.max(1)), f64::from(canvas.1.max(1)));
    let scale = (f64::from(max.0) / cw).min(f64::from(max.1) / ch).min(1.0);
    (
        ((cw * scale).round() as u32).max(1),
        ((ch * scale).round() as u32).max(1),
    )
}

/// Renders frame plans on the CPU. Keep one per render thread (it owns a
/// text renderer and its caches).
pub struct Compositor {
    text: TextRenderer,
    threads: usize,
    mips: mip::MipCache,
    blurs: effects::BlurCache,
    grades: crate::color::GradeCache,
    text_options: TextMotionOptions,
}

impl std::fmt::Debug for Compositor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compositor")
            .field("threads", &self.threads)
            .field("text", &self.text)
            .finish()
    }
}

/// Video layers drawn below this fraction of their display size (on both
/// axes) are asked for at a smaller size ([`LayerImages::video_scaled`]);
/// the same threshold as the first mip level.
const SCALED_REQUEST_BELOW: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Bands are at least this many rows, so small frames stay on one thread.
const MIN_BAND_ROWS: u32 = 32;

impl Compositor {
    pub fn new(fonts: FontRegistry) -> Compositor {
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8);
        Compositor {
            text: TextRenderer::new(fonts),
            threads,
            mips: mip::MipCache::default(),
            blurs: effects::BlurCache::default(),
            grades: crate::color::GradeCache::default(),
            text_options: TextMotionOptions::default(),
        }
    }

    /// Sets the options of per-unit text animation (TX-056's Typewriter
    /// caret, the Decode step) for the text layers this compositor draws
    /// from a plan ([`crate::motion::TextMotionOptions::apply`]).
    pub fn with_text_motion_options(mut self, options: TextMotionOptions) -> Compositor {
        self.text_options = options;
        self
    }

    pub fn text_motion_options(&self) -> &TextMotionOptions {
        &self.text_options
    }

    /// How many blurred layers are kept (diagnostics and tests).
    pub fn cached_blurs(&self) -> usize {
        self.blurs.len()
    }

    /// How many compiled colour stacks and graded pictures are kept
    /// (diagnostics and tests).
    pub fn cached_grades(&self) -> (usize, usize) {
        self.grades.len()
    }

    /// Sets the number of threads used per frame (at least 1). The output
    /// doesn't depend on it.
    pub fn with_threads(mut self, threads: usize) -> Compositor {
        self.threads = threads.max(1);
        self
    }

    pub fn text_renderer(&mut self) -> &mut TextRenderer {
        &mut self.text
    }

    /// The plan of the frame displayed at `t` in sequence `seq`, with the
    /// animations of `presets` and text boxes measured by this
    /// compositor's layout ([`crate::motion::plan_frame`]): the plan
    /// export draws, so a preview planned here matches it.
    pub fn plan(
        &mut self,
        doc: &Document,
        seq: SeqId,
        t: Tick,
        inputs: &PlanInputs<'_>,
        presets: &dyn PresetCatalog,
    ) -> Result<FramePlan, PlanError> {
        crate::motion::plan_frame(doc, seq, t, inputs, presets, &mut self.text)
    }

    /// Renders `plan` at `width × height` (the canvas is stretched to fill
    /// it; use [`fit_size`] to keep the aspect).
    pub fn render(
        &mut self,
        plan: &FramePlan,
        width: u32,
        height: u32,
        images: &mut dyn LayerImages,
    ) -> Frame {
        let mut frame = Frame::new(width, height);
        self.render_into(plan, &mut frame, images);
        frame
    }

    /// Renders into an existing frame (its size is the output size), reusing
    /// its buffer.
    pub fn render_into(
        &mut self,
        plan: &FramePlan,
        frame: &mut Frame,
        images: &mut dyn LayerImages,
    ) {
        let (width, height) = (frame.width, frame.height);
        frame.rgba.resize(width as usize * height as usize * 4, 0);
        for px in frame.rgba.as_chunks_mut::<4>().0 {
            *px = plan.canvas.background;
        }
        if width == 0 || height == 0 {
            return;
        }
        let to_output = DMat3::from_scale(DVec2::new(
            f64::from(width) / f64::from(plan.canvas.width.max(1)),
            f64::from(height) / f64::from(plan.canvas.height.max(1)),
        ));
        let length_scale = effects::output_length_scale(to_output);
        let mut layers: Vec<draw::Clipped> = Vec::with_capacity(plan.nodes.len());
        for node in &plan.nodes {
            let PlanNode::Layer(layer) = node;
            let fx = LayerEffects::of(layer);
            let Some(prepared) = self.prepare(layer, to_output, images, plan.frame, &fx) else {
                continue;
            };
            let prepared = if fx.blur_px > 0.0 {
                let radius = fx.blur_px * length_scale;
                match effects::blurred(
                    prepared,
                    radius,
                    (width, height),
                    self.threads,
                    &mut self.blurs,
                ) {
                    Some(p) => p,
                    None => continue,
                }
            } else {
                prepared
            };
            layers.extend(prepared.clip(width, height));
        }
        if layers.is_empty() {
            return;
        }
        let row_bytes = width as usize * 4;
        let bands = self
            .threads
            .min((height / MIN_BAND_ROWS).max(1) as usize)
            .max(1);
        if bands == 1 {
            draw::draw_rows(&mut frame.rgba, 0, width, &layers);
            return;
        }
        let rows_per_band = (height as usize).div_ceil(bands);
        std::thread::scope(|scope| {
            for (i, band) in frame.rgba.chunks_mut(rows_per_band * row_bytes).enumerate() {
                let layers = &layers;
                let y0 = (i * rows_per_band) as u32;
                scope.spawn(move || draw::draw_rows(band, y0, width, layers));
            }
        });
    }

    /// Resolves one layer into something drawable (with the wipe and tint
    /// of `fx`; the blur is the caller's), or `None` when it draws nothing.
    /// `frame` is the plan's frame duration.
    fn prepare(
        &mut self,
        layer: &LayerPlan,
        to_output: DMat3,
        images: &mut dyn LayerImages,
        frame: Tick,
        fx: &LayerEffects,
    ) -> Option<Prepared> {
        let opacity = (layer.opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
        if opacity == 0 {
            return None;
        }
        let m = to_output * layer.layer_to_canvas;
        let display = (
            f64::from(layer.display_size.0),
            f64::from(layer.display_size.1),
        );
        let rect = (0.0, 0.0, display.0, display.1);
        let picture = match &layer.source {
            LayerSource::Video { asset, source_time } => {
                // Output px per display px along each display axis.
                let fraction = (m.x_axis.truncate().length(), m.y_axis.truncate().length());
                if fraction.0.max(fraction.1) < SCALED_REQUEST_BELOW {
                    images.video_scaled(*asset, *source_time, fraction)
                } else {
                    images.video(*asset, *source_time)
                }
            }
            LayerSource::Still { asset } => images.still(*asset),
            LayerSource::Slate { .. } => {
                return Prepared::fill(m, display, SLATE, opacity)?.with_effects(m, rect, fx);
            }
            LayerSource::Text { .. } => {
                let mut text = images.text(layer)?;
                if let Some(motion) = text.motion.as_mut() {
                    self.text_options.apply(motion, layer, frame);
                }
                return self.prepare_text(&text, to_output, opacity, fx);
            }
            LayerSource::Generated { .. } => {
                return Prepared::fill(m, display, PLACEHOLDER, opacity)?.with_effects(m, rect, fx);
            }
        };
        // The colour stack grades the decoded (possibly scaled) picture
        // before mips and placement (CL-001, README C52), so mip levels are
        // made from graded pixels.
        let picture =
            picture
                .filter(|p| p.is_valid() && !p.is_empty())
                .map(|p| match &layer.color {
                    Some(stack) => self.grades.graded(&p, stack, self.threads),
                    None => p,
                });
        match picture {
            Some(picture) => Prepared::picture(m, display, picture, opacity)?
                .minified(&mut self.mips)
                .with_effects(m, rect, fx),
            None => Prepared::fill(m, display, SLATE, opacity)?.with_effects(m, rect, fx),
        }
    }

    fn prepare_text(
        &mut self,
        text: &TextLayer,
        to_output: DMat3,
        opacity: u32,
        fx: &LayerEffects,
    ) -> Option<Prepared> {
        let m = to_output * text.layer_to_canvas;
        let projective = !draw::is_affine(m);
        // Rasterize at the final output resolution: the larger axis scale of
        // the transform (for a layer turned in 3D, the largest over the
        // box's corners, where perspective magnifies most).
        let axis = if projective {
            let size = self.text.box_size(&text.spec);
            let (ax, ay) = text.anchor.point(size);
            let (x0, y0) = (-f64::from(ax), -f64::from(ay));
            let (x1, y1) = (x0 + f64::from(size.0), y0 + f64::from(size.1));
            [(x0, y0), (x1, y0), (x0, y1), (x1, y1), (0.0, 0.0)]
                .into_iter()
                .filter_map(|(x, y)| draw::local_scale(m, DVec2::new(x, y)))
                .fold(0.0, f64::max)
        } else {
            m.x_axis
                .truncate()
                .length()
                .max(m.y_axis.truncate().length())
        };
        if !axis.is_finite() || axis <= 0.0 {
            return None;
        }
        // An upright, unmirrored layer (the usual caption) is rasterized at
        // exactly its scale and, when its two axis scales agree closely
        // enough, placed on whole output pixels below, so the raster is
        // copied 1:1 instead of resampled (TX-004: crisp at the output
        // resolution). Other layers are resampled anyway; their scale is
        // rounded up to 1/64 so nearby scales share a raster.
        let tol = 1e-9 * axis;
        let upright = !projective
            && m.x_axis.y.abs() <= tol
            && m.y_axis.x.abs() <= tol
            && m.x_axis.x > 0.0
            && m.y_axis.y > 0.0;
        let scale = if upright {
            axis as f32
        } else {
            ((axis * 64.0).ceil() / 64.0) as f32
        };
        let raster = match &text.motion {
            Some(motion) => self.text.rasterize_animated(&text.spec, motion, scale),
            None => self.text.rasterize(&text.spec, scale),
        };
        if raster.image.is_empty() {
            return None;
        }
        // Image pixel → box px (undo the origin shift and the scale) → layer
        // coordinates (move the anchor to the origin).
        let s = f64::from(raster.scale);
        let anchor = text.anchor.point(raster.layout.size);
        let anchor = DVec2::new(f64::from(anchor.0), f64::from(anchor.1));
        let image_to_layer = DMat3::from_translation(-anchor)
            * DMat3::from_scale(DVec2::splat(1.0 / s))
            * DMat3::from_translation(DVec2::new(
                -f64::from(raster.origin.0),
                -f64::from(raster.origin.1),
            ));
        let size = (
            f64::from(raster.image.width),
            f64::from(raster.image.height),
        );
        let image_to_output = if projective {
            m * image_to_layer
        } else {
            snap_to_pixels(m * image_to_layer, size)
        };
        // The wipe runs over the text box at rest (layer coordinates).
        let (bw, bh) = (
            f64::from(raster.layout.size.0),
            f64::from(raster.layout.size.1),
        );
        let rect = (-anchor.x, -anchor.y, bw - anchor.x, bh - anchor.y);
        Prepared::premultiplied(image_to_output, Arc::clone(&raster.image), opacity)?
            .minified(&mut self.mips)
            .with_effects(m, rect, fx)
    }
}

/// Maps an image of `size` pixels to output pixels 1:1 when `m` is nearly a
/// translation: its linear part moves no image corner by more than half a
/// pixel from where a plain translation would put it (an imperceptible size
/// change, such as the preview's slightly different x and y output scales).
/// The translation is then rounded to whole pixels, so sampling copies the
/// image exactly. Otherwise `m` is returned unchanged.
fn snap_to_pixels(m: DMat3, size: (f64, f64)) -> DMat3 {
    const BUDGET: f64 = 0.5;
    let (w, h) = size;
    let identity = (m.x_axis.x - 1.0).abs() * w + m.y_axis.x.abs() * h <= BUDGET
        && (m.y_axis.y - 1.0).abs() * h + m.x_axis.y.abs() * w <= BUDGET;
    if identity && m.z_axis.x.is_finite() && m.z_axis.y.is_finite() {
        DMat3::from_translation(DVec2::new(m.z_axis.x.round(), m.z_axis.y.round()))
    } else {
        m
    }
}

impl Prepared {
    /// A flat colour over the layer rectangle.
    fn fill(m: DMat3, display: (f64, f64), color: [u8; 4], opacity: u32) -> Option<Prepared> {
        let to_layer = ToSource::inverse_of(m)?;
        let alpha = draw::mul255(u32::from(color[3]), opacity);
        if alpha == 0 {
            return None;
        }
        Some(Prepared {
            to_source: to_layer,
            source_size: display,
            source: PreparedSource::Fill([color[0], color[1], color[2]], alpha),
            opacity,
            bounds: draw::bounds(m, display),
            wipe: None,
            tint: None,
        })
    }

    /// A picture shown in the layer rectangle: display px are mapped back to
    /// the stored raster, undoing rotation, mirroring and pixel aspect.
    fn picture(
        m: DMat3,
        display: (f64, f64),
        picture: Arc<Picture>,
        opacity: u32,
    ) -> Option<Prepared> {
        let (dw, dh) = display;
        if dw <= 0.0 || dh <= 0.0 {
            return None;
        }
        let (w, h) = (f64::from(picture.width), f64::from(picture.height));
        let turned = matches!(picture.rotation, Rotation::Cw90 | Rotation::Cw270);
        // Display px → upright raster units (the raster turned for display).
        let (uw, uh) = if turned { (h, w) } else { (w, h) };
        let to_upright = DMat3::from_scale(DVec2::new(uw / dw, uh / dh));
        // Upright → stored raster (continuous coordinates).
        let unrotate = match picture.rotation {
            Rotation::None => DMat3::IDENTITY,
            // display (u, v) → stored (v, H - u)
            Rotation::Cw90 => DMat3::from_cols_array(&[0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, h, 1.0]),
            // → (W - u, H - v)
            Rotation::Cw180 => DMat3::from_cols_array(&[-1.0, 0.0, 0.0, 0.0, -1.0, 0.0, w, h, 1.0]),
            // → (W - v, u)
            Rotation::Cw270 => {
                DMat3::from_cols_array(&[0.0, 1.0, 0.0, -1.0, 0.0, 0.0, w, 0.0, 1.0])
            }
        };
        let unmirror = if picture.mirrored {
            DMat3::from_cols_array(&[-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, w, 0.0, 1.0])
        } else {
            DMat3::IDENTITY
        };
        let display_to_stored = unmirror * unrotate * to_upright;
        let to_source = ToSource::inverse_of(m)?.then(display_to_stored);
        Some(Prepared {
            to_source,
            source_size: (w, h),
            source: PreparedSource::Picture(picture),
            opacity,
            bounds: draw::bounds(m, display),
            wipe: None,
            tint: None,
        })
    }

    /// Draws a picture source from the mip level that matches its
    /// minification ([`mip::level_for`]); other layers are unchanged.
    fn minified(self, mips: &mut mip::MipCache) -> Prepared {
        let (pic, premultiplied) = match &self.source {
            PreparedSource::Picture(pic) => (pic, false),
            PreparedSource::Premultiplied(pic) => (pic, true),
            PreparedSource::Fill(..) => return self,
        };
        let m = self.to_source.matrix();
        // Source texels stepped per output pixel along x and y (for a layer
        // in perspective, the least at the picture's corners, so the near
        // side stays sharp).
        let rho = if self.to_source.is_projective() {
            let Some(forward) = draw::homography(m).inverse() else {
                return self;
            };
            let forward = draw::matrix_of(&forward);
            let (w, h) = self.source_size;
            let rho = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)]
                .into_iter()
                .filter_map(|(x, y)| draw::project(forward, DVec2::new(x, y)))
                .filter_map(|p| draw::local_scale(m, p))
                .fold(f64::INFINITY, f64::min);
            if rho.is_finite() { rho } else { 1.0 }
        } else {
            m.x_axis
                .truncate()
                .length()
                .max(m.y_axis.truncate().length())
        };
        let k = mip::level_for(rho);
        if k == 0 {
            return self;
        }
        let level = mips.level(pic, k, premultiplied);
        if Arc::ptr_eq(&level, pic) {
            return self;
        }
        let (w, h) = self.source_size;
        let (lw, lh) = (f64::from(level.width), f64::from(level.height));
        Prepared {
            to_source: self
                .to_source
                .then(DMat3::from_scale(DVec2::new(lw / w, lh / h))),
            source_size: (lw, lh),
            source: if premultiplied {
                PreparedSource::Premultiplied(level)
            } else {
                PreparedSource::Picture(level)
            },
            ..self
        }
    }

    /// An upright premultiplied image (a text raster) whose pixel space
    /// maps to output through `m`.
    fn premultiplied(m: DMat3, picture: Arc<Picture>, opacity: u32) -> Option<Prepared> {
        let size = (f64::from(picture.width), f64::from(picture.height));
        Some(Prepared {
            to_source: ToSource::inverse_of(m)?,
            source_size: size,
            source: PreparedSource::Premultiplied(picture),
            opacity,
            bounds: draw::bounds(m, size),
            wipe: None,
            tint: None,
        })
    }
}
