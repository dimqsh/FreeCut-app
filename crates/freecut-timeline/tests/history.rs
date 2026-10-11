//! Undo history semantics (TL-043).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_timeline::edit::{Edge, EditRequest, InsertTarget, MoveReq, OpKind, TrimReq};
use freecut_timeline::history::{Coalesce, CoalesceKey, HistoryLimits};
use freecut_timeline::{CommitError, CommitMeta, Committed, HistoryError};
use pretty_assertions::assert_eq;

fn nudge(fx: &Fixture, clip: freecut_timeline::ids::ClipId, frames: i64) -> EditRequest {
    EditRequest::Move(MoveReq {
        seq: fx.seq_id(),
        clips: vec![clip],
        delta: f(frames),
        track_shift: 0,
    })
}

fn commit_with(fx: &mut Fixture, req: &EditRequest, coalesce: Coalesce) -> Committed {
    let plan = fx.editor.plan(&fx.ctx(), req).unwrap();
    fx.editor
        .commit(
            plan,
            CommitMeta {
                selection_before: Vec::new(),
                coalesce,
            },
        )
        .unwrap()
}

#[test]
fn dirty_tracks_the_saved_position() {
    let mut fx = Fixture::new();
    assert!(!fx.editor.is_dirty(), "a new document has nothing to save");
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
    assert!(fx.editor.is_dirty());
    fx.editor.mark_saved();
    assert!(!fx.editor.is_dirty());
    fx.editor.undo().unwrap();
    assert!(fx.editor.is_dirty());
    fx.editor.redo().unwrap();
    assert!(!fx.editor.is_dirty(), "back at the saved state");
}

#[test]
fn a_new_edit_after_undo_makes_the_old_save_point_unreachable() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
    fx.editor.mark_saved();
    fx.editor.undo().unwrap();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 5)]);
    fx.editor.undo().unwrap();
    assert!(fx.editor.is_dirty());
    assert!(!fx.editor.history().can_redo() || fx.editor.is_dirty());
}

#[test]
fn a_gesture_is_one_entry_and_can_be_cancelled() {
    let mut fx = Fixture::new();
    let ids = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
    let before = fx.doc().clone();
    let entries = fx.editor.history().len();

    assert!(!fx.editor.gesture_open());
    let g = fx.editor.begin_gesture();
    assert!(fx.editor.gesture_open());
    for _ in 0..3 {
        let req = EditRequest::Trim(TrimReq {
            seq: fx.seq_id(),
            clip: ids[0],
            edge: Edge::Out,
            delta: f(2),
            ripple: false,
        });
        commit_with(&mut fx, &req, Coalesce::Gesture(g));
    }
    assert_eq!(fx.editor.undo().unwrap_err(), HistoryError::GestureOpen);
    fx.editor.end_gesture(g);
    assert!(!fx.editor.gesture_open());
    assert_eq!(fx.editor.history().len(), entries + 1);
    assert!(fx.render().starts_with("V1: 6@0+16"));
    fx.editor.undo().unwrap();
    assert_eq!(fx.doc(), &before);

    fx.editor.redo().unwrap();
    let g = fx.editor.begin_gesture();
    let req = nudge(&fx, ids[0], 4);
    commit_with(&mut fx, &req, Coalesce::Gesture(g));
    fx.editor.cancel_gesture(g).unwrap();
    assert!(
        fx.render().starts_with("V1: 6@0+16"),
        "cancel reverts the gesture"
    );
    assert_eq!(
        fx.editor.history().len(),
        entries + 1,
        "and leaves no entry"
    );
}

#[test]
fn repeats_merge_and_cancelling_repeats_leave_no_entry() {
    let mut fx = Fixture::new();
    let ids = fx.insert(InsertTarget::NewTrack, 10, &[(VIDEO_50, 0, 5)]);
    let entries = fx.editor.history().len();
    let key = CoalesceKey {
        op: OpKind::MoveClips,
        target: ids[0].get(),
    };
    let req = nudge(&fx, ids[0], 1);
    commit_with(&mut fx, &req, Coalesce::Repeat { key, now_ms: 1_000 });
    commit_with(&mut fx, &req, Coalesce::Repeat { key, now_ms: 1_400 });
    assert_eq!(
        fx.editor.history().len(),
        entries + 1,
        "two nudges, one entry"
    );
    let back = nudge(&fx, ids[0], -1);
    commit_with(&mut fx, &back, Coalesce::Repeat { key, now_ms: 1_800 });
    commit_with(&mut fx, &back, Coalesce::Repeat { key, now_ms: 2_200 });
    assert_eq!(fx.editor.history().len(), entries, "they cancelled out");
    assert!(fx.render().starts_with("V1: 6@10+5"));

    // Outside the 1 s window a repeat starts a new entry.
    commit_with(
        &mut fx,
        &req,
        Coalesce::Repeat {
            key,
            now_ms: 10_000,
        },
    );
    commit_with(
        &mut fx,
        &req,
        Coalesce::Repeat {
            key,
            now_ms: 20_000,
        },
    );
    assert_eq!(fx.editor.history().len(), entries + 2);
}

#[test]
fn a_plan_made_before_another_commit_is_stale() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let ids = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
    let old = fx.editor.plan(&fx.ctx(), &nudge(&fx, ids[0], 1)).unwrap();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
    assert_eq!(
        fx.editor.commit(old, CommitMeta::default()).unwrap_err(),
        CommitError::Stale
    );
}

#[test]
fn ids_are_never_reused_after_undo() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let first = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
    fx.editor.undo().unwrap();
    let second = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
    assert!(second[0] > first[0]);
}

#[test]
fn evicted_entries_move_the_base_state() {
    let mut fx = Fixture::with_limits(HistoryLimits {
        max_entries: 2,
        ..HistoryLimits::default()
    });
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 1)]);
    fx.editor.mark_saved();
    for _ in 0..3 {
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 1)]);
    }
    assert_eq!(fx.editor.history().len(), 2);
    fx.editor.undo().unwrap();
    fx.editor.undo().unwrap();
    assert_eq!(fx.editor.undo().unwrap_err(), HistoryError::Empty);
    assert!(fx.editor.is_dirty(), "the saved state was evicted");
}
