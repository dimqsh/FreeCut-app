//! Behavior of each edit operation. Every edit goes through
//! `Fixture::apply`, which also checks plan purity, validity and the
//! undo/redo round trip.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_timeline::edit::{
    AddTrackReq, DeleteReq, Edge, EditError, EditRequest, InsertReq, InsertTarget, MoveReq,
    NewClip, Notice, SplitReq, SplitTargets, TrackPosition, TrimReq,
};
use freecut_timeline::model::{TextClipData, TrackFlags, TrackHeader, TrackKind, TrackSlot};
use freecut_timeline::time::TimeRange;
use freecut_timeline::{Committed, model::MediaKind};
use pretty_assertions::assert_eq;

fn notices(c: Committed) -> Vec<Notice> {
    match c {
        Committed::Applied { notices, .. } => notices,
        Committed::NoOp => Vec::new(),
    }
}

// ---------------------------------------------------------------- insert

#[test]
fn first_clip_on_the_magnetic_main_track_lands_at_zero() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 40, &[(VIDEO_100, 0, 10)]);
    assert_eq!(fx.render(), "V0*: 6@0+10[0]");
}

#[test]
fn main_track_inserts_use_the_midpoint_cut_rule() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 0, 10), (VIDEO_100, 20, 10)],
    );
    // Frame 4 is in the first half of clip 6: insert before it.
    fx.insert(InsertTarget::Track(main), 4, &[(VIDEO_50, 0, 5)]);
    assert_eq!(fx.render(), "V0*: 8@0+5[0] 6@5+10[0] 7@15+10[20]");
    // Frame 10 is the exact midpoint of clip 6 ([5, 15)): ties go after.
    fx.insert(InsertTarget::Track(main), 10, &[(VIDEO_50, 10, 2)]);
    assert_eq!(
        fx.render(),
        "V0*: 8@0+5[0] 6@5+10[0] 9@15+2[10] 7@17+10[20]"
    );
}

#[test]
fn overlay_collisions_go_to_a_new_track_above() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 30)]);
    fx.insert(InsertTarget::NewTrack, 10, &[(VIDEO_50, 0, 10)]);
    let v1 = fx.track_at(TrackSlot::visual(1));
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::Track(v1),
        at: f(15),
        clips: vec![NewClip::media(
            VIDEO_50,
            MediaKind::Video,
            TimeRange::new(f(0), f(10)).unwrap(),
        )],
    });
    let n = notices(fx.apply(req).unwrap());
    assert!(matches!(n.as_slice(), [Notice::CreatedTrack(_)]));
    assert_eq!(
        fx.render(),
        "V2: 9@15+10[0]\nV1: 7@10+10[0]\nV0*: 6@0+30[0]"
    );
}

#[test]
fn audio_dropped_on_the_main_track_is_redirected_to_an_audio_track() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 3, &[(AUDIO_200, 0, 20)]);
    assert_eq!(fx.render(), "V0*:\nA0: 6@3+20[0]");
}

#[test]
fn generated_items_get_their_own_band_above_video() {
    let mut fx = Fixture::new();
    fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::NewTrack,
        at: f(2),
        clips: vec![NewClip::text(TextClipData::default(), f(3))],
    });
    fx.apply(req).unwrap();
    let kinds: Vec<TrackKind> = fx.seq().visual_tracks().iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![TrackKind::Video, TrackKind::Video, TrackKind::Text]
    );
}

#[test]
fn inserting_unprobed_or_out_of_range_media_is_refused() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let seq = fx.seq_id();
    let req = |asset, src, len| {
        EditRequest::Insert(InsertReq {
            seq,
            target: InsertTarget::Track(main),
            at: f(0),
            clips: vec![NewClip::media(
                asset,
                MediaKind::Video,
                TimeRange::new(f(src), f(len)).unwrap(),
            )],
        })
    };
    assert_eq!(
        fx.apply(req(PROBING, 0, 10)).unwrap_err(),
        EditError::UnknownMedia(PROBING)
    );
    assert_eq!(
        fx.apply(req(VIDEO_50, 45, 10)).unwrap_err(),
        EditError::SourceOutOfRange
    );
    assert_eq!(fx.render(), "V0*:");
}

#[test]
fn media_lengths_round_down_to_whole_frames() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let half_frame = freecut_timeline::time::Tick::from_raw(f(1).raw() / 2);
    let source = TimeRange::new(f(0), f(10).checked_add(half_frame).unwrap()).unwrap();
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::Track(main),
        at: f(0),
        clips: vec![NewClip::media(VIDEO_100, MediaKind::Video, source)],
    });
    fx.apply(req).unwrap();
    assert_eq!(fx.render(), "V0*: 6@0+10[0]");
}

// ------------------------------------------------------------------ move

fn mv(
    fx: &Fixture,
    clips: Vec<freecut_timeline::ids::ClipId>,
    delta: i64,
    shift: i32,
) -> EditRequest {
    EditRequest::Move(MoveReq {
        seq: fx.seq_id(),
        clips,
        delta: f(delta),
        track_shift: shift,
        followers: Vec::new(),
    })
}

#[test]
fn moving_on_the_magnetic_main_track_reorders() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let ids = fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 0, 10), (VIDEO_50, 0, 5), (VIDEO_100, 50, 20)],
    );
    // Without clip 6 the track is [7@0+5, 8@5+20]. A new start of 12 is in
    // the first half of clip 8, so clip 6 goes before it...
    fx.apply(mv(&fx, vec![ids[0]], 12, 0)).unwrap();
    assert_eq!(fx.render(), "V0*: 7@0+5[0] 6@5+10[0] 8@15+20[50]");
    // ...and dragging it 10 frames further puts it past the middle.
    fx.apply(mv(&fx, vec![ids[0]], 10, 0)).unwrap();
    assert_eq!(fx.render(), "V0*: 7@0+5[0] 8@5+20[50] 6@25+10[0]");
}

#[test]
fn moving_a_main_clip_up_closes_the_main_track() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let ids = fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 0, 10), (VIDEO_50, 0, 5)],
    );
    fx.apply(mv(&fx, vec![ids[0]], 3, 1)).unwrap();
    assert_eq!(fx.render(), "V1: 6@3+10[0]\nV0*: 7@0+5[0]");
}

#[test]
fn moving_an_overlay_clip_down_inserts_it_on_the_main_track() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 0, 10), (VIDEO_50, 0, 10)],
    );
    let over = fx.insert(InsertTarget::NewTrack, 30, &[(VIDEO_50, 20, 4)]);
    // Lands at frame 12: first half of the second clip, so before it.
    fx.apply(mv(&fx, over, -18, -1)).unwrap();
    assert_eq!(fx.render(), "V0*: 6@0+10[0] 8@10+4[20] 7@14+10[0]");
}

#[test]
fn moving_into_an_occupied_span_sends_a_single_clip_to_a_new_track() {
    let mut fx = Fixture::new();
    let a = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
    let v1 = fx.track_at(TrackSlot::visual(1));
    fx.insert(InsertTarget::Track(v1), 20, &[(VIDEO_50, 0, 10)]);
    fx.apply(mv(&fx, a, 15, 0)).unwrap();
    // The first clip took id 6 and its new track id 7, so the second clip is 8.
    assert_eq!(fx.render(), "V2: 6@15+10[0]\nV1: 8@20+10[0]\nV0*:");
}

#[test]
fn multi_clip_collisions_are_refused() {
    let mut fx = Fixture::new();
    let a = fx.insert(
        InsertTarget::NewTrack,
        0,
        &[(VIDEO_50, 0, 5), (VIDEO_50, 10, 5)],
    );
    let v1 = fx.track_at(TrackSlot::visual(1));
    fx.insert(InsertTarget::Track(v1), 20, &[(VIDEO_50, 0, 10)]);
    assert!(matches!(
        fx.apply(mv(&fx, a, 12, 0)),
        Err(EditError::Collision { .. })
    ));
}

#[test]
fn non_adjacent_main_clips_cant_move_as_a_block() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let ids = fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_50, 0, 5), (VIDEO_50, 5, 5), (VIDEO_50, 10, 5)],
    );
    assert_eq!(
        fx.apply(mv(&fx, vec![ids[0], ids[2]], 2, 0)).unwrap_err(),
        EditError::NonContiguousMain
    );
}

#[test]
fn overlay_moves_before_zero_are_refused_and_null_moves_are_no_ops() {
    let mut fx = Fixture::new();
    let a = fx.insert(InsertTarget::NewTrack, 5, &[(VIDEO_50, 0, 5)]);
    assert_eq!(
        fx.apply(mv(&fx, a.clone(), -6, 0)).unwrap_err(),
        EditError::BeforeZero
    );
    assert_eq!(fx.apply(mv(&fx, a, 0, 0)).unwrap(), Committed::NoOp);
}

#[test]
fn moving_audio_up_past_the_first_audio_track_is_refused() {
    let mut fx = Fixture::new();
    let a = fx.insert(InsertTarget::NewTrack, 0, &[(AUDIO_200, 0, 5)]);
    assert_eq!(
        fx.apply(mv(&fx, a, 0, 1)).unwrap_err(),
        EditError::TrackOutOfRange
    );
}

// ------------------------------------------------------------------ trim

fn trim(
    fx: &Fixture,
    clip: freecut_timeline::ids::ClipId,
    edge: Edge,
    delta: i64,
    ripple: bool,
) -> EditRequest {
    EditRequest::Trim(TrimReq {
        seq: fx.seq_id(),
        clip,
        edge,
        delta: f(delta),
        ripple,
        partners: Vec::new(),
    })
}

#[test]
fn trims_on_the_magnetic_main_track_ripple() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let ids = fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 10, 10), (VIDEO_50, 0, 5)],
    );
    fx.apply(trim(&fx, ids[0], Edge::Out, -4, false)).unwrap();
    assert_eq!(fx.render(), "V0*: 6@0+6[10] 7@6+5[0]");
    // Head trim keeps the start and slides the content.
    fx.apply(trim(&fx, ids[0], Edge::In, 2, false)).unwrap();
    assert_eq!(fx.render(), "V0*: 6@0+4[12] 7@4+5[0]");
    fx.apply(trim(&fx, ids[0], Edge::In, -12, false)).unwrap();
    assert_eq!(fx.render(), "V0*: 6@0+16[0] 7@16+5[0]");
}

#[test]
fn plain_trims_respect_neighbors_source_and_minimum_length() {
    let mut fx = Fixture::new();
    let ids = fx.insert(
        InsertTarget::NewTrack,
        0,
        &[(VIDEO_50, 40, 5), (VIDEO_50, 0, 5)],
    );
    assert!(matches!(
        fx.apply(trim(&fx, ids[0], Edge::Out, 1, false)),
        Err(EditError::Collision { .. })
    ));
    assert_eq!(
        fx.apply(trim(&fx, ids[0], Edge::In, 5, false)).unwrap_err(),
        EditError::TooShort
    );
    assert!(matches!(
        fx.apply(trim(&fx, ids[1], Edge::Out, 46, false)),
        Err(EditError::SourceLimit { .. })
    ));
    assert!(matches!(
        fx.apply(trim(&fx, ids[1], Edge::In, -1, false)),
        Err(EditError::SourceLimit { .. })
    ));
    fx.apply(trim(&fx, ids[1], Edge::Out, 45, false)).unwrap();
    fx.apply(trim(&fx, ids[0], Edge::In, 2, false)).unwrap();
    let v1 = fx.track_at(TrackSlot::visual(1));
    let rendered = fx.render();
    assert!(
        rendered.starts_with("V1: 6@2+3[42] 7@5+50[0]"),
        "{rendered} ({v1})"
    );
}

#[test]
fn ripple_trim_on_an_overlay_moves_later_clips() {
    let mut fx = Fixture::new();
    let ids = fx.insert(
        InsertTarget::NewTrack,
        0,
        &[(VIDEO_50, 0, 5), (VIDEO_50, 10, 5)],
    );
    fx.apply(trim(&fx, ids[0], Edge::Out, 3, true)).unwrap();
    assert_eq!(fx.render(), "V1: 6@0+8[0] 7@8+5[10]\nV0*:");
}

#[test]
fn zero_trims_are_no_ops_and_sub_frame_trims_snap_to_frames() {
    let mut fx = Fixture::new();
    let ids = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 5)]);
    let tiny = freecut_timeline::time::Tick::from_raw(f(1).raw() / 3);
    let req = EditRequest::Trim(TrimReq {
        seq: fx.seq_id(),
        clip: ids[0],
        edge: Edge::Out,
        delta: tiny,
        ripple: false,
        partners: Vec::new(),
    });
    assert_eq!(fx.apply(req).unwrap(), Committed::NoOp);
}

// ----------------------------------------------------------------- split

fn split(fx: &Fixture, at: i64, targets: SplitTargets) -> EditRequest {
    EditRequest::Split(SplitReq {
        seq: fx.seq_id(),
        at: f(at),
        targets,
    })
}

#[test]
fn split_keeps_the_left_id_and_selects_the_right_part() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let ids = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 20, 10)]);
    let c = fx.apply(split(&fx, 4, SplitTargets::Clips(ids))).unwrap();
    assert!(
        matches!(c, Committed::Applied { selection: Some(ref s), .. } if s.len() == 1 && s[0].get() == 7)
    );
    assert_eq!(fx.render(), "V0*: 6@0+4[20] 7@4+6[24]");
}

#[test]
fn splitting_at_an_edge_does_nothing() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let ids = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)]);
    assert_eq!(
        fx.apply(split(&fx, 10, SplitTargets::Clips(ids.clone())))
            .unwrap(),
        Committed::NoOp
    );
    assert_eq!(
        fx.apply(split(&fx, 0, SplitTargets::Clips(ids))).unwrap(),
        Committed::NoOp
    );
}

#[test]
fn split_all_skips_locked_tracks() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)]);
    fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 10)]);
    let v1 = fx.track_at(TrackSlot::visual(1));
    let locked = TrackHeader {
        name: None,
        flags: TrackFlags {
            locked: true,
            ..TrackFlags::default()
        },
    };
    fx.apply(EditRequest::SetTrackHeader {
        seq: fx.seq_id(),
        track: v1,
        header: locked,
    })
    .unwrap();
    fx.apply(split(&fx, 5, SplitTargets::AllTracks)).unwrap();
    assert_eq!(fx.render(), "V1L: 7@0+10[0]\nV0*: 6@0+5[0] 9@5+5[5]");
}

// ---------------------------------------------------------------- delete

fn delete(fx: &Fixture, clips: Vec<freecut_timeline::ids::ClipId>, ripple: bool) -> EditRequest {
    EditRequest::Delete(DeleteReq {
        seq: fx.seq_id(),
        clips,
        ripple,
    })
}

#[test]
fn delete_closes_the_main_track_but_leaves_overlay_gaps() {
    let mut fx = Fixture::new();
    // Overlays keep their place (TL-025 without TL-029's following).
    fx.follow_off();
    let main = fx.main();
    let m = fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_50, 0, 5), (VIDEO_50, 5, 5), (VIDEO_50, 10, 5)],
    );
    let o = fx.insert(
        InsertTarget::NewTrack,
        0,
        &[(VIDEO_50, 0, 5), (VIDEO_50, 5, 5), (VIDEO_50, 10, 5)],
    );
    fx.apply(delete(&fx, vec![m[1], o[1]], false)).unwrap();
    assert_eq!(
        fx.render(),
        "V1: 9@0+5[0] 11@10+5[10]\nV0*: 6@0+5[0] 8@5+5[10]"
    );
}

#[test]
fn ripple_delete_closes_overlay_spans() {
    let mut fx = Fixture::new();
    let o = fx.insert(
        InsertTarget::NewTrack,
        2,
        &[(VIDEO_50, 0, 5), (VIDEO_50, 5, 5), (VIDEO_50, 10, 5)],
    );
    fx.apply(delete(&fx, vec![o[0], o[1]], true)).unwrap();
    assert_eq!(fx.render(), "V1: 8@2+5[10]\nV0*:");
}

#[test]
fn emptied_tracks_are_removed_and_come_back_on_undo() {
    let mut fx = Fixture::new();
    let o = fx.insert(InsertTarget::NewTrack, 0, &[(VIDEO_50, 0, 5)]);
    let v1 = fx.track_at(TrackSlot::visual(1));
    let n = notices(fx.apply(delete(&fx, o, false)).unwrap());
    assert_eq!(n, vec![Notice::RemovedEmptyTracks(1)]);
    assert_eq!(fx.render(), "V0*:");
    fx.editor.undo().unwrap();
    assert_eq!(fx.track_at(TrackSlot::visual(1)), v1, "same id after undo");
}

// ---------------------------------------------------------------- tracks

#[test]
fn explicitly_added_empty_tracks_stay_and_respect_bands() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.apply(EditRequest::AddTrack(AddTrackReq {
        seq: fx.seq_id(),
        kind: TrackKind::Text,
        position: TrackPosition::Default,
    }))
    .unwrap();
    // "Below the main track" is clamped to directly above it.
    fx.apply(EditRequest::AddTrack(AddTrackReq {
        seq: fx.seq_id(),
        kind: TrackKind::Video,
        position: TrackPosition::Below(main),
    }))
    .unwrap();
    let kinds: Vec<TrackKind> = fx.seq().visual_tracks().iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![TrackKind::Video, TrackKind::Video, TrackKind::Text]
    );
    // An unrelated edit doesn't remove them.
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
    assert_eq!(fx.seq().visual_tracks().len(), 3);
}

#[test]
fn the_main_track_cant_be_removed() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let req = EditRequest::RemoveTrack {
        seq: fx.seq_id(),
        track: main,
    };
    assert_eq!(fx.apply(req).unwrap_err(), EditError::MainTrackRemoval);
}

#[test]
fn locked_main_track_refuses_magnetic_edits() {
    let mut fx = Fixture::new();
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
    let req = EditRequest::Insert(InsertReq {
        seq: fx.seq_id(),
        target: InsertTarget::Track(main),
        at: f(0),
        clips: vec![NewClip::media(
            VIDEO_50,
            MediaKind::Video,
            TimeRange::new(f(0), f(5)).unwrap(),
        )],
    });
    assert_eq!(fx.apply(req).unwrap_err(), EditError::MainTrackLocked);
}

#[test]
fn track_names_are_cleaned() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.apply(EditRequest::SetTrackHeader {
        seq: fx.seq_id(),
        track: main,
        header: TrackHeader {
            name: Some("  B-roll\u{7}  ".into()),
            flags: TrackFlags::default(),
        },
    })
    .unwrap();
    assert_eq!(
        fx.seq().main_track().unwrap().header.name.as_deref(),
        Some("B-roll")
    );
    let blank = TrackHeader {
        name: Some("   ".into()),
        flags: TrackFlags::default(),
    };
    fx.apply(EditRequest::SetTrackHeader {
        seq: fx.seq_id(),
        track: main,
        header: blank,
    })
    .unwrap();
    assert_eq!(fx.seq().main_track().unwrap().header.name, None);
}

#[test]
fn switching_the_magnet_on_closes_gaps_in_one_undo_step() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.apply(EditRequest::SetMagnet {
        seq: fx.seq_id(),
        on: false,
    })
    .unwrap();
    fx.insert(InsertTarget::Track(main), 3, &[(VIDEO_50, 0, 5)]);
    fx.insert(InsertTarget::Track(main), 12, &[(VIDEO_50, 0, 5)]);
    assert_eq!(fx.render(), "V0*: 6@3+5[0] 7@12+5[0]");
    let n = notices(
        fx.apply(EditRequest::SetMagnet {
            seq: fx.seq_id(),
            on: true,
        })
        .unwrap(),
    );
    assert_eq!(n, vec![Notice::ClosedGaps(2)]);
    assert_eq!(fx.render(), "V0*: 6@0+5[0] 7@5+5[0]");
}
