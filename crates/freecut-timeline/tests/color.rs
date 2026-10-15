//! The colour stack (CL-001, CL-007, CL-008, CL-023, CL-024, CL-028,
//! CL-035): `EditColor`, colour keyframes, `Clip::color_at` and the plan's
//! `LayerPlan::color`. Every edit goes through `Fixture::apply`, which
//! checks plan purity, validity and the undo/redo round trip.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    ColorEdit, ColorOp, EditError, EditRequest, InsertReq, InsertTarget, KeyEdit, NewClip, OpKind,
    SplitReq, SplitTargets,
};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    AutoCorrection, AutoOffsets, AutoSource, AutoStyle, Clip, ClipContent, ColorMask,
    ColorMaskParam, ColorParamId, ColorStack, Curve, CurveChannel, CurvePoint, Document,
    GeneratedKind, HslChannel, HueBand, Interp, InvariantViolation, LutInterp, LutRef, LutSlot,
    MAX_COLOR_MASKS, Mask, MaskCombine, MaskParam, MaskPoint, MaskShape, MaskShapeKind, MatchRef,
    MatchTransform, PerMille, PropGroup, PropPath, PropValue, Section, Sequence, SkinProtect,
    SkinScope, TextClipData, Track, TrackFlags, TrackHeader, ValidateCtx, WheelId,
};
use freecut_timeline::plan::{PlanInputs, PlanNode, frame_plan};
use pretty_assertions::assert_eq;

fn color(fx: &mut Fixture, clips: Vec<ClipId>, edit: ColorEdit) -> Result<Committed, EditError> {
    fx.apply(EditRequest::EditColor {
        seq: fx.seq_id(),
        clips,
        edit,
    })
}

fn stack(fx: &Fixture, clip: ClipId) -> ColorStack {
    ColorStack::of(fx.doc().clip(clip).unwrap().color.as_ref())
}

fn label(fx: &Fixture) -> (OpKind, u32) {
    let l = fx.editor.history().undo_label().unwrap();
    (l.op, l.count)
}

fn add(fx: &mut Fixture, new: NewClip) -> ClipId {
    let Committed::Applied { selection, .. } = fx
        .apply(EditRequest::Insert(InsertReq {
            seq: fx.seq_id(),
            target: InsertTarget::NewTrack,
            at: f(0),
            clips: vec![new],
        }))
        .unwrap()
    else {
        panic!("no-op");
    };
    selection.unwrap()[0]
}

fn key(fx: &mut Fixture, clip: ClipId, id: ColorParamId, at: i64, v: i16) {
    fx.apply(EditRequest::EditKeys {
        seq: fx.seq_id(),
        clips: vec![clip],
        prop: PropPath::Color(id),
        edit: KeyEdit::Add {
            at: f(at),
            value: Some(PropValue::Level(v)),
        },
    })
    .unwrap();
}

/// A video, a still image, an adjustment clip, a title and a sound.
fn kinds(fx: &mut Fixture) -> (ClipId, ClipId, ClipId, ClipId, ClipId) {
    let main = fx.main();
    let video = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 20)])[0];
    let image = add(fx, NewClip::image(VIDEO_50, f(20)));
    let adjust = add(fx, NewClip::generated(GeneratedKind::Adjustment, f(20)));
    let text = add(fx, NewClip::text(TextClipData::with_content("Hi"), f(20)));
    let sound = fx.insert(InsertTarget::NewTrack, 0, &[(AUDIO_200, 0, 20)])[0];
    (video, image, adjust, text, sound)
}

#[test]
fn every_color_edit_applies_undoes_and_redoes() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    let edits = [
        ColorEdit::SetParam(ColorParamId::Exposure, 30),
        ColorEdit::SetParams(vec![
            (ColorParamId::Temperature, 45),
            (ColorParamId::Tint, -20),
            (ColorParamId::Amount, 70),
        ]),
        ColorEdit::SetSectionEnabled(Section::Light, false),
        ColorEdit::SetBypass(true),
        ColorEdit::SetBypass(false),
        ColorEdit::SetSectionEnabled(Section::Light, true),
        ColorEdit::ResetSection(Section::Color),
        ColorEdit::ResetAll,
    ];
    for edit in edits {
        let op = edit.op();
        color(&mut fx, vec![video], edit.clone()).unwrap();
        assert_eq!(label(&fx), (OpKind::EditColor(op), 1), "{edit:?}");
    }
    assert_eq!(fx.doc().clip(video).unwrap().color, None, "reset all");
}

#[test]
fn values_are_clamped_and_defaults_store_nothing() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParams(vec![
            (ColorParamId::Exposure, 400),
            (ColorParamId::Vibrance, -999),
            (ColorParamId::Amount, 101),
        ]),
    )
    .unwrap();
    let s = stack(&fx, video);
    assert_eq!(
        (s.basic.exposure, s.basic.vibrance, s.amount),
        (100, -100, 100)
    );
    // Back to the defaults: the clip stores no stack.
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParams(vec![
            (ColorParamId::Exposure, 0),
            (ColorParamId::Vibrance, 0),
        ]),
    )
    .unwrap();
    assert_eq!(fx.doc().clip(video).unwrap().color, None);
    // Setting what is there changes nothing and records nothing.
    let before = fx.editor.history().undo_label();
    assert!(matches!(
        color(
            &mut fx,
            vec![video],
            ColorEdit::SetParam(ColorParamId::Hue, 0)
        )
        .unwrap(),
        Committed::NoOp
    ));
    assert_eq!(fx.editor.history().undo_label(), before);
}

#[test]
fn ineligible_clips_are_skipped_and_alone_refused() {
    let mut fx = Fixture::new();
    let (video, image, adjust, text, sound) = kinds(&mut fx);
    let all = vec![video, image, adjust, text, sound];
    color(
        &mut fx,
        all,
        ColorEdit::SetParam(ColorParamId::Saturation, -50),
    )
    .unwrap();
    assert_eq!(label(&fx), (OpKind::EditColor(ColorOp::Set), 3));
    for c in [video, image, adjust] {
        assert_eq!(stack(&fx, c).basic.saturation, -50);
    }
    for c in [text, sound] {
        assert_eq!(fx.doc().clip(c).unwrap().color, None);
    }
    assert_eq!(
        color(&mut fx, vec![text, sound], ColorEdit::ResetAll).unwrap_err(),
        EditError::NoColorClip
    );
    assert_eq!(
        color(&mut fx, vec![], ColorEdit::ResetAll).unwrap_err(),
        EditError::NothingSelected
    );
}

#[test]
fn clips_on_locked_tracks_refuse_the_edit() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    let main = fx.main();
    fx.apply(EditRequest::SetTrackHeader {
        seq: fx.seq_id(),
        track: main,
        header: TrackHeader {
            name: None,
            flags: TrackFlags {
                locked: true,
                ..TrackFlags::default()
            },
        },
    })
    .unwrap();
    assert_eq!(
        color(&mut fx, vec![video], ColorEdit::SetBypass(true)).unwrap_err(),
        EditError::MainTrackLocked
    );
}

#[test]
fn a_switched_off_section_keeps_its_values_and_keys() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParam(ColorParamId::Contrast, 25),
    )
    .unwrap();
    key(&mut fx, video, ColorParamId::Exposure, 2, 10);
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetSectionEnabled(Section::Light, false),
    )
    .unwrap();
    let clip = fx.doc().clip(video).unwrap().clone();
    let s = ColorStack::of(clip.color.as_ref());
    assert!(!s.sections_enabled.is_on(Section::Light));
    assert_eq!(s.basic.contrast, 25);
    assert!(
        clip.keys
            .is_animated(PropPath::Color(ColorParamId::Exposure))
    );
    assert!(s.is_identity(), "nothing else is on");
}

#[test]
fn a_section_reset_removes_its_keys_and_undo_restores_them() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    key(&mut fx, video, ColorParamId::Exposure, 0, -40);
    key(&mut fx, video, ColorParamId::Exposure, 10, 40);
    key(&mut fx, video, ColorParamId::Temperature, 5, 30);
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParam(ColorParamId::Amount, 50),
    )
    .unwrap();
    let before = fx.doc().clip(video).unwrap().clone();
    // `apply` checks that undo restores the keys and redo removes them.
    color(
        &mut fx,
        vec![video],
        ColorEdit::ResetSection(Section::Light),
    )
    .unwrap();
    assert_eq!(label(&fx), (OpKind::EditColor(ColorOp::Reset), 1));
    let after = fx.doc().clip(video).unwrap().clone();
    assert!(
        !after
            .keys
            .is_animated(PropPath::Color(ColorParamId::Exposure))
    );
    assert!(
        after
            .keys
            .is_animated(PropPath::Color(ColorParamId::Temperature)),
        "the Color section keeps its keys"
    );
    assert_eq!(ColorStack::of(after.color.as_ref()).amount, 50);
    fx.editor.undo().unwrap();
    assert_eq!(fx.doc().clip(video).unwrap().as_ref(), before.as_ref());
    fx.editor.redo().unwrap();
    color(&mut fx, vec![video], ColorEdit::ResetAll).unwrap();
    let all = fx.doc().clip(video).unwrap();
    assert_eq!(all.color, None);
    assert!(all.keys.is_empty());
    // Resetting what is already at defaults records nothing.
    assert!(matches!(
        color(
            &mut fx,
            vec![video],
            ColorEdit::ResetSection(Section::Light)
        )
        .unwrap(),
        Committed::NoOp
    ));
}

#[test]
fn color_at_interpolates_and_rounds_half_away_from_zero() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    let none = fx.doc().clip(video).unwrap().clone();
    assert_eq!(none.color_at(f(3)), None, "no stack, no keys");
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParam(ColorParamId::Tint, 7),
    )
    .unwrap();
    let plain = fx.doc().clip(video).unwrap().clone();
    let same = plain.color_at(f(3)).unwrap();
    assert!(
        Arc::ptr_eq(&same, plain.color.as_ref().unwrap()),
        "same Arc"
    );
    // 0 → 3 over 2 frames: 1.5 rounds away from zero to 2 at the middle;
    // −3 → 0 gives −1.5 → −2.
    key(&mut fx, video, ColorParamId::Exposure, 0, 0);
    key(&mut fx, video, ColorParamId::Exposure, 2, 3);
    key(&mut fx, video, ColorParamId::Temperature, 0, -3);
    key(&mut fx, video, ColorParamId::Temperature, 2, 0);
    let c = fx.doc().clip(video).unwrap().clone();
    let ct = c.content_time_at(f(1)).unwrap();
    let mid = c.color_at(ct).unwrap();
    assert_eq!((mid.basic.exposure, mid.basic.temperature), (2, -2));
    assert_eq!(mid.basic.tint, 7, "static values stay");
    let end = c.color_at(c.content_time_at(f(5)).unwrap()).unwrap();
    assert_eq!((end.basic.exposure, end.basic.temperature), (3, 0));
    // A hold key keeps its value to the next key.
    fx.apply(EditRequest::EditKeys {
        seq: fx.seq_id(),
        clips: vec![video],
        prop: PropPath::Color(ColorParamId::Exposure),
        edit: KeyEdit::SetInterp {
            at: f(0),
            interp: Interp::Hold,
        },
    })
    .unwrap();
    let c = fx.doc().clip(video).unwrap().clone();
    let held = c.color_at(c.content_time_at(f(1)).unwrap()).unwrap();
    assert_eq!(held.basic.exposure, 0);
}

#[test]
fn keys_are_clamped_and_only_eligible_clips_take_them() {
    let mut fx = Fixture::new();
    let (video, _, _, text, _) = kinds(&mut fx);
    key(&mut fx, video, ColorParamId::Amount, 0, 300);
    let c = fx.doc().clip(video).unwrap();
    assert_eq!(
        c.keys.keys(PropPath::Color(ColorParamId::Amount))[0].value,
        PropValue::Level(100)
    );
    let refused = fx.apply(EditRequest::EditKeys {
        seq: fx.seq_id(),
        clips: vec![text],
        prop: PropPath::Color(ColorParamId::Hue),
        edit: KeyEdit::Add {
            at: f(0),
            value: None,
        },
    });
    assert_eq!(refused.unwrap_err(), EditError::NoColorClip);
    // Key this card on a colour section keys each of its sliders.
    fx.apply(EditRequest::EditKeys {
        seq: fx.seq_id(),
        clips: vec![video],
        prop: PropPath::Color(ColorParamId::Hue),
        edit: KeyEdit::KeyCard {
            card: PropGroup::Color(Section::Color),
            at: f(4),
        },
    })
    .unwrap();
    let c = fx.doc().clip(video).unwrap();
    for id in Section::Color.params() {
        assert!(c.keys.is_animated(PropPath::Color(*id)), "{id:?}");
    }
}

#[test]
fn validate_refuses_a_stack_on_a_text_clip_and_out_of_range_values() {
    let mut fx = Fixture::new();
    let (video, _, _, text, _) = kinds(&mut fx);
    let doc = fx.doc().clone();
    let ctx = ValidateCtx {
        limits: Default::default(),
        id_high_water: None,
    };
    let with = |id: ClipId, stack: ColorStack| with_stack(&doc, id, stack);
    let warm = ColorStack::default().with(ColorParamId::Temperature, 10);
    assert!(with(video, warm.clone()).validate(&ctx).is_ok());
    let errs = with(text, warm).validate(&ctx).unwrap_err();
    assert_eq!(errs, vec![InvariantViolation::Color { clip: text }]);
    let wild = ColorStack {
        amount: 120,
        ..ColorStack::default()
    };
    let errs = with(video, wild).validate(&ctx).unwrap_err();
    assert_eq!(errs, vec![InvariantViolation::Color { clip: video }]);
    // The default stack is stored as `None`, never as a value.
    let errs = with(video, ColorStack::default())
        .validate(&ctx)
        .unwrap_err();
    assert_eq!(errs, vec![InvariantViolation::Color { clip: video }]);
}

/// `doc` with clip `id`'s stored stack replaced by `stack` (its keys kept).
fn with_stack(doc: &Document, id: ClipId, stack: ColorStack) -> Document {
    let rebuild = |tracks: &[Arc<Track>]| -> Vec<Arc<Track>> {
        tracks
            .iter()
            .map(|t| {
                let clips = t
                    .clips()
                    .iter()
                    .map(|c| {
                        let mut c = Clip::clone(c);
                        if c.id == id {
                            c.color = Some(Arc::new(stack.clone()));
                        }
                        Arc::new(c)
                    })
                    .collect();
                let mut out = Track::from_parts(t.id, t.kind, t.header.clone(), clips);
                out.mix = t.mix;
                Arc::new(out)
            })
            .collect()
    };
    let sequences: Vec<_> = doc
        .sequences()
        .map(|s| {
            let mut out = Sequence::from_parts(
                s.id,
                s.flags,
                rebuild(s.visual_tracks()),
                rebuild(s.audio_tracks()),
            );
            out.master = s.master;
            out.markers = s.markers.clone();
            Arc::new(out)
        })
        .collect();
    Document::from_parts(
        doc.root(),
        *doc.settings(),
        doc.library().clone(),
        sequences,
    )
    .unwrap()
}

#[test]
fn validate_refuses_colour_keys_for_values_the_stack_lacks() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    let ctx = ValidateCtx {
        limits: Default::default(),
        id_high_water: None,
    };
    let warm = ColorStack::default().with(ColorParamId::Temperature, 10);
    color(&mut fx, vec![video], ColorEdit::AddMask(ellipse())).unwrap();
    mask_key(
        &mut fx,
        video,
        mask_prop(1, MaskParam::Center),
        0,
        PropValue::MaskPoint(MaskPoint::new(9_000, 0)),
    );
    let masked = fx.doc().clone();
    let errs = with_stack(&masked, video, warm.clone())
        .validate(&ctx)
        .unwrap_err();
    assert_eq!(
        errs,
        vec![InvariantViolation::Keys { clip: video }],
        "mask keys"
    );
    color(&mut fx, vec![video], ColorEdit::RemoveMask(1)).unwrap();
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetLut(Some(lut(1, "a.cube"))),
    )
    .unwrap();
    key(&mut fx, video, ColorParamId::LutIntensity, 0, 0);
    let errs = with_stack(fx.doc(), video, warm.clone())
        .validate(&ctx)
        .unwrap_err();
    assert_eq!(
        errs,
        vec![InvariantViolation::Keys { clip: video }],
        "LUT keys"
    );
    color(&mut fx, vec![video], ColorEdit::SetLut(None)).unwrap();
    color(&mut fx, vec![video], ColorEdit::SetAuto(Some(auto(12, 10)))).unwrap();
    key(&mut fx, video, ColorParamId::AutoStrength, 0, 0);
    let errs = with_stack(fx.doc(), video, warm)
        .validate(&ctx)
        .unwrap_err();
    assert_eq!(
        errs,
        vec![InvariantViolation::Keys { clip: video }],
        "Strength keys"
    );
}

#[test]
fn split_gives_both_halves_the_stack_and_the_keys() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParam(ColorParamId::Hue, 60),
    )
    .unwrap();
    key(&mut fx, video, ColorParamId::Exposure, 0, -20);
    key(&mut fx, video, ColorParamId::Exposure, 19, 20);
    let Committed::Applied { selection, .. } = fx
        .apply(EditRequest::Split(SplitReq {
            seq: fx.seq_id(),
            at: f(8),
            targets: SplitTargets::Clips(vec![video]),
        }))
        .unwrap()
    else {
        panic!("no-op");
    };
    let right = selection.unwrap()[0];
    let (l, r) = (
        fx.doc().clip(video).unwrap().clone(),
        fx.doc().clip(right).unwrap().clone(),
    );
    assert_eq!(l.color, r.color);
    // Keys sit in source time, so the grade runs on across the cut.
    for t in [7, 8, 9] {
        let piece = if t < 8 { &l } else { &r };
        let ct = piece.content_time_at(f(t)).unwrap();
        let whole = ColorStack::of(piece.color_at(ct).as_ref());
        assert_eq!(whole.basic.hue, 60);
    }
    let at_cut_l = l.color_at(l.content_time_at(f(7)).unwrap()).unwrap();
    let at_cut_r = r.color_at(r.content_time_at(f(9)).unwrap()).unwrap();
    assert!(at_cut_l.basic.exposure < at_cut_r.basic.exposure);
}

#[test]
fn the_plan_carries_evaluated_grades_for_pictures_only() {
    let mut fx = Fixture::new();
    let (video, image, adjust, ..) = kinds(&mut fx);
    let offline = BTreeSet::new();
    let inputs = PlanInputs { offline: &offline };
    let plan_color = |fx: &Fixture, clip: ClipId, t: i64| {
        let plan = frame_plan(fx.doc(), fx.seq_id(), f(t), &inputs).unwrap();
        plan.nodes
            .into_iter()
            .filter_map(|n| match n {
                PlanNode::Layer(l) => Some(l),
                PlanNode::Adjust(_) => None,
            })
            .find(|l| l.clip == clip)
            .unwrap()
            .color
    };
    assert_eq!(plan_color(&fx, video, 2), None);
    color(
        &mut fx,
        vec![video, image, adjust],
        ColorEdit::SetParam(ColorParamId::Saturation, 40),
    )
    .unwrap();
    assert_eq!(plan_color(&fx, video, 2).unwrap().basic.saturation, 40);
    assert_eq!(plan_color(&fx, image, 2).unwrap().basic.saturation, 40);
    let adjust_node = frame_plan(fx.doc(), fx.seq_id(), f(2), &inputs)
        .unwrap()
        .nodes
        .into_iter()
        .find_map(|n| match n {
            PlanNode::Adjust(a) if a.clip == adjust => Some(a),
            _ => None,
        })
        .expect("adjustment clips grade with their own node (K4)");
    assert_eq!(adjust_node.color.basic.saturation, 40);
    // Bypassed: nothing to do.
    color(&mut fx, vec![video], ColorEdit::SetBypass(true)).unwrap();
    assert_eq!(plan_color(&fx, video, 2), None);
    color(&mut fx, vec![video], ColorEdit::SetBypass(false)).unwrap();
    // A keyed value is evaluated at the frame.
    key(&mut fx, video, ColorParamId::Exposure, 0, 0);
    key(&mut fx, video, ColorParamId::Exposure, 10, 100);
    assert_eq!(plan_color(&fx, video, 5).unwrap().basic.exposure, 50);
    // Offline media shows the slate ungraded.
    let offline: BTreeSet<_> = [VIDEO_100].into();
    let inputs = PlanInputs { offline: &offline };
    let plan = frame_plan(fx.doc(), fx.seq_id(), f(5), &inputs).unwrap();
    let slate = plan
        .nodes
        .iter()
        .filter_map(PlanNode::layer)
        .find(|l| l.clip == video)
        .unwrap();
    assert_eq!(slate.color, None);
}

#[test]
fn color_paths_round_trip_their_names() {
    for id in ColorParamId::ALL {
        let p = PropPath::Color(id);
        assert_eq!(PropPath::parse(&p.to_path_string()), Some(p));
        assert!(p.is_animatable());
        assert_eq!(p.group(), id.section().map(PropGroup::Color));
    }
    assert_eq!(
        PropPath::Color(ColorParamId::ToneLift).to_path_string(),
        "color.tone_lift"
    );
    assert_eq!(PropPath::parse("color.nope"), None);
    let content = ClipContent::Generated {
        kind: GeneratedKind::Sticker,
    };
    assert!(!content.takes_color());
}

// ---------------------------------------------------------------------------
// K2: HSL, Curves, Wheels (CL-013, CL-014, CL-015)
// ---------------------------------------------------------------------------

fn hsl(band: HueBand, channel: HslChannel) -> ColorParamId {
    ColorParamId::Hsl { band, channel }
}

fn s_curve() -> Curve {
    Curve::new(vec![
        CurvePoint::new(0, 0),
        CurvePoint::new(2_500, 1_800),
        CurvePoint::new(7_500, 8_200),
        CurvePoint::new(10_000, 10_000),
    ])
    .unwrap()
}

fn puck_key(fx: &mut Fixture, clip: ClipId, wheel: WheelId, at: i64, x: i16, y: i16) {
    fx.apply(EditRequest::EditKeys {
        seq: fx.seq_id(),
        clips: vec![clip],
        prop: PropPath::ColorPuck(wheel),
        edit: KeyEdit::Add {
            at: f(at),
            value: Some(PropValue::Puck { x, y }),
        },
    })
    .unwrap();
}

#[test]
fn every_k2_edit_applies_undoes_and_redoes() {
    let mut fx = Fixture::new();
    let (video, image, adjust, ..) = kinds(&mut fx);
    let edits = [
        ColorEdit::SetParam(hsl(HueBand::Green, HslChannel::Hue), 40),
        ColorEdit::SetParams(vec![
            (hsl(HueBand::Red, HslChannel::Saturation), -30),
            (hsl(HueBand::Red, HslChannel::Lightness), 25),
        ]),
        ColorEdit::SetCurve {
            channel: CurveChannel::Luma,
            curve: s_curve(),
        },
        ColorEdit::SetCurve {
            channel: CurveChannel::Blue,
            curve: s_curve(),
        },
        ColorEdit::SetPuck {
            wheel: WheelId::Shadows,
            x: 400,
            y: -300,
        },
        ColorEdit::SetParam(ColorParamId::WheelLuma(WheelId::Highlights), -20),
        ColorEdit::SetParam(ColorParamId::WheelStrength, 60),
        ColorEdit::SetSectionEnabled(Section::Curves, false),
        ColorEdit::ResetBand(HueBand::Red),
        ColorEdit::ResetWheel(WheelId::Shadows),
        ColorEdit::ResetSection(Section::Hsl),
        ColorEdit::ResetSection(Section::Curves),
        ColorEdit::ResetSection(Section::Wheels),
    ];
    for edit in edits {
        let op = edit.op();
        color(&mut fx, vec![video, image, adjust], edit.clone()).unwrap();
        assert_eq!(label(&fx), (OpKind::EditColor(op), 3), "{edit:?}");
    }
    for c in [video, image, adjust] {
        assert_eq!(fx.doc().clip(c).unwrap().color, None, "every reset");
    }
    assert_eq!(
        ColorEdit::SetCurve {
            channel: CurveChannel::Red,
            curve: s_curve()
        }
        .op(),
        ColorOp::Curve
    );
    assert_eq!(
        ColorEdit::SetPuck {
            wheel: WheelId::Offset,
            x: 0,
            y: 0
        }
        .op(),
        ColorOp::Puck
    );
}

#[test]
fn pucks_are_pulled_onto_the_disk_and_curves_set_whole() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetPuck {
            wheel: WheelId::Midtones,
            x: 3_000,
            y: 4_000,
        },
    )
    .unwrap();
    assert_eq!(stack(&fx, video).puck(WheelId::Midtones), (600, 800));
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetCurve {
            channel: CurveChannel::Green,
            curve: s_curve(),
        },
    )
    .unwrap();
    assert_eq!(stack(&fx, video).curves.green, s_curve());
    // Setting the same curve again records nothing.
    assert!(matches!(
        color(
            &mut fx,
            vec![video],
            ColorEdit::SetCurve {
                channel: CurveChannel::Green,
                curve: s_curve(),
            }
        )
        .unwrap(),
        Committed::NoOp
    ));
    // Back to the diagonal and the centre: nothing stored.
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetCurve {
            channel: CurveChannel::Green,
            curve: Curve::identity(),
        },
    )
    .unwrap();
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetPuck {
            wheel: WheelId::Midtones,
            x: 0,
            y: 0,
        },
    )
    .unwrap();
    assert_eq!(fx.doc().clip(video).unwrap().color, None);
}

#[test]
fn band_and_wheel_resets_remove_only_their_keys() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    key(&mut fx, video, hsl(HueBand::Blue, HslChannel::Hue), 0, 30);
    key(&mut fx, video, hsl(HueBand::Cyan, HslChannel::Hue), 0, 10);
    puck_key(&mut fx, video, WheelId::Highlights, 0, 100, 100);
    key(
        &mut fx,
        video,
        ColorParamId::WheelLuma(WheelId::Highlights),
        0,
        20,
    );
    key(
        &mut fx,
        video,
        ColorParamId::WheelLuma(WheelId::Offset),
        0,
        5,
    );
    color(&mut fx, vec![video], ColorEdit::ResetBand(HueBand::Blue)).unwrap();
    color(
        &mut fx,
        vec![video],
        ColorEdit::ResetWheel(WheelId::Highlights),
    )
    .unwrap();
    let c = fx.doc().clip(video).unwrap().clone();
    assert!(
        !c.keys
            .is_animated(PropPath::Color(hsl(HueBand::Blue, HslChannel::Hue)))
    );
    assert!(
        c.keys
            .is_animated(PropPath::Color(hsl(HueBand::Cyan, HslChannel::Hue)))
    );
    assert!(!c.keys.is_animated(PropPath::ColorPuck(WheelId::Highlights)));
    assert!(!c.keys.is_animated(PropPath::Color(ColorParamId::WheelLuma(
        WheelId::Highlights
    ))));
    assert!(
        c.keys
            .is_animated(PropPath::Color(ColorParamId::WheelLuma(WheelId::Offset)))
    );
    // The Wheels section reset takes the pucks' keys too; Reset all the rest.
    puck_key(&mut fx, video, WheelId::Shadows, 3, -200, 0);
    color(
        &mut fx,
        vec![video],
        ColorEdit::ResetSection(Section::Wheels),
    )
    .unwrap();
    let c = fx.doc().clip(video).unwrap().clone();
    assert!(!c.keys.is_animated(PropPath::ColorPuck(WheelId::Shadows)));
    assert!(
        !c.keys
            .is_animated(PropPath::Color(ColorParamId::WheelLuma(WheelId::Offset)))
    );
    assert!(
        c.keys
            .is_animated(PropPath::Color(hsl(HueBand::Cyan, HslChannel::Hue)))
    );
    puck_key(&mut fx, video, WheelId::Offset, 3, 50, 0);
    assert!(freecut_timeline::edit::has_color(
        fx.doc().clip(video).unwrap()
    ));
    color(&mut fx, vec![video], ColorEdit::ResetAll).unwrap();
    assert!(fx.doc().clip(video).unwrap().keys.is_empty());
}

#[test]
fn keyed_pucks_travel_straight_and_stay_on_the_disk() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    puck_key(&mut fx, video, WheelId::Shadows, 0, -1000, 0);
    puck_key(&mut fx, video, WheelId::Shadows, 4, 1000, 0);
    puck_key(&mut fx, video, WheelId::Offset, 0, 1000, 0);
    puck_key(&mut fx, video, WheelId::Offset, 4, 0, 1000);
    // Keys hold values on the disk, whatever was asked.
    puck_key(&mut fx, video, WheelId::Midtones, 0, 9_000, 9_000);
    let c = fx.doc().clip(video).unwrap().clone();
    let PropValue::Puck { x, y } = c.keys.keys(PropPath::ColorPuck(WheelId::Midtones))[0].value
    else {
        panic!("a puck key");
    };
    assert!(i32::from(x).pow(2) + i32::from(y).pow(2) <= 1_000_000);
    let at = |t: i64| c.color_at(c.content_time_at(f(t)).unwrap()).unwrap();
    assert_eq!(at(2).puck(WheelId::Shadows), (0, 0), "through the centre");
    assert_eq!(at(1).puck(WheelId::Shadows), (-500, 0));
    // Halfway along the chord between two rim points, inside the disk.
    assert_eq!(at(2).puck(WheelId::Offset), (500, 500));
    let s = at(2);
    assert!(s.is_valid());
    // The group of Wheels keys its pucks too.
    let props = PropGroup::Color(Section::Wheels).props(&c);
    assert!(props.contains(&PropPath::ColorPuck(WheelId::Offset)));
    assert!(props.contains(&PropPath::Color(ColorParamId::WheelStrength)));
    assert!(
        !PropGroup::Color(Section::Curves)
            .props(&c)
            .iter()
            .any(|p| p.is_animatable())
    );
}

#[test]
fn hsl_curves_and_wheels_reach_the_plan() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    let offline = BTreeSet::new();
    let inputs = PlanInputs { offline: &offline };
    let plan_color = |fx: &Fixture, t: i64| {
        let plan = frame_plan(fx.doc(), fx.seq_id(), f(t), &inputs).unwrap();
        plan.nodes
            .into_iter()
            .filter_map(|n| match n {
                PlanNode::Layer(l) => Some(l),
                PlanNode::Adjust(_) => None,
            })
            .find(|l| l.clip == video)
            .unwrap()
            .color
    };
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetCurve {
            channel: CurveChannel::Luma,
            curve: s_curve(),
        },
    )
    .unwrap();
    assert_eq!(plan_color(&fx, 1).unwrap().curves.luma, s_curve());
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetSectionEnabled(Section::Curves, false),
    )
    .unwrap();
    assert_eq!(
        plan_color(&fx, 1),
        None,
        "a switched-off curve does nothing"
    );
    // Wheels at Strength 0 do nothing either.
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParams(vec![
            (ColorParamId::WheelLuma(WheelId::Midtones), 30),
            (ColorParamId::WheelStrength, 0),
        ]),
    )
    .unwrap();
    assert_eq!(plan_color(&fx, 1), None);
    puck_key(&mut fx, video, WheelId::Offset, 0, 0, 0);
    puck_key(&mut fx, video, WheelId::Offset, 10, 0, -800);
    key(&mut fx, video, ColorParamId::WheelStrength, 0, 100);
    let s = plan_color(&fx, 5).unwrap();
    assert_eq!(s.puck(WheelId::Offset), (0, -400));
    assert_eq!(s.wheels.strength, 100);
}

#[test]
fn k2_paths_round_trip_their_names() {
    for w in WheelId::ALL {
        let p = PropPath::ColorPuck(w);
        assert_eq!(PropPath::parse(&p.to_path_string()), Some(p));
        assert_eq!(p.group(), Some(PropGroup::Color(Section::Wheels)));
    }
    assert_eq!(
        PropPath::ColorPuck(WheelId::Midtones).to_path_string(),
        "color.wheels.midtones.puck"
    );
    assert_eq!(
        PropPath::Color(hsl(HueBand::Orange, HslChannel::Lightness)).to_path_string(),
        "color.hsl.orange.lightness"
    );
    assert_eq!(
        PropPath::Color(ColorParamId::WheelLuma(WheelId::Offset)).to_path_string(),
        "color.wheels.offset.luma"
    );
    assert_eq!(PropPath::parse("color.wheels.nope.puck"), None);
    assert_eq!(PropPath::parse("color.hsl.red"), None);
}

fn lut(n: u8, name: &str) -> LutSlot {
    LutSlot::new(LutRef::new([n; 16], name))
}

#[test]
fn every_k3_edit_applies_undoes_and_redoes() {
    let mut fx = Fixture::new();
    let (video, image, adjust, ..) = kinds(&mut fx);
    let edits = [
        ColorEdit::SetLut(Some(lut(1, "teal.cube"))),
        ColorEdit::SetParam(ColorParamId::LutIntensity, 60),
        ColorEdit::SetSkin {
            scope: SkinScope::Lut,
            on: true,
        },
        ColorEdit::SetParam(ColorParamId::SkinAmount(SkinScope::Lut), 40),
        ColorEdit::SetLutInterp(LutInterp::Trilinear),
        ColorEdit::SetLut(Some(lut(2, "warm.3dl"))),
        ColorEdit::SetSkin {
            scope: SkinScope::Color,
            on: true,
        },
        ColorEdit::SetParam(ColorParamId::SkinAmount(SkinScope::Color), 90),
        ColorEdit::SetSectionEnabled(Section::Lut, false),
        ColorEdit::ResetSection(Section::Lut),
        ColorEdit::ResetSection(Section::Color),
        ColorEdit::SetLut(None),
    ];
    for edit in edits {
        let op = edit.op();
        color(&mut fx, vec![video, image, adjust], edit.clone()).unwrap();
        assert_eq!(label(&fx), (OpKind::EditColor(op), 3), "{edit:?}");
        if edit == ColorEdit::SetLut(Some(lut(2, "warm.3dl"))) {
            // Replacing kept the first LUT's settings (CL-017).
            let slot = stack(&fx, video).lut.unwrap();
            assert_eq!(slot.lut.file_name, "warm.3dl");
            assert_eq!(slot.intensity, 60);
            assert_eq!(slot.interp, LutInterp::Trilinear);
            assert_eq!(
                slot.skin,
                SkinProtect {
                    on: true,
                    amount: 40
                }
            );
        }
        if edit == ColorEdit::ResetSection(Section::Lut) {
            let s = stack(&fx, image);
            assert_eq!(s.lut, Some(lut(2, "warm.3dl")), "a reset keeps the file");
            assert!(s.sections_enabled.is_on(Section::Lut));
        }
    }
    for c in [video, image, adjust] {
        assert_eq!(fx.doc().clip(c).unwrap().color, None, "{c:?}");
    }
}

#[test]
fn lut_keys_need_a_lut_and_go_with_it() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    // No LUT: its Intensity takes no keys.
    assert!(
        fx.apply(EditRequest::EditKeys {
            seq: fx.seq_id(),
            clips: vec![video],
            prop: PropPath::Color(ColorParamId::LutIntensity),
            edit: KeyEdit::Add {
                at: f(0),
                value: Some(PropValue::Level(50)),
            },
        })
        .is_err()
    );
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetLut(Some(lut(3, "a.cube"))),
    )
    .unwrap();
    key(&mut fx, video, ColorParamId::LutIntensity, 0, 0);
    key(&mut fx, video, ColorParamId::LutIntensity, 10, 100);
    key(
        &mut fx,
        video,
        ColorParamId::SkinAmount(SkinScope::Color),
        0,
        20,
    );
    let clip = fx.doc().clip(video).unwrap().clone();
    assert_eq!(
        clip.color_at(f(5)).unwrap().get(ColorParamId::LutIntensity),
        50
    );
    assert_eq!(
        PropGroup::Color(Section::Lut).props(&clip),
        vec![
            PropPath::Color(ColorParamId::LutIntensity),
            PropPath::Color(ColorParamId::SkinAmount(SkinScope::Lut)),
        ]
    );
    // Clearing the slot takes the LUT's keys with it, in one entry; the
    // Color scope's skin key stays.
    color(&mut fx, vec![video], ColorEdit::SetLut(None)).unwrap();
    let props: Vec<PropPath> = fx
        .doc()
        .clip(video)
        .unwrap()
        .keys
        .tracks()
        .iter()
        .map(|t| t.prop())
        .collect();
    assert_eq!(
        props,
        vec![PropPath::Color(ColorParamId::SkinAmount(SkinScope::Color))]
    );
    fx.editor.undo().unwrap();
    assert_eq!(fx.doc().clip(video).unwrap().keys.tracks().len(), 2);
}

#[test]
fn the_lut_reaches_the_plan_and_intensity_0_skips_it() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    let offline = BTreeSet::new();
    let inputs = PlanInputs { offline: &offline };
    let plan_color = |fx: &Fixture| {
        let plan = frame_plan(fx.doc(), fx.seq_id(), f(1), &inputs).unwrap();
        plan.nodes
            .into_iter()
            .filter_map(|n| match n {
                PlanNode::Layer(l) => Some(l),
                PlanNode::Adjust(_) => None,
            })
            .find(|l| l.clip == video)
            .unwrap()
            .color
    };
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetLut(Some(lut(4, "b.cube"))),
    )
    .unwrap();
    assert_eq!(plan_color(&fx).unwrap().lut, Some(lut(4, "b.cube")));
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParam(ColorParamId::LutIntensity, 0),
    )
    .unwrap();
    assert_eq!(plan_color(&fx), None, "Intensity 0 hides the LUT");
    // Skin protection alone changes nothing either.
    color(&mut fx, vec![video], ColorEdit::SetLut(None)).unwrap();
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetSkin {
            scope: SkinScope::Color,
            on: true,
        },
    )
    .unwrap();
    assert!(fx.doc().clip(video).unwrap().color.is_some());
    assert_eq!(plan_color(&fx), None);
}

#[test]
fn k3_paths_round_trip_their_names() {
    for id in [
        ColorParamId::LutIntensity,
        ColorParamId::SkinAmount(SkinScope::Color),
        ColorParamId::SkinAmount(SkinScope::Lut),
    ] {
        let p = PropPath::Color(id);
        assert_eq!(PropPath::parse(&p.to_path_string()), Some(p));
    }
    assert_eq!(
        PropPath::Color(ColorParamId::LutIntensity).to_path_string(),
        "color.lut.intensity"
    );
    assert_eq!(
        PropPath::Color(ColorParamId::SkinAmount(SkinScope::Color)).group(),
        Some(PropGroup::Color(Section::Color))
    );
}

fn auto(exposure: i16, temperature: i16) -> AutoCorrection {
    AutoCorrection::new(
        AutoOffsets {
            exposure,
            temperature,
            blacks: -4,
            ..AutoOffsets::default()
        },
        AutoStyle::Natural,
        AutoSource::WholeClip,
    )
}

fn matched(clip: ClipId) -> MatchTransform {
    MatchTransform::new(
        [12_000, 9_000, 11_000],
        [-300, 150, -80],
        MatchRef::Clip {
            clip,
            at: f(3),
            graded: true,
        },
    )
}

#[test]
fn a_match_to_something_that_does_not_exist_is_refused() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    let before = fx.editor.document().clone();
    let never = std::num::NonZeroU64::new(999_999).unwrap();
    let missing_clip = MatchTransform::identity(MatchRef::Clip {
        clip: ClipId::from_raw(never),
        at: f(0),
        graded: false,
    });
    let missing_media = MatchTransform::identity(MatchRef::Media {
        media: freecut_timeline::ids::MediaId::from_raw(never),
        at: f(0),
    });
    assert!(
        color(
            &mut fx,
            vec![video],
            ColorEdit::SetMatch(Some(missing_clip))
        )
        .is_err()
    );
    assert_eq!(
        color(
            &mut fx,
            vec![video],
            ColorEdit::SetMatch(Some(missing_media))
        )
        .unwrap_err(),
        EditError::UnknownMedia(freecut_timeline::ids::MediaId::from_raw(never))
    );
    // Nothing changed: the document stays saveable.
    assert_eq!(*fx.editor.document(), before);
}

#[test]
fn every_k5_edit_applies_undoes_and_redoes() {
    let mut fx = Fixture::new();
    let (video, image, adjust, ..) = kinds(&mut fx);
    let edits = [
        ColorEdit::SetAuto(Some(auto(12, -8))),
        ColorEdit::SetParam(ColorParamId::AutoStrength, 50),
        ColorEdit::SetAuto(Some(auto(20, 4))),
        ColorEdit::SetSectionEnabled(Section::Auto, false),
        ColorEdit::SetSectionEnabled(Section::Auto, true),
        ColorEdit::ApplyAutoToSliders,
        ColorEdit::SetAuto(Some(auto(-6, 30))),
        ColorEdit::ResetSection(Section::Auto),
        ColorEdit::SetMatch(Some(matched(video))),
        ColorEdit::SetParam(ColorParamId::MatchIntensity, 40),
        ColorEdit::SetMatch(Some(MatchTransform::identity(MatchRef::Clip {
            clip: image,
            at: f(0),
            graded: false,
        }))),
        ColorEdit::SetSectionEnabled(Section::Match, false),
        ColorEdit::ResetSection(Section::Match),
        ColorEdit::SetMatch(Some(matched(video))),
        ColorEdit::SetMatch(None),
        ColorEdit::SetAuto(Some(auto(1, 1))),
        ColorEdit::SetAuto(None),
    ];
    for edit in edits {
        let op = edit.op();
        color(&mut fx, vec![video, image, adjust], edit.clone()).unwrap();
        assert_eq!(label(&fx), (OpKind::EditColor(op), 3), "{edit:?}");
        if edit == ColorEdit::SetAuto(Some(auto(20, 4))) {
            // A re-run replaces the offsets and keeps the Strength.
            let a = stack(&fx, video).auto.unwrap();
            assert_eq!((a.offsets.exposure, a.strength), (20, 50));
        }
        if edit == ColorEdit::ApplyAutoToSliders {
            // +20 and +4 at 50 % (rounded half away from zero), Blacks −4 → −2.
            let s = stack(&fx, image);
            assert_eq!(s.auto, None);
            assert_eq!(
                (s.basic.exposure, s.basic.temperature, s.basic.blacks),
                (10, 2, -2)
            );
        }
        if matches!(edit, ColorEdit::SetMatch(Some(ref m)) if m.is_identity()) {
            // Re-match keeps the Intensity.
            let m = stack(&fx, adjust).match_xf.unwrap();
            assert!(m.is_identity());
            assert_eq!(m.intensity, 40);
        }
    }
    // Apply to sliders left plain slider values behind.
    let s = stack(&fx, video);
    assert_eq!((s.auto, s.match_xf), (None, None));
    assert_eq!(s.basic.exposure, 10);
}

#[test]
fn auto_offsets_sit_under_the_sliders() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParams(vec![
            (ColorParamId::Exposure, 10),
            (ColorParamId::Temperature, 95),
        ]),
    )
    .unwrap();
    color(&mut fx, vec![video], ColorEdit::SetAuto(Some(auto(12, 10)))).unwrap();
    let s = stack(&fx, video);
    // The sliders keep the user's values; the renderer sees the sum,
    // clamped into the range.
    assert_eq!((s.basic.exposure, s.basic.temperature), (10, 95));
    let e = s.effective_basic();
    assert_eq!((e.exposure, e.temperature, e.blacks), (22, 100, -4));
    // Strength scales the offsets.
    let half = s.with(ColorParamId::AutoStrength, 50).effective_basic();
    assert_eq!(
        (half.exposure, half.temperature, half.blacks),
        (16, 100, -2)
    );
    // Auto off leaves the manual values; Light off leaves Auto's.
    let off = ColorStack {
        sections_enabled: s.sections_enabled.with(Section::Auto, false),
        ..s.clone()
    };
    assert_eq!(off.effective_basic().exposure, 10);
    let light_off = ColorStack {
        sections_enabled: s.sections_enabled.with(Section::Light, false),
        ..s.clone()
    };
    assert_eq!(light_off.effective_basic().exposure, 12);
    assert!(!s.section_is_default(Section::Auto) && !s.is_identity());
    // Apply to sliders adds and clamps.
    color(&mut fx, vec![video], ColorEdit::ApplyAutoToSliders).unwrap();
    let s = stack(&fx, video);
    assert_eq!(
        (
            s.auto,
            s.basic.exposure,
            s.basic.temperature,
            s.basic.blacks
        ),
        (None, 22, 100, -4)
    );
    // Without a correction it changes nothing and records nothing.
    assert!(matches!(
        color(&mut fx, vec![video], ColorEdit::ApplyAutoToSliders),
        Ok(Committed::NoOp) | Err(_)
    ));
    // A zero correction is no change for the renderer.
    let zero = ColorStack::default().with_auto(Some(AutoCorrection::new(
        AutoOffsets::default(),
        AutoStyle::Vivid,
        AutoSource::Playhead,
    )));
    assert!(zero.is_identity() && !zero.section_is_default(Section::Auto));
}

/// What the renderer sees at frames 0, 5 and 10 of `clip`.
fn seen(fx: &Fixture, clip: ClipId) -> Vec<freecut_timeline::model::BasicAdjust> {
    let c = fx.doc().clip(clip).unwrap();
    [0, 5, 10]
        .into_iter()
        .map(|t| {
            ColorStack::of(c.color_at(c.start().checked_add(f(t)).unwrap()).as_ref())
                .effective_basic()
        })
        .collect()
}

#[test]
fn apply_to_sliders_keeps_the_picture_or_is_refused() {
    for mask in 0..8u8 {
        let (auto_on, light_on, color_on) = (mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
        let mut fx = Fixture::new();
        let (video, ..) = kinds(&mut fx);
        color(
            &mut fx,
            vec![video],
            ColorEdit::SetParams(vec![(ColorParamId::Temperature, 30)]),
        )
        .unwrap();
        key(&mut fx, video, ColorParamId::Exposure, 0, 10);
        key(&mut fx, video, ColorParamId::Exposure, 10, 40);
        color(&mut fx, vec![video], ColorEdit::SetAuto(Some(auto(12, 10)))).unwrap();
        for (section, on) in [
            (Section::Auto, auto_on),
            (Section::Light, light_on),
            (Section::Color, color_on),
        ] {
            if !on {
                color(
                    &mut fx,
                    vec![video],
                    ColorEdit::SetSectionEnabled(section, false),
                )
                .unwrap();
            }
        }
        let before = seen(&fx, video);
        let blocker = stack(&fx, video).auto_apply_blocker();
        let what = format!("auto {auto_on} light {light_on} color {color_on}");
        match color(&mut fx, vec![video], ColorEdit::ApplyAutoToSliders) {
            Ok(_) => {
                assert_eq!(blocker, None, "{what}");
                assert_eq!(seen(&fx, video), before, "{what}: the picture stays");
                assert_eq!(stack(&fx, video).auto, None, "{what}");
            }
            Err(e) => {
                let section = blocker.expect(&what);
                assert_eq!(e, EditError::SectionSwitchedOff { section }, "{what}");
            }
        }
        // Only the all-on case applies here: every section takes an offset.
        assert_eq!(blocker.is_none(), auto_on && light_on && color_on, "{what}");
    }
}

#[test]
fn an_auto_offset_that_rounds_to_nothing_is_the_identity() {
    let weak = |strength: u8| {
        ColorStack::default().with_auto(Some(AutoCorrection {
            strength,
            ..AutoCorrection::new(
                AutoOffsets {
                    exposure: 1,
                    ..AutoOffsets::default()
                },
                AutoStyle::Natural,
                AutoSource::WholeClip,
            )
        }))
    };
    assert!(weak(49).is_identity());
    assert_eq!(weak(49).auto_apply_blocker(), None);
    assert!(!weak(50).is_identity());
}

#[test]
fn strength_and_intensity_keys_need_a_result_and_go_with_it() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    for id in [ColorParamId::AutoStrength, ColorParamId::MatchIntensity] {
        assert!(
            fx.apply(EditRequest::EditKeys {
                seq: fx.seq_id(),
                clips: vec![video],
                prop: PropPath::Color(id),
                edit: KeyEdit::Add {
                    at: f(0),
                    value: Some(PropValue::Level(50)),
                },
            })
            .is_err(),
            "{id:?} without a result takes no keys"
        );
    }
    color(&mut fx, vec![video], ColorEdit::SetAuto(Some(auto(30, 0)))).unwrap();
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetMatch(Some(matched(video))),
    )
    .unwrap();
    key(&mut fx, video, ColorParamId::AutoStrength, 0, 0);
    key(&mut fx, video, ColorParamId::AutoStrength, 10, 100);
    key(&mut fx, video, ColorParamId::MatchIntensity, 0, 20);
    key(&mut fx, video, ColorParamId::Exposure, 0, 50);
    let clip = fx.doc().clip(video).unwrap().clone();
    let mid = clip.color_at(f(5)).unwrap();
    assert_eq!(mid.get(ColorParamId::AutoStrength), 50);
    assert_eq!(mid.effective_basic().exposure, 65);
    assert_eq!(mid.match_xf.as_ref().unwrap().intensity, 20);
    // A re-run keeps the Strength keys (CL-019).
    color(&mut fx, vec![video], ColorEdit::SetAuto(Some(auto(40, 0)))).unwrap();
    assert!(
        fx.doc()
            .clip(video)
            .unwrap()
            .keys
            .is_animated(PropPath::Color(ColorParamId::AutoStrength))
    );
    // Apply to sliders moves the keyed Exposure by the offset at the static
    // Strength and drops the Strength keys, in one entry.
    color(&mut fx, vec![video], ColorEdit::ApplyAutoToSliders).unwrap();
    let clip = fx.doc().clip(video).unwrap().clone();
    let keys = clip.keys.keys(PropPath::Color(ColorParamId::Exposure));
    assert_eq!(keys[0].value, PropValue::Level(90));
    assert!(
        !clip
            .keys
            .is_animated(PropPath::Color(ColorParamId::AutoStrength))
    );
    // Removing the match takes its Intensity keys with it.
    color(&mut fx, vec![video], ColorEdit::SetMatch(None)).unwrap();
    let props: Vec<PropPath> = fx
        .doc()
        .clip(video)
        .unwrap()
        .keys
        .tracks()
        .iter()
        .map(|t| t.prop())
        .collect();
    assert_eq!(props, vec![PropPath::Color(ColorParamId::Exposure)]);
    fx.editor.undo().unwrap();
    assert!(
        fx.doc()
            .clip(video)
            .unwrap()
            .keys
            .is_animated(PropPath::Color(ColorParamId::MatchIntensity))
    );
}

#[test]
fn auto_and_match_reach_the_plan() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    let offline = BTreeSet::new();
    let inputs = PlanInputs { offline: &offline };
    let plan_color = |fx: &Fixture| {
        let plan = frame_plan(fx.doc(), fx.seq_id(), f(1), &inputs).unwrap();
        plan.nodes
            .into_iter()
            .filter_map(|n| match n {
                PlanNode::Layer(l) => Some(l),
                PlanNode::Adjust(_) => None,
            })
            .find(|l| l.clip == video)
            .unwrap()
            .color
    };
    color(&mut fx, vec![video], ColorEdit::SetAuto(Some(auto(12, 3)))).unwrap();
    assert_eq!(plan_color(&fx).unwrap().auto.unwrap().offsets.exposure, 12);
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParam(ColorParamId::AutoStrength, 0),
    )
    .unwrap();
    assert_eq!(plan_color(&fx), None, "Strength 0 changes nothing");
    color(&mut fx, vec![video], ColorEdit::SetAuto(None)).unwrap();
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetMatch(Some(MatchTransform::identity(MatchRef::Clip {
            clip: video,
            at: f(0),
            graded: true,
        }))),
    )
    .unwrap();
    assert!(fx.doc().clip(video).unwrap().color.is_some());
    assert_eq!(plan_color(&fx), None, "a self-match is the identity");
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetMatch(Some(matched(video))),
    )
    .unwrap();
    assert!(plan_color(&fx).unwrap().match_xf.is_some());
}

#[test]
fn k5_paths_round_trip_their_names() {
    for (id, name) in [
        (ColorParamId::AutoStrength, "color.auto.strength"),
        (ColorParamId::MatchIntensity, "color.match.intensity"),
    ] {
        let p = PropPath::Color(id);
        assert_eq!(p.to_path_string(), name);
        assert_eq!(PropPath::parse(name), Some(p));
    }
    assert_eq!(
        PropPath::Color(ColorParamId::AutoStrength).group(),
        Some(PropGroup::Color(Section::Auto))
    );
    assert_eq!(
        PropPath::Color(ColorParamId::MatchIntensity).group(),
        Some(PropGroup::Color(Section::Match))
    );
}

// K6: colour masks (CL-031).

fn ellipse() -> Mask {
    Mask::new(MaskShapeKind::Ellipse)
}

fn mask_prop(id: u16, param: MaskParam) -> PropPath {
    PropPath::ColorMask {
        id,
        param: ColorMaskParam::Shape(param),
    }
}

fn mask_key(fx: &mut Fixture, clip: ClipId, prop: PropPath, at: i64, value: PropValue) {
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
fn every_k6_edit_applies_undoes_and_redoes() {
    let mut fx = Fixture::new();
    let (video, image, adjust, ..) = kinds(&mut fx);
    let clips = vec![video, image, adjust];
    color(
        &mut fx,
        clips.clone(),
        ColorEdit::SetParam(ColorParamId::Exposure, 30),
    )
    .unwrap();
    let moved = ColorMask {
        mask: Mask {
            center: MaskPoint::new(2_000, -1_000),
            feather: PerMille::new(300),
            ..ellipse()
        },
        expand: 40,
        ..ColorMask::new(1, ellipse())
    };
    let edits = [
        ColorEdit::AddMask(ellipse()),
        ColorEdit::AddMask(Mask::new(MaskShapeKind::Rectangle)),
        ColorEdit::AddMask(Mask::new(MaskShapeKind::Linear)),
        ColorEdit::SetMask(moved),
        ColorEdit::SetMaskMode {
            id: 2,
            mode: MaskCombine::Subtract,
        },
        ColorEdit::ReorderMask { id: 3, to: 0 },
        ColorEdit::SetMaskParam {
            id: 2,
            param: ColorMaskParam::Shape(MaskParam::Roundness),
            value: PropValue::Ratio(PerMille::new(450)),
        },
        ColorEdit::RemoveMask(2),
        ColorEdit::SetSectionEnabled(Section::Masks, false),
        ColorEdit::SetSectionEnabled(Section::Masks, true),
        ColorEdit::AddMask(Mask::new(MaskShapeKind::Band)),
    ];
    for edit in edits {
        let op = edit.op();
        color(&mut fx, clips.clone(), edit.clone()).unwrap();
        assert_eq!(label(&fx), (OpKind::EditColor(op), 3), "{edit:?}");
    }
    let s = stack(&fx, adjust);
    // Ids stay with their masks; the removed id 2 is used again.
    let ids: Vec<(u16, MaskShapeKind)> = s.masks.iter().map(|m| (m.id, m.mask.kind())).collect();
    assert_eq!(
        ids,
        vec![
            (3, MaskShapeKind::Linear),
            (1, MaskShapeKind::Ellipse),
            (2, MaskShapeKind::Band)
        ]
    );
    assert_eq!(s.mask(1).unwrap().expand, 40);
    assert_eq!(
        stack(&fx, video).masks.len(),
        3,
        "the rectangle (2) was removed after its roundness was set"
    );
    assert!(s.masks_active());
    assert_eq!(ColorEdit::AddMask(ellipse()).op(), ColorOp::Mask);
    // The section reset removes every mask.
    color(
        &mut fx,
        clips.clone(),
        ColorEdit::ResetSection(Section::Masks),
    )
    .unwrap();
    assert!(stack(&fx, video).masks.is_empty());
}

#[test]
fn a_stack_holds_at_most_eight_masks() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    for _ in 0..MAX_COLOR_MASKS {
        color(&mut fx, vec![video], ColorEdit::AddMask(ellipse())).unwrap();
    }
    assert_eq!(stack(&fx, video).masks.len(), 8);
    assert!(matches!(
        color(&mut fx, vec![video], ColorEdit::AddMask(ellipse())),
        Ok(Committed::NoOp) | Err(_)
    ));
    assert_eq!(stack(&fx, video).masks.len(), 8);
    // Masks alone change nothing: the stack still needs a grade to show.
    assert!(stack(&fx, video).is_identity());
    // A mask stack on a text clip is refused like any stack.
    let text = add(
        &mut fx,
        NewClip::text(TextClipData::with_content("Hi"), f(20)),
    );
    assert_eq!(
        color(&mut fx, vec![text], ColorEdit::AddMask(ellipse())),
        Err(EditError::NoColorClip)
    );
}

#[test]
fn removing_a_mask_takes_its_keys_and_undo_brings_both_back() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParam(ColorParamId::Saturation, -60),
    )
    .unwrap();
    color(&mut fx, vec![video], ColorEdit::AddMask(ellipse())).unwrap();
    color(&mut fx, vec![video], ColorEdit::AddMask(ellipse())).unwrap();
    let center = mask_prop(1, MaskParam::Center);
    mask_key(
        &mut fx,
        video,
        center,
        0,
        PropValue::MaskPoint(MaskPoint::new(-4_000, 0)),
    );
    mask_key(
        &mut fx,
        video,
        center,
        10,
        PropValue::MaskPoint(MaskPoint::new(4_000, 0)),
    );
    mask_key(
        &mut fx,
        video,
        mask_prop(2, MaskParam::Feather),
        0,
        PropValue::Ratio(PerMille::new(200)),
    );
    let before = fx.doc().clip(video).unwrap().clone();
    color(&mut fx, vec![video], ColorEdit::RemoveMask(1)).unwrap();
    let after = fx.doc().clip(video).unwrap().clone();
    assert!(!after.keys.is_animated(center));
    assert!(after.keys.is_animated(mask_prop(2, MaskParam::Feather)));
    assert_eq!(ColorStack::of(after.color.as_ref()).masks.len(), 1);
    fx.editor.undo().unwrap();
    assert_eq!(fx.doc().clip(video).unwrap(), &before);
}

#[test]
fn a_keyed_ellipse_travels_and_shape_switches_drop_shape_only_keys() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParam(ColorParamId::Exposure, 40),
    )
    .unwrap();
    color(
        &mut fx,
        vec![video],
        ColorEdit::AddMask(Mask::new(MaskShapeKind::Rectangle)),
    )
    .unwrap();
    let center = mask_prop(1, MaskParam::Center);
    mask_key(
        &mut fx,
        video,
        center,
        0,
        PropValue::MaskPoint(MaskPoint::new(-4_000, 1_000)),
    );
    mask_key(
        &mut fx,
        video,
        center,
        10,
        PropValue::MaskPoint(MaskPoint::new(4_000, -1_000)),
    );
    let round = mask_prop(1, MaskParam::Roundness);
    mask_key(
        &mut fx,
        video,
        round,
        0,
        PropValue::Ratio(PerMille::new(100)),
    );
    mask_key(
        &mut fx,
        video,
        round,
        10,
        PropValue::Ratio(PerMille::new(9_000)),
    );
    let expand = PropPath::ColorMask {
        id: 1,
        param: ColorMaskParam::Expand,
    };
    mask_key(&mut fx, video, expand, 0, PropValue::Level(-300));
    let clip = fx.doc().clip(video).unwrap().clone();
    // Keys are clamped like edits.
    assert_eq!(
        clip.keys.keys(round)[1].value,
        PropValue::Ratio(PerMille::ONE)
    );
    assert_eq!(clip.keys.keys(expand)[0].value, PropValue::Level(-100));
    let mid = clip.color_at(f(5)).unwrap();
    let m = mid.mask(1).unwrap();
    assert_eq!(m.mask.center, MaskPoint::new(0, 0));
    assert_eq!(
        m.mask.shape,
        MaskShape::Rectangle {
            roundness: PerMille::new(550)
        }
    );
    assert_eq!(m.expand, -100);
    // The plan carries the evaluated mask.
    let offline = BTreeSet::new();
    let inputs = PlanInputs { offline: &offline };
    let plan = frame_plan(fx.doc(), fx.seq_id(), f(5), &inputs).unwrap();
    let layer = plan
        .nodes
        .iter()
        .find_map(|n| match n {
            PlanNode::Layer(l) if l.clip == video => Some(l.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        layer.color.unwrap().mask(1).unwrap().mask.center,
        MaskPoint::new(0, 0)
    );
    // A switch to an ellipse keeps the centre's keys and drops roundness's.
    let s = stack(&fx, video);
    let switched = ColorMask {
        mask: s.mask(1).unwrap().mask.with_kind(MaskShapeKind::Ellipse),
        ..*s.mask(1).unwrap()
    };
    color(&mut fx, vec![video], ColorEdit::SetMask(switched)).unwrap();
    let clip = fx.doc().clip(video).unwrap().clone();
    assert!(clip.keys.is_animated(center) && clip.keys.is_animated(expand));
    assert!(!clip.keys.is_animated(round));
    assert_eq!(round.get(&clip), None, "an ellipse has no roundness");
    // Roundness keys can't be added to an ellipse.
    assert!(
        fx.apply(EditRequest::EditKeys {
            seq: fx.seq_id(),
            clips: vec![video],
            prop: round,
            edit: KeyEdit::Add {
                at: f(0),
                value: Some(PropValue::Ratio(PerMille::new(10))),
            },
        })
        .is_err()
    );
    fx.editor.undo().unwrap();
    assert!(fx.doc().clip(video).unwrap().keys.is_animated(round));
}

#[test]
fn reset_all_and_the_masks_group_cover_mask_keys() {
    let mut fx = Fixture::new();
    let (video, ..) = kinds(&mut fx);
    color(
        &mut fx,
        vec![video],
        ColorEdit::SetParam(ColorParamId::Exposure, 40),
    )
    .unwrap();
    color(
        &mut fx,
        vec![video],
        ColorEdit::AddMask(Mask::new(MaskShapeKind::Band)),
    )
    .unwrap();
    let width = mask_prop(1, MaskParam::BandWidth);
    mask_key(
        &mut fx,
        video,
        width,
        0,
        PropValue::Ratio(PerMille::new(300)),
    );
    let clip = fx.doc().clip(video).unwrap().clone();
    let group = PropGroup::Color(Section::Masks);
    assert_eq!(width.group(), Some(group));
    let props = group.props(&clip);
    assert!(props.contains(&width) && props.contains(&mask_prop(1, MaskParam::Center)));
    assert!(!props.contains(&mask_prop(1, MaskParam::Roundness)));
    // Key this card on the Masks section keys every value of every mask.
    fx.apply(EditRequest::EditKeys {
        seq: fx.seq_id(),
        clips: vec![video],
        prop: width,
        edit: KeyEdit::KeyCard {
            card: group,
            at: f(4),
        },
    })
    .unwrap();
    assert!(
        fx.doc()
            .clip(video)
            .unwrap()
            .keys
            .is_animated(mask_prop(1, MaskParam::Rotation))
    );
    color(&mut fx, vec![video], ColorEdit::ResetAll).unwrap();
    let clip = fx.doc().clip(video).unwrap().clone();
    assert!(clip.keys.is_empty() && clip.color.is_none());
}

#[test]
fn k6_paths_round_trip_their_names() {
    for (p, name) in [
        (mask_prop(1, MaskParam::Center), "color.masks.1.center"),
        (
            mask_prop(12, MaskParam::BandWidth),
            "color.masks.12.band_width",
        ),
        (
            PropPath::ColorMask {
                id: 3,
                param: ColorMaskParam::Expand,
            },
            "color.masks.3.expand",
        ),
    ] {
        assert_eq!(p.to_path_string(), name);
        assert_eq!(PropPath::parse(name), Some(p));
        assert!(p.is_animatable());
    }
    for bad in [
        "color.masks.0.center",
        "color.masks.01.center",
        "color.masks.x.center",
        "color.masks.1.shape",
        "color.masks.1",
    ] {
        assert_eq!(PropPath::parse(bad), None, "{bad}");
    }
}

#[test]
fn validate_refuses_bad_mask_lists() {
    let e = ColorMask::new(1, ellipse());
    let graded = ColorStack::default().with(ColorParamId::Exposure, 10);
    let twins = ColorStack {
        masks: vec![e, e],
        ..graded.clone()
    };
    assert!(!twins.is_valid());
    let wide = ColorStack {
        masks: vec![ColorMask { expand: 500, ..e }],
        ..graded.clone()
    };
    assert!(!wide.is_valid());
    let c = wide.clamped();
    assert!(c.is_valid());
    assert_eq!(c.masks[0].expand, 100);
    let many = ColorStack {
        masks: (1..=9).map(|id| ColorMask::new(id, ellipse())).collect(),
        ..graded
    };
    assert!(!many.is_valid());
}
