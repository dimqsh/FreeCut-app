//! Regression tests from the adversarial review of M1. Each module pins
//! one confirmed finding.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

#[allow(unused_imports, dead_code)]
mod f00_magnet_toggle_refused_on_locked_main {
    //! set_magnet returns Err(MainTrackLocked) whenever the main track is locked, in both directions and whether or not compaction is needed

    use super::common::*;

    #[test]
    fn magnet_toggle_is_allowed_on_a_locked_main_track_when_nothing_moves() {
        use freecut_timeline::edit::{EditRequest, InsertTarget};
        use freecut_timeline::model::{TrackFlags, TrackHeader};
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
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
        fx.apply(EditRequest::SetMagnet {
            seq: fx.seq_id(),
            on: false,
        })
        .expect("magnet off must be allowed with a locked main track");
        fx.apply(EditRequest::SetMagnet {
            seq: fx.seq_id(),
            on: true,
        })
        .expect("magnet on must be allowed when the locked main track is contiguous");
        assert!(fx.seq().flags.main_magnet);
    }

    #[test]
    fn magnet_on_is_still_refused_when_a_locked_main_track_needs_compaction() {
        use freecut_timeline::edit::{EditError, EditRequest, InsertTarget};
        use freecut_timeline::model::{TrackFlags, TrackHeader};
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.apply(EditRequest::SetMagnet {
            seq: fx.seq_id(),
            on: false,
        })
        .unwrap();
        fx.insert(InsertTarget::Track(main), 3, &[(VIDEO_50, 0, 5)]);
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
            fx.apply(EditRequest::SetMagnet {
                seq: fx.seq_id(),
                on: true
            })
            .unwrap_err(),
            EditError::MainTrackLocked
        );
        assert!(!fx.seq().flags.main_magnet);
    }
}

#[allow(unused_imports, dead_code)]
mod f01_redirect_ignores_free_compatible_tracks {
    //! When an item's kind does not match the target track, resolve_target/nearest_of_kind picks the nearest track of that kind without checking whether it is free at the span

    use super::common::*;

    #[test]
    fn audio_added_via_the_main_track_uses_the_first_audio_track_free_at_that_span() {
        use freecut_timeline::edit::InsertTarget;
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.insert(InsertTarget::Track(main), 0, &[(AUDIO_200, 0, 20)]);
        fx.insert(InsertTarget::NewTrack, 100, &[(AUDIO_200, 0, 20)]);
        let a1 = fx.seq().audio_tracks()[1].id;
        let new = fx.insert(InsertTarget::Track(main), 0, &[(AUDIO_200, 50, 10)]);
        assert_eq!(fx.seq().audio_tracks().len(), 2, "{}", fx.render());
        assert_eq!(fx.doc().find_clip(new[0]).unwrap().track, a1);
    }
}

#[allow(unused_imports, dead_code)]
mod f02_gesture_split_and_partial_cancel_after_save {
    //! mark_saved() seals the top entry even while a gesture is open

    use super::common::*;

    #[test]
    fn cancelling_a_gesture_after_an_autosave_reverts_the_whole_gesture() {
        use freecut_timeline::CommitMeta;
        use freecut_timeline::edit::{EditRequest, InsertTarget, MoveReq};
        use freecut_timeline::history::Coalesce;
        let mut fx = Fixture::new();
        let ids = fx.insert(InsertTarget::NewTrack, 10, &[(VIDEO_50, 0, 5)]);
        let before = fx.doc().clone();
        let entries = fx.editor.history().len();
        let g = fx.editor.begin_gesture();
        for (i, d) in [2i64, 3].into_iter().enumerate() {
            let req = EditRequest::Move(MoveReq {
                seq: fx.seq_id(),
                clips: ids.clone(),
                delta: f(d),
                track_shift: 0,
            });
            let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
            fx.editor
                .commit(
                    plan,
                    CommitMeta {
                        selection_before: Vec::new(),
                        coalesce: Coalesce::Gesture(g),
                    },
                )
                .unwrap();
            if i == 0 {
                fx.editor.mark_saved();
            }
        }
        fx.editor.cancel_gesture(g).unwrap();
        assert_eq!(fx.editor.history().len(), entries, "gesture left an entry");
        assert_eq!(fx.doc(), &before, "Esc left part of the gesture applied");
    }
}

#[allow(unused_imports, dead_code)]
mod f03_noncontiguous_main_members_allowed_off_main {
    //! relayout_main checks contiguity (NonContiguousMain) only for main-track members that stay on the magnetic main track

    use super::common::*;

    #[test]
    fn non_adjacent_main_track_members_are_refused_even_when_moved_to_an_overlay() {
        use freecut_timeline::edit::{EditError, EditRequest, InsertTarget, MoveReq};
        let mut fx = Fixture::new();
        let main = fx.main();
        let ids = fx.insert(
            InsertTarget::Track(main),
            0,
            &[(VIDEO_50, 0, 5), (VIDEO_50, 5, 5), (VIDEO_50, 10, 5)],
        );
        assert!(fx.seq().flags.main_magnet, "magnet is on by default");
        let req = EditRequest::Move(MoveReq {
            seq: fx.seq_id(),
            clips: vec![ids[0], ids[2]],
            delta: f(0),
            track_shift: 1,
        });
        let result = fx.apply(req);
        println!("after:\n{}", fx.render());
        assert_eq!(result.unwrap_err(), EditError::NonContiguousMain);
    }
}

#[allow(unused_imports, dead_code)]
mod f04_split_drops_unsplit_clips_from_selection {
    //! After a split, selection_after is set to only the right-hand parts

    use super::common::*;

    #[test]
    fn split_keeps_selected_clips_that_were_not_split_selected() {
        use freecut_timeline::Committed;
        use freecut_timeline::edit::{EditRequest, InsertTarget, SplitReq, SplitTargets};
        let mut fx = Fixture::new();
        let main = fx.main();
        let a = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 10)]);
        let b = fx.insert(InsertTarget::NewTrack, 20, &[(VIDEO_50, 0, 10)]);
        let c = fx
            .apply(EditRequest::Split(SplitReq {
                seq: fx.seq_id(),
                at: f(5),
                targets: SplitTargets::Clips(vec![a[0], b[0]]),
            }))
            .unwrap();
        let Committed::Applied {
            selection: Some(sel),
            ..
        } = c
        else {
            panic!("expected an applied split with a selection");
        };
        assert!(
            sel.contains(&b[0]),
            "B was dropped from the selection: {sel:?}"
        );
    }
}

#[allow(unused_imports, dead_code)]
mod f05_move_not_quantized_to_grid_after_rate_change {
    //! move_clips quantizes the delta, not the landing position (start = old_start + quantize(delta))

    use super::common::*;

    #[test]
    fn a_move_lands_on_the_sequence_frame_grid_after_a_rate_change() {
        use freecut_timeline::CommitMeta;
        use freecut_timeline::edit::{EditCtx, EditRequest, InsertTarget, Limits, MoveReq};
        use freecut_timeline::time::FrameRate;
        let mut fx = Fixture::new();
        let ids = fx.insert(InsertTarget::NewTrack, 1, &[(VIDEO_50, 0, 10)]);
        let r30 = FrameRate::new(30, 1).unwrap();
        fx.set_rate(r30);
        let ctx = fx.ctx();
        let plan = fx
            .editor
            .plan(
                &ctx,
                &EditRequest::Move(MoveReq {
                    seq: fx.seq_id(),
                    clips: ids.clone(),
                    delta: r30.frames(1).unwrap(),
                    track_shift: 0,
                }),
            )
            .unwrap();
        fx.editor.commit(plan, CommitMeta::default()).unwrap();
        let start = fx.doc().clip(ids[0]).unwrap().start();
        assert!(
            r30.is_on_grid(start),
            "moved clip starts off-grid at {start:?}"
        );
    }
}

#[allow(unused_imports, dead_code)]
mod f06_history_missing_byte_budget {
    //! HistoryLimits has only max_entries

    #[test]
    fn history_limits_include_the_512_mb_change_data_budget() {
        use freecut_timeline::history::HistoryLimits;
        let dbg = format!("{:?}", HistoryLimits::default());
        assert!(
            dbg.contains("bytes") || dbg.contains("536870912"),
            "no byte budget in {dbg}"
        );
    }

    // Behaviour check, used after the fix (needs max_bytes and History::heap_bytes):
    // mod common; use super::common::*;
    // use freecut_timeline::edit::InsertTarget;
    // use freecut_timeline::history::HistoryLimits;
    // use freecut_timeline::{Editor, HistoryError};
    // fn fixture(max_bytes: usize) -> Fixture {
    //     Fixture { editor: Editor::new(HistoryLimits { max_entries: 200, max_bytes }).unwrap(), sources: sources() }
    // }
    // #[test]
    // fn byte_budget_drops_oldest_entries_but_keeps_the_newest() {
    //     let mut probe = fixture(usize::MAX);
    //     let main = probe.main();
    //     let mut totals = Vec::new();
    //     for _ in 0..3 { probe.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 1)]); totals.push(probe.editor.history().heap_bytes()); }
    //     let newest_two = totals[2] - totals[0];
    //     let mut fx = fixture(newest_two);
    //     let main = fx.main();
    //     for _ in 0..3 { fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 1)]); }
    //     assert_eq!(fx.editor.history().len(), 2);
    //     assert_eq!(fx.editor.history().heap_bytes(), newest_two);
    //     let mut tiny = fixture(1);
    //     let main = tiny.main();
    //     for _ in 0..3 { tiny.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 1)]); }
    //     assert_eq!(tiny.editor.history().len(), 1);
    //     tiny.editor.undo().unwrap();
    //     assert_eq!(tiny.editor.undo().unwrap_err(), HistoryError::Empty);
    // }
}

#[allow(unused_imports, dead_code)]
mod f07_autosave_mid_gesture_defeats_cancel {
    //! Editor::mark_saved has no gesture check

    use super::common::*;

    use freecut_timeline::CommitMeta;
    use freecut_timeline::edit::{Edge, EditRequest, InsertTarget, TrimReq};
    use freecut_timeline::history::{Coalesce, GestureId};
    use freecut_timeline::ids::ClipId;

    fn trim_step(fx: &mut Fixture, clip: ClipId, frames: i64, g: GestureId) {
        let trim = EditRequest::Trim(TrimReq {
            seq: fx.seq_id(),
            clip,
            edge: Edge::Out,
            delta: f(frames),
            ripple: false,
        });
        let plan = fx.editor.plan(&fx.ctx(), &trim).unwrap();
        fx.editor
            .commit(
                plan,
                CommitMeta {
                    selection_before: vec![],
                    coalesce: Coalesce::Gesture(g),
                },
            )
            .unwrap();
    }

    #[test]
    fn autosave_during_a_live_gesture_must_not_defeat_cancel() {
        let mut fx = Fixture::new();
        let ids = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
        let before = fx.doc().clone();
        let g = fx.editor.begin_gesture();
        trim_step(&mut fx, ids[0], 2, g);
        fx.editor.mark_saved(); // PM-014 autosave tick while the slider is still held
        fx.editor.cancel_gesture(g).unwrap(); // Esc
        assert_eq!(fx.doc(), &before, "a cancelled gesture leaves no trace");
    }

    #[test]
    fn autosave_during_a_live_gesture_must_not_split_it() {
        let mut fx = Fixture::new();
        let ids = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
        let before = fx.doc().clone();
        let len_before = fx.editor.history().len();
        let g = fx.editor.begin_gesture();
        trim_step(&mut fx, ids[0], 2, g);
        fx.editor.mark_saved();
        trim_step(&mut fx, ids[0], 3, g);
        fx.editor.end_gesture(g);
        assert_eq!(
            fx.editor.history().len(),
            len_before + 1,
            "a continuous gesture is one entry (N4)"
        );
        assert!(fx.editor.is_dirty(), "the saved mid-gesture state is gone");
        fx.editor.undo().unwrap();
        assert_eq!(fx.doc(), &before, "one undo reverts the whole gesture");
    }
}

#[allow(unused_imports, dead_code)]
mod f08_cancel_gesture_not_traceless {
    //! A gesture's first commit runs History::record, which has side effects that cannot be undone

    use super::common::*;

    use freecut_timeline::edit::{Edge, EditRequest, InsertTarget, TrimReq};
    use freecut_timeline::history::{Coalesce, HistoryLimits};
    use freecut_timeline::ids::ClipId;
    use freecut_timeline::{CommitMeta, Editor};

    fn trim_in_cancelled_gesture(fx: &mut Fixture, clip: ClipId) {
        let g = fx.editor.begin_gesture();
        let req = EditRequest::Trim(TrimReq {
            seq: fx.seq_id(),
            clip,
            edge: Edge::Out,
            delta: f(2),
            ripple: false,
        });
        let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
        fx.editor
            .commit(
                plan,
                CommitMeta {
                    selection_before: vec![],
                    coalesce: Coalesce::Gesture(g),
                },
            )
            .unwrap();
        fx.editor.cancel_gesture(g).unwrap();
    }

    #[test]
    fn cancelling_a_gesture_keeps_the_redo_branch_and_the_oldest_entry() {
        // 1. The redo branch survives a cancelled gesture.
        let mut fx = Fixture::new();
        let main = fx.main();
        let a = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 5)]);
        fx.editor.undo().unwrap();
        trim_in_cancelled_gesture(&mut fx, a[0]);
        assert!(
            fx.editor.history().can_redo(),
            "cancel discarded the redo branch"
        );
        // 2. A full history keeps its oldest entry across a cancelled gesture.
        let mut fx = Fixture::with_limits(HistoryLimits {
            max_entries: 2,
            ..HistoryLimits::default()
        });
        let a = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
        fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
        trim_in_cancelled_gesture(&mut fx, a[0]);
        assert_eq!(
            fx.editor.history().len(),
            2,
            "cancel evicted the oldest entry for good"
        );
    }

    #[test]
    fn cancelling_a_gesture_keeps_a_save_point_in_the_redo_branch_reachable() {
        let mut fx = Fixture::new();
        let main = fx.main();
        let a = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 5)]);
        fx.editor.mark_saved();
        fx.editor.undo().unwrap();
        trim_in_cancelled_gesture(&mut fx, a[0]);
        let _ = fx.editor.redo();
        assert!(
            !fx.editor.is_dirty(),
            "the save point in the redo branch became unreachable after a cancelled gesture"
        );
    }
}

#[allow(unused_imports, dead_code)]
mod f09_plan_staleness_not_bound_to_editor {
    //! Staleness is checked only by `plan.base_revision != self.revision`

    use super::common::*;

    use freecut_timeline::edit::{Edge, EditRequest, InsertTarget, TrimReq};
    use freecut_timeline::{CommitError, CommitMeta};

    #[test]
    fn a_plan_from_another_editor_is_refused_as_stale() {
        let mut a = Fixture::new();
        let mut b = Fixture::new();
        let (am, bm) = (a.main(), b.main());
        a.insert(InsertTarget::Track(am), 0, &[(VIDEO_50, 0, 10)]);
        b.insert(InsertTarget::Track(bm), 0, &[(VIDEO_100, 0, 20)]);
        // Same number of edits in each editor (on the unfixed code the two
        // revision counters are then equal).
        assert_ne!(a.doc(), b.doc());
        let clip = a.seq().main_track().unwrap().clips()[0].id;
        let req = EditRequest::Trim(TrimReq {
            seq: a.seq_id(),
            clip,
            edge: Edge::Out,
            delta: f(-2),
            ripple: false,
        });
        let plan = a.editor.plan(&a.ctx(), &req).unwrap();
        let b_before = b.doc().clone();
        let b_history = b.editor.history().len();
        // Today this is accepted and B's document is replaced wholesale by A's.
        assert_eq!(
            b.editor.commit(plan, CommitMeta::default()).unwrap_err(),
            CommitError::Stale
        );
        assert_eq!(b.doc(), &b_before);
        assert_eq!(b.editor.history().len(), b_history);
    }
}

#[allow(unused_imports, dead_code)]
mod f11_stale_gesture_id_closes_open_gesture {
    //! end_gesture acts only when `open_gesture == Some(id)`, but discard_gesture sets `open_gesture = None` for any id

    use super::common::*;

    #[test]
    fn cancelling_a_stale_gesture_id_leaves_the_open_gesture_alone() {
        use freecut_timeline::edit::{Edge, EditRequest, InsertTarget, TrimReq};
        use freecut_timeline::history::Coalesce;
        use freecut_timeline::{CommitMeta, HistoryError};
        let mut fx = Fixture::new();
        let ids = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
        let old = fx.editor.begin_gesture();
        fx.editor.end_gesture(old);
        let g = fx.editor.begin_gesture();
        let trim = EditRequest::Trim(TrimReq {
            seq: fx.seq_id(),
            clip: ids[0],
            edge: Edge::Out,
            delta: f(2),
            ripple: false,
        });
        let plan = fx.editor.plan(&fx.ctx(), &trim).unwrap();
        fx.editor
            .commit(
                plan,
                CommitMeta {
                    selection_before: vec![],
                    coalesce: Coalesce::Gesture(g),
                },
            )
            .unwrap();
        fx.editor.cancel_gesture(old).unwrap(); // late capture-lost for the finished gesture
        assert_eq!(
            fx.editor.undo().unwrap_err(),
            HistoryError::GestureOpen,
            "gesture g is still being dragged"
        );
    }
}

#[allow(unused_imports, dead_code)]
mod f12_merge_across_undo_after_empty_pop {
    //! When a merge cancels out, record() pops the top entry (line 219) but still sets `top_mergeable = true`

    use super::common::*;

    use freecut_timeline::CommitMeta;
    use freecut_timeline::edit::{EditRequest, InsertTarget, MoveReq, OpKind};
    use freecut_timeline::history::{Coalesce, CoalesceKey};
    use freecut_timeline::ids::ClipId;

    fn nudge(fx: &mut Fixture, clip: ClipId, frames: i64, c: Coalesce) {
        let req = EditRequest::Move(MoveReq {
            seq: fx.seq_id(),
            clips: vec![clip],
            delta: f(frames),
            track_shift: 0,
        });
        let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
        fx.editor
            .commit(
                plan,
                CommitMeta {
                    selection_before: vec![],
                    coalesce: c,
                },
            )
            .unwrap();
    }

    #[test]
    fn a_repeat_never_merges_into_an_entry_from_before_an_undo() {
        let mut fx = Fixture::new();
        let x = fx.insert(InsertTarget::NewTrack, 10, &[(VIDEO_50, 0, 5)])[0];
        let y = fx.insert(InsertTarget::NewTrack, 40, &[(VIDEO_50, 0, 5)])[0];
        let kx = CoalesceKey {
            op: OpKind::MoveClips,
            target: x.get(),
        };
        let ky = CoalesceKey {
            op: OpKind::MoveClips,
            target: y.get(),
        };
        nudge(&mut fx, x, 1, Coalesce::Repeat { key: kx, now_ms: 0 });
        let after_x1 = fx.render();
        nudge(&mut fx, y, 3, Coalesce::Never);
        fx.editor.undo().unwrap();
        let base = fx.editor.history().len();
        nudge(
            &mut fx,
            y,
            1,
            Coalesce::Repeat {
                key: ky,
                now_ms: 100,
            },
        );
        nudge(
            &mut fx,
            y,
            -1,
            Coalesce::Repeat {
                key: ky,
                now_ms: 200,
            },
        ); // cancels to nothing
        assert_eq!(
            fx.editor.history().len(),
            base,
            "the cancelled pair left no entry"
        );
        nudge(
            &mut fx,
            x,
            1,
            Coalesce::Repeat {
                key: kx,
                now_ms: 500,
            },
        );
        // Observable effect: one Undo should revert only the last x nudge.
        let len_after = fx.editor.history().len();
        fx.editor.undo().unwrap();
        let after_undo = fx.render();
        assert_eq!(
            len_after,
            base + 1,
            "merged across the undo into the old x entry; after one undo:\n{after_undo}\nexpected:\n{after_x1}"
        );
        assert_eq!(
            after_undo, after_x1,
            "one undo should revert only the last nudge"
        );
    }
}

#[allow(unused_imports, dead_code)]
mod f13_compact_misses_header_flag_track_pairs {
    //! ChangeSet::compact folds only ReplaceClip, ShiftClips and InsertClip/RemoveClip pairs; every other pair falls through `_ => None`

    use super::common::*;

    use freecut_timeline::edit::{
        DeleteReq, EditRequest, InsertReq, InsertTarget, NewClip, OpKind,
    };
    use freecut_timeline::history::{Coalesce, CoalesceKey};
    use freecut_timeline::model::{MediaKind, TrackFlags, TrackHeader};
    use freecut_timeline::time::TimeRange;
    use freecut_timeline::{CommitMeta, Committed};

    fn commit_with(fx: &mut Fixture, req: &EditRequest, coalesce: Coalesce) -> Committed {
        let plan = fx.editor.plan(&fx.ctx(), req).unwrap();
        fx.editor
            .commit(
                plan,
                CommitMeta {
                    selection_before: vec![],
                    coalesce,
                },
            )
            .unwrap()
    }

    #[test]
    fn a_header_toggle_and_its_reverse_merge_to_no_entry() {
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
        fx.editor.mark_saved();
        let saved_doc = fx.doc().clone();
        let len = fx.editor.history().len();
        let key = CoalesceKey {
            op: OpKind::SetTrackHeader,
            target: main.get(),
        };
        for (now_ms, locked) in [(0, true), (100, false)] {
            let req = EditRequest::SetTrackHeader {
                seq: fx.seq_id(),
                track: main,
                header: TrackHeader {
                    name: None,
                    flags: TrackFlags {
                        locked,
                        ..TrackFlags::default()
                    },
                },
            };
            commit_with(&mut fx, &req, Coalesce::Repeat { key, now_ms });
        }
        assert_eq!(fx.doc(), &saved_doc, "net no-op");
        assert_eq!(
            fx.editor.history().len(),
            len,
            "the merged edits cancel out"
        );
        assert!(!fx.editor.is_dirty(), "the document equals the saved one");
    }

    #[test]
    fn a_magnet_toggle_and_its_reverse_merge_to_no_entry() {
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
        fx.editor.mark_saved();
        let saved_doc = fx.doc().clone();
        let len = fx.editor.history().len();
        let on = fx.seq().flags.main_magnet;
        let key = CoalesceKey {
            op: OpKind::SetMagnet,
            target: 0,
        };
        for (now_ms, v) in [(0, !on), (100, on)] {
            let req = EditRequest::SetMagnet {
                seq: fx.seq_id(),
                on: v,
            };
            commit_with(&mut fx, &req, Coalesce::Repeat { key, now_ms });
        }
        assert_eq!(fx.doc(), &saved_doc, "net no-op");
        assert_eq!(
            fx.editor.history().len(),
            len,
            "the merged edits cancel out"
        );
        assert!(!fx.editor.is_dirty());
    }

    #[test]
    fn a_gesture_that_inserts_to_a_new_track_and_deletes_merges_to_no_entry() {
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
        fx.editor.mark_saved();
        let saved_doc = fx.doc().clone();
        let len = fx.editor.history().len();
        let g = fx.editor.begin_gesture();
        let req = EditRequest::Insert(InsertReq {
            seq: fx.seq_id(),
            target: InsertTarget::NewTrack,
            at: f(0),
            clips: vec![NewClip::media(
                VIDEO_50,
                MediaKind::Video,
                TimeRange::new(f(0), f(5)).unwrap(),
            )],
        });
        let ids = match commit_with(&mut fx, &req, Coalesce::Gesture(g)) {
            Committed::Applied { selection, .. } => selection.unwrap(),
            Committed::NoOp => panic!(),
        };
        let del = EditRequest::Delete(DeleteReq {
            seq: fx.seq_id(),
            clips: ids,
            ripple: false,
        });
        commit_with(&mut fx, &del, Coalesce::Gesture(g));
        fx.editor.end_gesture(g);
        assert_eq!(fx.doc(), &saved_doc, "net no-op");
        assert_eq!(
            fx.editor.history().len(),
            len,
            "the merged edits cancel out"
        );
        assert!(!fx.editor.is_dirty());
    }
}

#[allow(unused_imports, dead_code)]
mod f14_redo_drops_post_op_selection {
    //! commit stores `plan.selection_after` as is

    use super::common::*;

    #[test]
    fn redo_restores_the_post_op_selection() {
        use freecut_timeline::CommitMeta;
        use freecut_timeline::edit::{Edge, EditRequest, InsertTarget, TrimReq};
        use freecut_timeline::history::Coalesce;
        let mut fx = Fixture::new();
        let ids = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
        let req = EditRequest::Trim(TrimReq {
            seq: fx.seq_id(),
            clip: ids[0],
            edge: Edge::Out,
            delta: f(-2),
            ripple: false,
        });
        let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
        fx.editor
            .commit(
                plan,
                CommitMeta {
                    selection_before: ids.clone(),
                    coalesce: Coalesce::Never,
                },
            )
            .unwrap();
        assert_eq!(fx.editor.undo().unwrap().selection, Some(ids.clone()));
        assert_eq!(
            fx.editor.redo().unwrap().selection,
            Some(ids),
            "N7: redo restores the post-op selection"
        );
    }

    /// The same scenario as the app would drive it: the user changes the
    /// selection between Undo and Redo (selection changes are not entries).
    #[test]
    fn redo_restores_the_post_op_selection_after_user_reselects() {
        use freecut_timeline::CommitMeta;
        use freecut_timeline::edit::{Edge, EditRequest, InsertTarget, TrimReq};
        use freecut_timeline::history::Coalesce;
        let mut fx = Fixture::new();
        let ids = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
        let mut app_selection = ids.clone();
        let req = EditRequest::Trim(TrimReq {
            seq: fx.seq_id(),
            clip: ids[0],
            edge: Edge::Out,
            delta: f(-2),
            ripple: false,
        });
        let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
        if let freecut_timeline::Committed::Applied {
            selection: Some(s), ..
        } = fx
            .editor
            .commit(
                plan,
                CommitMeta {
                    selection_before: app_selection.clone(),
                    coalesce: Coalesce::Never,
                },
            )
            .unwrap()
        {
            app_selection = s;
        }
        let post_op = app_selection.clone();
        if let Some(s) = fx.editor.undo().unwrap().selection {
            app_selection = s;
        }
        app_selection.clear(); // user clicks empty timeline
        assert!(fx.editor.history().can_redo());
        if let Some(s) = fx.editor.redo().unwrap().selection {
            app_selection = s;
        }
        assert_eq!(
            app_selection, post_op,
            "N7: redo restores the post-op selection"
        );
    }
}

#[allow(unused_imports, dead_code)]
mod f15_split_creates_sub_frame_parts_off_grid {
    //! Split only skips a clip when `at` is on or outside its edges (`strictly_contains`)

    use super::common::*;

    #[test]
    fn split_after_a_rate_change_never_creates_a_sub_frame_part() {
        use freecut_timeline::CommitMeta;
        use freecut_timeline::edit::{
            EditCtx, EditRequest, InsertTarget, Limits, SplitReq, SplitTargets,
        };
        use freecut_timeline::time::FrameRate;
        let mut fx = Fixture::new();
        let main = fx.main();
        // 1 frame at 25 fps = 28_224_000 ticks = 1.2 frames at 30 fps (rate changed; TL-001 keeps ticks).
        let ids = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 1)]);
        let r30 = FrameRate::new(30, 1).unwrap();
        fx.set_rate(r30);
        let ctx = fx.ctx();
        let req = EditRequest::Split(SplitReq {
            seq: fx.seq_id(),
            at: r30.frames(1).unwrap(),
            targets: SplitTargets::Clips(ids),
        });
        if let Ok(plan) = fx.editor.plan(&ctx, &req) {
            fx.editor.commit(plan, CommitMeta::default()).unwrap();
        }
        for (_, t) in fx.seq().tracks() {
            for c in t.clips() {
                assert!(
                    c.duration() >= r30.frame_ticks(),
                    "clip {} is {} ticks, shorter than one 30 fps frame",
                    c.id,
                    c.duration().raw()
                );
            }
        }
    }
}

#[allow(unused_imports, dead_code)]
mod f16_set_magnet_refused_whenever_main_locked {
    //! set_magnet returns MainTrackLocked whenever the main track is locked, in both directions

    use super::common::*;

    #[test]
    fn magnet_toggles_that_move_nothing_are_allowed_on_a_locked_main_track() {
        use freecut_timeline::edit::{EditRequest, InsertTarget};
        use freecut_timeline::model::{TrackFlags, TrackHeader};
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
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
        fx.apply(EditRequest::SetMagnet {
            seq: fx.seq_id(),
            on: false,
        })
        .expect("TL-005: switching the magnet off is always allowed");
        assert!(!fx.seq().flags.main_magnet);
        fx.apply(EditRequest::SetMagnet {
            seq: fx.seq_id(),
            on: true,
        })
        .expect(
            "TL-005: switching it on is allowed when the main track is already contiguous from 0",
        );
        assert!(fx.seq().flags.main_magnet);
    }

    #[test]
    fn magnet_on_that_would_compact_a_locked_main_track_is_refused() {
        use freecut_timeline::edit::{EditError, EditRequest, InsertTarget};
        use freecut_timeline::model::{TrackFlags, TrackHeader};
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.apply(EditRequest::SetMagnet {
            seq: fx.seq_id(),
            on: false,
        })
        .unwrap();
        fx.insert(InsertTarget::Track(main), 3, &[(VIDEO_50, 0, 5)]);
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
            fx.apply(EditRequest::SetMagnet {
                seq: fx.seq_id(),
                on: true
            })
            .unwrap_err(),
            EditError::MainTrackLocked
        );
        assert!(!fx.seq().flags.main_magnet);
    }
}

#[allow(unused_imports, dead_code)]
mod f17_redirected_insert_ignores_free_compatible_track {
    //! When an item is redirected (for example audio added to the main track), resolve_target picks the compatible track nearest by index, whether or not it has room

    use super::common::*;

    #[test]
    fn audio_added_to_the_main_track_lands_on_the_first_free_audio_track() {
        use freecut_timeline::edit::InsertTarget;
        use freecut_timeline::model::TrackSlot;
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.insert(InsertTarget::Track(main), 0, &[(AUDIO_200, 0, 10)]); // A0: [0, 10)
        fx.insert(InsertTarget::NewTrack, 50, &[(AUDIO_200, 0, 10)]); // A1: [50, 60)
        let a1 = fx.track_at(TrackSlot::audio(1));
        let ids = fx.insert(InsertTarget::Track(main), 0, &[(AUDIO_200, 0, 5)]);
        assert_eq!(
            fx.seq().audio_tracks().len(),
            2,
            "A1 is free at [0, 5):\n{}",
            fx.render()
        );
        assert!(fx.seq().find_track(a1).unwrap().1.clip(ids[0]).is_some());
    }
}

#[allow(unused_imports, dead_code)]
mod f18_ripple_trim_overflow_reported_as_internal_bug {
    //! An overlay ripple trim never checks whether the later clips it shifts stay inside the i64 tick range

    use super::common::*;

    #[test]
    fn ripple_trim_overflow_is_reported_as_time_overflow_not_an_internal_bug() {
        use freecut_timeline::edit::{
            Edge, EditError, EditRequest, InsertReq, InsertTarget, NewClip, TrimReq,
        };
        use freecut_timeline::model::TrackSlot;
        use freecut_timeline::time::{Tick, TimeError};
        let mut fx = Fixture::new();
        let a = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 5)]);
        let v1 = fx.track_at(TrackSlot::visual(1));
        // A still on the same track, ending 2 frames below the top of the tick range.
        let fr = f(1).raw();
        fx.apply(EditRequest::Insert(InsertReq {
            seq: fx.seq_id(),
            target: InsertTarget::Track(v1),
            at: Tick::from_raw((i64::MAX / fr - 7) * fr),
            clips: vec![NewClip::image(VIDEO_50, f(5))],
        }))
        .unwrap();
        let err = fx
            .apply(EditRequest::Trim(TrimReq {
                seq: fx.seq_id(),
                clip: a[0],
                edge: Edge::Out,
                delta: f(10),
                ripple: true,
            }))
            .unwrap_err();
        assert_eq!(err, EditError::Time(TimeError::Overflow), "got {err:?}");
    }
}

#[allow(unused_imports, dead_code)]
mod f19_zero_delta_trim_changes_off_grid_edge {
    //! Trim computes `d = quantize(old_edge + delta) - old_edge`

    use super::common::*;

    #[test]
    fn a_zero_delta_trim_on_an_off_grid_edge_changes_nothing() {
        use freecut_timeline::edit::{Edge, EditCtx, EditRequest, InsertTarget, Limits, TrimReq};
        use freecut_timeline::time::{FrameRate, Tick};
        let mut fx = Fixture::new();
        // A = [0, 2f), B = [2f, 5f) at 25 fps; 2 frames @25 fps = 2.4 frames @30 fps.
        let ids = fx.insert(
            InsertTarget::NewTrack,
            0,
            &[(VIDEO_50, 0, 2), (VIDEO_50, 10, 3)],
        );
        let r30 = FrameRate::new(30, 1).unwrap();
        fx.set_rate(r30);
        let ctx = fx.ctx();
        for (clip, edge) in [(ids[1], Edge::In), (ids[0], Edge::Out)] {
            let req = EditRequest::Trim(TrimReq {
                seq: fx.seq_id(),
                clip,
                edge,
                delta: Tick::ZERO,
                ripple: false,
            });
            let plan = fx
                .editor
                .plan(&ctx, &req)
                .unwrap_or_else(|e| panic!("zero trim of {clip} {edge:?} refused: {e:?}"));
            assert!(
                plan.is_noop(),
                "zero trim of {clip} {edge:?} changed the clip"
            );
        }
    }
}

#[allow(unused_imports, dead_code)]
mod f20_noncontiguous_main_members_allowed_when_leaving_main {
    //! The NonContiguousMain check in relayout_main only looks at main-track members that stay on the main track (`stays == true`)

    use super::common::*;

    #[test]
    fn non_adjacent_main_clips_cant_move_off_the_magnetic_main_track_together() {
        use freecut_timeline::edit::{EditError, EditRequest, InsertTarget, MoveReq};
        let mut fx = Fixture::new();
        assert!(fx.seq().flags.main_magnet, "fixture magnet must be on");
        let main = fx.main();
        let ids = fx.insert(
            InsertTarget::Track(main),
            0,
            &[(VIDEO_50, 0, 5), (VIDEO_50, 5, 5), (VIDEO_50, 10, 5)],
        );
        let req = EditRequest::Move(MoveReq {
            seq: fx.seq_id(),
            clips: vec![ids[0], ids[2]],
            delta: f(0),
            track_shift: 1,
        });
        let res = fx.apply(req);
        if res.is_ok() {
            eprintln!("ACCEPTED; timeline after move:\n{}", fx.render());
        }
        assert_eq!(res.unwrap_err(), EditError::NonContiguousMain);
    }
}

#[allow(unused_imports, dead_code)]
mod f21_stream_ts_at_or_before_overflows_at_top_of_range {
    //! stream_ts_at_or_before probes `ticks_from_stream(ts + 1)` with `?`
    use super::common::*;

    #[test]
    fn stream_ts_at_or_before_works_at_the_top_of_the_tick_range() {
        use freecut_timeline::time::{
            StreamTimeBase, Tick, stream_ts_at_or_before, ticks_from_stream,
        };
        let tb = StreamTimeBase::new(1, 90_000).unwrap(); // exactly 7_840 ticks per unit
        let ts = stream_ts_at_or_before(Tick::MAX, tb)
            .expect("floor(i64::MAX / 7840) is representable and maps to <= Tick::MAX");
        assert_eq!(ts, i64::MAX / 7_840);
        assert!(ticks_from_stream(ts, tb).unwrap() <= Tick::MAX);
    }
}

#[allow(unused_imports, dead_code)]
mod f22_from_document_max_id_leaves_allocator_behind {
    //! Editor::from_document advances the allocator only when `max_id + 1` fits in a NonZeroU64

    use super::common::*;

    #[test]
    fn loading_a_document_holding_the_largest_id_never_reuses_ids() {
        use core::num::NonZeroU64;
        use freecut_timeline::Editor;
        use freecut_timeline::edit::{
            EditCtx, EditError, EditRequest, InsertReq, InsertTarget, Limits, NewClip,
        };
        use freecut_timeline::history::HistoryLimits;
        use freecut_timeline::ids::{IdAlloc, SeqId, TrackId};
        use freecut_timeline::model::{Document, TextClipData};
        let doc = Document::new(
            SeqId::from_raw(NonZeroU64::new(1).unwrap()),
            TrackId::from_raw(NonZeroU64::MAX),
            settings(),
        );
        let ed = Editor::from_document(doc, IdAlloc::default(), HistoryLimits::default());
        let ctx = EditCtx {
            limits: Limits::default(),
        };
        let root = ed.document().root();
        let main = ed
            .document()
            .sequence(root)
            .unwrap()
            .main_track()
            .unwrap()
            .id;
        let err = ed
            .plan(
                &ctx,
                &EditRequest::Insert(InsertReq {
                    seq: root,
                    target: InsertTarget::Track(main),
                    at: f(0),
                    clips: vec![NewClip::text(TextClipData::default(), f(5))],
                }),
            )
            .unwrap_err();
        assert_eq!(
            err,
            EditError::IdSpaceExhausted,
            "allocator stayed at 1 and handed out a used id"
        );
    }
}

#[allow(unused_imports, dead_code)]
mod f23_commit_accepts_plans_from_other_documents {
    //! `edit::plan` (edit/mod.rs:165) is public and stamps every plan `base_revision: 0` (edit/tx.rs:79)

    use super::common::*;
    use freecut_timeline::CommitMeta;
    use freecut_timeline::edit::{self, AddTrackReq, EditRequest, InsertTarget, TrackPosition};
    use freecut_timeline::model::TrackKind;

    #[test]
    fn a_plan_made_against_another_document_is_not_committed() {
        let mut fx = Fixture::new(); // revision 0, empty main track
        let original = fx.doc().clone();
        let mut other = Fixture::new();
        let other_main = other.main();
        other.insert(InsertTarget::Track(other_main), 0, &[(VIDEO_50, 0, 5)]);
        let req = EditRequest::AddTrack(AddTrackReq {
            seq: other.seq_id(),
            kind: TrackKind::Audio,
            position: TrackPosition::Default,
        });
        // Public free planner, e.g. a background job planning on a snapshot.
        let plan = other.editor.planner().plan(&other.ctx(), &req).unwrap();
        match fx.editor.commit(plan, CommitMeta::default()) {
            Err(_) => {}
            Ok(_) => {
                fx.editor.undo().unwrap();
                assert_eq!(
                    fx.doc(),
                    &original,
                    "undo must give back the document this editor held"
                );
            }
        }
    }

    /// Uses only Editor::plan: the app replaces the editor with a freshly loaded one.
    #[test]
    fn a_plan_from_a_replaced_editor_is_stale() {
        let mut old = Fixture::new();
        let req = EditRequest::AddTrack(AddTrackReq {
            seq: old.seq_id(),
            kind: TrackKind::Audio,
            position: TrackPosition::Default,
        });
        let plan = old.editor.plan(&old.ctx(), &req).unwrap();
        let mut other = Fixture::new();
        let main = other.main();
        other.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
        let loaded = freecut_timeline::Editor::from_document(
            other.doc().clone(),
            other.editor.ids().clone(),
            freecut_timeline::history::HistoryLimits::default(),
        );
        old.editor = loaded;
        let before = old.doc().clone();
        assert_eq!(
            old.editor.commit(plan, CommitMeta::default()).unwrap_err(),
            freecut_timeline::CommitError::Stale,
            "a plan from another editor must not replace the loaded document"
        );
        assert_eq!(old.doc(), &before);
    }

    /// Two clones of one editor diverge at equal revision counts.
    #[test]
    fn a_plan_from_a_diverged_clone_is_stale() {
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
        let mut twin = Fixture {
            editor: fx.editor.clone(),
        };
        twin.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 7)]);
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 3)]);
        let req = EditRequest::AddTrack(AddTrackReq {
            seq: twin.seq_id(),
            kind: TrackKind::Audio,
            position: TrackPosition::Default,
        });
        let plan = twin.editor.plan(&twin.ctx(), &req).unwrap();
        assert_eq!(
            fx.editor.commit(plan, CommitMeta::default()).unwrap_err(),
            freecut_timeline::CommitError::Stale
        );
    }

    /// The off-thread path keeps working: plan on an up-to-date editor clone in a worker.
    #[test]
    fn a_plan_from_an_up_to_date_clone_commits() {
        // Off-thread planning goes through a Planner snapshot; a cloned Editor
        // is a separate editor whose plans are refused.
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
        let planner = fx.editor.planner();
        let seq = fx.seq_id();
        let plan = std::thread::spawn(move || {
            let ctx = freecut_timeline::edit::EditCtx::default();
            let req = EditRequest::AddTrack(AddTrackReq {
                seq,
                kind: TrackKind::Audio,
                position: TrackPosition::Default,
            });
            planner.plan(&ctx, &req).unwrap()
        })
        .join()
        .unwrap();
        let before = fx.doc().clone();
        fx.editor.commit(plan, CommitMeta::default()).unwrap();
        fx.editor.undo().unwrap();
        assert_eq!(fx.doc(), &before);
    }

    #[test]
    fn a_planner_from_before_another_commit_is_stale() {
        let mut fx = Fixture::new();
        let main = fx.main();
        let planner = fx.editor.planner();
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
        let req = EditRequest::AddTrack(AddTrackReq {
            seq: fx.seq_id(),
            kind: TrackKind::Audio,
            position: TrackPosition::Default,
        });
        let plan = planner.plan(&fx.ctx(), &req).unwrap();
        assert_eq!(
            fx.editor.commit(plan, CommitMeta::default()).unwrap_err(),
            freecut_timeline::CommitError::Stale
        );
    }
}

#[allow(unused_imports, dead_code)]
mod f25_main_track_single_clip_ops_quadratic {
    //! `Change::ShiftClips` apply calls the linear `Track::position_of` once per shifted id, so applying a shift of k clips on an n-clip track costs O(k*n)

    use super::common::*;
    use freecut_timeline::CommitMeta;
    use freecut_timeline::edit::{Edge, EditRequest, InsertReq, InsertTarget, NewClip, TrimReq};
    use std::time::{Duration, Instant};

    #[test]
    fn a_one_clip_trim_on_the_main_track_scales_linearly() {
        fn time(n: usize) -> Duration {
            let mut fx = Fixture::new();
            let req = EditRequest::Insert(InsertReq {
                seq: fx.seq_id(),
                target: InsertTarget::Track(fx.main()),
                at: f(0),
                clips: (0..n).map(|_| NewClip::image(VIDEO_100, f(1))).collect(),
            });
            let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
            fx.editor.commit(plan, CommitMeta::default()).unwrap();
            let first = fx.seq().main_track().unwrap().clips()[0].id;
            let trim = EditRequest::Trim(TrimReq {
                seq: fx.seq_id(),
                clip: first,
                edge: Edge::Out,
                delta: f(1),
                ripple: false,
            });
            (0..3)
                .map(|_| {
                    let t = Instant::now();
                    fx.editor.plan(&fx.ctx(), &trim).unwrap();
                    t.elapsed()
                })
                .min()
                .unwrap()
        }
        let (small, large) = (time(2_000), time(8_000));
        let ratio = large.as_secs_f64() / small.as_secs_f64();
        // 4x the clips: ~4-5x if O(n log n), ~16x if O(n^2). Today: ~14.7x.
        assert!(
            ratio < 8.0,
            "4x the clips cost {ratio:.1}x ({small:?} -> {large:?})"
        );
    }
}

#[allow(unused_imports, dead_code)]
mod f26_multi_clip_ops_quadratic_no_clip_index {
    //! `resolve_clips` runs `Sequence::find_clip(id)` for every requested id

    use std::time::{Duration, Instant};

    use super::common::*;
    use freecut_timeline::CommitMeta;
    use freecut_timeline::edit::{EditRequest, InsertReq, InsertTarget, MoveReq, NewClip};
    use freecut_timeline::model::TextClipData;

    #[test]
    fn select_all_nudge_scales_linearly() {
        fn time(n: usize) -> Duration {
            let mut fx = Fixture::new();
            let req = EditRequest::Insert(InsertReq {
                seq: fx.seq_id(),
                target: InsertTarget::NewTrack,
                at: f(0),
                clips: (0..n)
                    .map(|_| NewClip::text(TextClipData::default(), f(1)))
                    .collect(),
            });
            let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
            fx.editor.commit(plan, CommitMeta::default()).unwrap();
            let all = fx.seq().visual_tracks()[1]
                .clips()
                .iter()
                .map(|c| c.id)
                .collect();
            let nudge = EditRequest::Move(MoveReq {
                seq: fx.seq_id(),
                clips: all,
                delta: f(1),
                track_shift: 0,
            });
            (0..3)
                .map(|_| {
                    let t = Instant::now();
                    fx.editor.plan(&fx.ctx(), &nudge).unwrap();
                    t.elapsed()
                })
                .min()
                .unwrap()
        }
        let (small, large) = (time(2_000), time(8_000));
        let ratio = large.as_secs_f64() / small.as_secs_f64();
        // Today: ~15.6x for 4x the clips.
        assert!(
            ratio < 8.0,
            "4x the clips cost {ratio:.1}x ({small:?} -> {large:?})"
        );
    }
}

#[allow(unused_imports, dead_code)]
mod f27_no_public_document_builders_blocks_m2_load {
    //! Code outside the crate cannot build a Document that holds tracks or clips

    use core::num::NonZeroU64;
    use std::sync::Arc;

    use super::common::*;
    use freecut_timeline::Editor;
    use freecut_timeline::edit::InsertTarget;
    use freecut_timeline::history::HistoryLimits;
    use freecut_timeline::ids::{ClipId, IdAlloc, MediaId, SeqId, TrackId};
    use freecut_timeline::model::{
        Clip, ClipContent, Document, InvariantViolation, Library, Sequence, SequenceFlags, Track,
        TrackHeader, TrackKind,
    };
    use freecut_timeline::time::TimeRange;

    fn id(n: u64) -> NonZeroU64 {
        NonZeroU64::new(n).unwrap()
    }

    // Fails today: E0599, no `from_parts` (or any equivalent) on Track, Sequence or Document.
    #[test]
    fn a_loader_outside_the_crate_can_rebuild_a_saved_document() {
        // What freecut-project's format::map reads from a file (persisted ids, TL-006).
        let clip = Clip::new(
            ClipId::from_raw(id(7)),
            TimeRange::new(f(0), f(5)).unwrap(),
            ClipContent::Image {
                asset: MediaId::from_raw(id(9)),
            },
        );
        let main = Track::from_parts(
            TrackId::from_raw(id(2)),
            TrackKind::Video,
            TrackHeader::default(),
            vec![Arc::new(clip)],
        );
        let seq = Sequence::from_parts(
            SeqId::from_raw(id(1)),
            SequenceFlags::default(),
            vec![Arc::new(main)],
            Vec::new(),
        );
        let doc = Document::from_parts(
            SeqId::from_raw(id(1)),
            settings(),
            Library::default(),
            vec![Arc::new(seq)],
        )
        .unwrap();
        assert!(doc.clip(ClipId::from_raw(id(7))).is_some());
    }

    /// Taking a document apart through its getters and putting it back together
    /// gives an equal document, and the editor accepts it.
    #[test]
    fn rebuilding_an_edited_document_from_its_parts_is_lossless() {
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.insert(
            InsertTarget::Track(main),
            0,
            &[(VIDEO_100, 0, 10), (VIDEO_50, 0, 5)],
        );
        fx.insert(InsertTarget::NewTrack, 3, &[(AUDIO_200, 0, 20)]);
        let original = fx.doc().clone();

        let sequences = original
            .sequences()
            .map(|s| {
                let rebuild = |tracks: &[Arc<Track>]| -> Vec<Arc<Track>> {
                    tracks
                        .iter()
                        .map(|t| {
                            Arc::new(Track::from_parts(
                                t.id,
                                t.kind,
                                t.header.clone(),
                                t.clips().iter().rev().cloned().collect(), // any order
                            ))
                        })
                        .collect()
                };
                Arc::new(Sequence::from_parts(
                    s.id,
                    s.flags,
                    rebuild(s.visual_tracks()),
                    rebuild(s.audio_tracks()),
                ))
            })
            .collect();
        let rebuilt = Document::from_parts(
            original.root(),
            *original.settings(),
            original.library().clone(),
            sequences,
        )
        .unwrap();
        assert_eq!(rebuilt, original);

        let editor = Editor::from_document(rebuilt, IdAlloc::default(), HistoryLimits::default());
        assert_eq!(editor.document(), &original);
        assert_eq!(editor.ids(), fx.editor.ids());
    }

    #[test]
    fn a_document_without_its_root_or_with_a_repeated_sequence_id_is_refused() {
        let seq = |n| {
            Arc::new(Sequence::new(
                SeqId::from_raw(id(n)),
                TrackId::from_raw(id(n + 10)),
            ))
        };
        assert_eq!(
            Document::from_parts(
                SeqId::from_raw(id(1)),
                settings(),
                Library::default(),
                vec![seq(2)]
            ),
            Err(InvariantViolation::RootMissing(SeqId::from_raw(id(1))))
        );
        assert_eq!(
            Document::from_parts(
                SeqId::from_raw(id(1)),
                settings(),
                Library::default(),
                vec![seq(1), seq(1)]
            ),
            Err(InvariantViolation::DuplicateId(1))
        );
    }
}

#[allow(unused_imports, dead_code)]
mod f28_mark_saved_has_no_generation {
    //! `Editor::mark_saved()` always marks the current history state as saved

    use super::common::*;
    use freecut_timeline::edit::InsertTarget;

    #[test]
    fn an_edit_made_while_a_save_runs_stays_dirty() {
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
        // Save: snapshot and its generation on the UI thread; write on a worker.
        let (snapshot, at) = (fx.doc().clone(), fx.editor.generation());
        // The user keeps editing while the worker writes `snapshot`.
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 5)]);
        assert_ne!(fx.doc(), &snapshot);
        fx.editor.mark_saved_at(at);
        assert!(fx.editor.is_dirty(), "the second insert is not in the file");
        fx.editor.undo().unwrap();
        assert!(!fx.editor.is_dirty(), "back at the state that was written");
    }

    #[test]
    fn an_undo_while_a_save_runs_leaves_the_document_dirty() {
        let mut fx = Fixture::new();
        let main = fx.main();
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
        let at = fx.editor.generation(); // the worker writes this state
        fx.editor.undo().unwrap(); // the user undoes while the write runs
        fx.editor.mark_saved_at(at);
        assert!(
            fx.editor.is_dirty(),
            "the file holds the insert, the document does not"
        );
        fx.editor.redo().unwrap();
        assert!(!fx.editor.is_dirty());
    }

    #[test]
    fn a_repaired_load_can_open_dirty() {
        let mut fx = Fixture::new();
        fx.editor.mark_unsaved();
        assert!(fx.editor.is_dirty());
    }

    // After the fix, the same scenarios pass when written against the new API:
    // read `let at = fx.editor.generation();` together with the snapshot, then call
    // `fx.editor.mark_saved_at(at)` instead of `mark_saved()`. They stay dirty, and
    // undoing back to the snapshot's state makes them clean (tests are in the diff).
}

#[allow(unused_imports, dead_code)]
mod f29_save_mid_gesture_splits_and_half_cancels {
    //! `mark_saved()` seals the top entry even when that entry belongs to the open gesture

    use super::common::*;
    use freecut_timeline::CommitMeta;
    use freecut_timeline::edit::{Edge, EditRequest, InsertTarget, TrimReq};
    use freecut_timeline::history::Coalesce;

    #[test]
    fn a_save_finishing_mid_gesture_does_not_split_or_half_cancel_it() {
        let mut fx = Fixture::new();
        let ids = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
        let before = fx.doc().clone();
        let entries = fx.editor.history().len();
        let trim = EditRequest::Trim(TrimReq {
            seq: fx.seq_id(),
            clip: ids[0],
            edge: Edge::Out,
            delta: f(2),
            ripple: false,
        });
        let g = fx.editor.begin_gesture();
        let step = |fx: &mut Fixture| {
            let plan = fx.editor.plan(&fx.ctx(), &trim).unwrap();
            fx.editor
                .commit(
                    plan,
                    CommitMeta {
                        selection_before: Vec::new(),
                        coalesce: Coalesce::Gesture(g),
                    },
                )
                .unwrap();
        };
        step(&mut fx);
        fx.editor.mark_saved(); // background save completes mid-drag
        step(&mut fx);
        fx.editor.cancel_gesture(g).unwrap(); // Esc
        assert_eq!(
            fx.doc(),
            &before,
            "cancel reverts everything the gesture did"
        );
        assert_eq!(fx.editor.history().len(), entries, "and leaves no entry");
    }
}

#[allow(unused_imports, dead_code)]
mod f30_cancel_gesture_drops_redo_stack {
    //! The first commit of a gesture goes through `History::record`, which clears the redo stack (`self.undone.clear()`)

    use super::common::*;
    use freecut_timeline::CommitMeta;
    use freecut_timeline::edit::{Edge, EditRequest, InsertTarget, TrimReq};
    use freecut_timeline::history::Coalesce;

    #[test]
    fn cancelling_a_gesture_keeps_the_redo_stack() {
        let mut fx = Fixture::new();
        let ids = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
        let main = fx.main();
        fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 5)]);
        let doc_after_insert = fx.doc().clone();
        fx.editor.mark_saved();
        fx.editor.undo().unwrap();
        assert!(fx.editor.history().can_redo());
        let doc_before = fx.doc().clone();
        let g = fx.editor.begin_gesture();
        let trim = EditRequest::Trim(TrimReq {
            seq: fx.seq_id(),
            clip: ids[0],
            edge: Edge::Out,
            delta: f(2),
            ripple: false,
        });
        let plan = fx.editor.plan(&fx.ctx(), &trim).unwrap();
        fx.editor
            .commit(
                plan,
                CommitMeta {
                    selection_before: Vec::new(),
                    coalesce: Coalesce::Gesture(g),
                },
            )
            .unwrap();
        fx.editor.cancel_gesture(g).unwrap();
        assert_eq!(fx.doc(), &doc_before, "cancel reverts the document");
        assert!(
            fx.editor.history().can_redo(),
            "a cancelled gesture leaves no trace"
        );
        fx.editor.redo().unwrap();
        assert_eq!(fx.doc(), &doc_after_insert);
        assert!(!fx.editor.is_dirty(), "redo returns to the saved state");
    }
}
