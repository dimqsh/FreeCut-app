//! Files with several audio streams (AU-038) and linked sound out of sync
//! (TL-030): the stream notice, picker and "Add stream as clip", the
//! out-of-sync offset and Resync. Every edit goes through `Fixture::apply`,
//! which also checks the undo/redo round trip.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::sync::Arc;

use common::*;
use freecut_timeline::Committed;
use freecut_timeline::edit::{
    AudioEdit, Edge, EditError, EditRequest, InsertReq, InsertTarget, MoveReq, NewClip, NewMedia,
    Notice, OpKind, SplitReq, SplitTargets, SyncOffset, TrimReq, stream_doubled_by, sync_offset,
    sync_offsets,
};
use freecut_timeline::ids::{ClipId, MediaId};
use freecut_timeline::mix::mix_sources;
use freecut_timeline::model::{
    Clip, ClipContent, EmbeddedAudio, GainDb, MediaKind, MediaSource, StoredPath, StreamSel,
    TrackSlot,
};
use freecut_timeline::time::{FrameRate, Tick, TimeRange};
use pretty_assertions::assert_eq;

// ---------------------------------------------------------------- helpers

fn clip(fx: &Fixture, id: ClipId) -> Arc<Clip> {
    Arc::clone(fx.doc().clip(id).unwrap())
}

fn applied(c: Committed) -> (Vec<Notice>, Vec<ClipId>) {
    match c {
        Committed::Applied { notices, selection } => (notices, selection.unwrap_or_default()),
        Committed::NoOp => panic!("expected a change"),
    }
}

fn stream_of(fx: &Fixture, id: ClipId) -> Option<StreamSel> {
    match clip(fx, id).content {
        ClipContent::Media { stream, .. } => stream,
        _ => panic!("not media"),
    }
}

/// A video of 100 frames with three audio streams (indices 1, 2, 3; the
/// second flagged default) on the main track at 0, source 0..20.
fn three_stream_video(fx: &mut Fixture) -> (MediaId, ClipId) {
    let media = fx.import("/media/dual.mov", media_info_streams(100, true, 3, Some(1)));
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(media, 0, 20)])[..] else {
        panic!()
    };
    (media, v)
}

fn pick(
    fx: &mut Fixture,
    clips: &[ClipId],
    sel: Option<StreamSel>,
) -> Result<Committed, EditError> {
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetAudioStream {
        seq,
        clips: clips.to_vec(),
        sel,
    })
}

fn add_stream(fx: &mut Fixture, clip: ClipId, stream: u32) -> Result<Committed, EditError> {
    let seq = fx.seq_id();
    fx.apply(EditRequest::AddStreamAsClip { seq, clip, stream })
}

fn detach(fx: &mut Fixture, v: ClipId) -> ClipId {
    let seq = fx.seq_id();
    applied(
        fx.apply(EditRequest::DetachAudio {
            seq,
            clips: vec![v],
        })
        .unwrap(),
    )
    .1[0]
}

fn mv(fx: &mut Fixture, clips: &[ClipId], frames: i64) {
    let seq = fx.seq_id();
    fx.apply(EditRequest::Move(MoveReq {
        seq,
        clips: clips.to_vec(),
        delta: f(frames),
        track_shift: 0,
    }))
    .unwrap();
}

fn trim(fx: &mut Fixture, clip: ClipId, edge: Edge, frames: i64) {
    let seq = fx.seq_id();
    fx.apply(EditRequest::Trim(TrimReq {
        seq,
        clip,
        edge,
        delta: f(frames),
        ripple: false,
    }))
    .unwrap();
}

fn magnet_off(fx: &mut Fixture) {
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetMagnet { seq, on: false }).unwrap();
}

fn offset(fx: &Fixture, clip: ClipId) -> Option<SyncOffset> {
    sync_offset(fx.doc(), fx.seq_id(), clip)
}

fn frames(fx: &Fixture, clip: ClipId) -> Option<i64> {
    offset(fx, clip).map(|o| o.frames)
}

fn resync(fx: &mut Fixture, clips: &[ClipId]) -> Result<Committed, EditError> {
    let seq = fx.seq_id();
    fx.apply(EditRequest::Resync {
        seq,
        clips: clips.to_vec(),
    })
}

// ------------------------------------------------------------ the notice

#[test]
fn importing_a_file_with_several_streams_says_how_many_and_which_plays() {
    let mut fx = Fixture::new();
    let req = EditRequest::ImportMedia(vec![
        NewMedia {
            path: StoredPath {
                absolute: "/media/dual.mov".into(),
                relative: None,
            },
            info: media_info_streams(100, true, 3, Some(1)),
            fingerprint: None,
            display_name: None,
        },
        NewMedia {
            path: StoredPath {
                absolute: "/media/plain.mp4".into(),
                relative: None,
            },
            info: media_info(100, true),
            fingerprint: None,
            display_name: None,
        },
        NewMedia {
            path: StoredPath {
                absolute: "/media/two.mka".into(),
                relative: None,
            },
            info: media_info_streams(100, false, 2, None),
            fingerprint: None,
            display_name: None,
        },
    ]);
    let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
    let ids = plan.imported().to_vec();
    let (notices, _) = applied(fx.apply(req).unwrap());
    assert_eq!(
        notices,
        vec![
            Notice::MultipleAudioStreams {
                media: ids[0],
                count: 3,
                default_stream: 2,
            },
            Notice::MultipleAudioStreams {
                media: ids[2],
                count: 2,
                default_stream: 0,
            },
        ]
    );
}

#[test]
fn the_default_stream_is_the_flagged_one_else_the_first() {
    let flagged = media_info_streams(10, true, 3, Some(2));
    assert_eq!(flagged.default_audio().unwrap().index, 3);
    let unflagged = media_info_streams(10, true, 3, None);
    assert_eq!(unflagged.default_audio().unwrap().index, 1);
    assert_eq!(
        unflagged.played_streams(Some(StreamSel::Index(9)))[0].index,
        1,
        "a missing stream falls back to the default"
    );
    assert_eq!(unflagged.played_streams(Some(StreamSel::AllMixed)).len(), 3);
    assert!(
        media_info_with(10, true, false)
            .played_streams(None)
            .is_empty()
    );
}

// ------------------------------------------------------------ the picker

#[test]
fn the_stream_picker_sets_a_stream_all_streams_or_the_default() {
    let mut fx = Fixture::new();
    let (media, v) = three_stream_video(&mut fx);
    let seq = fx.seq_id();
    let window = TimeRange::new(f(0), f(5)).unwrap();
    let streams = |fx: &Fixture| -> Vec<(u32, u32)> {
        mix_sources(fx.doc(), seq, window)
            .iter()
            .map(|s| (s.stream, s.mixed))
            .collect()
    };
    assert_eq!(streams(&fx), vec![(2, 1)], "the flagged default");

    pick(&mut fx, &[v], Some(StreamSel::Index(3))).unwrap();
    assert_eq!(stream_of(&fx, v), Some(StreamSel::Index(3)));
    assert_eq!(streams(&fx), vec![(3, 1)]);
    assert_eq!(
        fx.editor.history().undo_label().unwrap().op,
        OpKind::SetAudioStream
    );

    pick(&mut fx, &[v], Some(StreamSel::AllMixed)).unwrap();
    assert_eq!(streams(&fx), vec![(1, 3), (2, 3), (3, 3)]);
    assert!(
        mix_sources(fx.doc(), seq, window)
            .iter()
            .all(|s| s.clip == v && s.asset == media)
    );

    pick(&mut fx, &[v], None).unwrap();
    assert_eq!(streams(&fx), vec![(2, 1)]);
    assert_eq!(pick(&mut fx, &[v], None).unwrap(), Committed::NoOp);
    // Other settings stay.
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetClipAudio {
        seq,
        clips: vec![v],
        edit: AudioEdit::Gain(GainDb::from_hundredths(-300)),
    })
    .unwrap();
    pick(&mut fx, &[v], Some(StreamSel::Index(1))).unwrap();
    assert_eq!(
        *clip(&fx, v).audio.active().unwrap().gain.static_value(),
        GainDb::from_hundredths(-300)
    );
}

#[test]
fn the_picker_skips_detached_and_missing_streams_with_a_reason() {
    let mut fx = Fixture::new();
    let (media, v) = three_stream_video(&mut fx);
    assert_eq!(
        pick(&mut fx, &[v], Some(StreamSel::Index(7))),
        Err(EditError::UnknownStream { media, stream: 7 })
    );
    let a = detach(&mut fx, v);
    assert_eq!(
        pick(&mut fx, &[v], Some(StreamSel::Index(1))),
        Err(EditError::SoundDetached)
    );
    // The detached clip plays the same file and takes the choice.
    pick(&mut fx, &[v, a], Some(StreamSel::Index(1))).unwrap();
    assert_eq!(stream_of(&fx, a), Some(StreamSel::Index(1)));
    assert_eq!(stream_of(&fx, v), None);
    // Clips without sound.
    let seq = fx.seq_id();
    let t = applied(
        fx.apply(EditRequest::Insert(InsertReq {
            seq,
            target: InsertTarget::NewTrack,
            at: f(0),
            clips: vec![NewClip::text(Default::default(), f(5))],
        }))
        .unwrap(),
    )
    .1[0];
    assert_eq!(pick(&mut fx, &[t], None), Err(EditError::NoClipAudio));
}

#[test]
fn detaching_keeps_the_chosen_stream() {
    let mut fx = Fixture::new();
    let (_, v) = three_stream_video(&mut fx);
    pick(&mut fx, &[v], Some(StreamSel::AllMixed)).unwrap();
    let a = detach(&mut fx, v);
    assert_eq!(stream_of(&fx, a), Some(StreamSel::AllMixed));
    let seq = fx.seq_id();
    let mix = mix_sources(fx.doc(), seq, TimeRange::new(f(0), f(5)).unwrap());
    assert_eq!(mix.len(), 3);
    assert!(mix.iter().all(|s| s.clip == a && s.mixed == 3));
}

#[test]
fn a_chosen_stream_the_relinked_file_lacks_falls_back_to_the_default() {
    let mut fx = Fixture::new();
    let (media, v) = three_stream_video(&mut fx);
    pick(&mut fx, &[v], Some(StreamSel::Index(3))).unwrap();
    fx.apply(EditRequest::UpdateMediaInfo {
        media,
        info: media_info_streams(100, true, 2, None),
    })
    .unwrap();
    assert_eq!(stream_of(&fx, v), Some(StreamSel::Index(3)), "kept");
    let seq = fx.seq_id();
    let mix = mix_sources(fx.doc(), seq, TimeRange::new(f(0), f(5)).unwrap());
    assert_eq!(
        mix.iter().map(|s| s.stream).collect::<Vec<_>>(),
        vec![1],
        "plays the new default"
    );
}

// ------------------------------------------------------ stream as a clip

#[test]
fn a_stream_goes_on_its_own_linked_audio_clip_through_the_extracted_item() {
    let mut fx = Fixture::new();
    let (media, v) = three_stream_video(&mut fx);
    let (_, sel) = applied(add_stream(&mut fx, v, 3).unwrap());
    let s3 = sel[0];
    let video = clip(&fx, v);
    let sound = clip(&fx, s3);
    let link = video.link.expect("a link was made");
    assert_eq!(sound.link, Some(link));
    assert_eq!(sound.range, video.range);
    let ClipContent::Media {
        asset,
        kind,
        source,
        stream,
    } = sound.content.clone()
    else {
        panic!()
    };
    assert_eq!(
        (kind, source, stream),
        (MediaKind::Audio, video.content.source().unwrap(), None)
    );
    let item = fx.doc().library().get(asset).unwrap();
    assert_eq!(
        item.source,
        MediaSource::AudioOf {
            source: media,
            stream: 3
        }
    );
    assert_eq!(fx.doc().find_clip(s3).unwrap().slot, TrackSlot::audio(0));
    // The video keeps playing its own stream; both sound.
    let seq = fx.seq_id();
    let mix = mix_sources(fx.doc(), seq, TimeRange::new(f(0), f(5)).unwrap());
    assert_eq!(
        mix.iter().map(|s| (s.clip, s.stream)).collect::<Vec<_>>(),
        vec![(v, 2), (s3, 3)]
    );
    assert_eq!(
        fx.editor.history().undo_label().unwrap().op,
        OpKind::AddStreamAsClip
    );

    // The next stream goes below, joins the same link, and reuses an
    // extracted item when there is one.
    let extract = EditRequest::ExtractAudio {
        media,
        stream: Some(1),
    };
    let plan = fx.editor.plan(&fx.ctx(), &extract).unwrap();
    let extracted = plan.imported()[0];
    fx.apply(extract).unwrap();
    let s1 = applied(add_stream(&mut fx, v, 1).unwrap()).1[0];
    assert_eq!(clip(&fx, s1).link, Some(link));
    assert_eq!(clip(&fx, s1).content.asset(), Some(extracted));
    assert_eq!(fx.doc().find_clip(s1).unwrap().slot, TrackSlot::audio(1));
    assert_eq!(sync_offset(fx.doc(), seq, s1), None, "in sync");
}

#[test]
fn adding_the_stream_already_playing_is_refused_so_detach_can_be_offered() {
    let mut fx = Fixture::new();
    let (_, v) = three_stream_video(&mut fx);
    assert_eq!(
        add_stream(&mut fx, v, 2),
        Err(EditError::StreamAlreadyPlaying { clip: v, stream: 2 })
    );
    pick(&mut fx, &[v], Some(StreamSel::AllMixed)).unwrap();
    assert_eq!(
        add_stream(&mut fx, v, 1),
        Err(EditError::StreamAlreadyPlaying { clip: v, stream: 1 })
    );
}

#[test]
fn single_stream_media_other_clips_and_unknown_streams_are_refused() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [plain] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    // Only one stream: Detach audio covers it.
    assert_eq!(
        add_stream(&mut fx, plain, 1),
        Err(EditError::SingleStream(VIDEO_100))
    );
    pick(&mut fx, &[plain], Some(StreamSel::AllMixed)).unwrap();
    pick(&mut fx, &[plain], None).unwrap();
    let (media, v) = three_stream_video(&mut fx);
    assert_eq!(
        add_stream(&mut fx, v, 9),
        Err(EditError::UnknownStream { media, stream: 9 })
    );
    let a = detach(&mut fx, v);
    assert_eq!(
        add_stream(&mut fx, a, 1),
        Err(EditError::NotVideo { clip: a })
    );
    let silent = fx.import("/media/silent.mp4", media_info_with(50, true, false));
    let [s] = fx.insert(InsertTarget::Track(main), 0, &[(silent, 0, 10)])[..] else {
        panic!()
    };
    assert_eq!(add_stream(&mut fx, s, 1), Err(EditError::NoSound(silent)));
    let single = fx.import("/media/one.mov", media_info_streams(50, true, 1, None));
    let [one] = fx.insert(InsertTarget::Track(main), 0, &[(single, 0, 5)])[..] else {
        panic!()
    };
    assert_eq!(
        add_stream(&mut fx, one, 2),
        Err(EditError::UnknownStream {
            media: single,
            stream: 2
        })
    );
    assert_eq!(
        add_stream(&mut fx, one, 1),
        Err(EditError::SingleStream(single))
    );
}

#[test]
fn a_stream_of_a_detached_video_joins_its_detached_link_and_restore_takes_it_away() {
    let mut fx = Fixture::new();
    let (_, v) = three_stream_video(&mut fx);
    let d = detach(&mut fx, v);
    let link = clip(&fx, v).link.unwrap();
    assert_eq!(
        add_stream(&mut fx, v, 2),
        Err(EditError::StreamAlreadyPlaying { clip: v, stream: 2 }),
        "the detached clip plays it"
    );
    let s1 = applied(add_stream(&mut fx, v, 1).unwrap()).1[0];
    assert_eq!(clip(&fx, s1).link, Some(link));
    assert_eq!(
        fx.doc().find_clip(s1).unwrap().slot,
        TrackSlot::audio(1),
        "below the detached sound"
    );
    // Restore gives back the detached clip's settings and removes every
    // linked sound (after the app's confirmation).
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetClipAudio {
        seq,
        clips: vec![d],
        edit: AudioEdit::Gain(GainDb::from_hundredths(-700)),
    })
    .unwrap();
    fx.apply(EditRequest::SetClipAudio {
        seq,
        clips: vec![s1],
        edit: AudioEdit::Muted(true),
    })
    .unwrap();
    fx.apply(EditRequest::RestoreAudio {
        seq,
        clips: vec![v],
    })
    .unwrap();
    let audio = clip(&fx, v).audio.active().unwrap().clone();
    assert_eq!(*audio.gain.static_value(), GainDb::from_hundredths(-700));
    assert!(!audio.muted, "settings come from the detached sound");
    assert!(fx.doc().clip(s1).is_none() && fx.doc().clip(d).is_none());
}

#[test]
fn a_dissolved_detached_link_is_reused() {
    let mut fx = Fixture::new();
    let (_, v) = three_stream_video(&mut fx);
    let d = detach(&mut fx, v);
    let link = clip(&fx, v).audio.detached().unwrap();
    let seq = fx.seq_id();
    fx.apply(EditRequest::Delete(freecut_timeline::edit::DeleteReq {
        seq,
        clips: vec![d],
        ripple: false,
    }))
    .unwrap();
    assert_eq!(clip(&fx, v).link, None);
    let s = applied(add_stream(&mut fx, v, 3).unwrap()).1[0];
    assert_eq!(
        (clip(&fx, v).link, clip(&fx, s).link),
        (Some(link), Some(link))
    );
    assert_eq!(clip(&fx, v).audio, EmbeddedAudio::Detached(link));
}

/// The streams `mix_sources` hears at frame 0..5, by clip.
fn heard(fx: &Fixture) -> Vec<(ClipId, u32)> {
    let seq = fx.seq_id();
    mix_sources(fx.doc(), seq, TimeRange::new(f(0), f(5)).unwrap())
        .iter()
        .map(|s| (s.clip, s.stream))
        .collect()
}

#[test]
fn detaching_a_video_keeps_its_streams_added_as_clips_in_the_link() {
    let mut fx = Fixture::new();
    magnet_off(&mut fx);
    let (_, v) = three_stream_video(&mut fx);
    let s3 = applied(add_stream(&mut fx, v, 3).unwrap()).1[0];
    let link = clip(&fx, v).link.unwrap();
    mv(&mut fx, &[s3], 4);
    assert_eq!(frames(&fx, s3), Some(4));
    let d = detach(&mut fx, v);
    assert_eq!(clip(&fx, v).audio, EmbeddedAudio::Detached(link));
    assert_eq!(
        (clip(&fx, d).link, clip(&fx, s3).link),
        (Some(link), Some(link)),
        "the stream clip stays with the detached sound"
    );
    assert_eq!(frames(&fx, s3), Some(4), "the badge still shows");
    assert_eq!(
        add_stream(&mut fx, v, 3),
        Err(EditError::StreamAlreadyPlaying { clip: v, stream: 3 }),
        "never doubled"
    );
    let seq = fx.seq_id();
    assert_eq!(
        freecut_timeline::edit::linked_audio(fx.doc(), seq, v),
        vec![d, s3]
    );
    fx.apply(EditRequest::RestoreAudio {
        seq,
        clips: vec![v],
    })
    .unwrap();
    assert!(fx.doc().clip(s3).is_none() && fx.doc().clip(d).is_none());
    assert_eq!(heard(&fx), vec![(v, 2)]);
}

#[test]
fn a_stream_already_on_a_linked_clip_is_refused() {
    let mut fx = Fixture::new();
    let (_, v) = three_stream_video(&mut fx);
    let s3 = applied(add_stream(&mut fx, v, 3).unwrap()).1[0];
    assert_eq!(
        add_stream(&mut fx, v, 3),
        Err(EditError::StreamAlreadyPlaying { clip: v, stream: 3 })
    );
    assert_eq!(heard(&fx), vec![(v, 2), (s3, 3)]);
    // Once that clip is gone, the stream can go on a clip again.
    let seq = fx.seq_id();
    fx.apply(EditRequest::Delete(freecut_timeline::edit::DeleteReq {
        seq,
        clips: vec![s3],
        ripple: false,
    }))
    .unwrap();
    add_stream(&mut fx, v, 3).unwrap();
}

#[test]
fn a_linked_clip_elsewhere_in_time_does_not_count_as_playing() {
    let mut fx = Fixture::new();
    magnet_off(&mut fx);
    let (_, v) = three_stream_video(&mut fx);
    let s3 = applied(add_stream(&mut fx, v, 3).unwrap()).1[0];
    // Slipped past the video's end: the two never sound together.
    mv(&mut fx, &[s3], 40);
    add_stream(&mut fx, v, 3).unwrap();
}

#[test]
fn detached_sound_counts_by_what_its_clips_play_now() {
    let mut fx = Fixture::new();
    let (_, v) = three_stream_video(&mut fx);
    let d = detach(&mut fx, v);
    pick(&mut fx, &[d], Some(StreamSel::Index(3))).unwrap();
    assert_eq!(heard(&fx), vec![(d, 3)]);
    assert_eq!(
        add_stream(&mut fx, v, 3),
        Err(EditError::StreamAlreadyPlaying { clip: v, stream: 3 }),
        "the detached clip plays it"
    );
    let s2 = applied(add_stream(&mut fx, v, 2).unwrap()).1[0];
    assert_eq!(heard(&fx), vec![(d, 3), (s2, 2)], "nobody played stream 2");
}

#[test]
fn choosing_a_stream_a_linked_clip_already_plays_is_refused() {
    let mut fx = Fixture::new();
    let (_, v) = three_stream_video(&mut fx);
    let s3 = applied(add_stream(&mut fx, v, 3).unwrap()).1[0];
    let doubled = Err(EditError::StreamPlaysElsewhere {
        clip: v,
        stream: 3,
        by: s3,
    });
    assert_eq!(pick(&mut fx, &[v], Some(StreamSel::Index(3))), doubled);
    // "All streams mixed" plays every stream, stream 3 included.
    assert_eq!(pick(&mut fx, &[v], Some(StreamSel::AllMixed)), doubled);
    assert_eq!(heard(&fx), vec![(v, 2), (s3, 3)], "nothing changed");
    pick(&mut fx, &[v], Some(StreamSel::Index(1))).unwrap();
    assert_eq!(heard(&fx), vec![(v, 1), (s3, 3)]);
    // The read-only helper the app's pickers use agrees with the planner.
    let seq = fx.seq_id();
    assert_eq!(
        stream_doubled_by(fx.doc(), seq, v, Some(StreamSel::Index(3))),
        Some(s3)
    );
    assert_eq!(
        stream_doubled_by(fx.doc(), seq, v, Some(StreamSel::AllMixed)),
        Some(s3)
    );
    assert_eq!(stream_doubled_by(fx.doc(), seq, v, None), None);
    assert_eq!(
        stream_doubled_by(fx.doc(), seq, v, Some(StreamSel::Index(1))),
        None,
        "the current choice changes nothing"
    );
}

#[test]
fn a_detached_clip_cannot_pick_a_stream_already_on_its_own_clip() {
    let mut fx = Fixture::new();
    let (_, v) = three_stream_video(&mut fx);
    let d = detach(&mut fx, v);
    let s1 = applied(add_stream(&mut fx, v, 1).unwrap()).1[0];
    assert_eq!(
        pick(&mut fx, &[d], Some(StreamSel::Index(1))),
        Err(EditError::StreamPlaysElsewhere {
            clip: d,
            stream: 1,
            by: s1
        })
    );
    assert_eq!(
        pick(&mut fx, &[d], Some(StreamSel::AllMixed)),
        Err(EditError::StreamPlaysElsewhere {
            clip: d,
            stream: 1,
            by: s1
        })
    );
    pick(&mut fx, &[d], Some(StreamSel::Index(3))).unwrap();
    assert_eq!(heard(&fx), vec![(d, 3), (s1, 1)]);
    let seq = fx.seq_id();
    assert_eq!(
        stream_doubled_by(fx.doc(), seq, d, Some(StreamSel::Index(1))),
        Some(s1)
    );
    // The stream clip itself plays only its stream, whatever is picked.
    assert_eq!(
        stream_doubled_by(fx.doc(), seq, s1, Some(StreamSel::AllMixed)),
        None
    );
}

#[test]
fn a_stream_clip_elsewhere_in_time_does_not_block_the_choice() {
    let mut fx = Fixture::new();
    magnet_off(&mut fx);
    let (_, v) = three_stream_video(&mut fx);
    let s3 = applied(add_stream(&mut fx, v, 3).unwrap()).1[0];
    mv(&mut fx, &[s3], 40);
    pick(&mut fx, &[v], Some(StreamSel::Index(3))).unwrap();
    // Moving it back doubles the stream (moves don't check); choosing the
    // same stream again is a no-op, not a refusal.
    mv(&mut fx, &[s3], -40);
    assert_eq!(
        pick(&mut fx, &[v], Some(StreamSel::Index(3))),
        Ok(Committed::NoOp)
    );
    let seq = fx.seq_id();
    assert_eq!(
        stream_doubled_by(fx.doc(), seq, v, Some(StreamSel::Index(3))),
        None
    );
}

#[test]
fn several_clips_skip_the_ones_that_would_double() {
    let mut fx = Fixture::new();
    let (media, v1) = three_stream_video(&mut fx);
    let s3 = applied(add_stream(&mut fx, v1, 3).unwrap()).1[0];
    let main = fx.main();
    let [v2] = fx.insert(InsertTarget::Track(main), 20, &[(media, 0, 20)])[..] else {
        panic!()
    };
    applied(pick(&mut fx, &[v1, v2], Some(StreamSel::Index(3))).unwrap());
    let label = fx.editor.history().undo_label().unwrap();
    assert_eq!((label.op, label.count), (OpKind::SetAudioStream, 1));
    assert_eq!(stream_of(&fx, v1), None, "skipped: its stream clip plays 3");
    assert_eq!(stream_of(&fx, v2), Some(StreamSel::Index(3)));
    // Doubling is a more specific reason than detached sound.
    let d = detach(&mut fx, v2);
    assert_eq!(
        pick(&mut fx, &[v1, v2], Some(StreamSel::AllMixed)),
        Err(EditError::StreamPlaysElsewhere {
            clip: v1,
            stream: 3,
            by: s3
        })
    );
    // Clips of different links never block each other.
    pick(&mut fx, &[v1, d], Some(StreamSel::Index(1))).unwrap();
    assert_eq!(
        (stream_of(&fx, v1), stream_of(&fx, d)),
        (Some(StreamSel::Index(1)), Some(StreamSel::Index(1)))
    );
}

#[test]
fn an_extracted_item_relinked_to_a_shorter_file_is_not_reused_past_its_end() {
    let mut fx = Fixture::new();
    let (media, v) = three_stream_video(&mut fx);
    let extract = EditRequest::ExtractAudio {
        media,
        stream: Some(1),
    };
    let plan = fx.editor.plan(&fx.ctx(), &extract).unwrap();
    let extracted = plan.imported()[0];
    fx.apply(extract).unwrap();
    // The extracted item alone now reads a 10-frame file; the video clip
    // uses 20 frames of the original.
    fx.apply(EditRequest::RelinkMedia {
        media: extracted,
        path: StoredPath {
            absolute: "/media/short.mov".into(),
            relative: None,
        },
        info: media_info_streams(10, true, 3, Some(1)),
        fingerprint: None,
    })
    .unwrap();
    assert_eq!(add_stream(&mut fx, v, 1), Err(EditError::SourceOutOfRange));
    // A stream without an extracted item still goes on a clip.
    add_stream(&mut fx, v, 3).unwrap();
}

#[test]
fn restore_gives_back_the_stream_the_detached_clip_played() {
    let mut fx = Fixture::new();
    let (_, v) = three_stream_video(&mut fx);
    let d = detach(&mut fx, v);
    pick(&mut fx, &[d], Some(StreamSel::Index(3))).unwrap();
    assert_eq!(heard(&fx), vec![(d, 3)]);
    let seq = fx.seq_id();
    fx.apply(EditRequest::RestoreAudio {
        seq,
        clips: vec![v],
    })
    .unwrap();
    assert_eq!(stream_of(&fx, v), Some(StreamSel::Index(3)));
    assert_eq!(heard(&fx), vec![(v, 3)], "sounds as the detached clip did");
}

#[test]
fn restore_without_a_detached_clip_keeps_the_videos_stream() {
    let mut fx = Fixture::new();
    let (_, v) = three_stream_video(&mut fx);
    pick(&mut fx, &[v], Some(StreamSel::Index(1))).unwrap();
    let d = detach(&mut fx, v);
    let s3 = applied(add_stream(&mut fx, v, 3).unwrap()).1[0];
    let seq = fx.seq_id();
    fx.apply(EditRequest::Delete(freecut_timeline::edit::DeleteReq {
        seq,
        clips: vec![d],
        ripple: false,
    }))
    .unwrap();
    fx.apply(EditRequest::RestoreAudio {
        seq,
        clips: vec![v],
    })
    .unwrap();
    assert!(fx.doc().clip(s3).is_none());
    assert_eq!(stream_of(&fx, v), Some(StreamSel::Index(1)));
}

#[test]
fn a_new_track_for_a_stream_goes_to_the_bottom_of_the_audio_band() {
    let mut fx = Fixture::new();
    let (_, v) = three_stream_video(&mut fx);
    // An occupied audio track.
    fx.insert(InsertTarget::NewTrack, 0, &[(AUDIO_200, 0, 50)]);
    let (notices, sel) = applied(add_stream(&mut fx, v, 1).unwrap());
    let slot = fx.doc().find_clip(sel[0]).unwrap().slot;
    assert_eq!(slot, TrackSlot::audio(1));
    assert!(matches!(notices[..], [Notice::CreatedTrack(_)]));
}

// --------------------------------------------------------- out of sync

#[test]
fn detached_sound_is_in_sync_until_one_side_moves() {
    let mut fx = Fixture::new();
    magnet_off(&mut fx);
    // The sound must stay put when its video moves.
    fx.follow_off();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 10, &[(VIDEO_100, 5, 30)])[..] else {
        panic!()
    };
    let a = detach(&mut fx, v);
    assert_eq!((offset(&fx, a), offset(&fx, v)), (None, None));
    mv(&mut fx, &[a], 12);
    assert_eq!(
        offset(&fx, a),
        Some(SyncOffset {
            frames: 12,
            ticks: f(12),
            partner: v
        })
    );
    assert_eq!(
        offset(&fx, v),
        Some(SyncOffset {
            frames: 12,
            ticks: f(12),
            partner: a
        })
    );
    mv(&mut fx, &[v], 20);
    assert_eq!(frames(&fx, a), Some(-8), "the sound is now early");
    // Moving both together keeps the offset.
    mv(&mut fx, &[v, a], 3);
    assert_eq!(frames(&fx, a), Some(-8));
    let all = sync_offsets(fx.doc(), fx.seq_id());
    assert_eq!(all.get(&a), offset(&fx, a).as_ref());
    assert_eq!(all.get(&v), offset(&fx, v).as_ref());
    assert_eq!(all.len(), 2);
}

#[test]
fn trims_and_splits_keep_sync() {
    let mut fx = Fixture::new();
    magnet_off(&mut fx);
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 10, &[(VIDEO_100, 5, 30)])[..] else {
        panic!()
    };
    let a = detach(&mut fx, v);
    trim(&mut fx, a, Edge::In, 4);
    trim(&mut fx, v, Edge::Out, -6);
    trim(&mut fx, v, Edge::In, 2);
    assert_eq!((offset(&fx, a), offset(&fx, v)), (None, None));
    let seq = fx.seq_id();
    let right = applied(
        fx.apply(EditRequest::Split(SplitReq {
            seq,
            at: f(20),
            targets: SplitTargets::Clips(vec![a]),
        }))
        .unwrap(),
    )
    .1[0];
    assert_eq!((offset(&fx, a), offset(&fx, right)), (None, None));
    // One piece slips; only it and the video show it.
    mv(&mut fx, &[right], 2);
    assert_eq!((frames(&fx, a), frames(&fx, right)), (None, Some(2)));
    assert_eq!(
        offset(&fx, v),
        Some(SyncOffset {
            frames: 2,
            ticks: f(2),
            partner: right
        })
    );
}

#[test]
fn a_split_video_is_measured_against_the_piece_closest_to_sync() {
    let mut fx = Fixture::new();
    magnet_off(&mut fx);
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 30)])[..] else {
        panic!()
    };
    let a = detach(&mut fx, v);
    let seq = fx.seq_id();
    let v2 = applied(
        fx.apply(EditRequest::Split(SplitReq {
            seq,
            at: f(15),
            targets: SplitTargets::Clips(vec![v]),
        }))
        .unwrap(),
    )
    .1[0];
    mv(&mut fx, &[v2], 5);
    assert_eq!(offset(&fx, a), None, "still in sync with the first piece");
    assert_eq!(frames(&fx, v2), Some(-5));
    assert_eq!(offset(&fx, v), None);
}

#[test]
fn offsets_round_to_the_nearest_frame_with_halves_toward_zero() {
    let mut fx = Fixture::new();
    magnet_off(&mut fx);
    // The sound must stay put when its video moves.
    fx.follow_off();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 30)])[..] else {
        panic!()
    };
    let a = detach(&mut fx, v);
    // At 50 fps a 25 fps frame is two frames; a 50 fps move is half of one.
    fx.set_rate(FrameRate::new(50, 1).unwrap());
    let seq = fx.seq_id();
    fx.apply(EditRequest::Move(MoveReq {
        seq,
        clips: vec![a],
        delta: Tick::from_raw(f(1).raw() / 2),
        track_shift: 0,
    }))
    .unwrap();
    fx.set_rate(rate());
    let o = offset(&fx, a).unwrap();
    assert_eq!((o.frames, o.ticks), (0, Tick::from_raw(f(1).raw() / 2)));
    fx.set_rate(FrameRate::new(50, 1).unwrap());
    fx.apply(EditRequest::Move(MoveReq {
        seq,
        clips: vec![v],
        delta: Tick::from_raw(f(1).raw() * 2),
        track_shift: 0,
    }))
    .unwrap();
    fx.set_rate(rate());
    // −1.5 frames: the half goes toward zero.
    assert_eq!(frames(&fx, a), Some(-1));
}

#[test]
fn unlinked_and_foreign_clips_have_no_offset() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 30)])[..] else {
        panic!()
    };
    assert_eq!(offset(&fx, v), None);
    let unknown = ClipId::from_raw(core::num::NonZeroU64::new(9_999).unwrap());
    assert_eq!(offset(&fx, unknown), None);
    assert!(sync_offsets(fx.doc(), fx.seq_id()).is_empty());
}

// ----------------------------------------------------------------- resync

#[test]
fn resync_moves_the_sound_back_on_its_track() {
    let mut fx = Fixture::new();
    magnet_off(&mut fx);
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 10, &[(VIDEO_100, 5, 30)])[..] else {
        panic!()
    };
    let a = detach(&mut fx, v);
    let home = fx.doc().find_clip(a).unwrap().track;
    mv(&mut fx, &[a], 7);
    let entries = fx.editor.history().len();
    resync(&mut fx, &[a]).unwrap();
    assert_eq!(fx.editor.history().len(), entries + 1);
    assert_eq!(clip(&fx, a).start(), f(10));
    assert_eq!(fx.doc().find_clip(a).unwrap().track, home);
    assert_eq!((offset(&fx, a), offset(&fx, v)), (None, None));
    assert_eq!(fx.editor.history().undo_label().unwrap().op, OpKind::Resync);
    assert_eq!(resync(&mut fx, &[a, v]), Err(EditError::NothingToResync));
    assert_eq!(resync(&mut fx, &[]), Err(EditError::NothingSelected));
}

#[test]
fn resync_from_the_video_moves_every_slipped_partner() {
    let mut fx = Fixture::new();
    magnet_off(&mut fx);
    let (_, v) = three_stream_video(&mut fx);
    let s1 = applied(add_stream(&mut fx, v, 1).unwrap()).1[0];
    let s3 = applied(add_stream(&mut fx, v, 3).unwrap()).1[0];
    mv(&mut fx, &[s1], 4);
    mv(&mut fx, &[s3], 9);
    assert_eq!(frames(&fx, v), Some(9), "the largest offset");
    resync(&mut fx, &[v]).unwrap();
    assert_eq!((clip(&fx, s1).start(), clip(&fx, s3).start()), (f(0), f(0)));
    assert_eq!(offset(&fx, v), None);
}

#[test]
fn a_blocked_resync_uses_the_first_free_audio_track() {
    let mut fx = Fixture::new();
    magnet_off(&mut fx);
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 20)])[..] else {
        panic!()
    };
    let a = detach(&mut fx, v);
    mv(&mut fx, &[a], 30);
    // Something now sits where the sound belongs on its track.
    let a0 = fx.track_at(TrackSlot::audio(0));
    fx.insert(InsertTarget::Track(a0), 0, &[(AUDIO_200, 0, 10)]);
    let (notices, _) = applied(resync(&mut fx, &[a]).unwrap());
    assert_eq!(clip(&fx, a).start(), f(0));
    let loc = fx.doc().find_clip(a).unwrap();
    assert_ne!(loc.track, a0);
    assert!(matches!(notices[..], [Notice::CreatedTrack(_)]));
    assert_eq!(offset(&fx, a), None);
}

#[test]
fn resync_refuses_locked_tracks_and_starts_before_zero() {
    let mut fx = Fixture::new();
    magnet_off(&mut fx);
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 20, 20)])[..] else {
        panic!()
    };
    let a = detach(&mut fx, v);
    // Moved later, then its head extended back into the source: its sync
    // point is now before 0.
    mv(&mut fx, &[a], 30);
    trim(&mut fx, a, Edge::In, -15);
    assert_eq!(frames(&fx, a), Some(30));
    assert_eq!(resync(&mut fx, &[a]), Err(EditError::BeforeZero));
    trim(&mut fx, a, Edge::In, 15);
    let track = fx.doc().find_clip(a).unwrap().track;
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetTrackHeader {
        seq,
        track,
        header: freecut_timeline::model::TrackHeader {
            name: None,
            flags: freecut_timeline::model::TrackFlags {
                locked: true,
                ..Default::default()
            },
        },
    })
    .unwrap();
    assert_eq!(resync(&mut fx, &[a]), Err(EditError::TrackLocked { track }));
}
