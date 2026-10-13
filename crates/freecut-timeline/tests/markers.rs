//! Markers (TL-037, toolbar slice T2). Every edit goes through
//! `Fixture::apply`, which checks apply → undo → redo.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    DeleteReq, EditError, EditRequest, InsertReq, InsertTarget, MarkerOwner, MarkerPatch,
    MarkerSelector, MoveReq, NewClip, Notice, SplitReq, SplitTargets, TrimReq,
};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    Document, InvariantViolation, Library, Marker, MarkerOrigin, MarkerRef, MarkerScope,
    PaletteColor, Sequence, TextClipData, TrackFlags, TrackHeader, ValidateCtx, next_marker,
    prev_marker,
};
use freecut_timeline::snap::{SnapIndex, SnapKind};
use freecut_timeline::time::{MarkRange, Tick};
use pretty_assertions::assert_eq;

fn add(fx: &mut Fixture, at: i64, owner: MarkerOwner, color: Option<PaletteColor>) -> MarkerRef {
    let seq = fx.seq_id();
    match fx
        .apply(EditRequest::AddMarker {
            seq,
            at: f(at),
            owner,
            color,
        })
        .unwrap()
    {
        Committed::Applied { notices, .. } => match notices.as_slice() {
            [Notice::AddedMarker(r)] => *r,
            other => panic!("{other:?}"),
        },
        Committed::NoOp => panic!("no marker added"),
    }
}

fn timeline_marker(fx: &Fixture, r: MarkerRef) -> Marker {
    fx.seq().markers.get(r.id()).cloned().unwrap()
}

fn clip_marker(fx: &Fixture, r: MarkerRef) -> Marker {
    let MarkerRef::Clip(c, id) = r else { panic!() };
    fx.doc().clip(c).unwrap().markers.get(id).cloned().unwrap()
}

/// Main track: A 0+10 (source 20) and B 10+10 (source 50).
fn main_clips(fx: &mut Fixture) -> (ClipId, ClipId) {
    let main = fx.main();
    let ids = fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 20, 10), (VIDEO_100, 50, 10)],
    );
    (ids[0], ids[1])
}

#[test]
fn m_adds_a_clip_marker_only_for_one_selected_clip_under_the_playhead() {
    let mut fx = Fixture::new();
    let (a, b) = main_clips(&mut fx);
    let on_clip = add(&mut fx, 3, MarkerOwner::Auto(vec![a]), None);
    let MarkerRef::Clip(owner, _) = on_clip else {
        panic!("{on_clip:?}")
    };
    assert_eq!(owner, a);
    let m = clip_marker(&fx, on_clip);
    assert_eq!(m.at, MarkRange::point(f(23)), "stored in source time");
    assert_eq!((m.name.as_str(), m.color), ("Marker 1", PaletteColor::Blue));
    for selection in [vec![], vec![a, b], vec![b]] {
        let r = add(&mut fx, 4, MarkerOwner::Auto(selection.clone()), None);
        assert!(matches!(r, MarkerRef::Timeline(_)), "{selection:?}");
    }
    assert_eq!(fx.seq().markers.len(), 3);
    let names: Vec<_> = fx.seq().markers.iter().map(|m| m.name.clone()).collect();
    assert_eq!(
        names,
        ["Marker 1", "Marker 2", "Marker 3"],
        "numbered per owner"
    );
}

#[test]
fn the_next_colour_comes_from_the_caller_and_defaults_to_blue() {
    let mut fx = Fixture::new();
    let first = add(&mut fx, 1, MarkerOwner::Timeline, None);
    let last = timeline_marker(&fx, first).color;
    let second = add(&mut fx, 2, MarkerOwner::Timeline, Some(last.next()));
    assert_eq!(timeline_marker(&fx, second).color, PaletteColor::Teal);
}

#[test]
fn a_clip_marker_needs_the_time_inside_an_unlocked_clip() {
    let mut fx = Fixture::new();
    let (a, _) = main_clips(&mut fx);
    let seq = fx.seq_id();
    assert_eq!(
        fx.apply(EditRequest::AddMarker {
            seq,
            at: f(12),
            owner: MarkerOwner::Clip(a),
            color: None,
        }),
        Err(EditError::NotUnderPlayhead)
    );
    let main = fx.main();
    fx.apply(EditRequest::SetTrackHeader {
        seq,
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
        fx.apply(EditRequest::AddMarker {
            seq,
            at: f(2),
            owner: MarkerOwner::Clip(a),
            color: None,
        }),
        Err(EditError::MainTrackLocked)
    );
}

#[test]
fn edit_changes_name_note_colour_and_duration_in_one_entry() {
    let mut fx = Fixture::new();
    let r = add(&mut fx, 5, MarkerOwner::Timeline, None);
    let seq = fx.seq_id();
    let long: String = "x".repeat(130);
    fx.apply(EditRequest::EditMarker {
        seq,
        marker: r,
        patch: MarkerPatch {
            name: Some(format!("  Intro\u{7}{long}")),
            note: Some("n".repeat(2_100)),
            color: Some(PaletteColor::Red),
            duration: Some(Tick::from_raw(f(3).raw() + 7)),
        },
    })
    .unwrap();
    let m = timeline_marker(&fx, r);
    assert_eq!(m.name.chars().count(), 120);
    assert!(m.name.starts_with("Introxx"));
    assert_eq!(m.note.chars().count(), 2_000);
    assert_eq!(m.color, PaletteColor::Red);
    assert_eq!(m.at, MarkRange::new(f(5), f(3)).unwrap(), "quantized");
    // The same values again change nothing.
    let same = fx.apply(EditRequest::EditMarker {
        seq,
        marker: r,
        patch: MarkerPatch {
            color: Some(PaletteColor::Red),
            ..MarkerPatch::default()
        },
    });
    assert_eq!(same.unwrap(), Committed::NoOp);
    // A negative duration makes a point marker.
    fx.apply(EditRequest::EditMarker {
        seq,
        marker: r,
        patch: MarkerPatch {
            duration: Some(f(-2)),
            ..MarkerPatch::default()
        },
    })
    .unwrap();
    assert!(timeline_marker(&fx, r).at.is_point());
}

#[test]
fn moves_are_quantized_and_clamped() {
    let mut fx = Fixture::new();
    let (a, _) = main_clips(&mut fx);
    let seq = fx.seq_id();
    let t = add(&mut fx, 5, MarkerOwner::Timeline, None);
    fx.apply(EditRequest::MoveMarker {
        seq,
        marker: t,
        to: Tick::from_raw(f(8).raw() + f(1).raw() / 3),
    })
    .unwrap();
    assert_eq!(timeline_marker(&fx, t).at.start(), f(8));
    fx.apply(EditRequest::MoveMarker {
        seq,
        marker: t,
        to: f(-4),
    })
    .unwrap();
    assert_eq!(timeline_marker(&fx, t).at.start(), f(0), "stops at 0");
    let c = add(&mut fx, 3, MarkerOwner::Clip(a), None);
    fx.apply(EditRequest::MoveMarker {
        seq,
        marker: c,
        to: f(30),
    })
    .unwrap();
    assert_eq!(
        clip_marker(&fx, c).at.start(),
        f(29),
        "the clip's last frame"
    );
    fx.apply(EditRequest::MoveMarker {
        seq,
        marker: c,
        to: f(-3),
    })
    .unwrap();
    assert_eq!(
        clip_marker(&fx, c).at.start(),
        f(20),
        "the clip's first frame"
    );
    assert_eq!(
        fx.apply(EditRequest::MoveMarker {
            seq,
            marker: MarkerRef::Timeline(c.id()),
            to: f(1),
        }),
        Err(EditError::StaleMarker(c.id()))
    );
}

#[test]
fn each_delete_selector_removes_its_markers_in_one_entry() {
    let mut fx = Fixture::new();
    let (a, b) = main_clips(&mut fx);
    let seq = fx.seq_id();
    let t1 = add(&mut fx, 1, MarkerOwner::Timeline, None);
    let t2 = add(&mut fx, 2, MarkerOwner::Timeline, Some(PaletteColor::Red));
    let c1 = add(&mut fx, 3, MarkerOwner::Clip(a), Some(PaletteColor::Red));
    let c2 = add(&mut fx, 4, MarkerOwner::Clip(a), None);
    let c3 = add(&mut fx, 12, MarkerOwner::Clip(b), None);
    let count = |fx: &Fixture| {
        fx.seq().markers.len()
            + fx.doc().clip(a).unwrap().markers.len()
            + fx.doc().clip(b).unwrap().markers.len()
    };
    assert_eq!(count(&fx), 5);
    let run = |fx: &mut Fixture, which: MarkerSelector| {
        fx.apply(EditRequest::DeleteMarkers { seq, which }).unwrap();
    };
    run(&mut fx, MarkerSelector::Color(PaletteColor::Red));
    assert_eq!(count(&fx), 3, "t2 and c1");
    assert!(fx.seq().markers.get(t2.id()).is_none());
    fx.editor.undo().unwrap();
    run(&mut fx, MarkerSelector::OnClip(a));
    assert_eq!(count(&fx), 3, "c1 and c2");
    fx.editor.undo().unwrap();
    run(&mut fx, MarkerSelector::AllTimeline);
    assert_eq!(count(&fx), 3, "t1 and t2");
    fx.editor.undo().unwrap();
    run(&mut fx, MarkerSelector::Refs(vec![t1, c2, c3, c3]));
    assert_eq!(count(&fx), 2);
    assert!(fx.doc().clip(a).unwrap().markers.get(c1.id()).is_some());
    // Nothing to delete records nothing.
    assert_eq!(
        fx.apply(EditRequest::DeleteMarkers {
            seq,
            which: MarkerSelector::OnClip(b)
        })
        .unwrap(),
        Committed::NoOp
    );
}

#[test]
fn a_split_hands_each_clip_marker_to_the_part_that_shows_it() {
    let mut fx = Fixture::new();
    let (a, _) = main_clips(&mut fx);
    let seq = fx.seq_id();
    let early = add(&mut fx, 2, MarkerOwner::Clip(a), None);
    let late = add(&mut fx, 7, MarkerOwner::Clip(a), None);
    // Hide one marker by trimming the head (in place: the magnet keeps a's
    // start, so its content moves a frame left).
    let hidden = add(&mut fx, 0, MarkerOwner::Clip(a), None);
    fx.apply(EditRequest::Trim(TrimReq {
        seq,
        clip: a,
        edge: freecut_timeline::edit::Edge::In,
        delta: f(1),
        ripple: false,
    }))
    .unwrap();
    let c = fx
        .apply(EditRequest::Split(SplitReq {
            seq,
            at: f(5),
            targets: SplitTargets::Clips(vec![a]),
        }))
        .unwrap();
    let Committed::Applied { selection, .. } = c else {
        panic!()
    };
    let right = selection.unwrap()[0];
    let ids = |c: ClipId| -> BTreeSet<u64> {
        fx.doc()
            .clip(c)
            .unwrap()
            .markers
            .iter()
            .map(|m| m.id.get())
            .collect()
    };
    assert_eq!(ids(a), [early.id().get(), hidden.id().get()].into());
    assert_eq!(ids(right), [late.id().get()].into());
    let r = fx.doc().clip(right).unwrap();
    assert_eq!(
        r.marker_start(r.markers.get(late.id()).unwrap()),
        Some(f(6))
    );
    let l = fx.doc().clip(a).unwrap();
    assert_eq!(
        l.marker_start(l.markers.get(hidden.id()).unwrap()),
        None,
        "still hidden"
    );
}

#[test]
fn ripple_edits_leave_timeline_markers_but_clip_markers_move() {
    let mut fx = Fixture::new();
    let (a, b) = main_clips(&mut fx);
    let seq = fx.seq_id();
    let t = add(&mut fx, 15, MarkerOwner::Timeline, None);
    let c = add(&mut fx, 15, MarkerOwner::Clip(b), None);
    fx.apply(EditRequest::Delete(DeleteReq {
        seq,
        clips: vec![a],
        ripple: true,
    }))
    .unwrap();
    assert_eq!(timeline_marker(&fx, t).at.start(), f(15));
    let clip = fx.doc().clip(b).unwrap();
    assert_eq!(clip.marker_start(&clip_marker(&fx, c)), Some(f(5)));
    fx.apply(EditRequest::Move(MoveReq {
        seq,
        clips: vec![b],
        delta: f(0),
        track_shift: 1,
    }))
    .unwrap();
    let clip = fx.doc().clip(b).unwrap();
    assert_eq!(
        clip.marker_start(&clip_marker(&fx, c)),
        Some(f(5)),
        "moves with it"
    );
}

#[test]
fn markers_on_generated_clips_keep_their_place_through_head_trims() {
    let mut fx = Fixture::new();
    let seq = fx.seq_id();
    fx.apply(EditRequest::Insert(InsertReq {
        seq,
        target: InsertTarget::NewTrack,
        at: f(10),
        clips: vec![NewClip::text(TextClipData::with_content("Hi"), f(10))],
    }))
    .unwrap();
    let text = fx.seq().visual_tracks()[1].clips()[0].id;
    let m = add(&mut fx, 15, MarkerOwner::Clip(text), None);
    fx.apply(EditRequest::Trim(TrimReq {
        seq,
        clip: text,
        edge: freecut_timeline::edit::Edge::In,
        delta: f(3),
        ripple: false,
    }))
    .unwrap();
    let clip = fx.doc().clip(text).unwrap();
    assert_eq!(clip.marker_start(&clip_marker(&fx, m)), Some(f(15)));
}

#[test]
fn snapping_reaches_marker_starts_and_range_ends() {
    let mut fx = Fixture::new();
    let (a, b) = main_clips(&mut fx);
    let seq = fx.seq_id();
    let t = add(&mut fx, 33, MarkerOwner::Timeline, None);
    fx.apply(EditRequest::EditMarker {
        seq,
        marker: t,
        patch: MarkerPatch {
            duration: Some(f(4)),
            ..MarkerPatch::default()
        },
    })
    .unwrap();
    add(&mut fx, 13, MarkerOwner::Clip(b), None);
    let none = BTreeSet::new();
    let index = SnapIndex::build(fx.seq(), None, &none);
    let threshold = f(1).raw() as u64;
    for (from, to) in [(32, 33), (36, 37), (12, 13)] {
        let snap = index.snap(&[f(from)], Tick::ZERO, threshold).unwrap();
        assert_eq!(snap.delta, f(to - from), "{from} → {to}");
    }
    // Dragging b: its own markers don't attract it.
    let exclude: BTreeSet<_> = [b].into();
    let index = SnapIndex::build(fx.seq(), None, &exclude);
    assert!(
        index
            .snap(&[f(12)], Tick::ZERO, f(1).raw() as u64 / 2)
            .is_none()
    );
    let points = SnapIndex::from_points(vec![(f(13), SnapKind::Marker)]);
    assert!(points.snap(&[f(13)], Tick::ZERO, 0).is_some());
    let _ = a;
}

#[test]
fn navigation_stops_at_the_ends_and_skips_hidden_tracks() {
    let mut fx = Fixture::new();
    let (_, b) = main_clips(&mut fx);
    add(&mut fx, 4, MarkerOwner::Timeline, None);
    add(&mut fx, 15, MarkerOwner::Clip(b), None);
    add(&mut fx, 30, MarkerOwner::Timeline, None);
    let seq = fx.seq();
    let scope = MarkerScope::VisibleTracks;
    assert_eq!(next_marker(seq, f(0), scope), Some(f(4)));
    assert_eq!(next_marker(seq, f(4), scope), Some(f(15)));
    assert_eq!(next_marker(seq, f(15), scope), Some(f(30)));
    assert_eq!(next_marker(seq, f(30), scope), None, "no wrap-around");
    assert_eq!(prev_marker(seq, f(30), scope), Some(f(15)));
    assert_eq!(prev_marker(seq, f(4), scope), None);
    assert_eq!(
        next_marker(seq, f(4), MarkerScope::TimelineOnly),
        Some(f(30))
    );
    let seq_id = fx.seq_id();
    let main = fx.main();
    fx.apply(EditRequest::SetTrackHeader {
        seq: seq_id,
        track: main,
        header: TrackHeader {
            name: None,
            flags: TrackFlags {
                hidden: true,
                ..TrackFlags::default()
            },
        },
    })
    .unwrap();
    assert_eq!(
        next_marker(fx.seq(), f(4), MarkerScope::VisibleTracks),
        Some(f(30))
    );
    assert_eq!(
        next_marker(fx.seq(), f(4), MarkerScope::AllTracks),
        Some(f(15))
    );
}

#[test]
fn validation_rejects_beats_on_the_timeline_and_unsorted_lists() {
    let fx = Fixture::new();
    let doc = fx.doc();
    let build = |markers: Vec<Marker>| {
        let mut seq: Sequence = fx.seq().clone();
        seq.markers = freecut_timeline::model::Markers::new(markers);
        Document::from_parts(
            doc.root(),
            *doc.settings(),
            Library::from_parts(doc.library().items().cloned().collect()).unwrap(),
            vec![Arc::new(seq)],
        )
        .unwrap()
    };
    let id = |n| freecut_timeline::ids::MarkerId::from_raw(core::num::NonZeroU64::new(n).unwrap());
    let ctx = ValidateCtx {
        limits: Default::default(),
        id_high_water: None,
    };
    let mut beat = Marker::new(id(50), f(1));
    beat.origin = MarkerOrigin::Beat { auto: true };
    let errors = build(vec![beat]).validate(&ctx).unwrap_err();
    assert!(errors.contains(&InvariantViolation::SeqMarkers { seq: doc.root() }));
    let ok = build(vec![Marker::new(id(50), f(1)), Marker::new(id(51), f(1))]);
    assert_eq!(ok.validate(&ctx), Ok(()));
    let dup = build(vec![Marker::new(id(50), f(1)), Marker::new(id(50), f(2))]);
    assert!(
        dup.validate(&ctx)
            .unwrap_err()
            .contains(&InvariantViolation::DuplicateId(50))
    );
    assert_eq!(ok.max_id().max(51), ok.max_id());
}
