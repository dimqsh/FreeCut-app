//! Follow main track (TL-029, toolbar slice T4): attached items keep their
//! anchor frame through every op that moves main-track clips. Every edit
//! goes through `Fixture::apply`, which checks apply → undo → redo.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    Cut, DeleteReq, Edge, EditRequest, InsertReq, InsertTarget, MoveReq, NewClip, Notice, SplitReq,
    SplitTargets, TrimReq, TrimTargets,
};
use freecut_timeline::ids::{ClipId, TrackId};
use freecut_timeline::model::{
    Attachment, FollowKind, FollowKinds, GeneratedKind, TextClipData, TrackFlags, TrackHeader,
    attach_all, expected_start,
};
use freecut_timeline::time::Tick;
use pretty_assertions::assert_eq;

/// Main: A 0+10 (source 0), B 10+10 (source 40), C 20+10 (source 70).
fn main_abc(fx: &mut Fixture) -> [ClipId; 3] {
    let main = fx.main();
    let ids = fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 0, 10), (VIDEO_100, 40, 10), (VIDEO_100, 70, 10)],
    );
    [ids[0], ids[1], ids[2]]
}

fn add_text(fx: &mut Fixture, at: i64, len: i64) -> ClipId {
    add(
        fx,
        at,
        NewClip::text(TextClipData::with_content("Title"), f(len)),
    )
}

fn add_sticker(fx: &mut Fixture, at: i64, len: i64) -> ClipId {
    add(fx, at, NewClip::generated(GeneratedKind::Sticker, f(len)))
}

fn add(fx: &mut Fixture, at: i64, clip: NewClip) -> ClipId {
    let seq = fx.seq_id();
    match fx
        .apply(EditRequest::Insert(InsertReq {
            seq,
            target: InsertTarget::FirstFree,
            at: f(at),
            clips: vec![clip],
        }))
        .unwrap()
    {
        Committed::Applied { selection, .. } => selection.unwrap()[0],
        Committed::NoOp => panic!(),
    }
}

fn start(fx: &Fixture, id: ClipId) -> Tick {
    fx.doc().clip(id).unwrap().start()
}

fn attach(fx: &Fixture, id: ClipId) -> Option<Attachment> {
    fx.doc().clip(id).unwrap().attach
}

fn in_sync(fx: &Fixture, id: ClipId) -> bool {
    let a = attach(fx, id).expect("attached");
    expected_start(fx.seq(), a) == Some(start(fx, id))
}

fn track_of(fx: &Fixture, id: ClipId) -> TrackId {
    fx.doc().find_clip(id).unwrap().track
}

fn lock(fx: &mut Fixture, track: TrackId, locked: bool) {
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetTrackHeader {
        seq,
        track,
        header: TrackHeader {
            name: None,
            flags: TrackFlags {
                locked,
                ..TrackFlags::default()
            },
        },
    })
    .unwrap();
}

fn mv(fx: &mut Fixture, clips: Vec<ClipId>, frames: i64) {
    let seq = fx.seq_id();
    fx.apply(EditRequest::Move(MoveReq {
        seq,
        clips,
        delta: f(frames),
        track_shift: 0,
    }))
    .unwrap();
}

#[test]
fn placing_an_item_attaches_it_to_the_main_clip_under_its_start() {
    let mut fx = Fixture::new();
    let [_, b, _] = main_abc(&mut fx);
    let t = add_text(&mut fx, 13, 4);
    assert_eq!(
        attach(&fx, t),
        Some(Attachment {
            owner: b,
            owner_source_time: f(43)
        })
    );
    let past = add_sticker(&mut fx, 35, 4);
    assert_eq!(attach(&fx, past), None, "nothing under its start");
    // Moving it re-anchors it.
    mv(&mut fx, vec![t], 10);
    assert_eq!(attach(&fx, t).unwrap().owner_source_time, f(73));
}

#[test]
fn a_reorder_carries_attached_items_with_their_owner() {
    let mut fx = Fixture::new();
    let [a, b, c] = main_abc(&mut fx);
    let t = add_text(&mut fx, 13, 4);
    let s = add_sticker(&mut fx, 22, 3);
    mv(&mut fx, vec![b], 15); // b goes after c
    assert_eq!(
        (start(&fx, a), start(&fx, c), start(&fx, b)),
        (f(0), f(10), f(20))
    );
    assert_eq!(start(&fx, t), f(23));
    assert_eq!(start(&fx, s), f(12));
    assert!(in_sync(&fx, t) && in_sync(&fx, s));
}

#[test]
fn ripple_trims_and_cut_aways_keep_the_anchor_frame() {
    let mut fx = Fixture::new();
    let [a, b, _] = main_abc(&mut fx);
    let t = add_text(&mut fx, 13, 4);
    let early = add_sticker(&mut fx, 2, 3);
    let seq = fx.seq_id();
    // Ripple head trim of b by 2: its content moves 2 left, so does t.
    fx.apply(EditRequest::Trim(TrimReq {
        seq,
        clip: b,
        edge: Edge::In,
        delta: f(2),
        ripple: false,
    }))
    .unwrap();
    assert_eq!(start(&fx, t), f(11));
    assert!(in_sync(&fx, t));
    // Cut away the first 4 frames of a: everything after shifts left by 4,
    // and the sticker whose frame was cut away goes to a's first frame.
    fx.apply(EditRequest::TrimToPlayhead {
        seq,
        targets: TrimTargets::Clips(vec![a]),
        at: f(4),
        cut: Cut::Head,
        ripple_others: false,
    })
    .unwrap();
    assert_eq!(start(&fx, t), f(7));
    assert_eq!(start(&fx, early), f(0));
    assert_eq!(attach(&fx, early).unwrap().owner_source_time, f(4));
    assert!(in_sync(&fx, t) && in_sync(&fx, early));
}

#[test]
fn deleting_an_owner_deletes_its_items_with_a_notice_and_undo_restores_them() {
    let mut fx = Fixture::new();
    let [_, b, c] = main_abc(&mut fx);
    let t = add_text(&mut fx, 13, 4);
    let s = add_sticker(&mut fx, 24, 3);
    let seq = fx.seq_id();
    let before = fx.doc().clone();
    let committed = fx
        .apply(EditRequest::Delete(DeleteReq {
            seq,
            clips: vec![b],
            ripple: false,
        }))
        .unwrap();
    let Committed::Applied { notices, .. } = committed else {
        panic!()
    };
    assert!(
        notices.contains(&Notice::AlsoRemovedAttached(1)),
        "{notices:?}"
    );
    assert!(fx.doc().clip(t).is_none());
    assert_eq!((start(&fx, c), start(&fx, s)), (f(10), f(14)));
    fx.editor.undo().unwrap();
    assert_eq!(fx.doc(), &before);
}

#[test]
fn split_owners_hand_items_to_the_piece_that_shows_their_frame() {
    let mut fx = Fixture::new();
    let [_, b, _] = main_abc(&mut fx);
    let left = add_text(&mut fx, 11, 2);
    let right = add_sticker(&mut fx, 16, 2);
    let seq = fx.seq_id();
    let c = fx
        .apply(EditRequest::Split(SplitReq {
            seq,
            at: f(15),
            targets: SplitTargets::Clips(vec![b]),
        }))
        .unwrap();
    let Committed::Applied { selection, .. } = c else {
        panic!()
    };
    let piece = selection.unwrap()[0];
    assert_eq!(attach(&fx, left).unwrap().owner, b);
    assert_eq!(
        attach(&fx, right),
        Some(Attachment {
            owner: piece,
            owner_source_time: f(46)
        })
    );
    assert_eq!((start(&fx, left), start(&fx, right)), (f(11), f(16)));
}

#[test]
fn magnet_compaction_carries_items_along() {
    let mut fx = Fixture::new();
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetMagnet { seq, on: false }).unwrap();
    let main = fx.main();
    let a = fx.insert(InsertTarget::Track(main), 5, &[(VIDEO_100, 0, 10)])[0];
    let t = add_text(&mut fx, 8, 4);
    fx.apply(EditRequest::SetMagnet { seq, on: true }).unwrap();
    assert_eq!((start(&fx, a), start(&fx, t)), (f(0), f(3)));
    assert!(in_sync(&fx, t));
}

#[test]
fn unchecked_kinds_stay_put_while_checked_ones_follow() {
    let mut fx = Fixture::new();
    let [_, b, _] = main_abc(&mut fx);
    let t = add_text(&mut fx, 13, 4);
    let s = add_sticker(&mut fx, 14, 3);
    let seq = fx.seq_id();
    let before = fx.doc().clone();
    let kinds = FollowKinds::ALL.with(FollowKind::Text, false);
    fx.apply(EditRequest::SetFollowKinds { seq, kinds })
        .unwrap();
    assert_eq!(attach(&fx, t), None, "unchecking clears");
    assert!(attach(&fx, s).is_some());
    assert_eq!(
        (start(&fx, t), start(&fx, s)),
        (f(13), f(14)),
        "nothing moves"
    );
    mv(&mut fx, vec![b], 15);
    assert_eq!((start(&fx, t), start(&fx, s)), (f(13), f(24)));
    // Undoing both steps restores the attachments exactly.
    fx.editor.undo().unwrap();
    fx.editor.undo().unwrap();
    assert_eq!(fx.doc(), &before);
}

#[test]
fn toggling_follow_moves_nothing_and_undo_restores_attachments() {
    let mut fx = Fixture::new();
    let [_, b, _] = main_abc(&mut fx);
    let t = add_text(&mut fx, 13, 4);
    let seq = fx.seq_id();
    let attached = attach(&fx, t);
    fx.apply(EditRequest::SetFollowMain { seq, on: false })
        .unwrap();
    assert_eq!(attach(&fx, t), None);
    assert!(!fx.seq().flags.follow_main);
    mv(&mut fx, vec![b], 15);
    assert_eq!(start(&fx, t), f(13), "stays fixed while off");
    fx.editor.undo().unwrap();
    fx.editor.undo().unwrap();
    assert_eq!(attach(&fx, t), attached);
    // Turning it on attaches by position.
    fx.apply(EditRequest::SetFollowMain { seq, on: false })
        .unwrap();
    mv(&mut fx, vec![b], 15);
    fx.apply(EditRequest::SetFollowMain { seq, on: true })
        .unwrap();
    assert_eq!(start(&fx, t), f(13));
    let c = fx.seq().main_track().unwrap().clips()[1].id;
    assert_eq!(attach(&fx, t).unwrap().owner, c);
    let all = attach_all(fx.seq(), fx.doc().library());
    assert!(all.contains(&(t, attach(&fx, t))));
}

#[test]
fn items_on_locked_tracks_stay_and_are_out_of_sync() {
    let mut fx = Fixture::new();
    let [_, b, _] = main_abc(&mut fx);
    let t = add_text(&mut fx, 13, 4);
    let track = track_of(&fx, t);
    lock(&mut fx, track, true);
    mv(&mut fx, vec![b], 15);
    assert_eq!(start(&fx, t), f(13));
    assert!(!in_sync(&fx, t), "flagged out of sync");
    assert_eq!(
        expected_start(fx.seq(), attach(&fx, t).unwrap()),
        Some(f(23))
    );
}

#[test]
fn collisions_with_fixed_items_move_the_follower_to_a_new_track() {
    let mut fx = Fixture::new();
    let [_, b, _] = main_abc(&mut fx);
    let t = add_text(&mut fx, 13, 4);
    let text_track = track_of(&fx, t);
    let seq = fx.seq_id();
    // A fixed (unattached) title past the end of the main track, where b
    // is about to go.
    let req = EditRequest::Insert(InsertReq {
        seq,
        target: InsertTarget::Track(text_track),
        at: f(32),
        clips: vec![NewClip::text(TextClipData::with_content("Fixed"), f(4))],
    });
    let fixed = match fx.apply(req).unwrap() {
        Committed::Applied { selection, .. } => selection.unwrap()[0],
        Committed::NoOp => panic!(),
    };
    assert_eq!(attach(&fx, fixed), None);
    let tracks = fx.seq().track_count();
    // Move b to the end of the main track and a 10-frame clip after it, so
    // b's frame 43 sits at 33, over the fixed title.
    let main = fx.main();
    let seq_len = fx.seq().main_track().unwrap().end();
    assert_eq!(seq_len, f(30));
    fx.insert(InsertTarget::Track(main), 30, &[(VIDEO_50, 0, 10)]);
    mv(&mut fx, vec![b], 25);
    assert_eq!(start(&fx, b), f(30));
    assert_eq!(start(&fx, t), f(33));
    assert_ne!(track_of(&fx, t), text_track, "moved to a new track");
    assert_eq!(fx.seq().track_count(), tracks + 1);
    assert_eq!(track_of(&fx, fixed), text_track);
    assert!(in_sync(&fx, t));
}
