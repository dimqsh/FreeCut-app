//! Cut away before / after the playhead (TL-026, toolbar slice T1). Every
//! edit goes through `Fixture::apply`, which checks apply → undo → redo.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    Cut, EditError, EditRequest, InsertTarget, Notice, OpKind, TrimTargets, Unavailable,
    sync_offsets, trim_to_playhead_available,
};
use freecut_timeline::ids::{ClipId, TrackId};
use freecut_timeline::model::{TrackFlags, TrackHeader};
use pretty_assertions::assert_eq;

fn cut(targets: TrimTargets, at: i64, cut: Cut, ripple_others: bool, fx: &Fixture) -> EditRequest {
    EditRequest::TrimToPlayhead {
        seq: fx.seq_id(),
        targets,
        at: f(at),
        cut,
        ripple_others,
    }
}

fn notices(c: Committed) -> Vec<Notice> {
    match c {
        Committed::Applied { notices, .. } => notices,
        Committed::NoOp => panic!("expected a change"),
    }
}

fn lock(fx: &mut Fixture, track: TrackId) {
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetTrackHeader {
        seq,
        track,
        header: TrackHeader {
            name: None,
            flags: TrackFlags {
                locked: true,
                ..TrackFlags::default()
            },
        },
    })
    .unwrap();
}

/// Main track: A 0+10 (source 0) and B 10+10 (source 20), magnet on.
fn two_on_main(fx: &mut Fixture) -> (ClipId, ClipId) {
    let main = fx.main();
    let ids = fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 0, 10), (VIDEO_100, 20, 10)],
    );
    (ids[0], ids[1])
}

#[test]
fn q_on_the_magnetic_main_track_keeps_the_start_ripples_and_moves_the_playhead() {
    let mut fx = Fixture::new();
    let (a, b) = two_on_main(&mut fx);
    let req = cut(TrimTargets::MainUnderPlayhead, 4, Cut::Head, false, &fx);
    let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
    assert_eq!(plan.label().op, OpKind::TrimToPlayhead(Cut::Head));
    let n = notices(fx.apply(req).unwrap());
    assert_eq!(
        fx.render(),
        format!("V0*: {}@0+6[4] {}@6+10[20]", a.get(), b.get())
    );
    assert_eq!(n, vec![Notice::PlayheadTo(f(0))]);
}

#[test]
fn w_on_the_magnetic_main_track_ripples_without_moving_the_playhead() {
    let mut fx = Fixture::new();
    let (a, b) = two_on_main(&mut fx);
    let n = notices(
        fx.apply(cut(TrimTargets::Clips(vec![b]), 14, Cut::Tail, false, &fx))
            .unwrap(),
    );
    assert_eq!(
        fx.render(),
        format!("V0*: {}@0+10[0] {}@10+4[20]", a.get(), b.get())
    );
    assert!(n.is_empty(), "{n:?}");
}

/// Overlay track V1: O 2+10 (source 0) and P 14+5 (source 30).
fn overlay(fx: &mut Fixture) -> (ClipId, ClipId) {
    let ids = fx.insert(InsertTarget::NewTrack, 2, &[(VIDEO_100, 0, 10)]);
    let track = fx.doc().find_clip(ids[0]).unwrap().track;
    let more = fx.insert(InsertTarget::Track(track), 14, &[(VIDEO_100, 30, 5)]);
    (ids[0], more[0])
}

#[test]
fn on_an_overlay_the_edge_moves_to_the_playhead_and_nothing_else_moves() {
    let mut fx = Fixture::new();
    let (o, p) = overlay(&mut fx);
    let n = notices(
        fx.apply(cut(TrimTargets::Clips(vec![o]), 5, Cut::Head, false, &fx))
            .unwrap(),
    );
    assert_eq!(
        fx.render(),
        format!("V1: {}@5+7[3] {}@14+5[30]\nV0*:", o.get(), p.get())
    );
    assert!(n.is_empty(), "no playhead move off the main track: {n:?}");
}

#[test]
fn with_shift_an_overlay_cut_ripples_its_track() {
    let mut fx = Fixture::new();
    let (o, p) = overlay(&mut fx);
    fx.apply(cut(TrimTargets::Clips(vec![o]), 5, Cut::Head, true, &fx))
        .unwrap();
    assert_eq!(
        fx.render(),
        format!("V1: {}@2+7[3] {}@11+5[30]\nV0*:", o.get(), p.get())
    );
    fx.editor.undo().unwrap();
    fx.apply(cut(TrimTargets::Clips(vec![o]), 5, Cut::Tail, true, &fx))
        .unwrap();
    assert_eq!(
        fx.render(),
        format!("V1: {}@2+3[0] {}@7+5[30]\nV0*:", o.get(), p.get())
    );
}

#[test]
fn a_playhead_on_the_first_frame_at_the_end_or_outside_is_skipped_with_a_reason() {
    let mut fx = Fixture::new();
    let (a, b) = two_on_main(&mut fx);
    let seq = fx.seq_id();
    for at in [0, 10, 20, 35] {
        assert_eq!(
            fx.apply(cut(TrimTargets::Clips(vec![a]), at, Cut::Head, false, &fx)),
            Err(EditError::NothingToTrim),
            "at {at}"
        );
        assert_eq!(
            trim_to_playhead_available(fx.doc(), seq, &TrimTargets::Clips(vec![a]), f(at)),
            Err(Unavailable::PlayheadOutside)
        );
    }
    // Past the end there is no main-track clip under the playhead at all.
    assert_eq!(
        trim_to_playhead_available(fx.doc(), seq, &TrimTargets::MainUnderPlayhead, f(35)),
        Err(Unavailable::NoTarget)
    );
    // Several targets: the ones the playhead misses are skipped and named.
    let n = notices(
        fx.apply(cut(
            TrimTargets::Clips(vec![a, b]),
            15,
            Cut::Tail,
            false,
            &fx,
        ))
        .unwrap(),
    );
    assert_eq!(
        n,
        vec![Notice::Skipped(vec![(a, Unavailable::PlayheadOutside)])]
    );
    assert_eq!(
        fx.render(),
        format!("V0*: {}@0+10[0] {}@10+5[20]", a.get(), b.get())
    );
    assert_eq!(
        trim_to_playhead_available(fx.doc(), seq, &TrimTargets::Clips(vec![a, b]), f(12)),
        Ok(())
    );
}

#[test]
fn several_targets_on_different_tracks_cut_in_one_entry() {
    let mut fx = Fixture::new();
    let (a, b) = two_on_main(&mut fx);
    let (o, _) = overlay(&mut fx);
    let c = fx
        .apply(cut(
            TrimTargets::Clips(vec![a, b, o]),
            6,
            Cut::Head,
            false,
            &fx,
        ))
        .unwrap();
    assert!(matches!(c, Committed::Applied { .. }));
    let plan_label = fx.editor.undo().unwrap().label;
    assert_eq!(plan_label.op, OpKind::TrimToPlayhead(Cut::Head));
    assert_eq!(plan_label.count, 2, "a and o; b doesn't hold the playhead");
    fx.editor.redo().unwrap();
    let text = fx.render();
    assert!(text.contains(&format!("{}@6+6[4]", o.get())), "{text}");
    assert!(text.contains(&format!("{}@0+4[6]", a.get())), "{text}");
}

#[test]
fn locked_targets_are_skipped_and_a_locked_only_request_is_refused() {
    let mut fx = Fixture::new();
    let (o, _) = overlay(&mut fx);
    let track = fx.doc().find_clip(o).unwrap().track;
    lock(&mut fx, track);
    let seq = fx.seq_id();
    assert_eq!(
        trim_to_playhead_available(fx.doc(), seq, &TrimTargets::Clips(vec![o]), f(5)),
        Err(Unavailable::LockedTrack)
    );
    assert_eq!(
        fx.apply(cut(TrimTargets::Clips(vec![o]), 5, Cut::Head, false, &fx)),
        Err(EditError::NothingToTrim)
    );
}

#[test]
fn a_linked_sound_of_a_rippled_main_clip_stays_in_sync() {
    let mut fx = Fixture::new();
    let (a, _) = two_on_main(&mut fx);
    let seq = fx.seq_id();
    let sound = match fx
        .apply(EditRequest::DetachAudio {
            seq,
            clips: vec![a],
        })
        .unwrap()
    {
        Committed::Applied { selection, .. } => selection.unwrap()[0],
        Committed::NoOp => panic!(),
    };
    fx.apply(cut(
        TrimTargets::Clips(vec![a, sound]),
        4,
        Cut::Head,
        false,
        &fx,
    ))
    .unwrap();
    assert!(sync_offsets(fx.doc(), seq).is_empty(), "{}", fx.render());
    let s = fx.doc().clip(sound).unwrap();
    assert_eq!((s.start(), s.duration()), (f(0), f(6)));
}

#[test]
fn the_cut_snaps_to_the_frame_grid() {
    let mut fx = Fixture::new();
    let (a, _) = two_on_main(&mut fx);
    let req = EditRequest::TrimToPlayhead {
        seq: fx.seq_id(),
        targets: TrimTargets::Clips(vec![a]),
        at: freecut_timeline::time::Tick::from_raw(f(4).raw() + f(1).raw() / 3),
        cut: Cut::Tail,
        ripple_others: false,
    };
    fx.apply(req).unwrap();
    assert_eq!(fx.doc().clip(a).unwrap().duration(), f(4));
}
