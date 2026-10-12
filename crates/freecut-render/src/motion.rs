//! The renderer's side of the timeline's animations (TX-022, TX-023,
//! FX-018, FX-030): planning frames with the preset catalogue, and turning
//! what the plan carries for a text layer into a [`TextMotion`].
//!
//! The frame plan has done most of the work in TX-022's composition order
//! before anything reaches here: keyed style values are already evaluated
//! into the layer's text data (step 1), whole-block offsets are folded into
//! `layer_to_canvas` (step 2) and the slot's opacity is multiplied into the
//! layer's (step 3). What is left is the per-unit work of text presets
//! ([`TextAnimPlan`]) and TX-021's Reveal key, both in
//! [`LayerMotion`]. [`text_motion`] maps them onto the renderer's own
//! request: unit, order, timing and Decode are translated, and every
//! unit's pose is the timeline's own [`TextAnimPlan::unit_pose`] at that
//! unit's progress, so the curves, easing, intensity, directions, seeded
//! channels and TX-022's loop seam envelope are evaluated in one place for
//! blocks and units alike. The renderer supplies what only its layout
//! knows ([`UnitContext`]): each unit's reading-order index (Scatter, Fall
//! away, Shake, Wiggle, Sparkle and Flicker are seeded per unit), the way
//! from the unit's centre to its line's centre (Spread) and its line's
//! height (Word rise, whose units are drawn clipped to their line's box,
//! [`GlyphAnim::line_clip`]).
//!
//! [`plan_frame`] plans a frame with a preset catalogue and text boxes
//! measured by this renderer's layout (FX-030's **From edge** and pivots
//! need the box at rest). Preview and export both plan through it (export
//! via [`crate::ExportRenderer`], preview via [`crate::Compositor::plan`]),
//! so they draw the same pixels.
//!
//! A whole layer's blur, wipe and white tint ([`LayerMotion::blur_px`],
//! `reveal`, `tint`) are the compositor's ([`crate::cpu::LayerEffects`]),
//! as is the perspective of a whole-layer 3D turn (a projective
//! `layer_to_canvas`). Per unit, the style channel `style.lightness`
//! (Shimmer's band) becomes [`GlyphAnim::tint`] toward white,
//! `style.hue` turns the unit's fill and outline ([`GlyphAnim::hue`]) and
//! `style.glow_intensity` scales the glow around it ([`GlyphAnim::glow`]).
//!
//! Typewriter's **Caret** is stored with the slot (`AnimParams::caret`,
//! TX-056) and drawn by [`text_motion`]; Decode's stand-ins change every
//! two frames of the sequence. [`TextMotionOptions`] lets an integrator
//! override both (a caret on every stepped reveal, another Decode step);
//! the app uses the defaults for preview and export alike.

use std::cell::RefCell;

use freecut_timeline::glam::{DMat3, DVec2};
use freecut_timeline::ids::{ClipId, SeqId};
use freecut_timeline::model::{
    AnimOrder, AnimUnit as ModelUnit, Direction, Document, PerMille, Slot, TextClipData, Travel,
};
use freecut_timeline::motion::{
    BlockOffsets, ChannelProp, Curve, MotionScale, PresetCatalog, PresetKind, PresetManifest,
    UnitContext,
};
use freecut_timeline::plan::{
    AnimContext, FramePlan, LayerMotion, LayerPlan, LayerSource, PlanError, PlanInputs,
    TextAnimPlan, frame_plan_with,
};
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};

use crate::cpu::TextAnchor;
use crate::text::animate::{
    AnimUnit, Caret, ChannelTable, Decode, GlyphAnim, TextMotion, UnitChannels, UnitFn, UnitOrder,
    UnitSample, UnitTiming,
};
use crate::text::{TextRenderer, TextSpec};

/// How long each Decode stand-in shows when the frame duration isn't
/// known: two frames at 30 fps. [`text_motion`] takes TX-056's two frames
/// of the plan's sequence ([`TextAnimPlan::frame`]); the compositor
/// applies [`TextMotionOptions::decode_step`].
pub const DECODE_STEP: Tick = Tick::from_raw(TICKS_PER_SECOND / 15);

/// TX-056's Typewriter caret blink: 0.5 s on, 0.5 s off.
pub const CARET_BLINK: Tick = Tick::from_raw(TICKS_PER_SECOND);

/// How long each Decode stand-in shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecodeStep {
    /// This many frames of the sequence (TX-056: 2); at least one tick.
    Frames(u32),
    /// A fixed time, whatever the frame rate.
    Fixed(Tick),
}

impl Default for DecodeStep {
    fn default() -> DecodeStep {
        DecodeStep::Frames(2)
    }
}

impl DecodeStep {
    /// The step for frames `frame` long; [`DECODE_STEP`] for `Frames` when
    /// `frame` isn't positive. Never below one tick.
    pub fn resolve(self, frame: Tick) -> Tick {
        let raw = match self {
            DecodeStep::Frames(n) if frame.raw() > 0 => frame.raw().saturating_mul(i64::from(n)),
            DecodeStep::Frames(_) => DECODE_STEP.raw(),
            DecodeStep::Fixed(t) => t.raw(),
        };
        Tick::from_raw(raw.max(1))
    }
}

/// Compositor-wide overrides of per-unit text animation, set by the
/// integrator ([`crate::Compositor::with_text_motion_options`]): the
/// defaults follow the project (each slot's stored caret, TX-056's 2-frame
/// Decode step).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextMotionOptions {
    /// Typewriter's **Caret** option (TX-056; off by default): the bar
    /// rides after the last revealed letter and blinks with this period
    /// once typing ends ([`CARET_BLINK`] per TX-056). Drawn for presets
    /// whose letters appear with a step (Typewriter) in an In or Out slot.
    pub caret: Option<Caret>,
    /// How long each Decode stand-in shows.
    pub decode_step: DecodeStep,
}

impl TextMotionOptions {
    /// Applies the options to `motion`, the per-unit motion of `layer` in a
    /// plan whose frames are `frame` long: Decode's step, and the caret
    /// when the layer's playing preset is a stepped reveal. A motion that
    /// already has a caret keeps it.
    pub fn apply(&self, motion: &mut TextMotion, layer: &LayerPlan, frame: Tick) {
        if let Some(decode) = motion.decode.as_mut() {
            decode.step = self.decode_step.resolve(frame);
        }
        let stepped = layer
            .motion
            .as_deref()
            .and_then(|m| m.text.as_ref())
            .is_some_and(|plan| plan.slot != Slot::Continuous && is_stepped(&plan.manifest));
        if motion.caret.is_none() && stepped {
            motion.caret = self.caret;
        }
    }
}

/// Whether `manifest` shows its units with a step (Typewriter): an
/// opacity or reveal channel on a [`Curve::Step`].
pub fn is_stepped(manifest: &PresetManifest) -> bool {
    manifest.channels.iter().any(|c| {
        matches!(c.curve, Curve::Step)
            && matches!(c.property, ChannelProp::Opacity | ChannelProp::Reveal)
    })
}

/// The plan of the frame displayed at `t` in sequence `seq`, with the
/// animations of `presets` (a preset missing there plays as no animation,
/// FX-018) and text boxes at rest measured by `text`'s layout.
pub fn plan_frame(
    doc: &Document,
    seq: SeqId,
    t: Tick,
    inputs: &PlanInputs<'_>,
    presets: &dyn PresetCatalog,
    text: &mut TextRenderer,
) -> Result<FramePlan, PlanError> {
    let text = RefCell::new(text);
    let bounds = |_: ClipId, data: &TextClipData, scale_px: f64| {
        // The plan asks for one box at a time; a nested call (none today)
        // falls back to the plan's estimate.
        let mut renderer = text.try_borrow_mut().ok()?;
        text_rest_bounds(&mut renderer, data, scale_px)
    };
    let ctx = AnimContext {
        presets,
        text_bounds: Some(&bounds),
    };
    frame_plan_with(doc, seq, t, inputs, &ctx)
}

/// The text box of `data` at rest in text layer coordinates (`(min, max)`,
/// origin at the anchor the plan's module docs describe), from the
/// layout. `None` for empty text or an unusable scale.
pub fn text_rest_bounds(
    text: &mut TextRenderer,
    data: &TextClipData,
    scale_px: f64,
) -> Option<(DVec2, DVec2)> {
    if data.content.is_empty() || !scale_px.is_finite() || scale_px <= 0.0 {
        return None;
    }
    let spec = TextSpec::from_clip(data, scale_px);
    let (w, h) = text.box_size(&spec);
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let (ax, ay) = TextAnchor::for_v_anchor(data.layout.v_anchor).point((w, h));
    let (ax, ay, w, h) = (f64::from(ax), f64::from(ay), f64::from(w), f64::from(h));
    Some((DVec2::new(-ax, -ay), DVec2::new(w - ax, h - ay)))
}

/// The per-unit motion of a text layer: its playing per-unit preset and
/// its Reveal key. `None` when the layer isn't text or shows its text at
/// rest.
pub fn layer_text_motion(layer: &LayerPlan) -> Option<TextMotion> {
    let LayerSource::Text { data, scale_px } = &layer.source else {
        return None;
    };
    let motion: &LayerMotion = layer.motion.as_deref()?;
    let reveal = reveal_key(motion.text_reveal);
    match &motion.text {
        Some(plan) => {
            let lengths = UnitLengths::estimate(
                data,
                *scale_px,
                layer.layer_to_canvas,
                layer.display_size,
                plan.direction,
            );
            let mut m = text_motion(plan, &lengths);
            m.reveal = reveal;
            Some(m)
        }
        None => reveal.map(|r| TextMotion {
            reveal: Some(r),
            ..TextMotion::new(
                AnimUnit::Grapheme,
                UnitOrder::Forward,
                UnitTiming::In {
                    span: Tick::ZERO,
                    overlap: PerMille::ONE,
                },
                Tick::ZERO,
                UnitChannels::Table(ChannelTable::default()),
            )
        }),
    }
}

/// TX-021's Reveal key (0..=1) as the renderer's share of letters; `None`
/// when everything shows.
fn reveal_key(v: f64) -> Option<PerMille> {
    if v.is_nan() || v >= 1.0 {
        return None;
    }
    // In 0..=1000 after the clamp.
    Some(PerMille::new((v.max(0.0) * 1000.0).round() as u16))
}

/// The lengths a per-unit preset measures in, in em of the text: the unit
/// of [`GlyphAnim`] offsets, applied in the text's own frame before
/// `layer_to_canvas` (which carries the clip's scale).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitLengths {
    /// The canvas short edge (FX-030's `se`), in em as drawn.
    pub short_edge_em: f64,
    /// FX-030's **From edge** distance for the plan's direction, in em: a
    /// unit anywhere in the text starts just outside the canvas edge.
    pub from_edge_em: f64,
}

impl UnitLengths {
    /// The lengths for a text layer of `data` at `scale_px` canvas px per
    /// ref px, placed by `layer_to_canvas` on a `canvas`-sized canvas.
    ///
    /// The From edge distance uses the plan's estimate of the box at rest
    /// (0.55 em per character, 1.2 em per line): the distance from the
    /// layer's anchor to the edge on `direction`'s side plus the whole box
    /// and one em, so even the farthest unit starts outside.
    pub fn estimate(
        data: &TextClipData,
        scale_px: f64,
        layer_to_canvas: DMat3,
        canvas: (u32, u32),
        direction: Option<Direction>,
    ) -> UnitLengths {
        // Layer px per em, and canvas px per layer px (the clip's scale).
        let em = data.style.size.as_f64() * scale_px;
        let m = layer_to_canvas;
        let s = (m.x_axis.x * m.y_axis.y - m.y_axis.x * m.x_axis.y)
            .abs()
            .sqrt();
        let em_canvas = em * s;
        if !(em_canvas.is_finite() && em_canvas > 0.0) {
            return UnitLengths {
                short_edge_em: 0.0,
                from_edge_em: 0.0,
            };
        }
        let (cw, ch) = (f64::from(canvas.0), f64::from(canvas.1));
        let content = data.content.as_str();
        let lines = content.split('\n').count().max(1);
        let widest = content
            .split('\n')
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0);
        let spacing = 1.0 + data.style.letter_spacing.as_f64();
        let w = (widest as f64 * 0.55 * spacing).max(0.55) * em_canvas;
        let h = lines as f64 * 1.2 * data.layout.line_spacing.as_f64() * em_canvas;
        let a = m.transform_point2(DVec2::ZERO);
        let (to_edge, extent) = match direction.unwrap_or(Direction::Left) {
            Direction::Left => (a.x, w),
            Direction::Right => (cw - a.x, w),
            Direction::Up => (a.y, h),
            Direction::Down => (ch - a.y, h),
            Direction::Clockwise | Direction::CounterClockwise => (0.0, 0.0),
        };
        let from_edge = if extent > 0.0 {
            (to_edge + extent + em_canvas).max(extent + em_canvas)
        } else {
            0.0
        };
        UnitLengths {
            short_edge_em: cw.min(ch) / em_canvas,
            from_edge_em: from_edge / em_canvas,
        }
    }
}

/// How a slot's time runs through each unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// In, drifts and combined presets: progress 0 → 1 over the unit's
    /// window.
    Forward,
    /// Out: the renderer hands over progress already reversed.
    Backward,
    /// Loops: the unit's phase in the cycle.
    Cycle,
}

/// The renderer's request for a playing per-unit text preset (TX-023):
/// the plan's unit, order (Random seeded by the spec), and timing (In and
/// Out windows over the slot window, a Loop's delays over its cycle,
/// counted from the clip start plus the phase as the timeline does), with
/// each unit posed by [`TextAnimPlan::unit_pose`]. Decode presets get
/// seeded stand-ins.
pub fn text_motion(plan: &TextAnimPlan, lengths: &UnitLengths) -> TextMotion {
    let spec = &plan.spec;
    let overlap = PerMille::new(spec.params.overlap.get().min(1000));
    let in_window = Tick::from_raw(plan.local_t.raw().saturating_sub(plan.window.start.raw()));
    let (timing, at, pass) = match (plan.slot, plan.manifest.kind) {
        (Slot::Entrance, _) => (
            UnitTiming::In {
                span: plan.window.len,
                overlap,
            },
            in_window,
            Pass::Forward,
        ),
        (Slot::Exit, _) => (
            UnitTiming::Out {
                span: plan.window.len,
                overlap,
            },
            in_window,
            Pass::Backward,
        ),
        (Slot::Continuous, PresetKind::Loop) => (
            UnitTiming::Loop {
                cycle: spec.duration,
                overlap,
            },
            Tick::from_raw(plan.local_t.raw().saturating_add(spec.phase.raw())),
            Pass::Cycle,
        ),
        (Slot::Continuous, _) => (
            UnitTiming::In {
                span: plan.window.len,
                overlap,
            },
            in_window,
            Pass::Forward,
        ),
    };
    let scale = MotionScale {
        se_px: lengths.short_edge_em,
        em_px: 1.0,
        travel_px: match spec.params.travel {
            // unit_pose applies k.
            Travel::Short => 3.0,
            Travel::FromEdge => lengths.from_edge_em,
        },
    };
    let decode = plan
        .manifest
        .channels
        .iter()
        .any(|c| c.property == ChannelProp::Decode)
        .then_some(Decode {
            seed: spec.params.seed,
            step: DecodeStep::default().resolve(plan.frame),
        });
    let mut motion = TextMotion::new(
        unit(plan.unit),
        order(spec.params.order, spec.params.seed),
        timing,
        at,
        UnitChannels::Callback(unit_poses(plan.clone(), pass, scale)),
    );
    motion.decode = decode;
    // TX-056's Caret, stored per slot: only a stepped reveal (Typewriter)
    // in an In or Out has one.
    if spec.params.caret && plan.slot != Slot::Continuous && is_stepped(&plan.manifest) {
        motion.caret = Some(Caret { blink: CARET_BLINK });
    }
    motion
}

fn unit(u: ModelUnit) -> AnimUnit {
    match u {
        ModelUnit::Letter => AnimUnit::Grapheme,
        ModelUnit::Word => AnimUnit::Word,
        // The plan never hands over Block for per-unit work; the whole
        // text as lines is the nearest.
        ModelUnit::Line | ModelUnit::Block => AnimUnit::Line,
    }
}

fn order(o: AnimOrder, seed: u32) -> UnitOrder {
    match o {
        AnimOrder::Forward => UnitOrder::Forward,
        AnimOrder::Backward => UnitOrder::Backward,
        AnimOrder::CenterOut => UnitOrder::CenterOut,
        AnimOrder::EdgesIn => UnitOrder::EdgesIn,
        AnimOrder::Random => UnitOrder::Random { seed },
    }
}

/// Each unit's pose: the plan's manifest at the unit's own progress
/// ([`TextAnimPlan::unit_pose`]), told which unit it is and where it sits
/// in its line ([`unit_context`]).
fn unit_poses(plan: TextAnimPlan, pass: Pass, scale: MotionScale) -> UnitFn {
    UnitFn::new(move |s: &UnitSample| {
        let progress = f64::from(s.progress);
        let p = match pass {
            // A unit whose window has started counts as started even at
            // progress 0, so a step (Typewriter) shows it from the first
            // frame of its window; an idle unit holds its start pose.
            Pass::Forward if s.idle => 0.0,
            Pass::Forward => progress.max(f64::MIN_POSITIVE),
            Pass::Backward => 1.0 - progress,
            Pass::Cycle => progress,
        };
        glyph_anim(&plan.unit_pose(p, &scale, &unit_context(s)))
    })
}

/// What the timeline's evaluator needs to know about the unit of `s`
/// (TX-023, TX-056): its reading-order index (seeded channels: Scatter,
/// Fall away, Shake, Wiggle, Sparkle, Flicker) and, in em like every
/// per-unit length, the way to its line's centre (Spread) and its line's
/// height (Word rise). The plan fills in the frame and has applied the
/// seam envelope to its manifest (TX-022).
fn unit_context(s: &UnitSample) -> UnitContext {
    let g = &s.geometry;
    let finite = |v: f32| if v.is_finite() { f64::from(v) } else { 0.0 };
    UnitContext {
        index: s.unit,
        to_line_center: DVec2::new(finite(g.to_line_center.0), finite(g.to_line_center.1)),
        line_height: (g.line_height.is_finite() && g.line_height > 0.0)
            .then_some(f64::from(g.line_height)),
        frame: None,
        amount: 1.0,
    }
}

/// The timeline's offsets (with lengths in em) as a glyph pose. The
/// lightness style channel (Shimmer's band, "fill lightened by 40·k %")
/// and the white tint both move the glyph's fill toward white
/// ([`GlyphAnim::tint`], with [`TextMotion::tint_color`] white); the hue
/// turn and the glow factor carry over as they are ([`GlyphAnim::hue`],
/// [`GlyphAnim::glow`]). Decode's amount has no per-glyph form (the
/// renderer substitutes letters instead).
fn glyph_anim(o: &BlockOffsets) -> GlyphAnim {
    let (sx, sy) = (o.scale.x as f32, o.scale.y as f32);
    let (scale, scale_xy) = if sx == sy {
        (sx, (1.0, 1.0))
    } else {
        (1.0, (sx, sy))
    };
    GlyphAnim {
        offset: (o.translate.x as f32, o.translate.y as f32),
        scale,
        scale_xy,
        rotation: o.rotation as f32,
        turn: (o.turn.x as f32, o.turn.y as f32),
        opacity: o.opacity as f32,
        blur: o.blur as f32,
        reveal: o.reveal as f32,
        tint: (o.lightness.max(0.0) + o.tint).clamp(0.0, 1.0) as f32,
        hue: o.hue as f32,
        glow: o.glow as f32,
        line_clip: o.line_clip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reveal_key_maps_to_letters_shown() {
        assert_eq!(reveal_key(1.0), None);
        assert_eq!(reveal_key(f64::NAN), None);
        assert_eq!(reveal_key(0.25), Some(PerMille::new(250)));
        assert_eq!(reveal_key(-1.0), Some(PerMille::new(0)));
    }

    #[test]
    fn rest_offsets_are_a_rest_pose() {
        assert_eq!(glyph_anim(&BlockOffsets::REST), GlyphAnim::REST);
        let o = BlockOffsets {
            scale: DVec2::new(2.0, 0.5),
            ..BlockOffsets::REST
        };
        let g = glyph_anim(&o);
        assert_eq!((g.scale, g.scale_xy), (1.0, (2.0, 0.5)));
    }

    #[test]
    fn a_sample_tells_the_evaluator_its_unit_and_line() {
        use crate::text::animate::UnitGeometry;
        let s = UnitSample {
            unit: 3,
            rank: 0,
            count: 5,
            progress: 0.5,
            window: (Tick::ZERO, Tick::from_raw(10)),
            idle: false,
            geometry: UnitGeometry {
                to_line_center: (-0.5, 0.25),
                line_height: 1.2,
            },
        };
        let c = unit_context(&s);
        assert_eq!(c.index, 3);
        assert_eq!(c.to_line_center, DVec2::new(-0.5, 0.25));
        assert_eq!(c.line_height, Some(f64::from(1.2f32)));
        assert_eq!((c.frame, c.amount), (None, 1.0));
        // A unit without a box: the evaluator's own line height, no pull.
        let bare = unit_context(&UnitSample {
            geometry: UnitGeometry {
                to_line_center: (f32::NAN, 0.0),
                line_height: 0.0,
            },
            ..s
        });
        assert_eq!(bare.to_line_center, DVec2::ZERO);
        assert_eq!(bare.line_height, None);
    }

    #[test]
    fn the_line_clip_of_word_rise_reaches_the_glyph() {
        let rising = BlockOffsets {
            translate: DVec2::new(0.0, 0.6),
            line_clip: true,
            ..BlockOffsets::REST
        };
        let g = glyph_anim(&rising);
        assert!(g.line_clip);
        assert_eq!(g.offset, (0.0, 0.6));
    }

    #[test]
    fn lightness_and_white_tint_become_a_glyph_tint() {
        let shimmer = BlockOffsets {
            lightness: 0.4,
            ..BlockOffsets::REST
        };
        assert_eq!(glyph_anim(&shimmer).tint, 0.4);
        let both = BlockOffsets {
            lightness: 0.8,
            tint: 0.5,
            ..BlockOffsets::REST
        };
        assert_eq!(glyph_anim(&both).tint, 1.0);
    }

    #[test]
    fn hue_turn_and_glow_factor_carry_over_to_the_glyph() {
        let rest = glyph_anim(&BlockOffsets::REST);
        assert_eq!((rest.hue, rest.glow), (0.0, 1.0));
        let styled = BlockOffsets {
            hue: 120.0,
            glow: 0.25,
            ..BlockOffsets::REST
        };
        let g = glyph_anim(&styled);
        assert_eq!((g.hue, g.glow), (120.0, 0.25));
        // Neither touches the tint.
        assert_eq!(g.tint, 0.0);
    }

    #[test]
    fn decode_steps_count_frames_or_hold_a_fixed_time() {
        let frame = Tick::from_raw(TICKS_PER_SECOND / 60);
        assert_eq!(
            DecodeStep::default().resolve(frame),
            Tick::from_raw(TICKS_PER_SECOND / 30)
        );
        assert_eq!(DecodeStep::Frames(3).resolve(frame).raw(), frame.raw() * 3);
        assert_eq!(DecodeStep::Frames(2).resolve(Tick::ZERO), DECODE_STEP);
        assert_eq!(DecodeStep::Frames(0).resolve(frame), Tick::from_raw(1));
        assert_eq!(
            DecodeStep::Fixed(Tick::from_raw(5)).resolve(frame),
            Tick::from_raw(5)
        );
    }

    #[test]
    fn options_set_the_decode_step_but_add_a_caret_only_to_stepped_presets() {
        use freecut_timeline::ids::{ClipId, TrackId};
        use freecut_timeline::plan::BlendMode;
        let layer = LayerPlan {
            clip: ClipId::from_raw(std::num::NonZeroU64::MIN),
            track: TrackId::from_raw(std::num::NonZeroU64::MIN),
            source: LayerSource::Text {
                data: std::sync::Arc::new(TextClipData::default()),
                scale_px: 1.0,
            },
            display_size: (10, 10),
            layer_to_canvas: DMat3::IDENTITY,
            opacity: 1.0,
            blend: BlendMode::Normal,
            motion: None,
            ligatures: true,
            color: None,
        };
        let mut motion = TextMotion::new(
            AnimUnit::Grapheme,
            UnitOrder::Forward,
            UnitTiming::In {
                span: Tick::from_raw(100),
                overlap: PerMille::ZERO,
            },
            Tick::ZERO,
            UnitChannels::Table(ChannelTable::default()),
        );
        motion.decode = Some(Decode {
            seed: 1,
            step: DECODE_STEP,
        });
        let options = TextMotionOptions {
            caret: Some(Caret { blink: CARET_BLINK }),
            decode_step: DecodeStep::Frames(3),
        };
        options.apply(&mut motion, &layer, Tick::from_raw(1000));
        assert_eq!(motion.decode.map(|d| d.step), Some(Tick::from_raw(3000)));
        assert_eq!(motion.caret, None, "no stepped preset plays");
        let typewriter = freecut_timeline::motion::samples()
            .into_iter()
            .find(|m| m.id.as_str() == "fc.in.typewriter")
            .unwrap();
        assert!(is_stepped(&typewriter));
        let fade = freecut_timeline::motion::samples()
            .into_iter()
            .find(|m| m.id.as_str() == "fc.in.fade")
            .unwrap();
        assert!(!is_stepped(&fade));
    }

    #[test]
    fn a_stored_caret_rides_only_on_a_stepped_in_or_out() {
        use freecut_timeline::model::{AnimUnit as ModelUnit, SlotWindow};
        let samples = freecut_timeline::motion::samples();
        let plan = |id: &str, slot: Slot, caret: bool| {
            let manifest = std::sync::Arc::new(
                samples
                    .iter()
                    .find(|m| m.id.as_str() == id)
                    .cloned()
                    .unwrap(),
            );
            let mut spec = manifest.default_spec();
            spec.params.caret = caret;
            TextAnimPlan {
                slot,
                local_t: Tick::ZERO,
                window: SlotWindow {
                    start: Tick::ZERO,
                    len: spec.duration,
                },
                spec,
                manifest,
                unit: ModelUnit::Letter,
                direction: None,
                progress: 0.0,
                envelope: 1.0,
                frame: Tick::from_raw(TICKS_PER_SECOND / 30),
                style: Default::default(),
            }
        };
        let lengths = UnitLengths {
            short_edge_em: 10.0,
            from_edge_em: 10.0,
        };
        let caret = |id, slot, on| text_motion(&plan(id, slot, on), &lengths).caret;
        let bar = Some(Caret { blink: CARET_BLINK });
        assert_eq!(caret("fc.in.typewriter", Slot::Entrance, true), bar);
        assert_eq!(caret("fc.in.typewriter", Slot::Exit, true), bar);
        assert_eq!(caret("fc.in.typewriter", Slot::Entrance, false), None);
        // Only a stepped reveal draws it, and never in a Loop.
        assert_eq!(caret("fc.loop.wave", Slot::Continuous, true), None);
        assert_eq!(caret("fc.in.typewriter", Slot::Continuous, true), None);
    }

    #[test]
    fn lengths_follow_the_font_size_and_the_clip_scale() {
        use freecut_timeline::model::{RefPx, RichText, TextStyle};
        let data = TextClipData {
            content: RichText::new("ab"),
            style: TextStyle {
                size: RefPx::from_px(100),
                ..TextStyle::default()
            },
            ..TextClipData::default()
        };
        // 1080 × 1080 canvas, scale_px 1, the clip at 200 % in the middle.
        let m = DMat3::from_translation(DVec2::new(540.0, 540.0))
            * DMat3::from_scale(DVec2::splat(2.0));
        let l = UnitLengths::estimate(&data, 1.0, m, (1080, 1080), Some(Direction::Left));
        // One em is 200 canvas px.
        assert!((l.short_edge_em - 5.4).abs() < 1e-9);
        // 540 to the edge + 1.1 em of box + 1 em.
        assert!((l.from_edge_em - (2.7 + 1.1 + 1.0)).abs() < 1e-9, "{l:?}");
    }
}
