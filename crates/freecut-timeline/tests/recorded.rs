//! Placing a recorded voiceover take (AU-027, toolbar slice T6b). Every
//! edit goes through `Fixture::apply`, which checks apply → undo → redo.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use core::time::Duration;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    EditError, EditRequest, InsertTarget, NewMedia, OpKind, RecordTarget, first_free_audio_track,
    recorded_start,
};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    ClipContent, FollowKind, GeneratedMediaKind, MediaOrigin, StoredPath, TrackFlags, TrackHeader,
    follow_kind,
};
use freecut_timeline::time::{SampleRate, Tick, TimeRange};
use pretty_assertions::assert_eq;

fn take(frames: i64) -> NewMedia {
    NewMedia {
        path: StoredPath {
            absolute: "/project/Media/Recordings/Voiceover 1.wav".into(),
            relative: Some("Media/Recordings/Voiceover 1.wav".into()),
        },
        info: media_info(frames, false),
        fingerprint: None,
        display_name: None,
    }
}

fn record(
    fx: &mut Fixture,
    at: Tick,
    handle_in: Tick,
    target: RecordTarget,
    replace: Option<ClipId>,
) -> Result<ClipId, EditError> {
    let seq = fx.seq_id();
    match fx.apply(EditRequest::InsertRecordedClip {
        seq,
        media: take(40),
        at,
        handle_in,
        target,
        replace,
    })? {
        Committed::Applied { selection, .. } => Ok(selection.unwrap()[0]),
        Committed::NoOp => panic!("no take placed"),
    }
}

fn source(fx: &Fixture, id: ClipId) -> TimeRange {
    fx.doc().clip(id).unwrap().content.source().unwrap()
}

#[test]
fn a_take_goes_on_the_first_free_audio_track_as_one_entry() {
    let mut fx = Fixture::new();
    fx.insert(InsertTarget::NewTrack, 0, &[(AUDIO_200, 0, 10)]);
    let busy = fx.seq().audio_tracks()[0].id;
    let first = record(&mut fx, f(5), f(2), RecordTarget::FirstFree, None).unwrap();
    let clip = fx.doc().clip(first).unwrap();
    assert_eq!((clip.start(), clip.duration()), (f(5), f(38)));
    assert_eq!(source(&fx, first), TimeRange::new(f(2), f(38)).unwrap());
    let track = fx.doc().find_clip(first).unwrap().track;
    assert_ne!(track, busy, "the first track is busy at 5");
    // The library item is a voiceover and the clip counts as speech.
    let ClipContent::Media { asset, .. } = clip.content else {
        panic!()
    };
    let item = fx.doc().library().get(asset).unwrap();
    assert_eq!(
        item.origin,
        MediaOrigin::Generated {
            kind: GeneratedMediaKind::Voiceover
        }
    );
    let (_, t) = fx.seq().find_track(track).unwrap();
    assert_eq!(follow_kind(clip, t, fx.doc().library()), FollowKind::Speech);
    // One undo removes the clip, the new track and the library item.
    let label = fx.editor.undo().unwrap().label;
    assert_eq!(label.op, OpKind::InsertRecordedClip);
    assert!(fx.doc().library().get(asset).is_none());
    assert_eq!(fx.seq().audio_tracks().len(), 1);
}

#[test]
fn stacked_takes_and_the_new_track_target() {
    let mut fx = Fixture::new();
    let a = record(&mut fx, f(0), f(0), RecordTarget::FirstFree, None).unwrap();
    let b = record(&mut fx, f(0), f(0), RecordTarget::FirstFree, None).unwrap();
    let tracks: Vec<_> = fx.seq().audio_tracks().iter().map(|t| t.id).collect();
    assert_eq!(tracks.len(), 2);
    assert_eq!(fx.doc().find_clip(a).unwrap().track, tracks[0]);
    assert_eq!(
        fx.doc().find_clip(b).unwrap().track,
        tracks[1],
        "below the first take"
    );
    // A new track always goes on top of the audio tracks, even with room.
    let c = record(&mut fx, f(100), f(0), RecordTarget::NewTrack, None).unwrap();
    assert_eq!(
        fx.doc().find_clip(c).unwrap().slot,
        freecut_timeline::model::TrackSlot::audio(0)
    );
    assert_eq!(fx.seq().audio_tracks().len(), 3);
}

#[test]
fn locked_tracks_are_skipped() {
    let mut fx = Fixture::new();
    let a = record(&mut fx, f(0), f(0), RecordTarget::FirstFree, None).unwrap();
    let track = fx.doc().find_clip(a).unwrap().track;
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
    let span = TimeRange::new(f(100), f(10)).unwrap();
    assert_eq!(first_free_audio_track(fx.seq(), span), None);
    let b = record(&mut fx, f(100), f(0), RecordTarget::FirstFree, None).unwrap();
    assert_ne!(fx.doc().find_clip(b).unwrap().track, track);
}

#[test]
fn redo_last_take_replaces_the_old_take_in_one_entry() {
    let mut fx = Fixture::new();
    let first = record(&mut fx, f(3), f(0), RecordTarget::FirstFree, None).unwrap();
    let track = fx.doc().find_clip(first).unwrap().track;
    let library = fx.doc().library().len();
    let second = record(&mut fx, f(3), f(0), RecordTarget::FirstFree, Some(first)).unwrap();
    assert!(fx.doc().clip(first).is_none());
    assert_eq!(
        fx.doc().find_clip(second).unwrap().track,
        track,
        "the freed track"
    );
    assert_eq!(fx.doc().library().len(), library + 1, "the old file stays");
    fx.editor.undo().unwrap();
    assert!(fx.doc().clip(first).is_some());
    assert!(fx.doc().clip(second).is_none());
}

#[test]
fn placement_past_the_end_and_before_zero() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 10)]);
    let late = record(&mut fx, f(500), f(0), RecordTarget::FirstFree, None).unwrap();
    assert_eq!(fx.doc().clip(late).unwrap().start(), f(500));
    assert_eq!(fx.seq().duration(), f(540), "the timeline grows");
    // A latency-compensated start before 0 drops the part before 0.
    let early = record(&mut fx, f(-3), f(1), RecordTarget::FirstFree, None).unwrap();
    assert_eq!(fx.doc().clip(early).unwrap().start(), f(0));
    assert_eq!(source(&fx, early), TimeRange::new(f(4), f(36)).unwrap());
}

#[test]
fn an_off_grid_start_lands_on_the_grid_without_moving_the_sound() {
    let mut fx = Fixture::new();
    let third = Tick::from_raw(f(1).raw() / 3);
    let at = Tick::from_raw(f(10).raw() + third.raw());
    let id = record(&mut fx, at, f(2), RecordTarget::FirstFree, None).unwrap();
    let clip = fx.doc().clip(id).unwrap();
    assert_eq!(clip.start(), f(10));
    // The in-point moved by the same third of a frame: the file sample at
    // `at` still plays at `at`.
    let src = source(&fx, id);
    assert_eq!(src.start(), Tick::from_raw(f(2).raw() - third.raw()));
    assert_eq!(clip.duration(), f(38), "whole frames that fit");
    // Rounding down would need sound from before the file's start: with no
    // handle the start moves to the next frame instead.
    let at = Tick::from_raw(f(20).raw() + third.raw());
    let id = record(&mut fx, at, Tick::ZERO, RecordTarget::FirstFree, None).unwrap();
    let clip = fx.doc().clip(id).unwrap();
    assert_eq!(clip.start(), f(21));
    assert_eq!(
        source(&fx, id).start(),
        Tick::from_raw(f(1).raw() - third.raw())
    );
}

#[test]
fn a_take_too_short_for_a_frame_is_refused() {
    let mut fx = Fixture::new();
    assert_eq!(
        record(&mut fx, f(0), f(40), RecordTarget::FirstFree, None),
        Err(EditError::TooShort)
    );
    assert_eq!(fx.doc().library().len(), 3);
}

#[test]
fn recorded_start_compensates_latency_on_the_sample_grid() {
    let rate = SampleRate::new(48_000).unwrap();
    let start = f(100);
    let placed = recorded_start(start, Duration::from_millis(10), 0, rate);
    assert_eq!(start.raw() - placed.raw(), rate.sample_ticks().raw() * 480);
    let back = recorded_start(start, Duration::from_millis(10), 10, rate);
    assert_eq!(back, start, "the user offset can undo the latency");
}
