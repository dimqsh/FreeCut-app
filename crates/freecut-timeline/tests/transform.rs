//! Clip transform (TR-001, TR-003, TR-015): the `SetTransform` request and
//! how other edits carry the transform. Every edit goes through
//! `Fixture::apply`, which checks plan purity, validity and the undo/redo
//! round trip.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    EditError, EditRequest, InsertReq, InsertTarget, MoveReq, NewClip, OpKind, SplitReq,
    SplitTargets, TransformEdit, TrimReq,
};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    Angle, CanvasPos, GeneratedKind, PerMille, Scale, TextClipData, TrackFlags, TrackHeader,
    Transform,
};
use pretty_assertions::assert_eq;

fn set(fx: &mut Fixture, clips: Vec<ClipId>, edit: TransformEdit) -> Result<Committed, EditError> {
    fx.apply(EditRequest::SetTransform {
        seq: fx.seq_id(),
        clips,
        edit,
    })
}

fn tf(fx: &Fixture, clip: ClipId) -> Transform {
    fx.doc().clip(clip).unwrap().transform
}

fn moved() -> Transform {
    Transform {
        position: CanvasPos::new(-120_000, 330_000),
        scale: Scale::from_basis_points(12_345),
        rotation: Angle::from_hundredths(-4_550),
        flip_h: true,
        flip_v: true,
        opacity: PerMille::new(600),
    }
}

fn apply_all(fx: &mut Fixture, clips: Vec<ClipId>, t: Transform) {
    for edit in [
        TransformEdit::Position(t.position),
        TransformEdit::Scale(t.scale),
        TransformEdit::Rotation(t.rotation),
        TransformEdit::FlipH(t.flip_h),
        TransformEdit::FlipV(t.flip_v),
        TransformEdit::Opacity(t.opacity),
    ] {
        set(fx, clips.clone(), edit).unwrap();
        assert_eq!(
            fx.editor.history().undo_label().unwrap().op,
            OpKind::SetTransform
        );
    }
}

#[test]
fn every_transform_value_can_be_set_on_every_visual_kind() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let video = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    let mut others = Vec::new();
    for new in [
        NewClip::text(TextClipData::default(), f(10)),
        NewClip::generated(GeneratedKind::Sticker, f(10)),
    ] {
        let Committed::Applied { selection, .. } = fx
            .apply(EditRequest::Insert(InsertReq {
                seq: fx.seq_id(),
                target: InsertTarget::FirstFree,
                at: f(0),
                clips: vec![new],
            }))
            .unwrap()
        else {
            panic!("no-op");
        };
        others.extend(selection.unwrap());
    }
    let mut all = vec![video];
    all.extend(&others);
    apply_all(&mut fx, all.clone(), moved());
    for clip in all {
        assert_eq!(tf(&fx, clip), moved());
    }
}

#[test]
fn values_are_clamped_to_their_ranges() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let v = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    set(
        &mut fx,
        vec![v],
        TransformEdit::Scale(Scale::from_basis_points(-10)),
    )
    .unwrap();
    assert_eq!(tf(&fx, v).scale, Scale::from_percent(1));
    set(
        &mut fx,
        vec![v],
        TransformEdit::Scale(Scale::from_percent(5000)),
    )
    .unwrap();
    assert_eq!(tf(&fx, v).scale, Scale::from_percent(1000));
    set(
        &mut fx,
        vec![v],
        TransformEdit::Rotation(Angle::from_degrees(370)),
    )
    .unwrap();
    assert_eq!(
        tf(&fx, v).rotation,
        Angle::from_degrees(370),
        "never wrapped"
    );
    set(
        &mut fx,
        vec![v],
        TransformEdit::Rotation(Angle::from_degrees(-9000)),
    )
    .unwrap();
    assert_eq!(tf(&fx, v).rotation, Angle::from_degrees(-3600));
    set(
        &mut fx,
        vec![v],
        TransformEdit::Position(CanvasPos::new(i32::MIN, 7_000_000)),
    )
    .unwrap();
    assert_eq!(tf(&fx, v).position, CanvasPos::new(-4_000_000, 4_000_000));
    set(
        &mut fx,
        vec![v],
        TransformEdit::Opacity(PerMille::new(5000)),
    )
    .unwrap();
    assert_eq!(tf(&fx, v).opacity, PerMille::ONE);
}

#[test]
fn setting_the_current_value_records_nothing() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let v = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    assert_eq!(
        set(&mut fx, vec![v], TransformEdit::Scale(Scale::ONE)).unwrap(),
        Committed::NoOp
    );
    assert_eq!(
        set(&mut fx, vec![v], TransformEdit::Reset).unwrap(),
        Committed::NoOp
    );
    // A clamped value equal to the current one is a no-op too.
    set(&mut fx, vec![v], TransformEdit::Scale(Scale::MAX)).unwrap();
    assert_eq!(
        set(
            &mut fx,
            vec![v],
            TransformEdit::Scale(Scale::from_percent(2000))
        )
        .unwrap(),
        Committed::NoOp
    );
}

#[test]
fn reset_restores_the_transform_section_but_keeps_opacity() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let v = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    apply_all(&mut fx, vec![v], moved());
    set(&mut fx, vec![v], TransformEdit::Reset).unwrap();
    assert_eq!(
        tf(&fx, v),
        Transform {
            opacity: PerMille::new(600),
            ..Transform::IDENTITY
        }
    );
}

#[test]
fn audio_clips_are_skipped_and_audio_only_selections_refused() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let v = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
    let a = fx.insert(InsertTarget::FirstFree, 0, &[(AUDIO_200, 0, 10)])[0];
    set(&mut fx, vec![v, a], TransformEdit::FlipH(true)).unwrap();
    assert!(tf(&fx, v).flip_h);
    assert_eq!(tf(&fx, a), Transform::IDENTITY);
    assert_eq!(fx.editor.history().undo_label().unwrap().count, 1);
    assert_eq!(
        set(&mut fx, vec![a], TransformEdit::FlipH(true)).unwrap_err(),
        EditError::NoVisualClip
    );
    assert_eq!(
        set(&mut fx, vec![], TransformEdit::FlipH(true)).unwrap_err(),
        EditError::NothingSelected
    );
}

#[test]
fn clips_on_locked_tracks_refuse_the_edit() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let v = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[0];
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
        set(&mut fx, vec![v], TransformEdit::FlipV(true)).unwrap_err(),
        EditError::MainTrackLocked
    );
}

#[test]
fn split_trim_move_and_detach_keep_the_transform() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let v = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_100, 0, 20)])[0];
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 40)]);
    apply_all(&mut fx, vec![v], moved());

    let Committed::Applied { selection, .. } = fx
        .apply(EditRequest::Split(SplitReq {
            seq: fx.seq_id(),
            at: f(8),
            targets: SplitTargets::Clips(vec![v]),
        }))
        .unwrap()
    else {
        panic!("no-op");
    };
    let right = selection.unwrap()[0];
    assert_eq!((tf(&fx, v), tf(&fx, right)), (moved(), moved()));

    fx.apply(EditRequest::Trim(TrimReq {
        seq: fx.seq_id(),
        clip: right,
        edge: freecut_timeline::edit::Edge::Out,
        delta: f(-3),
        ripple: false,
    }))
    .unwrap();
    assert_eq!(tf(&fx, right), moved());

    fx.apply(EditRequest::Move(MoveReq {
        seq: fx.seq_id(),
        clips: vec![right],
        delta: f(30),
        track_shift: 1,
    }))
    .unwrap();
    assert_eq!(tf(&fx, right), moved());

    let Committed::Applied { selection, .. } = fx
        .apply(EditRequest::DetachAudio {
            seq: fx.seq_id(),
            clips: vec![v],
        })
        .unwrap()
    else {
        panic!("no-op");
    };
    assert_eq!(tf(&fx, v), moved());
    assert_eq!(tf(&fx, selection.unwrap()[0]), Transform::IDENTITY);
}
