//! Linked selection (TL-057, toolbar slice T3): the pure expansion and
//! edge helpers, and moves of an expanded selection keeping sync.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use core::num::NonZeroU64;
use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    Edge, EditRequest, InsertTarget, MoveReq, Notice, TrimReq, Unavailable, sync_offsets,
};
use freecut_timeline::ids::{ClipId, GroupId, LinkId, MediaId, SeqId, TrackId};
use freecut_timeline::model::{
    Clip, ClipContent, MediaKind, Sequence, SequenceFlags, Track, TrackFlags, TrackHeader,
    TrackKind,
};
use freecut_timeline::select::{Expand, expand_selection, linked_edges};
use freecut_timeline::time::TimeRange;
use pretty_assertions::assert_eq;

fn n(v: u64) -> NonZeroU64 {
    NonZeroU64::new(v).unwrap()
}

fn cid(v: u64) -> ClipId {
    ClipId::from_raw(n(v))
}

fn media(id: u64, kind: MediaKind, start: i64, len: i64, src: i64) -> Clip {
    Clip::new(
        cid(id),
        TimeRange::new(f(start), f(len)).unwrap(),
        ClipContent::Media {
            asset: MediaId::from_raw(n(3)),
            kind,
            source: TimeRange::new(f(src), f(len)).unwrap(),
            stream: None,
        },
    )
}

fn track(id: u64, kind: TrackKind, locked: bool, clips: Vec<Clip>) -> Arc<Track> {
    Arc::new(Track::from_parts(
        TrackId::from_raw(n(id)),
        kind,
        TrackHeader {
            name: None,
            flags: TrackFlags {
                locked,
                ..TrackFlags::default()
            },
        },
        clips.into_iter().map(Arc::new).collect(),
    ))
}

/// V (main) —link 1— A —group 2— X —link 3— Y; Z alone. `lock_a` locks A's
/// track.
fn chain(lock_a: bool) -> Sequence {
    let link = |v| Some(LinkId::from_raw(n(v)));
    let group = |v| Some(GroupId::from_raw(n(v)));
    let mut v = media(10, MediaKind::Video, 0, 10, 0);
    v.link = link(1);
    let mut a = media(11, MediaKind::Audio, 0, 10, 0);
    a.link = link(1);
    a.group = group(2);
    let mut x = media(12, MediaKind::Audio, 20, 5, 0);
    x.group = group(2);
    x.link = link(3);
    let mut y = media(13, MediaKind::Video, 20, 5, 0);
    y.link = link(3);
    let z = media(14, MediaKind::Video, 40, 5, 0);
    Sequence::from_parts(
        SeqId::from_raw(n(1)),
        SequenceFlags::default(),
        vec![
            track(2, TrackKind::Video, false, vec![v, z]),
            track(3, TrackKind::Video, false, vec![y]),
        ],
        vec![
            track(4, TrackKind::Audio, lock_a, vec![a]),
            track(5, TrackKind::Audio, false, vec![x]),
        ],
    )
}

fn set(ids: &[u64]) -> BTreeSet<ClipId> {
    ids.iter().map(|v| cid(*v)).collect()
}

#[test]
fn expansion_follows_links_and_groups_to_a_fixed_point() {
    let seq = chain(false);
    let both = Expand {
        groups: true,
        links: true,
    };
    assert_eq!(
        expand_selection(&seq, &set(&[10]), both),
        set(&[10, 11, 12, 13])
    );
    assert_eq!(
        expand_selection(&seq, &set(&[13]), both),
        set(&[10, 11, 12, 13])
    );
    assert_eq!(
        expand_selection(
            &seq,
            &set(&[10]),
            Expand {
                groups: false,
                links: true
            }
        ),
        set(&[10, 11])
    );
    assert_eq!(
        expand_selection(
            &seq,
            &set(&[11]),
            Expand {
                groups: true,
                links: false
            }
        ),
        set(&[11, 12])
    );
    assert_eq!(expand_selection(&seq, &set(&[14]), both), set(&[14]));
}

#[test]
fn with_the_preference_off_only_the_picked_clips_are_selected() {
    let seq = chain(false);
    let off = Expand {
        groups: false,
        links: false,
    };
    assert_eq!(expand_selection(&seq, &set(&[10, 12]), off), set(&[10, 12]));
}

#[test]
fn partners_on_locked_tracks_are_skipped() {
    let seq = chain(true);
    let both = Expand {
        groups: true,
        links: true,
    };
    assert_eq!(expand_selection(&seq, &set(&[10]), both), set(&[10]));
    // A locked clip picked on purpose keeps its unlocked partners.
    assert_eq!(
        expand_selection(&seq, &set(&[11]), both),
        set(&[10, 11, 12, 13])
    );
}

#[test]
fn partners_trim_together_only_when_their_edge_is_aligned_and_in_sync() {
    let link = Some(LinkId::from_raw(n(1)));
    let mut v = media(10, MediaKind::Video, 0, 10, 0);
    v.link = link;
    let mut aligned = media(11, MediaKind::Audio, 0, 10, 0);
    aligned.link = link;
    let mut shorter = media(12, MediaKind::Audio, 0, 8, 0);
    shorter.link = link;
    let mut slipped = media(13, MediaKind::Audio, 0, 10, 5);
    slipped.link = link;
    let mut locked = media(14, MediaKind::Audio, 0, 10, 0);
    locked.link = link;
    let seq = Sequence::from_parts(
        SeqId::from_raw(n(1)),
        SequenceFlags::default(),
        vec![track(2, TrackKind::Video, false, vec![v])],
        vec![
            track(3, TrackKind::Audio, false, vec![aligned]),
            track(4, TrackKind::Audio, false, vec![shorter]),
            track(5, TrackKind::Audio, false, vec![slipped]),
            track(6, TrackKind::Audio, true, vec![locked]),
        ],
    );
    assert_eq!(linked_edges(&seq, cid(10), Edge::Out), vec![cid(11)]);
    assert_eq!(
        linked_edges(&seq, cid(10), Edge::In),
        vec![cid(11), cid(12)]
    );
    assert_eq!(linked_edges(&seq, cid(12), Edge::Out), Vec::<ClipId>::new());
    assert_eq!(linked_edges(&seq, cid(99), Edge::Out), Vec::<ClipId>::new());
}

fn detach(fx: &mut Fixture, v: ClipId) -> ClipId {
    let seq = fx.seq_id();
    match fx
        .apply(EditRequest::DetachAudio {
            seq,
            clips: vec![v],
        })
        .unwrap()
    {
        Committed::Applied { selection, .. } => selection.unwrap()[0],
        Committed::NoOp => panic!(),
    }
}

#[test]
fn moving_a_detached_video_with_its_sound_keeps_them_in_sync() {
    for follow in [true, false] {
        let mut fx = Fixture::new();
        if !follow {
            fx.follow_off();
        }
        let main = fx.main();
        let ids = fx.insert(
            InsertTarget::Track(main),
            0,
            &[(VIDEO_100, 0, 10), (VIDEO_100, 30, 20)],
        );
        let (v, b) = (ids[0], ids[1]);
        let sound = detach(&mut fx, v);
        let seq = fx.seq_id();
        let picked: BTreeSet<_> = [v].into();
        let both = expand_selection(
            fx.seq(),
            &picked,
            Expand {
                groups: true,
                links: true,
            },
        );
        assert_eq!(both, [v, sound].into());
        // A reorder on the magnetic main track: the video lands after b, and
        // its sound moves by the video's change (20 frames), not the
        // pointer's 23.
        fx.apply(EditRequest::Move(MoveReq {
            seq,
            clips: both.iter().copied().collect(),
            delta: f(23),
            track_shift: 0,
            followers: Vec::new(),
        }))
        .unwrap();
        assert_eq!(fx.doc().clip(b).unwrap().start(), f(0));
        assert_eq!(fx.doc().clip(v).unwrap().start(), f(20));
        assert_eq!(fx.doc().clip(sound).unwrap().start(), f(20));
        assert!(sync_offsets(fx.doc(), seq).is_empty(), "follow {follow}");
    }
}

fn trim(fx: &Fixture, clip: ClipId, edge: Edge, delta: i64, partners: Vec<ClipId>) -> EditRequest {
    EditRequest::Trim(TrimReq {
        seq: fx.seq_id(),
        clip,
        edge,
        delta: f(delta),
        ripple: false,
        partners,
    })
}

/// Main track: A 0+10 (source 0) and B 10+10 (source 30), A's sound
/// detached onto A0.
fn main_with_sound(fx: &mut Fixture) -> (ClipId, ClipId, ClipId) {
    let main = fx.main();
    let ids = fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 0, 10), (VIDEO_100, 30, 10)],
    );
    let sound = detach(fx, ids[0]);
    (ids[0], ids[1], sound)
}

#[test]
fn an_in_trim_on_the_magnetic_main_track_trims_the_linked_sound_in_place() {
    for follow in [true, false] {
        let mut fx = Fixture::new();
        if !follow {
            fx.follow_off();
        }
        let (a, b, sound) = main_with_sound(&mut fx);
        let seq = fx.seq_id();
        assert_eq!(linked_edges(fx.seq(), a, Edge::In), vec![sound]);
        let n = history_len(&fx);
        fx.apply(trim(&fx, a, Edge::In, 4, vec![sound])).unwrap();
        assert_eq!(
            fx.render(),
            format!(
                "V0*: {}@0+6[4] {}@6+10[30]\nA0: {}@0+6[4]",
                a.get(),
                b.get(),
                sound.get()
            ),
            "follow {follow}"
        );
        assert!(sync_offsets(fx.doc(), seq).is_empty(), "follow {follow}");
        assert_eq!(history_len(&fx), n + 1, "one entry");
        // Dragging the sound's In edge trims the video in place too.
        fx.editor.undo().unwrap();
        fx.apply(trim(&fx, sound, Edge::In, 2, vec![a])).unwrap();
        assert_eq!(fx.doc().clip(sound).unwrap().start(), f(0));
        assert_eq!(fx.doc().clip(a).unwrap().duration(), f(8));
        assert!(sync_offsets(fx.doc(), seq).is_empty(), "follow {follow}");
    }
}

#[test]
fn an_out_trim_on_the_magnetic_main_track_trims_the_linked_sound_with_it() {
    for follow in [true, false] {
        let mut fx = Fixture::new();
        if !follow {
            fx.follow_off();
        }
        let (a, b, sound) = main_with_sound(&mut fx);
        let seq = fx.seq_id();
        fx.apply(trim(&fx, a, Edge::Out, -3, vec![sound])).unwrap();
        assert_eq!(
            fx.render(),
            format!(
                "V0*: {}@0+7[0] {}@7+10[30]\nA0: {}@0+7[0]",
                a.get(),
                b.get(),
                sound.get()
            ),
            "follow {follow}"
        );
        assert!(sync_offsets(fx.doc(), seq).is_empty(), "follow {follow}");
    }
}

#[test]
fn a_partner_without_room_is_skipped_and_reported() {
    let mut fx = Fixture::new();
    let (a, _, sound) = main_with_sound(&mut fx);
    // Music right after the sound: the sound can't grow into it.
    let track = fx.doc().find_clip(sound).unwrap().track;
    fx.insert(InsertTarget::Track(track), 10, &[(AUDIO_200, 0, 5)]);
    fx.apply(trim(&fx, a, Edge::In, 4, vec![sound])).unwrap();
    let c = fx.apply(trim(&fx, a, Edge::In, -2, vec![sound])).unwrap();
    let Committed::Applied { notices, .. } = c else {
        panic!("expected a change")
    };
    assert!(
        notices.contains(&Notice::Skipped(vec![(sound, Unavailable::NoRoom)])),
        "{notices:?}"
    );
}

#[test]
fn only_the_clicked_member_changes_track_its_partners_keep_theirs() {
    let mut fx = Fixture::new();
    let (a, _, sound) = main_with_sound(&mut fx);
    let seq = fx.seq_id();
    let sound_track = fx.doc().find_clip(sound).unwrap().track;
    // Main → a new overlay track: the sound stays on A0.
    fx.apply(EditRequest::Move(MoveReq {
        seq,
        clips: vec![a, sound],
        delta: f(0),
        track_shift: 1,
        followers: vec![sound],
    }))
    .unwrap();
    let a_track = fx.doc().find_clip(a).unwrap().track;
    assert_ne!(a_track, fx.main());
    assert_eq!(fx.doc().find_clip(sound).unwrap().track, sound_track);
    assert!(sync_offsets(fx.doc(), seq).is_empty());
    // Back down onto the main track with music on A1: the sound stays on
    // A0 and follows the video's change in time.
    fx.insert(InsertTarget::NewTrack, 40, &[(AUDIO_200, 0, 5)]);
    fx.apply(EditRequest::Move(MoveReq {
        seq,
        clips: vec![a, sound],
        delta: f(0),
        track_shift: -1,
        followers: vec![sound],
    }))
    .unwrap();
    assert_eq!(fx.doc().find_clip(a).unwrap().track, fx.main());
    assert_eq!(fx.doc().find_clip(sound).unwrap().track, sound_track);
    assert!(sync_offsets(fx.doc(), seq).is_empty(), "{}", fx.render());
}

fn history_len(fx: &Fixture) -> usize {
    fx.editor.history().len()
}
