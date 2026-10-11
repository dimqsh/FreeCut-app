//! Clip animations (FX-018, FX-019, TX-022, TX-023): `SetClipAnimation`
//! through `Fixture::apply` (plan purity, validity, apply/undo/redo), the
//! slot policies, FX-018's limits and proportional scaling, the split
//! rule, Pace, and TX-022's composition order in the frame plan.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    Edge, EditError, EditRequest, InsertReq, InsertTarget, KeyEdit, NewClip, OpKind, SplitReq,
    SplitTargets, TextStyleEdit, TrimReq,
};
use freecut_timeline::glam::{DMat3, DVec2, DVec3};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    AnimUnit, AnimationSpec, CanvasPos, Fill, PerMille, PresetId, PropPath, PropValue, Rgba8,
    RichText, Slot, SlotPolicy, SlotWindow, TextClipData, TransformProp,
};
use freecut_timeline::motion::{
    Channel, ChannelProp, ChannelUnit, Curve, CurvePoint, ParamDefaults, Pivot, PresetKind,
    PresetManifest, PresetTable, pace_duration, samples,
};
use freecut_timeline::plan::{
    AnimContext, LayerPlan, LayerSource, PlanInputs, PlanNode, frame_plan_with,
};
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};
use pretty_assertions::assert_eq;

fn ms(v: i64) -> Tick {
    Tick::from_raw(TICKS_PER_SECOND / 1000 * v)
}

fn insert_text(fx: &mut Fixture, at: i64, len: i64, text: &str) -> ClipId {
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::FirstFree,
        at: f(at),
        clips: vec![NewClip::text(TextClipData::with_content(text), f(len))],
    });
    match fx.apply(req).unwrap() {
        Committed::Applied { selection, .. } => selection.unwrap()[0],
        Committed::NoOp => panic!("insert was a no-op"),
    }
}

fn spec(id: &str, d: Tick) -> AnimationSpec {
    AnimationSpec::new(PresetId::new(id), d)
}

fn set(
    fx: &mut Fixture,
    clips: &[ClipId],
    slot: Slot,
    s: Option<AnimationSpec>,
) -> Result<Committed, EditError> {
    fx.apply(EditRequest::SetClipAnimation {
        seq: fx.seq_id(),
        clips: clips.to_vec(),
        slot,
        spec: s,
    })
}

fn anims(fx: &Fixture, clip: ClipId) -> freecut_timeline::model::ClipAnimations {
    fx.doc().clip(clip).unwrap().animations.clone()
}

fn label(fx: &Fixture) -> (OpKind, u32) {
    let l = fx.editor.history().undo_label().unwrap();
    (l.op, l.count)
}

#[test]
fn text_slots_coexist() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 75, "Hello");
    set(
        &mut fx,
        &[clip],
        Slot::Continuous,
        Some(spec("fc.loop.wave", ms(1200))),
    )
    .unwrap();
    assert_eq!(label(&fx), (OpKind::SetClipAnimation, 1));
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.fade", ms(500))),
    )
    .unwrap();
    set(
        &mut fx,
        &[clip],
        Slot::Exit,
        Some(spec("fc.out.fade", ms(500))),
    )
    .unwrap();
    let a = anims(&fx, clip);
    assert!(a.entrance.is_some() && a.exit.is_some() && a.continuous.is_some());
    // Changing Loop keeps In and Out, and the reverse.
    set(
        &mut fx,
        &[clip],
        Slot::Continuous,
        Some(spec("fc.loop.wave", ms(800))),
    )
    .unwrap();
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.rise", ms(400))),
    )
    .unwrap();
    let a = anims(&fx, clip);
    assert_eq!(a.entrance.as_ref().unwrap().preset.as_str(), "fc.in.rise");
    assert_eq!(a.exit.as_ref().unwrap().preset.as_str(), "fc.out.fade");
    assert_eq!(a.continuous.as_ref().unwrap().duration, ms(800));
    // Clearing one slot leaves the others; setting the same spec again is
    // a no-op.
    set(&mut fx, &[clip], Slot::Exit, None).unwrap();
    assert!(anims(&fx, clip).exit.is_none() && anims(&fx, clip).continuous.is_some());
    let same = anims(&fx, clip).entrance.unwrap();
    assert_eq!(
        set(&mut fx, &[clip], Slot::Entrance, Some((*same).clone())).unwrap(),
        Committed::NoOp
    );
}

#[test]
fn other_clips_use_the_exclusive_policy_and_audio_is_skipped() {
    let mut fx = Fixture::new();
    let video = fx.insert(InsertTarget::Track(fx.main()), 0, &[(VIDEO_100, 0, 80)])[0];
    let audio = fx.insert(InsertTarget::NewTrack, 0, &[(AUDIO_200, 0, 80)])[0];
    set(
        &mut fx,
        &[video, audio],
        Slot::Entrance,
        Some(spec("fc.in.fade", ms(500))),
    )
    .unwrap();
    assert_eq!(label(&fx), (OpKind::SetClipAnimation, 1));
    assert!(anims(&fx, audio).entrance.is_none());
    set(
        &mut fx,
        &[video],
        Slot::Exit,
        Some(spec("fc.out.fade", ms(500))),
    )
    .unwrap();
    let before = fx.doc().clone();
    // Choosing a Continuous preset clears In and Out in the same entry.
    set(
        &mut fx,
        &[video],
        Slot::Continuous,
        Some(spec("fc.loop.wave", ms(1000))),
    )
    .unwrap();
    let a = anims(&fx, video);
    assert!(a.entrance.is_none() && a.exit.is_none() && a.continuous.is_some());
    fx.editor.undo().unwrap();
    assert_eq!(fx.doc(), &before);
    fx.editor.redo().unwrap();
    // And the reverse.
    set(
        &mut fx,
        &[video],
        Slot::Entrance,
        Some(spec("fc.in.fade", ms(500))),
    )
    .unwrap();
    assert!(anims(&fx, video).continuous.is_none());
    assert_eq!(
        set(
            &mut fx,
            &[audio],
            Slot::Entrance,
            Some(spec("fc.in.fade", ms(500)))
        ),
        Err(EditError::NoVisualClip)
    );
}

#[test]
fn lengths_follow_fx_018s_limits() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 50, "Hi");
    // Snapped to frames: 0.53 s → 13 frames at 25 fps.
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.fade", ms(530))),
    )
    .unwrap();
    assert_eq!(anims(&fx, clip).entrance.unwrap().duration, f(13));
    // At least 0.1 s, rounded up to whole frames (3 frames).
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.fade", ms(10))),
    )
    .unwrap();
    assert_eq!(anims(&fx, clip).entrance.unwrap().duration, f(3));
    // At most what Out leaves of the clip.
    set(
        &mut fx,
        &[clip],
        Slot::Exit,
        Some(spec("fc.out.fade", f(30))),
    )
    .unwrap();
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.fade", f(40))),
    )
    .unwrap();
    assert_eq!(anims(&fx, clip).entrance.unwrap().duration, f(20));
    // Loop cycles within 0.1–10 s; settings clamped.
    let mut wild = spec("fc.loop.wave", Tick::from_raw(TICKS_PER_SECOND * 60));
    wild.params.intensity = PerMille::new(5000);
    set(&mut fx, &[clip], Slot::Continuous, Some(wild)).unwrap();
    let c = anims(&fx, clip).continuous.unwrap();
    assert_eq!(c.duration, AnimationSpec::MAX_CYCLE);
    assert_eq!(c.params.intensity, PerMille::ONE);
    // A clip too short for the minimum still stores it (TX-022).
    let tiny = insert_text(&mut fx, 100, 1, "x");
    set(
        &mut fx,
        &[tiny],
        Slot::Entrance,
        Some(spec("fc.in.fade", ms(500))),
    )
    .unwrap();
    assert_eq!(
        anims(&fx, tiny).entrance.unwrap().duration,
        ms(480),
        "12 frames"
    );
}

#[test]
fn trimming_below_in_plus_out_scales_them_and_keeps_the_stored_values() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 50, "Hi");
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.fade", f(20))),
    )
    .unwrap();
    set(
        &mut fx,
        &[clip],
        Slot::Exit,
        Some(spec("fc.out.fade", f(10))),
    )
    .unwrap();
    set(
        &mut fx,
        &[clip],
        Slot::Continuous,
        Some(spec("fc.loop.wave", ms(1200))),
    )
    .unwrap();
    fx.apply(EditRequest::Trim(TrimReq {
        seq: fx.seq_id(),
        clip,
        edge: Edge::Out,
        delta: f(-35),
        ripple: false,
    }))
    .unwrap();
    let c = fx.doc().clip(clip).unwrap();
    let a = &c.animations;
    assert_eq!(
        (
            a.entrance.as_ref().unwrap().duration,
            a.exit.as_ref().unwrap().duration
        ),
        (f(20), f(10))
    );
    let w = a.windows(SlotPolicy::LoopBetween, c.duration(), f(1));
    assert_eq!(
        w.entrance,
        Some(SlotWindow {
            start: f(0),
            len: f(10)
        })
    );
    assert_eq!(
        w.exit,
        Some(SlotWindow {
            start: f(10),
            len: f(5)
        })
    );
    assert_eq!(
        w.continuous, None,
        "Loop waits until the clip is long enough"
    );
}

#[test]
fn split_keeps_in_left_out_right_and_continues_the_loop() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 60, "Wave");
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.fade", f(10))),
    )
    .unwrap();
    set(
        &mut fx,
        &[clip],
        Slot::Exit,
        Some(spec("fc.out.fade", f(10))),
    )
    .unwrap();
    let mut wave = spec("fc.loop.wave", ms(1200));
    wave.params.unit = Some(AnimUnit::Letter);
    set(&mut fx, &[clip], Slot::Continuous, Some(wave)).unwrap();
    let table = PresetTable::samples();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    let progress = |fx: &Fixture, frame| {
        layer(fx, frame, &ctx)
            .motion
            .unwrap()
            .text
            .clone()
            .unwrap()
            .progress
    };
    let before = (progress(&fx, 29), progress(&fx, 30), progress(&fx, 41));
    fx.apply(EditRequest::Split(SplitReq {
        seq: fx.seq_id(),
        at: f(30),
        targets: SplitTargets::Clips(vec![clip]),
    }))
    .unwrap();
    let right = fx
        .seq()
        .tracks()
        .flat_map(|(_, t)| t.clips().iter())
        .find(|c| c.start() == f(30))
        .unwrap()
        .id;
    let (l, r) = (anims(&fx, clip), anims(&fx, right));
    assert!(l.entrance.is_some() && l.exit.is_none());
    assert!(r.entrance.is_none() && r.exit.is_some());
    assert_eq!(r.continuous.as_ref().unwrap().phase, f(30));
    assert_eq!(
        (progress(&fx, 29), progress(&fx, 30), progress(&fx, 41)),
        before,
        "the loop continues seamlessly"
    );
}

#[test]
fn pace_sets_the_length_and_text_edits_recompute_it_in_the_same_entry() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 100, "Hello");
    let mut tw = samples()
        .into_iter()
        .find(|m| m.id.as_str() == "fc.in.typewriter")
        .unwrap()
        .default_spec();
    tw.duration = ms(999);
    set(&mut fx, &[clip], Slot::Entrance, Some(tw)).unwrap();
    // 5 letters × 0.06 s = 0.3 s = 7.5 frames → 7 (ties down).
    let paced = |fx: &Fixture| anims(fx, clip).entrance.unwrap().duration;
    assert_eq!(
        paced(&fx),
        pace_duration(5, ms(60), PerMille::ZERO, rate()).unwrap()
    );
    assert_eq!(paced(&fx), f(7));
    let before = fx.doc().clone();
    fx.apply(EditRequest::SetTextContent {
        seq: fx.seq_id(),
        clip,
        content: RichText::new("Hello world, again"),
    })
    .unwrap();
    assert_eq!(label(&fx), (OpKind::SetTextContent, 1));
    // 16 letters × 0.06 s = 0.96 s = 24 frames.
    assert_eq!(paced(&fx), f(24));
    fx.editor.undo().unwrap();
    assert_eq!(fx.doc(), &before, "one undo restores text and length");
    // Nothing to reveal: the 0.1 s minimum.
    fx.editor.redo().unwrap();
    fx.apply(EditRequest::SetTextContent {
        seq: fx.seq_id(),
        clip,
        content: RichText::new("   "),
    })
    .unwrap();
    assert_eq!(paced(&fx), f(3));
}

// ----------------------------------------------------- the frame plan

fn layer(fx: &Fixture, frame: i64, ctx: &AnimContext<'_>) -> LayerPlan {
    let none = BTreeSet::new();
    let plan = frame_plan_with(
        fx.doc(),
        fx.seq_id(),
        f(frame),
        &PlanInputs { offline: &none },
        ctx,
    )
    .unwrap();
    let PlanNode::Layer(l) = plan.nodes.last().unwrap().clone();
    l
}

/// A point through a possibly projective matrix.
fn map(m: DMat3, p: DVec2) -> DVec2 {
    let v = m * DVec3::new(p.x, p.y, 1.0);
    DVec2::new(v.x / v.z, v.y / v.z)
}

fn keys(fx: &mut Fixture, clip: ClipId, prop: PropPath, at: i64, value: PropValue) {
    fx.apply(EditRequest::EditKeys {
        seq: fx.seq_id(),
        clips: vec![clip],
        prop,
        edit: KeyEdit::Add {
            at: f(at),
            value: Some(value),
        },
    })
    .unwrap();
}

#[test]
fn composition_folds_the_slide_into_the_keyed_transform() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 50, "Title");
    let pos = PropPath::Transform(TransformProp::Position);
    keys(
        &mut fx,
        clip,
        pos,
        0,
        PropValue::Position(CanvasPos::new(0, 0)),
    );
    keys(
        &mut fx,
        clip,
        pos,
        10,
        PropValue::Position(CanvasPos::new(100_000, 0)),
    );
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.slide", ms(600))),
    )
    .unwrap();
    let table = PresetTable::samples();
    let bounds = |_: ClipId, _: &TextClipData, _: f64| {
        Some((DVec2::new(-100.0, -20.0), DVec2::new(100.0, 20.0)))
    };
    let ctx = AnimContext {
        presets: &table,
        text_bounds: Some(&bounds),
    };
    // Frame 5: the keyed position is +96 px (anchor 1056); the slide is a
    // third through 15 frames, ease-out cubic, from just past the left
    // edge (the box's right side at rest is at 1156).
    let l = layer(&fx, 5, &ctx);
    let width = f64::from(fx.doc().settings().width);
    let anchor = width / 2.0 + 0.05 * width;
    let p: f64 = 5.0 / 15.0;
    let remaining = (1.0 - p).powi(3);
    let want = anchor - remaining * (anchor + 100.0);
    let origin = map(l.layer_to_canvas, DVec2::ZERO);
    assert!((origin.x - want).abs() < 1e-6, "{} vs {want}", origin.x);
    assert!((origin.y - f64::from(fx.doc().settings().height) / 2.0).abs() < 1e-9);
    // After the In: the keyed transform alone, no motion record.
    let l = layer(&fx, 20, &ctx);
    let keyed = freecut_timeline::plan::clip_placement(
        &fx.doc().clip(clip).unwrap().transform_at(f(20)),
        (fx.doc().settings().width, fx.doc().settings().height),
    );
    assert!(l.layer_to_canvas.abs_diff_eq(keyed, 1e-9));
    assert!(l.motion.is_none());
    // The same document without a catalogue: keyframes only.
    let keys_only = layer(&fx, 5, &AnimContext::KEYS_ONLY);
    let keyed5 = freecut_timeline::plan::clip_placement(
        &fx.doc().clip(clip).unwrap().transform_at(f(5)),
        (fx.doc().settings().width, fx.doc().settings().height),
    );
    assert!(keys_only.layer_to_canvas.abs_diff_eq(keyed5, 1e-9));
}

#[test]
fn animation_opacity_multiplies_the_keyed_opacity() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 50, "Fade");
    keys(
        &mut fx,
        clip,
        PropPath::Transform(TransformProp::Opacity),
        0,
        PropValue::Ratio(PerMille::new(500)),
    );
    set(
        &mut fx,
        &[clip],
        Slot::Exit,
        Some(spec("fc.out.fade", f(10))),
    )
    .unwrap();
    let table = PresetTable::samples();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    // Half-way through the Out: the mirrored quadratic gives 0.75.
    let l = layer(&fx, 45, &ctx);
    assert!((l.opacity - 0.5 * 0.75).abs() < 1e-12, "{}", l.opacity);
    assert!((layer(&fx, 20, &ctx).opacity - 0.5).abs() < 1e-12);
}

#[test]
fn per_unit_presets_go_to_the_renderer_and_leave_the_matrix_alone() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 50, "Type me");
    let tw = samples()
        .into_iter()
        .find(|m| m.id.as_str() == "fc.in.typewriter")
        .unwrap()
        .default_spec();
    set(&mut fx, &[clip], Slot::Entrance, Some(tw)).unwrap();
    let table = PresetTable::samples();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    let l = layer(&fx, 2, &ctx);
    let plan = l.motion.as_ref().unwrap().text.clone().unwrap();
    assert_eq!((plan.slot, plan.unit), (Slot::Entrance, AnimUnit::Letter));
    assert_eq!(plan.manifest.id.as_str(), "fc.in.typewriter");
    assert_eq!(plan.window.start, Tick::ZERO);
    assert_eq!(plan.local_t, f(2));
    let rest = layer(&fx, 40, &ctx);
    assert_eq!(l.layer_to_canvas, rest.layer_to_canvas);
    assert_eq!(l.opacity, 1.0);
    // An unknown preset plays as none (Unavailable animation).
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.from-the-future", ms(500))),
    )
    .unwrap();
    assert!(layer(&fx, 2, &ctx).motion.is_none());
}

#[test]
fn a_whole_text_loop_modulates_the_style_at_t_and_never_stores_it() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 100, "Hue");
    fx.apply(EditRequest::SetTextStyle {
        seq: fx.seq_id(),
        clips: vec![clip],
        edit: TextStyleEdit::Fill(Fill::Solid(Rgba8::new(255, 0, 0, 255))),
    })
    .unwrap();
    let hue = PresetManifest {
        id: PresetId::new("test.loop.hue"),
        kind: PresetKind::Loop,
        units: None,
        directions: None,
        default_duration: ms(4000),
        easing: freecut_timeline::model::Interp::Linear,
        pivot: Pivot::Anchor,
        channels: vec![Channel {
            property: ChannelProp::Style(freecut_timeline::motion::StyleChannel::Hue),
            curve: Curve::sampled(
                vec![
                    CurvePoint { t: 0.0, v: 0.0 },
                    CurvePoint { t: 1.0, v: 360.0 },
                ],
                false,
            )
            .unwrap(),
            amplitude: 1.0,
            unit: ChannelUnit::Deg,
            scales_with_intensity: false,
        }],
        defaults: ParamDefaults::default(),
    };
    let table = PresetTable::new([hue]);
    set(
        &mut fx,
        &[clip],
        Slot::Continuous,
        Some(spec("test.loop.hue", ms(4000))),
    )
    .unwrap();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    // Half a cycle (2 s = 50 frames): red has turned to cyan.
    let l = layer(&fx, 50, &ctx);
    let LayerSource::Text { data, .. } = &l.source else {
        panic!("not text");
    };
    assert_eq!(data.style.fill, Fill::Solid(Rgba8::new(0, 255, 255, 255)));
    let stored = fx.doc().clip(clip).unwrap().content.text().unwrap().clone();
    assert_eq!(stored.style.fill, Fill::Solid(Rgba8::new(255, 0, 0, 255)));
    // Unmodulated frames share the stored Arc.
    let l0 = layer(&fx, 0, &ctx);
    let LayerSource::Text { data, .. } = &l0.source else {
        panic!("not text");
    };
    assert!(Arc::ptr_eq(data, &stored));
}

#[test]
fn clips_shorter_than_two_frames_play_no_animation() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 1, "x");
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.fade", ms(500))),
    )
    .unwrap();
    let table = PresetTable::samples();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    assert_eq!(layer(&fx, 0, &ctx).opacity, 1.0);
}

// ------------------------------------------------ TX-022's loop seams

fn layer_at(fx: &Fixture, t: Tick, ctx: &AnimContext<'_>) -> LayerPlan {
    let none = BTreeSet::new();
    let plan = frame_plan_with(
        fx.doc(),
        fx.seq_id(),
        t,
        &PlanInputs { offline: &none },
        ctx,
    )
    .unwrap();
    let PlanNode::Layer(l) = plan.nodes.last().unwrap().clone();
    l
}

/// A whole-text float: up and down 0.3 em, sine, 1.2 s cycle.
fn float_loop() -> PresetManifest {
    PresetManifest {
        id: PresetId::new("test.loop.float"),
        kind: PresetKind::Loop,
        units: None,
        directions: None,
        default_duration: ms(1200),
        easing: freecut_timeline::model::Interp::Linear,
        pivot: Pivot::Anchor,
        channels: vec![Channel {
            property: ChannelProp::TranslateY,
            curve: Curve::Sine,
            amplitude: -0.3,
            unit: ChannelUnit::Em,
            scales_with_intensity: true,
        }],
        defaults: ParamDefaults::default(),
    }
}

/// A text clip of 100 frames with a 10-frame fade In and Out and `looped`
/// in the Loop slot, which plays in frames 10..90.
fn in_loop_out(looped: AnimationSpec) -> (Fixture, ClipId) {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 100, "Seams");
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.fade", f(10))),
    )
    .unwrap();
    set(
        &mut fx,
        &[clip],
        Slot::Exit,
        Some(spec("fc.out.fade", f(10))),
    )
    .unwrap();
    set(&mut fx, &[clip], Slot::Continuous, Some(looped)).unwrap();
    (fx, clip)
}

/// `t` moved by `d` ticks.
fn plus(t: Tick, d: i64) -> Tick {
    Tick::from_raw(t.raw() + d)
}

#[test]
fn a_whole_text_loop_eases_in_after_in_and_out_before_out() {
    let (fx, clip) = in_loop_out(spec("test.loop.float", ms(1200)));
    let mut all = samples();
    all.push(float_loop());
    let table = PresetTable::new(all);
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    let y = |t: Tick| map(layer_at(&fx, t, &ctx).layer_to_canvas, DVec2::ZERO).y;
    let rest_y = f64::from(fx.doc().settings().height) / 2.0;
    let (start, end) = (f(10), f(90));
    // Without the rule the loop would start mid-wave: 0.4 s into a 1.2 s
    // cycle is sin(120°), most of its 0.3 em swing.
    let m = float_loop();
    let em = 1.0; // The swing measured in em: only its size matters here.
    let raw = freecut_timeline::motion::pose_at(
        &m,
        &spec("test.loop.float", ms(1200)),
        Slot::Continuous,
        1.0 / 3.0,
        &freecut_timeline::motion::MotionScale {
            se_px: 1.0,
            em_px: em,
            travel_px: 1.0,
        },
    );
    assert!(raw.translate.y.abs() > 0.25, "{}", raw.translate.y);
    // Mid-loop it really moves, at full strength.
    assert!((y(f(50)) - rest_y).abs() > 1.0, "the loop moves");
    // In → Loop: at rest on both sides of the seam.
    assert!((y(plus(start, -1)) - rest_y).abs() < 1e-6);
    assert!((y(start) - rest_y).abs() < 1e-9, "no jump into the loop");
    // Loop → Out: the same.
    assert!(
        (y(plus(end, -1)) - rest_y).abs() < 1e-6,
        "no jump out of the loop"
    );
    assert!((y(end) - rest_y).abs() < 1e-9);
    // Continuous everywhere around both seams: 1 ms steps move less than
    // 1 px (the swing is a few px per ms at most).
    let step = ms(1).raw();
    for (from, to) in [
        (plus(start, -40 * step), plus(start, 400 * step)),
        (plus(end, -400 * step), plus(end, 40 * step)),
    ] {
        let mut t = from;
        let mut last = y(t);
        while t < to {
            t = plus(t, step);
            let now = y(t);
            assert!((now - last).abs() < 1.0, "a pop at {t:?}: {last} → {now}");
            last = now;
        }
    }
    // The stored animation is untouched by the envelope.
    assert_eq!(anims(&fx, clip).continuous.unwrap().duration, ms(1200));
}

#[test]
fn a_per_unit_loop_hands_the_renderer_an_eased_manifest_at_the_seams() {
    let mut wave = spec("fc.loop.wave", ms(1200));
    wave.params.unit = Some(AnimUnit::Letter);
    let (fx, _) = in_loop_out(wave);
    let table = PresetTable::samples();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    let plan_at = |t: Tick| layer_at(&fx, t, &ctx).motion.unwrap().text.clone().unwrap();
    let scale = freecut_timeline::motion::MotionScale {
        se_px: 10.0,
        em_px: 1.0,
        travel_px: 1.0,
    };
    let unit = freecut_timeline::motion::UnitContext::BLOCK;
    let at_start = plan_at(f(10));
    assert_eq!(at_start.envelope, 0.0);
    assert_eq!(at_start.frame, f(1));
    // Every unit at rest where In hands over, whatever its phase.
    for i in 0..=20 {
        let p = f64::from(i) / 20.0;
        assert!(at_start.unit_pose(p, &scale, &unit).is_rest(1e-12));
    }
    // The raw wave would not be: it pops without the envelope.
    let raw = freecut_timeline::motion::PresetCatalog::get(&table, &PresetId::new("fc.loop.wave"))
        .unwrap();
    assert!(
        !freecut_timeline::motion::pose_at(&raw, &at_start.spec, Slot::Continuous, 0.25, &scale)
            .is_rest(1e-3)
    );
    // Half-way up the ramp, half-way strength (smoothstep at ½).
    let half = plan_at(plus(f(10), ms(125).raw()));
    assert!((half.envelope - 0.5).abs() < 1e-12);
    let o = half.unit_pose(0.25, &scale, &unit);
    assert!((o.translate.y + 0.125).abs() < 1e-12, "{}", o.translate.y);
    // Past the ramp: the catalogue's own manifest at full strength.
    let mid = plan_at(f(50));
    assert_eq!(mid.envelope, 1.0);
    assert!(Arc::ptr_eq(&mid.manifest, &raw));
    // Just before Out: nearly at rest again.
    let last = plan_at(plus(f(90), -1));
    assert!(last.envelope < 1e-9);
    assert!(last.unit_pose(0.25, &scale, &unit).is_rest(1e-9));
}

#[test]
fn a_loop_over_the_whole_clip_has_no_ramps() {
    // No In or Out: the loop starts with the clip and ends with it, and a
    // split piece's edge is no seam (TX-022's split stays seamless).
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 100, "Whole");
    let mut wave = spec("fc.loop.wave", ms(1200));
    wave.params.unit = Some(AnimUnit::Letter);
    set(&mut fx, &[clip], Slot::Continuous, Some(wave)).unwrap();
    let table = PresetTable::samples();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    for frame in [0, 1, 50, 99] {
        let plan = layer(&fx, frame, &ctx)
            .motion
            .unwrap()
            .text
            .clone()
            .unwrap();
        assert_eq!(plan.envelope, 1.0, "frame {frame}");
    }
    // With only an In, the loop eases in but runs to the clip's end.
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.fade", f(10))),
    )
    .unwrap();
    let plan = |frame| {
        layer(&fx, frame, &ctx)
            .motion
            .unwrap()
            .text
            .clone()
            .unwrap()
    };
    assert_eq!(plan(10).envelope, 0.0);
    assert_eq!(plan(99).envelope, 1.0);
}

/// Shimmer as its file defines it: a band of 40 % lightness over a fifth
/// of each letter's 1.5 s cycle.
fn shimmer() -> PresetManifest {
    PresetManifest {
        id: PresetId::new("test.loop.shimmer"),
        kind: PresetKind::Loop,
        units: Some(vec![AnimUnit::Letter]),
        directions: None,
        default_duration: ms(1500),
        easing: freecut_timeline::model::Interp::Linear,
        pivot: Pivot::Anchor,
        channels: vec![Channel {
            property: ChannelProp::Style(freecut_timeline::motion::StyleChannel::Lightness),
            curve: Curve::sampled(
                vec![
                    CurvePoint { t: 0.0, v: 0.0 },
                    CurvePoint { t: 0.1, v: 1.0 },
                    CurvePoint { t: 0.2, v: 0.0 },
                    CurvePoint { t: 1.0, v: 0.0 },
                ],
                true,
            )
            .unwrap(),
            amplitude: 0.4,
            unit: ChannelUnit::Ratio,
            scales_with_intensity: true,
        }],
        defaults: ParamDefaults::default(),
    }
}

#[test]
fn per_unit_plans_carry_the_style_channels_and_the_frame() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 100, "Shine");
    let table = PresetTable::new([shimmer()]);
    set(
        &mut fx,
        &[clip],
        Slot::Continuous,
        Some(spec("test.loop.shimmer", ms(1500))),
    )
    .unwrap();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    let plan = layer(&fx, 20, &ctx).motion.unwrap().text.clone().unwrap();
    assert!(plan.style.lightness && plan.style.any());
    assert!(!plan.style.hue && !plan.style.glow && !plan.style.tint);
    assert!(!plan.needs_line_geometry());
    assert_eq!(plan.frame, f(1));
    // A unit at the band's peak is lightened 40 %.
    let scale = freecut_timeline::motion::MotionScale {
        se_px: 1.0,
        em_px: 1.0,
        travel_px: 1.0,
    };
    let o = plan.unit_pose(0.1, &scale, &freecut_timeline::motion::UnitContext::BLOCK);
    assert!((o.lightness - 0.4).abs() < 1e-12);
}

// ------------------------------------------- FX-030 over FX-019's cycle

#[test]
fn a_loop_plays_no_faster_than_its_shortest_safe_cycle() {
    use freecut_timeline::motion::{PresetCatalog, photosensitivity};
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 100, "Shine");
    let table = PresetTable::new([shimmer()]);
    let id = PresetId::new("test.loop.shimmer");
    // FX-019 lets the cycle go down to 0.1 s, and it is stored as set.
    set(
        &mut fx,
        &[clip],
        Slot::Continuous,
        Some(spec("test.loop.shimmer", AnimationSpec::MIN_DURATION)),
    )
    .unwrap();
    assert_eq!(
        anims(&fx, clip).continuous.unwrap().duration,
        AnimationSpec::MIN_DURATION
    );
    // At 0.1 s the band would sweep ten times a second: a strobe.
    let mut fast = shimmer();
    fast.default_duration = AnimationSpec::MIN_DURATION;
    assert!(!photosensitivity(&fast).is_safe());
    // So it plays at the shortest cycle that passes FX-030's rule.
    let floor = table.min_cycle(&id);
    assert!(
        floor > AnimationSpec::MIN_DURATION && floor <= ms(1500),
        "{floor:?}"
    );
    let mut at_floor = shimmer();
    at_floor.default_duration = floor;
    assert!(photosensitivity(&at_floor).is_safe());
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    let plan = layer(&fx, 20, &ctx).motion.unwrap().text.clone().unwrap();
    assert_eq!(plan.spec.duration, floor);
    let elapsed = f(20).raw() % floor.raw();
    assert!((plan.progress - elapsed as f64 / floor.raw() as f64).abs() < 1e-12);
    // A cycle at or above the floor plays as set.
    set(
        &mut fx,
        &[clip],
        Slot::Continuous,
        Some(spec("test.loop.shimmer", ms(1500))),
    )
    .unwrap();
    let plan = layer(&fx, 20, &ctx).motion.unwrap().text.clone().unwrap();
    assert_eq!(plan.spec.duration, ms(1500));
    // A loop that never changes brightness may cycle at 0.1 s.
    assert_eq!(
        PresetTable::samples().min_cycle(&PresetId::new("fc.loop.wave")),
        AnimationSpec::MIN_DURATION
    );
}

// -------------------------------- TX-022's seams survive a split (F2)

/// What a frame shows of the text clip at `t`: its matrix, opacity and,
/// for a per-unit preset, the slot, progress and seam envelope.
fn shown(fx: &Fixture, t: Tick, ctx: &AnimContext<'_>) -> (DMat3, f64, Option<(Slot, f64, f64)>) {
    let l = layer_at(fx, t, ctx);
    let text = l
        .motion
        .as_ref()
        .and_then(|m| m.text.as_ref())
        .map(|p| (p.slot, p.progress, p.envelope));
    (l.layer_to_canvas, l.opacity, text)
}

fn split_at(fx: &mut Fixture, clip: ClipId, at: i64) {
    fx.apply(EditRequest::Split(SplitReq {
        seq: fx.seq_id(),
        at: f(at),
        targets: SplitTargets::Clips(vec![clip]),
    }))
    .unwrap();
}

fn right_piece(fx: &Fixture, start: i64) -> ClipId {
    fx.seq()
        .tracks()
        .flat_map(|(_, t)| t.clips().iter())
        .find(|c| c.start() == f(start))
        .unwrap()
        .id
}

#[test]
fn a_split_never_changes_a_frame_of_in_loop_and_out() {
    let mut all = samples();
    all.push(float_loop());
    let table = PresetTable::new(all);
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    let mut wave = spec("fc.loop.wave", ms(1200));
    wave.params.unit = Some(AnimUnit::Letter);
    let looped = [wave, spec("test.loop.float", ms(1200))];
    // Every frame, and sub-frame times next to the seams at 10 and 90.
    let mut times: Vec<Tick> = (0..100).map(f).collect();
    for seam in [10, 90] {
        for d in [-7, -3, 3, 7] {
            times.push(plus(f(seam), d * ms(1).raw()));
        }
    }
    for looped in looped {
        let (fx, _) = in_loop_out(looped.clone());
        let before: Vec<_> = times.iter().map(|&t| shown(&fx, t, &ctx)).collect();
        // Cuts anywhere from In's end to Out's start, then the right piece
        // cut again: In stays left and Out right, so nothing may change.
        for cut in 10..=90 {
            let (mut fx, clip) = in_loop_out(looped.clone());
            split_at(&mut fx, clip, cut);
            if cut < 85 {
                let right = right_piece(&fx, cut);
                split_at(&mut fx, right, cut + 5);
            }
            for (t, want) in times.iter().zip(&before) {
                let got = shown(&fx, *t, &ctx);
                let same = got.0.abs_diff_eq(want.0, 1e-9)
                    && got.1 == want.1
                    && match (got.2, want.2) {
                        (Some(a), Some(b)) => {
                            a.0 == b.0 && (a.1 - b.1).abs() < 1e-12 && (a.2 - b.2).abs() < 1e-12
                        }
                        (a, b) => a == b,
                    };
                assert!(
                    same,
                    "{} split at {cut}: {t:?} shows {got:?}, not {want:?}",
                    looped.preset
                );
            }
        }
    }
}

#[test]
fn a_piece_whose_in_was_removed_eases_no_more() {
    // The stored seam was the In's end; without the In the Loop starts
    // with the clip (TX-022: no ramp at a clip edge).
    let mut wave = spec("fc.loop.wave", ms(1200));
    wave.params.unit = Some(AnimUnit::Letter);
    let (mut fx, clip) = in_loop_out(wave);
    split_at(&mut fx, clip, 50);
    set(&mut fx, &[clip], Slot::Entrance, None).unwrap();
    let table = PresetTable::samples();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    let envelope = |frame| {
        layer(&fx, frame, &ctx)
            .motion
            .unwrap()
            .text
            .clone()
            .unwrap()
            .envelope
    };
    assert_eq!(envelope(0), 1.0);
    assert_eq!(envelope(12), 1.0);
}

// ------------------------------- the ligature policy, per-unit glow

#[test]
fn a_clip_that_moves_its_letters_is_shaped_without_ligatures() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 100, "office");
    let table = PresetTable::samples();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    assert!(layer(&fx, 50, &ctx).ligatures, "no animation");
    // A per-letter typewriter In: off for the whole clip, at rest too.
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.typewriter", f(10))),
    )
    .unwrap();
    assert!(!layer(&fx, 5, &ctx).ligatures);
    assert!(
        !layer(&fx, 50, &ctx).ligatures,
        "the rest pose keeps its shape"
    );
    // A whole-text fade moves no letters.
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("fc.in.fade", f(10))),
    )
    .unwrap();
    assert!(layer(&fx, 50, &ctx).ligatures);
    // A preset this build lacks plays as nothing.
    set(
        &mut fx,
        &[clip],
        Slot::Entrance,
        Some(spec("org.example.in.letters", f(10))),
    )
    .unwrap();
    assert!(layer(&fx, 50, &ctx).ligatures);
}

#[test]
fn a_per_unit_glow_plays_on_a_temporary_glow() {
    let mut fx = Fixture::new();
    let clip = insert_text(&mut fx, 0, 100, "Glow");
    let mut glow = shimmer();
    glow.id = PresetId::new("test.loop.glow");
    glow.channels[0].property =
        ChannelProp::Style(freecut_timeline::motion::StyleChannel::GlowIntensity);
    let table = PresetTable::new([glow]);
    set(
        &mut fx,
        &[clip],
        Slot::Continuous,
        Some(spec("test.loop.glow", ms(1500))),
    )
    .unwrap();
    let ctx = AnimContext {
        presets: &table,
        text_bounds: None,
    };
    let l = layer(&fx, 20, &ctx);
    assert!(l.motion.as_ref().unwrap().text.as_ref().unwrap().style.glow);
    let LayerSource::Text { data, .. } = &l.source else {
        panic!("not text");
    };
    // TX-056: the fill colour at 25 % radius, the stored card untouched.
    assert!(data.style.glow.on);
    assert_eq!(data.style.glow.value.color, Rgba8::WHITE);
    assert_eq!(data.style.glow.value.radius, PerMille::new(250));
    assert!(
        !fx.doc()
            .clip(clip)
            .unwrap()
            .content
            .text()
            .unwrap()
            .style
            .glow
            .on
    );
}
