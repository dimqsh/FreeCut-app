//! Per-frame render plans (FX-001): what is visible at a sequence time, from
//! which source time, and where it lands on the canvas. Pure data; the
//! preview and the exporter render the same plan.
//!
//! # Coordinates
//!
//! Canvas pixels have their origin at the top-left corner, +x right, +y
//! down; a positive rotation is clockwise on screen. Each layer carries
//! `layer_to_canvas`, a 3×3 matrix (affine for now) from the layer's own
//! coordinates to canvas pixels, with the clip's transform (TR-001, TR-003)
//! already folded in. Its inverse maps canvas points back into the layer,
//! for hit-testing on-canvas handles (TR-007).
//!
//! - **Pictures** (video, stills, slates, generated items): layer pixels,
//!   origin top-left, `display_size` big. The matrix is
//!   `placement · translate(−canvas_centre) · fit`, where `fit`
//!   ([`fit_transform`]) is the TR-002 Fit baseline (scaled to fit the
//!   canvas, centred) and `placement` ([`clip_placement`]) applies the
//!   transform about the fitted picture's centre.
//! - **Text**: origin at the text box's anchor, units of canvas pixels at
//!   100 % scale. The anchor is the box's horizontal centre and, vertically,
//!   the edge named by `TextLayout::v_anchor` (TX-005): the top edge for
//!   `Top`, the centre for `Middle` (the default), the bottom edge for
//!   `Bottom`, so that edge stays put as lines come and go. The matrix is
//!   `placement` alone. The renderer lays the text out in canvas pixels
//!   (every ref-px length times [`LayerSource::Text::scale_px`]), offsets
//!   the laid-out box so its anchor sits at the origin, then applies
//!   `layer_to_canvas`.
//!   The clip's scale therefore never changes the layout (TX-004), and
//!   glyphs can be rasterized at their final size.
//!
//! # Keyframes and animations (TX-022's composition order)
//!
//! At sequence time `t` a clip is evaluated at its content time
//! ([`crate::model::Clip::content_time_at`]):
//! 1. **Style at `t`:** keyed transform and text values (TR-027, TX-021),
//!    then the style modulation of a playing whole-text Loop (hue cycle,
//!    breathing glow); `LayerSource::Text::data` is the result, the same
//!    `Arc` as the clip's when nothing changed.
//! 2. **Whole-block motion** of the playing slot (FX-018, FX-030) is folded
//!    into `layer_to_canvas` at L8: position offsets add in canvas pixels,
//!    scale factors multiply and rotations add about the preset's pivot
//!    (the transform anchor by default). A 3D turn makes the matrix
//!    projective (divide by the third coordinate).
//!    A Continuous slot plays at TX-022's seam envelope
//!    ([`crate::motion::seam_envelope`]): eased in after an In and out
//!    before an Out, so the pose is at rest at both seams.
//! 3. **Opacity:** the slot's opacity multiplies the clip's (L9).
//! 4. What a matrix can't carry (blur, a wipe, a white tint, TX-021's
//!    Reveal key) and the per-unit work of text presets
//!    ([`TextAnimPlan`], TX-023: drawn by `freecut-render::text::animate`,
//!    which knows the layout) travel in [`LayerPlan::motion`].
//!
//! [`frame_plan`] has no preset catalogue, so it plays keyframes only;
//! [`frame_plan_with`] takes the catalogue and plays animations too.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

use glam::{DMat3, DVec2};

use crate::ids::{ClipId, MediaId, SeqId, TrackId};
use crate::model::{
    AnimUnit, AnimationSpec, Clip, ClipContent, ColorStack, Direction, Document, Fill,
    GeneratedKind, Library, MediaItem, PerMille, Rgba8, Rotation, Slot, SlotWindow, TextClipData,
    TextStyle, Transform, slot_policy,
};
use crate::motion::{
    BlockOffsets, MotionScale, NoPresets, Pivot, PresetCatalog, PresetKind, PresetManifest,
    RestBounds, StyleUse, UnitContext, animation_offsets_with, effective_direction, effective_unit,
    pose_with, seam_envelope, slot_progress,
};
use crate::time::{Tick, TimeError};

/// The output canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasSpec {
    pub width: u32,
    pub height: u32,
    /// Straight RGBA code values.
    pub background: [u8; 4],
}

/// How a layer combines with what is below it (OV-004; Normal only in the
/// MVP).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BlendMode {
    #[default]
    Normal,
}

/// Where a layer's pixels come from.
#[derive(Debug, Clone, PartialEq)]
pub enum LayerSource {
    /// The video frame displayed at `source_time` (media time).
    Video { asset: MediaId, source_time: Tick },
    /// A still picture.
    Still { asset: MediaId },
    /// A text clip, laid out and rasterized by the renderer in the text
    /// layer coordinates described in the [module docs](self).
    Text {
        data: Arc<TextClipData>,
        /// Canvas pixels per ref px: the canvas's short edge / 1080 (TX
        /// conventions).
        scale_px: f64,
    },
    /// Generated content, rasterized by the renderer at clip-local time.
    Generated {
        kind: GeneratedKind,
        local_time: Tick,
    },
    /// The "media offline" slate for media whose file can't be found (EX-005).
    Slate { asset: MediaId },
}

/// One visible layer, with everything already evaluated.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerPlan {
    pub clip: ClipId,
    pub track: TrackId,
    pub source: LayerSource,
    /// Display size of the source in pixels: rotated and corrected for
    /// non-square pixels. For text, the canvas size: the box size is only
    /// known after layout.
    pub display_size: (u32, u32),
    /// Maps layer coordinates to canvas pixels, the clip's transform
    /// included (see the [module docs](self) for both layer coordinate
    /// systems).
    pub layer_to_canvas: DMat3,
    /// 0..=1: the clip's opacity (TR-015) at this time, times the playing
    /// animation's.
    pub opacity: f64,
    pub blend: BlendMode,
    /// Animation work the matrix and opacity can't carry; `None` when there
    /// is none.
    pub motion: Option<Arc<LayerMotion>>,
    /// Text: optional ligatures (`liga`, `clig`, `dlig`) may form. `false`
    /// for a clip with an animation that moves its letters one by one (in
    /// any slot), so every letter is its own glyph for the clip's whole
    /// length and the rest pose never changes shape (the text
    /// architecture's ligature policy). `true` for other layers.
    pub ligatures: bool,
    /// The clip's colour stack at this time, keyed values evaluated
    /// (CL-028, [`Clip::color_at`]), for video and still layers. `None`
    /// when there is none, it is an identity stack or it is bypassed
    /// (CL-001, CL-023), and on slates: offline media shows its warning
    /// ungraded.
    pub color: Option<Arc<ColorStack>>,
}

/// FX-030's Wipe reveal: the soft edge is 8 % of the layer's size along
/// the wipe.
pub const WIPE_FEATHER: f64 = 0.08;

/// What an animated layer needs besides its matrix and opacity.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerMotion {
    /// Blur radius of the whole layer, in canvas pixels (FX-030).
    pub blur_px: f64,
    /// The share of the layer a wipe shows, 0..=1, from `reveal_from`'s
    /// side (FX-030's wipe).
    pub reveal: f64,
    pub reveal_from: Direction,
    /// The wipe's soft edge as a share of the layer's size along the wipe
    /// ([`WIPE_FEATHER`] while a wipe plays, else 0). The edge sits at the
    /// `reveal` share, fading over this width on the hidden side, so
    /// `reveal` 0 shows nothing and 1 everything.
    pub reveal_feather: f64,
    /// White tint, 0..=1.
    pub tint: f64,
    /// TX-021's Reveal key on text: the share of characters shown, 0..=1.
    pub text_reveal: f64,
    /// The playing per-unit text animation (TX-023).
    pub text: Option<TextAnimPlan>,
}

/// A playing per-unit text animation, for `freecut-render::text::animate`
/// (TX-022 step 3, TX-023): it splits `spec`'s units from the layout, gives
/// each its window (or loop delay), and applies
/// [`crate::motion::pose_at`] to each about the unit's own centre.
#[derive(Debug, Clone, PartialEq)]
pub struct TextAnimPlan {
    pub slot: Slot,
    /// Clip-local time.
    pub local_t: Tick,
    /// Where the slot plays, in clip-local time.
    pub window: SlotWindow,
    pub spec: AnimationSpec,
    pub manifest: Arc<PresetManifest>,
    /// The effective unit (TX-023): never [`AnimUnit::Block`] here.
    pub unit: AnimUnit,
    pub direction: Option<Direction>,
    /// Slot progress 0..=1 ([`crate::motion::slot_progress`]): of the
    /// window for In and Out, of the cycle (from the clip start plus the
    /// phase) for loops.
    pub progress: f64,
    /// TX-022's seam envelope at `local_t` ([`seam_envelope`]): below 1
    /// only while a Loop eases in after In or out before Out. `manifest`
    /// already plays at this strength (its amplitudes are scaled), so a
    /// unit posed from it needs nothing more.
    pub envelope: f64,
    /// The sequence frame duration: Decode's stand-ins change, and
    /// Flicker's dips last, every 2 frames (TX-056).
    pub frame: Tick,
    /// The style values the preset modulates per unit (Shimmer's band of
    /// lightness, TX-056): the renderer applies each unit pose's `hue`,
    /// `glow`, `lightness` and `tint` to that unit's glyphs (TX-022 step
    /// 1's modulation, per unit).
    pub style: StyleUse,
}

impl TextAnimPlan {
    /// One unit's pose (TX-023): the manifest at the unit's own progress
    /// `p` (In: 0 = start pose, 1 = rest; Out: already reversed; Loop: the
    /// unit's phase), with lengths per `scale` and the unit's index and
    /// line geometry from `unit` (see [`UnitContext`] for what the layout
    /// supplies). The plan's frame fills `unit.frame` when it is `None`;
    /// the envelope is already in the manifest.
    pub fn unit_pose(&self, p: f64, scale: &MotionScale, unit: &UnitContext) -> BlockOffsets {
        let ctx = UnitContext {
            frame: unit.frame.or(Some(self.frame)),
            ..*unit
        };
        pose_with(&self.manifest, &self.spec, self.slot, p, scale, &ctx)
    }

    /// Whether posing units needs their line geometry (Spread, Word rise).
    pub fn needs_line_geometry(&self) -> bool {
        self.manifest.needs_line_geometry()
    }
}

/// Rest bounds of a text box, from the renderer's layout: `(min, max)` in
/// text layer coordinates (see the [module docs](self)).
pub type TextBounds<'a> = &'a dyn Fn(ClipId, &TextClipData, f64) -> Option<(DVec2, DVec2)>;

/// What animations need from outside the document.
#[derive(Clone, Copy)]
pub struct AnimContext<'a> {
    /// The loaded presets; a preset missing here plays as no animation
    /// (FX-018's **Unavailable animation**).
    pub presets: &'a dyn PresetCatalog,
    /// The laid-out box of a text clip at rest, for distances and pivots.
    /// Without it (or when it answers `None`) the box is estimated from the
    /// font size and the character count.
    pub text_bounds: Option<TextBounds<'a>>,
}

impl core::fmt::Debug for AnimContext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnimContext")
            .field("text_bounds", &self.text_bounds.is_some())
            .finish_non_exhaustive()
    }
}

impl AnimContext<'static> {
    /// No presets and no layout: keyframes only.
    pub const KEYS_ONLY: AnimContext<'static> = AnimContext {
        presets: &NoPresets,
        text_bounds: None,
    };
}

/// A node in render order (bottom first).
#[derive(Debug, Clone, PartialEq)]
pub enum PlanNode {
    Layer(LayerPlan),
}

/// Everything needed to render one output frame.
#[derive(Debug, Clone, PartialEq)]
pub struct FramePlan {
    /// Sequence time of this frame.
    pub time: Tick,
    /// Sequence frame duration.
    pub frame: Tick,
    /// Index of the frame on the sequence grid.
    pub frame_index: i64,
    pub canvas: CanvasSpec,
    /// Bottom to top. Hidden tracks and disabled clips are already left out.
    pub nodes: Vec<PlanNode>,
}

/// Runtime facts the document doesn't hold.
#[derive(Debug, Clone, Copy)]
pub struct PlanInputs<'a> {
    /// Media whose file is currently unavailable.
    pub offline: &'a BTreeSet<MediaId>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("sequence {0} doesn't exist")]
    NoSequence(SeqId),
    #[error(transparent)]
    Time(#[from] TimeError),
}

/// The plan for the frame displayed at `t` in sequence `seq`, with keyed
/// values evaluated; animations need a catalogue ([`frame_plan_with`]).
pub fn frame_plan(
    doc: &Document,
    seq: SeqId,
    t: Tick,
    inputs: &PlanInputs<'_>,
) -> Result<FramePlan, PlanError> {
    frame_plan_with(doc, seq, t, inputs, &AnimContext::KEYS_ONLY)
}

/// The plan for the frame displayed at `t` in sequence `seq`: keyed values
/// and the playing animations evaluated in TX-022's composition order (see
/// the [module docs](self)). Pure, so preview and export agree.
pub fn frame_plan_with(
    doc: &Document,
    seq: SeqId,
    t: Tick,
    inputs: &PlanInputs<'_>,
    anim: &AnimContext<'_>,
) -> Result<FramePlan, PlanError> {
    let sequence = doc.sequence(seq).ok_or(PlanError::NoSequence(seq))?;
    let settings = doc.settings();
    let canvas = CanvasSpec {
        width: settings.width,
        height: settings.height,
        background: [0, 0, 0, 255],
    };
    let canvas_size = (canvas.width, canvas.height);
    let scale_px = ref_px_scale(canvas_size);
    let frame = settings.frame_rate.frame_ticks();
    let mut nodes = Vec::new();
    for track in sequence.visual_tracks() {
        if track.header.flags.hidden {
            continue;
        }
        let Some(clip) = track.clip_at(t).filter(|c| c.enabled) else {
            continue;
        };
        let local = t.checked_sub(clip.start())?;
        // Content time anchors keys (TR-027); an overflow there falls back
        // to clip-local time.
        let content_t = clip.content_time_at(t).unwrap_or(local);
        let picture = |asset: MediaId, live: LayerSource| {
            let source = if inputs.offline.contains(&asset) {
                LayerSource::Slate { asset }
            } else {
                live
            };
            (source, display_size(doc.library(), asset))
        };
        let playing = active_animation(clip, local, frame, anim);
        let (source, display_size) = match &clip.content {
            ClipContent::Media { asset, source, .. } => {
                let source_time = source.start().checked_add(local)?;
                picture(
                    *asset,
                    LayerSource::Video {
                        asset: *asset,
                        source_time,
                    },
                )
            }
            ClipContent::Image { asset } => picture(*asset, LayerSource::Still { asset: *asset }),
            ClipContent::Text(_) => {
                let mut data = clip
                    .text_at(content_t)
                    .unwrap_or_else(|| Arc::new(TextClipData::default()));
                if let Some(p) = playing.as_ref().filter(|p| p.slot == Slot::Continuous)
                    && p.per_unit.is_none()
                {
                    modulate_style(&mut data, &p.offsets);
                }
                // A per-unit glow scales each unit's glow (the renderer's):
                // with Glow off, it plays on the temporary glow.
                if playing
                    .as_ref()
                    .and_then(|p| p.per_unit.as_ref())
                    .is_some_and(|plan| plan.style.glow)
                    && !data.style.glow.on
                {
                    temporary_glow(&mut Arc::make_mut(&mut data).style);
                }
                (LayerSource::Text { data, scale_px }, canvas_size)
            }
            ClipContent::Generated { kind } => (
                LayerSource::Generated {
                    kind: *kind,
                    local_time: local,
                },
                canvas_size,
            ),
        };
        let display_size = if display_size.0 == 0 || display_size.1 == 0 {
            canvas_size
        } else {
            display_size
        };
        let transform = clip.transform_at(content_t);
        let is_text = matches!(source, LayerSource::Text { .. });
        // Layer coordinates → centred placement coordinates (see the
        // module docs).
        let to_centred = match &source {
            LayerSource::Text { .. } => DMat3::IDENTITY,
            _ => {
                let centre = DVec2::new(f64::from(canvas.width), f64::from(canvas.height)) / 2.0;
                DMat3::from_translation(-centre) * fit_transform(display_size, canvas_size)
            }
        };
        // The layer's bounds at rest in centred placement coordinates.
        let local_bounds = || match &source {
            LayerSource::Text { data, .. } => text_rest_bounds(clip.id, data, scale_px, anim),
            _ => {
                let half = fit_transform(display_size, canvas_size).transform_vector2(DVec2::new(
                    f64::from(display_size.0),
                    f64::from(display_size.1),
                )) / 2.0;
                (-half, half)
            }
        };
        let mut layer_to_canvas = clip_placement(&transform, canvas_size) * to_centred;
        let mut opacity = transform.opacity.as_f64();
        let mut motion = LayerMotion {
            blur_px: 0.0,
            reveal: 1.0,
            reveal_from: Direction::Left,
            reveal_feather: 0.0,
            tint: 0.0,
            text_reveal: if is_text {
                f64::from(clip.reveal_at(content_t).get()) / 1000.0
            } else {
                1.0
            },
            text: None,
        };
        if let Some(p) = playing {
            if let Some(plan) = p.per_unit {
                motion.text = Some(plan);
            } else {
                let bounds = local_bounds();
                let rest = rest_bounds(&transform, canvas_size, bounds, is_text, &source);
                let ctx = UnitContext {
                    frame: Some(frame),
                    amount: p.envelope,
                    ..UnitContext::BLOCK
                };
                let o = animation_offsets_with(
                    &p.manifest,
                    &p.spec,
                    p.slot,
                    local,
                    p.window,
                    &rest,
                    &ctx,
                );
                layer_to_canvas = folded(&transform, canvas_size, bounds, &o) * to_centred;
                opacity *= o.opacity;
                motion.blur_px = o.blur;
                motion.reveal = o.reveal;
                motion.reveal_from = o.reveal_from;
                if o.reveal < 1.0 {
                    motion.reveal_feather = WIPE_FEATHER;
                }
                motion.tint = o.tint;
            }
        }
        let still = motion.text.is_none()
            && motion.blur_px == 0.0
            && motion.reveal >= 1.0
            && motion.tint == 0.0
            && motion.text_reveal >= 1.0;
        let color = match source {
            LayerSource::Video { .. } | LayerSource::Still { .. } => clip
                .color_at(content_t)
                .filter(|stack| !stack.is_identity()),
            _ => None,
        };
        nodes.push(PlanNode::Layer(LayerPlan {
            clip: clip.id,
            track: track.id,
            source,
            display_size,
            layer_to_canvas,
            opacity,
            blend: BlendMode::Normal,
            motion: (!still).then(|| Arc::new(motion)),
            ligatures: !(is_text && moves_letters(clip, anim)),
            color,
        }));
    }
    let rate = settings.frame_rate;
    Ok(FramePlan {
        time: t,
        frame: rate.frame_ticks(),
        frame_index: rate.frame_index(t),
        canvas,
        nodes,
    })
}

/// Whether a slot of `clip` plays a loaded preset on letters (TX-023's
/// Letter unit, whatever the text's script).
fn moves_letters(clip: &Clip, anim: &AnimContext<'_>) -> bool {
    Slot::ALL.into_iter().any(|slot| {
        clip.animations.get(slot).is_some_and(|spec| {
            anim.presets
                .resolve(slot, &spec.preset)
                .is_some_and(|m| effective_unit(&m, spec, true) == AnimUnit::Letter)
        })
    })
}

/// The slot playing on `clip` at clip-local `local`, with its manifest.
struct Playing<'c> {
    slot: Slot,
    window: SlotWindow,
    /// As stored, or a Loop with its cycle raised to the preset's shortest
    /// safe cycle ([`PresetCatalog::min_cycle`]).
    spec: Cow<'c, AnimationSpec>,
    manifest: Arc<PresetManifest>,
    /// TX-022's seam envelope (1 outside a Loop's ramps).
    envelope: f64,
    /// Block-level offsets for the style modulation of loops (only
    /// meaningful without `per_unit`); distances are not needed for style.
    offsets: BlockOffsets,
    per_unit: Option<TextAnimPlan>,
}

fn active_animation<'c>(
    clip: &'c Clip,
    local: Tick,
    frame: Tick,
    anim: &AnimContext<'_>,
) -> Option<Playing<'c>> {
    if clip.animations.is_empty() {
        return None;
    }
    let policy = slot_policy(&clip.content)?;
    let active = clip
        .animations
        .active_at(policy, clip.duration(), frame, local)?;
    let manifest = anim.presets.resolve(active.slot, &active.spec.preset)?;
    // FX-030 over FX-019: a Loop never cycles faster than it may without
    // strobing, whatever cycle is stored (an older or hand-edited file).
    let mut spec = Cow::Borrowed(active.spec);
    if active.slot == Slot::Continuous && manifest.kind == PresetKind::Loop {
        let floor = anim.presets.min_cycle(&active.spec.preset);
        if spec.duration < floor {
            spec.to_mut().duration = floor;
        }
    }
    let text = clip.content.text().is_some();
    let unit = effective_unit(&manifest, &spec, text);
    let progress = slot_progress(&manifest, &spec, local, active.window);
    let envelope = if active.slot == Slot::Continuous {
        seam_envelope(&spec, active.window, clip.duration(), local)
    } else {
        1.0
    };
    let per_unit = (unit != AnimUnit::Block).then(|| TextAnimPlan {
        slot: active.slot,
        local_t: local,
        window: active.window,
        spec: spec.clone().into_owned(),
        // The renderer poses units from this manifest: scaled while the
        // seam envelope ramps, so every unit eases in and out with it.
        manifest: if envelope < 1.0 {
            Arc::new(manifest.scaled(envelope))
        } else {
            Arc::clone(&manifest)
        },
        unit,
        direction: effective_direction(&manifest, &spec, active.slot),
        progress,
        envelope,
        frame,
        style: manifest.style_channels(),
    });
    let unit_scale = MotionScale {
        se_px: 0.0,
        em_px: 0.0,
        travel_px: 0.0,
    };
    let ctx = UnitContext {
        frame: Some(frame),
        amount: envelope,
        ..UnitContext::BLOCK
    };
    let offsets = pose_with(&manifest, &spec, active.slot, progress, &unit_scale, &ctx);
    Some(Playing {
        slot: active.slot,
        window: active.window,
        spec,
        manifest,
        envelope,
        offsets,
        per_unit,
    })
}

/// The text box at rest in text layer coordinates: from the renderer's
/// layout when the context has it, else an estimate (0.55 em per
/// character, 1.2 em per line, anchored as TX-005 says).
fn text_rest_bounds(
    clip: ClipId,
    data: &TextClipData,
    scale_px: f64,
    anim: &AnimContext<'_>,
) -> (DVec2, DVec2) {
    if let Some(bounds) = anim.text_bounds.and_then(|f| f(clip, data, scale_px)) {
        return bounds;
    }
    let em = data.style.size.as_f64() * scale_px;
    let lines: Vec<&str> = data.content.as_str().split('\n').collect();
    let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let spacing = 1.0 + data.style.letter_spacing.as_f64();
    let w = (widest as f64 * 0.55 * spacing * em).max(em * 0.55);
    let h = lines.len().max(1) as f64 * 1.2 * data.layout.line_spacing.as_f64() * em;
    let (top, bottom) = match data.layout.v_anchor {
        crate::model::VAnchor::Top => (0.0, h),
        crate::model::VAnchor::Middle => (-h / 2.0, h / 2.0),
        crate::model::VAnchor::Bottom => (-h, 0.0),
    };
    (DVec2::new(-w / 2.0, top), DVec2::new(w / 2.0, bottom))
}

/// The layer at rest on the canvas, for FX-030's distances.
fn rest_bounds(
    transform: &Transform,
    canvas: (u32, u32),
    (min, max): (DVec2, DVec2),
    text: bool,
    source: &LayerSource,
) -> RestBounds {
    let m = clip_placement(transform, canvas);
    let corners = [min, DVec2::new(max.x, min.y), max, DVec2::new(min.x, max.y)]
        .map(|c| m.transform_point2(c));
    let lo = corners.iter().fold(DVec2::INFINITY, |a, c| a.min(*c));
    let hi = corners.iter().fold(DVec2::NEG_INFINITY, |a, c| a.max(*c));
    let canvas_v = DVec2::new(f64::from(canvas.0), f64::from(canvas.1));
    let em_px = match source {
        LayerSource::Text { data, scale_px } => {
            data.style.size.as_f64() * scale_px * transform.scale.factor()
        }
        _ => canvas_v.x.min(canvas_v.y) / 16.0,
    };
    RestBounds {
        canvas: canvas_v,
        min: lo,
        max: hi,
        em_px,
        text,
    }
}

/// [`clip_placement`] with block offsets folded in at L8 (FX-030): the
/// translation added to the position in canvas pixels, the rotation, the
/// width and height factors and any 3D turn applied about the pivot,
/// inside the clip's own scale and rotation and outside its flips.
fn folded(
    transform: &Transform,
    canvas: (u32, u32),
    (min, max): (DVec2, DVec2),
    o: &BlockOffsets,
) -> DMat3 {
    let flip = DVec2::new(
        if transform.flip_h { -1.0 } else { 1.0 },
        if transform.flip_v { -1.0 } else { 1.0 },
    );
    // The bounds as seen after the flips, where the offsets act.
    let (a, b) = (min * flip, max * flip);
    let (lo, hi) = (a.min(b), a.max(b));
    let pivot = match o.pivot {
        Pivot::Anchor => DVec2::ZERO,
        Pivot::TopCenter => DVec2::new((lo.x + hi.x) / 2.0, lo.y),
        Pivot::BottomCenter => DVec2::new((lo.x + hi.x) / 2.0, hi.y),
        Pivot::Edge => match o.reveal_from {
            Direction::Right => DVec2::new(hi.x, (lo.y + hi.y) / 2.0),
            Direction::Up => DVec2::new((lo.x + hi.x) / 2.0, lo.y),
            Direction::Down => DVec2::new((lo.x + hi.x) / 2.0, hi.y),
            _ => DVec2::new(lo.x, (lo.y + hi.y) / 2.0),
        },
    };
    let eye = 1.5 * (hi.x - lo.x).max(1.0);
    let turn = |deg_x: f64, deg_y: f64| {
        let (ty, tx) = (deg_y.to_radians(), deg_x.to_radians());
        let about_vertical = DMat3::from_cols(
            glam::DVec3::new(ty.cos(), 0.0, ty.sin() / eye),
            glam::DVec3::Y,
            glam::DVec3::Z,
        );
        let about_horizontal = DMat3::from_cols(
            glam::DVec3::X,
            glam::DVec3::new(0.0, tx.cos(), tx.sin() / eye),
            glam::DVec3::Z,
        );
        about_horizontal * about_vertical
    };
    let local = DMat3::from_translation(pivot)
        * DMat3::from_angle(o.rotation.to_radians())
        * DMat3::from_scale(o.scale)
        * turn(o.turn.x, o.turn.y)
        * DMat3::from_translation(-pivot);
    let (px, py) = transform.position.to_px(canvas.0, canvas.1);
    let anchor = DVec2::new(
        f64::from(canvas.0) / 2.0 + px,
        f64::from(canvas.1) / 2.0 + py,
    ) + o.translate;
    DMat3::from_translation(anchor)
        * DMat3::from_angle(transform.rotation.degrees().to_radians())
        * DMat3::from_scale(DVec2::splat(transform.scale.factor()))
        * local
        * DMat3::from_scale(flip)
}

/// A whole-text Loop's style modulation (TX-022 step 1, TX-056): hue turned
/// on fill and outline, glow intensity scaled (a temporary glow in the
/// fill colour at 25 % radius when Glow is off, leaving the card off), fill
/// lightened. Never written back: only the plan's copy changes.
fn modulate_style(data: &mut Arc<TextClipData>, o: &BlockOffsets) {
    let hue = o.hue.abs() > 1e-9;
    let glow = (o.glow - 1.0).abs() > 1e-9;
    let light = o.lightness.abs() > 1e-9;
    if !(hue || glow || light) {
        return;
    }
    let d = Arc::make_mut(data);
    let s = &mut d.style;
    let recolour = |c: Rgba8| {
        let c = if hue { turn_hue(c, o.hue) } else { c };
        if light { lighten(c, o.lightness) } else { c }
    };
    match &mut s.fill {
        Fill::Solid(c) => *c = recolour(*c),
        Fill::LinearGradient(g) => g.stops = recoloured(&g.stops, recolour),
        Fill::RadialGradient(g) => g.stops = recoloured(&g.stops, recolour),
    }
    if hue {
        s.stroke.value.color = turn_hue(s.stroke.value.color, o.hue);
    }
    if glow {
        if !s.glow.on {
            temporary_glow(s);
        }
        let v = f64::from(s.glow.value.intensity.get()) * o.glow;
        // Clamped into u16 range.
        s.glow.value.intensity = PerMille::new(v.round().clamp(0.0, 1000.0) as u16);
    }
}

/// TX-056's temporary glow for a glow-modulating preset on text whose
/// Glow is off: the set intensity in the fill colour at 25 % radius. Only
/// the plan's copy changes; the Glow card stays off.
fn temporary_glow(s: &mut TextStyle) {
    s.glow.on = true;
    s.glow.value.color = s.fill.colors().first().copied().unwrap_or(Rgba8::WHITE);
    s.glow.value.radius = PerMille::new(250);
}

fn recoloured(
    stops: &crate::model::GradientStops,
    f: impl Fn(Rgba8) -> Rgba8,
) -> crate::model::GradientStops {
    let mut out = stops.clone();
    for (i, stop) in stops.as_slice().iter().enumerate() {
        if let Some(next) = out.with_color(i, f(stop.color)) {
            out = next;
        }
    }
    out
}

/// `c` with its HSL hue turned by `deg`, lightness and saturation kept.
fn turn_hue(c: Rgba8, deg: f64) -> Rgba8 {
    let (r, g, b) = (
        f64::from(c.r) / 255.0,
        f64::from(c.g) / 255.0,
        f64::from(c.b) / 255.0,
    );
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d <= 1e-12 {
        return c;
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    let h = (h + deg).rem_euclid(360.0);
    let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = chroma * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - chroma / 2.0;
    let (r1, g1, b1) = match (h / 60.0) as u8 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    // Each channel is in 0..=1 by construction; clamped for rounding.
    let to8 = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgba8::new(to8(r1), to8(g1), to8(b1), c.a)
}

/// `c` moved `amount` (0..=1) of the way to white.
fn lighten(c: Rgba8, amount: f64) -> Rgba8 {
    let a = amount.clamp(0.0, 1.0);
    // In 0..=255 by construction.
    let up = |v: u8| {
        (f64::from(v) + (255.0 - f64::from(v)) * a)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Rgba8::new(up(c.r), up(c.g), up(c.b), c.a)
}

/// The media's picture size as displayed: rotation applied and non-square
/// pixels corrected (width stretched by the sample aspect ratio).
pub fn display_size(library: &Library, asset: MediaId) -> (u32, u32) {
    library
        .get(asset)
        .map_or((0, 0), |item| item_display_size(item))
}

fn item_display_size(item: &MediaItem) -> (u32, u32) {
    if let Some(v) = item.info.video.first() {
        let (sar_n, sar_d) = if v.sar.0 == 0 || v.sar.1 == 0 {
            (1, 1)
        } else {
            v.sar
        };
        let width = u32::try_from(u64::from(v.width) * u64::from(sar_n) / u64::from(sar_d))
            .unwrap_or(u32::MAX);
        match v.rotation {
            Rotation::Cw90 | Rotation::Cw270 => (v.height, width),
            Rotation::None | Rotation::Cw180 => (width, v.height),
        }
    } else {
        item.info.still.unwrap_or((0, 0))
    }
}

/// Canvas pixels per ref px for a canvas of this size: its short edge /
/// 1080 (TX conventions).
pub fn ref_px_scale(canvas: (u32, u32)) -> f64 {
    f64::from(canvas.0.min(canvas.1)) / 1080.0
}

/// The clip transform as a matrix from centred layer coordinates (origin at
/// the layer's centre, canvas pixels at 100 % scale) to canvas pixels:
/// flip, then scale, then rotate clockwise about the origin, then move the
/// origin to the canvas centre plus the clip's position (TR-001, TR-003,
/// TR shared conventions).
pub fn clip_placement(transform: &Transform, canvas: (u32, u32)) -> DMat3 {
    let (px, py) = transform.position.to_px(canvas.0, canvas.1);
    let anchor = DVec2::new(
        f64::from(canvas.0) / 2.0 + px,
        f64::from(canvas.1) / 2.0 + py,
    );
    let flip = DVec2::new(
        if transform.flip_h { -1.0 } else { 1.0 },
        if transform.flip_v { -1.0 } else { 1.0 },
    );
    // With +y down, glam's counter-clockwise rotation turns clockwise on
    // screen.
    DMat3::from_translation(anchor)
        * DMat3::from_angle(transform.rotation.degrees().to_radians())
        * DMat3::from_scale(DVec2::splat(transform.scale.factor()))
        * DMat3::from_scale(flip)
}

/// Scales `layer` to fit inside `canvas` (aspect preserved) and centers it.
pub fn fit_transform(layer: (u32, u32), canvas: (u32, u32)) -> DMat3 {
    let (lw, lh) = (f64::from(layer.0.max(1)), f64::from(layer.1.max(1)));
    let (cw, ch) = (f64::from(canvas.0), f64::from(canvas.1));
    let scale = (cw / lw).min(ch / lh);
    let (ox, oy) = ((cw - lw * scale) / 2.0, (ch - lh * scale) / 2.0);
    DMat3::from_translation(glam::DVec2::new(ox, oy)) * DMat3::from_scale(glam::DVec2::splat(scale))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Angle, CanvasPos, Scale};

    fn near(a: DVec2, b: DVec2) -> bool {
        (a - b).length() < 1e-9
    }

    #[test]
    fn identity_placement_centres_the_origin() {
        let m = clip_placement(&Transform::IDENTITY, (1920, 1080));
        assert!(near(
            m.transform_point2(DVec2::ZERO),
            DVec2::new(960.0, 540.0)
        ));
        assert!(near(
            m.transform_point2(DVec2::new(10.0, 5.0)),
            DVec2::new(970.0, 545.0)
        ));
    }

    #[test]
    fn placement_flips_then_scales_then_rotates_clockwise_then_moves() {
        let t = Transform {
            position: CanvasPos::new(250_000, -500_000), // +480 px, -540 px
            scale: Scale::from_percent(200),
            rotation: Angle::from_degrees(90),
            flip_h: true,
            ..Transform::IDENTITY
        };
        let m = clip_placement(&t, (1920, 1080));
        // (10, 0): flipped to (-10, 0), scaled to (-20, 0), rotated 90°
        // clockwise to (0, -20) (up on screen), moved by (1440, 0).
        assert!(near(
            m.transform_point2(DVec2::new(10.0, 0.0)),
            DVec2::new(1440.0, -20.0)
        ));
        // (0, 10) (down): rotated clockwise it points left.
        assert!(near(
            m.transform_point2(DVec2::new(0.0, 10.0)),
            DVec2::new(1420.0, 0.0)
        ));
    }

    #[test]
    fn ref_px_follow_the_short_edge() {
        assert!((ref_px_scale((1920, 1080)) - 1.0).abs() < 1e-12);
        assert!((ref_px_scale((1080, 1920)) - 1.0).abs() < 1e-12);
        assert!((ref_px_scale((3840, 2160)) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn fit_letterboxes_wide_footage_on_a_tall_canvas() {
        let m = fit_transform((1920, 1080), (1080, 1920));
        let top_left = m.transform_point2(DVec2::ZERO);
        let bottom_right = m.transform_point2(DVec2::new(1920.0, 1080.0));
        assert!((top_left.x - 0.0).abs() < 1e-9);
        assert!((bottom_right.x - 1080.0).abs() < 1e-9);
        // 1080 wide → scale 0.5625 → 607.5 tall, centered vertically.
        assert!((bottom_right.y - top_left.y - 607.5).abs() < 1e-9);
        assert!((top_left.y - (1920.0 - 607.5) / 2.0).abs() < 1e-9);
    }

    #[test]
    fn fit_is_identity_for_matching_sizes() {
        let m = fit_transform((1920, 1080), (1920, 1080));
        assert!(m.abs_diff_eq(DMat3::IDENTITY, 1e-12));
    }
}
