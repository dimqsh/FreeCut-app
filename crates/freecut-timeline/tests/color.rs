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
    Clip, ClipContent, ColorParamId, ColorStack, Curve, CurveChannel, CurvePoint, Document,
    GeneratedKind, HslChannel, HueBand, Interp, InvariantViolation, PropGroup, PropPath, PropValue,
    Section, Sequence, TextClipData, Track, TrackFlags, TrackHeader, ValidateCtx, WheelId,
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
    let with = |id: ClipId, stack: ColorStack| {
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
    };
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
            .map(|PlanNode::Layer(l)| l)
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
    assert_eq!(
        plan_color(&fx, adjust, 2),
        None,
        "adjustment clips grade with their own node (K4)"
    );
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
    let PlanNode::Layer(slate) = plan
        .nodes
        .iter()
        .find(|n| matches!(n, PlanNode::Layer(l) if l.clip == video))
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
    key(&mut fx, video, ColorParamId::WheelLuma(WheelId::Offset), 0, 5);
    color(&mut fx, vec![video], ColorEdit::ResetBand(HueBand::Blue)).unwrap();
    color(&mut fx, vec![video], ColorEdit::ResetWheel(WheelId::Highlights)).unwrap();
    let c = fx.doc().clip(video).unwrap().clone();
    assert!(!c.keys.is_animated(PropPath::Color(hsl(HueBand::Blue, HslChannel::Hue))));
    assert!(c.keys.is_animated(PropPath::Color(hsl(HueBand::Cyan, HslChannel::Hue))));
    assert!(!c.keys.is_animated(PropPath::ColorPuck(WheelId::Highlights)));
    assert!(!c.keys.is_animated(PropPath::Color(ColorParamId::WheelLuma(
        WheelId::Highlights
    ))));
    assert!(c.keys.is_animated(PropPath::Color(ColorParamId::WheelLuma(WheelId::Offset))));
    // The Wheels section reset takes the pucks' keys too; Reset all the rest.
    puck_key(&mut fx, video, WheelId::Shadows, 3, -200, 0);
    color(&mut fx, vec![video], ColorEdit::ResetSection(Section::Wheels)).unwrap();
    let c = fx.doc().clip(video).unwrap().clone();
    assert!(!c.keys.is_animated(PropPath::ColorPuck(WheelId::Shadows)));
    assert!(!c.keys.is_animated(PropPath::Color(ColorParamId::WheelLuma(WheelId::Offset))));
    assert!(c.keys.is_animated(PropPath::Color(hsl(HueBand::Cyan, HslChannel::Hue))));
    puck_key(&mut fx, video, WheelId::Offset, 3, 50, 0);
    assert!(freecut_timeline::edit::has_color(fx.doc().clip(video).unwrap()));
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
    assert!(!PropGroup::Color(Section::Curves).props(&c).iter().any(|p| p.is_animatable()));
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
            .map(|PlanNode::Layer(l)| l)
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
    assert_eq!(plan_color(&fx, 1), None, "a switched-off curve does nothing");
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
