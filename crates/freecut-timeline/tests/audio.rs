//! Clip sound: detach and restore (TL-030, AU-003), clip settings (AU-005,
//! AU-006, AU-007), extracted audio items (AU-004, PM-041), audio placement
//! (AU-001, TL-012) and the mix view (AU-010, AU-011). Every edit goes
//! through `Fixture::apply`, which also checks the undo/redo round trip.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::sync::Arc;

use common::*;
use freecut_timeline::edit::{
    AudioEdit, DeleteReq, EditError, EditRequest, InsertReq, InsertTarget, MoveReq, NewClip,
    Notice, OpKind, SplitReq, SplitTargets, linked_audio,
};
use freecut_timeline::history::Coalesce;
use freecut_timeline::ids::{ClipId, MediaId, TrackId};
use freecut_timeline::mix::{MixSource, mix_sources};
use freecut_timeline::model::{
    Clip, ClipAudio, ClipContent, Document, EmbeddedAudio, GainDb, InvariantViolation, Library,
    MasterAudio, MediaInfo, MediaKind, MediaSource, Sequence, TextClipData, Track, TrackAudio,
    TrackFlags, TrackHeader, TrackKind, TrackSlot, ValidateCtx,
};
use freecut_timeline::time::{Tick, TimeRange};
use freecut_timeline::{CommitMeta, Committed};
use pretty_assertions::assert_eq;

// ---------------------------------------------------------------- helpers

fn clip(fx: &Fixture, id: ClipId) -> Arc<Clip> {
    Arc::clone(fx.doc().clip(id).unwrap())
}

fn sound(fx: &Fixture, id: ClipId) -> EmbeddedAudio {
    clip(fx, id).audio.clone()
}

fn settings(fx: &Fixture, id: ClipId) -> ClipAudio {
    clip(fx, id).audio.active().unwrap().clone()
}

fn applied(c: Committed) -> (Vec<Notice>, Vec<ClipId>) {
    match c {
        Committed::Applied { notices, selection } => (notices, selection.unwrap_or_default()),
        Committed::NoOp => panic!("expected a change"),
    }
}

fn set(fx: &mut Fixture, clips: &[ClipId], edit: AudioEdit) -> Result<Committed, EditError> {
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetClipAudio {
        seq,
        clips: clips.to_vec(),
        edit,
    })
}

fn detach(fx: &mut Fixture, clips: &[ClipId]) -> Result<Committed, EditError> {
    let seq = fx.seq_id();
    fx.apply(EditRequest::DetachAudio {
        seq,
        clips: clips.to_vec(),
    })
}

fn restore(fx: &mut Fixture, clips: &[ClipId]) -> Result<Committed, EditError> {
    let seq = fx.seq_id();
    fx.apply(EditRequest::RestoreAudio {
        seq,
        clips: clips.to_vec(),
    })
}

fn extract(fx: &mut Fixture, media: MediaId, stream: Option<u32>) -> MediaId {
    let req = EditRequest::ExtractAudio { media, stream };
    let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
    let id = plan.imported()[0];
    fx.apply(req).unwrap();
    id
}

fn insert_one(fx: &mut Fixture, target: InsertTarget, at: i64, new: NewClip) -> ClipId {
    let seq = fx.seq_id();
    let c = fx
        .apply(EditRequest::Insert(InsertReq {
            seq,
            target,
            at: f(at),
            clips: vec![new],
        }))
        .unwrap();
    applied(c).1[0]
}

fn media_clip(asset: MediaId, kind: MediaKind, src: i64, len: i64) -> NewClip {
    NewClip::media(asset, kind, TimeRange::new(f(src), f(len)).unwrap())
}

fn text(len: i64) -> NewClip {
    NewClip::text(TextClipData::default(), f(len))
}

fn set_flags(fx: &mut Fixture, track: TrackId, flags: TrackFlags) {
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetTrackHeader {
        seq,
        track,
        header: TrackHeader { name: None, flags },
    })
    .unwrap();
}

fn db(hundredths: i32) -> GainDb {
    GainDb::from_hundredths(hundredths)
}

/// The document with `edit` applied to clip `id`, rebuilt from its parts
/// the way a loader would.
fn rebuilt(doc: &Document, id: ClipId, edit: impl Fn(&mut Clip)) -> Document {
    let track = |t: &Arc<Track>| {
        let clips = t
            .clips()
            .iter()
            .map(|c| {
                let mut c = (**c).clone();
                if c.id == id {
                    edit(&mut c);
                }
                Arc::new(c)
            })
            .collect();
        Arc::new(Track::from_parts(t.id, t.kind, t.header.clone(), clips))
    };
    let seqs = doc
        .sequences()
        .map(|s| {
            Arc::new(Sequence::from_parts(
                s.id,
                s.flags,
                s.visual_tracks().iter().map(track).collect(),
                s.audio_tracks().iter().map(track).collect(),
            ))
        })
        .collect();
    Document::from_parts(doc.root(), *doc.settings(), doc.library().clone(), seqs).unwrap()
}

fn violations(doc: &Document) -> Vec<InvariantViolation> {
    let ctx = ValidateCtx {
        limits: Default::default(),
        id_high_water: None,
    };
    doc.validate(&ctx).err().unwrap_or_default()
}

// ---------------------------------------------------------------- insert

#[test]
fn video_with_sound_plays_it_and_silent_video_has_no_sound_settings() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let silent = fx.import("/media/silent.mov", media_info_with(80, true, false));
    let [a] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    let b = insert_one(
        &mut fx,
        InsertTarget::Track(main),
        10,
        media_clip(silent, MediaKind::Video, 0, 10),
    );
    assert_eq!(sound(&fx, a), EmbeddedAudio::Active(ClipAudio::default()));
    assert_eq!(sound(&fx, b), EmbeddedAudio::None);
    let t = insert_one(&mut fx, InsertTarget::NewTrack, 0, text(5));
    assert_eq!(sound(&fx, t), EmbeddedAudio::None);
}

#[test]
fn audio_only_media_asked_for_as_video_still_lands_on_an_audio_track() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let a = insert_one(
        &mut fx,
        InsertTarget::Track(main),
        3,
        media_clip(AUDIO_200, MediaKind::Video, 0, 20),
    );
    assert_eq!(fx.render(), "V0*:\nA0: 6@3+20[0]");
    assert!(matches!(
        clip(&fx, a).content,
        ClipContent::Media {
            kind: MediaKind::Audio,
            ..
        }
    ));
    assert_eq!(sound(&fx, a), EmbeddedAudio::Active(ClipAudio::default()));
    // A new track for audio content is an audio track.
    insert_one(
        &mut fx,
        InsertTarget::NewTrack,
        0,
        media_clip(AUDIO_200, MediaKind::Audio, 0, 5),
    );
    assert_eq!(fx.seq().audio_tracks().len(), 2);
    assert_eq!(fx.seq().visual_tracks().len(), 1);
}

#[test]
fn first_free_takes_the_top_most_free_audio_track_or_adds_one() {
    let mut fx = Fixture::new();
    let a = |src, len| media_clip(AUDIO_200, MediaKind::Audio, src, len);
    insert_one(&mut fx, InsertTarget::FirstFree, 0, a(0, 20));
    insert_one(&mut fx, InsertTarget::FirstFree, 10, a(0, 20));
    assert_eq!(fx.render(), "V0*:\nA0: 6@0+20[0]\nA1: 8@10+20[0]");
    // Free on A0 from frame 20: the top-most free track wins.
    insert_one(&mut fx, InsertTarget::FirstFree, 25, a(0, 5));
    assert_eq!(
        fx.render(),
        "V0*:\nA0: 6@0+20[0] 10@25+5[0]\nA1: 8@10+20[0]"
    );
    // A locked track is passed over even where it is free.
    let a0 = fx.track_at(TrackSlot::audio(0));
    set_flags(
        &mut fx,
        a0,
        TrackFlags {
            locked: true,
            ..TrackFlags::default()
        },
    );
    insert_one(&mut fx, InsertTarget::FirstFree, 40, a(0, 5));
    assert_eq!(
        fx.render(),
        "V0*:\nA0L: 6@0+20[0] 10@25+5[0]\nA1: 8@10+20[0] 11@40+5[0]"
    );
}

#[test]
fn first_free_puts_visual_items_on_the_lowest_free_overlay_never_the_main_track() {
    let mut fx = Fixture::new();
    let v = media_clip(VIDEO_50, MediaKind::Video, 0, 10);
    // The main track is empty, but "first free" means an overlay.
    insert_one(&mut fx, InsertTarget::FirstFree, 0, v.clone());
    assert_eq!(fx.render(), "V1: 6@0+10[0]\nV0*:");
    insert_one(&mut fx, InsertTarget::FirstFree, 5, v.clone());
    assert_eq!(fx.render(), "V2: 8@5+10[0]\nV1: 6@0+10[0]\nV0*:");
    insert_one(&mut fx, InsertTarget::FirstFree, 10, v);
    assert_eq!(
        fx.render(),
        "V2: 8@5+10[0]\nV1: 6@0+10[0] 10@10+10[0]\nV0*:"
    );
    // Text goes on a text track, above the video band.
    insert_one(&mut fx, InsertTarget::FirstFree, 0, text(5));
    let kinds: Vec<TrackKind> = fx.seq().visual_tracks().iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        [
            TrackKind::Video,
            TrackKind::Video,
            TrackKind::Video,
            TrackKind::Text
        ]
    );
}

// ---------------------------------------------------------------- set clip audio

#[test]
fn clip_settings_apply_to_clips_with_sound_and_skip_the_rest() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    let a = insert_one(
        &mut fx,
        InsertTarget::FirstFree,
        0,
        media_clip(AUDIO_200, MediaKind::Audio, 0, 10),
    );
    let t = insert_one(&mut fx, InsertTarget::NewTrack, 0, text(5));
    let entries = fx.editor.history().len();
    set(&mut fx, &[v, a, t], AudioEdit::Gain(db(-600))).unwrap();
    assert_eq!(fx.editor.history().len(), entries + 1, "one entry for all");
    assert_eq!(*settings(&fx, v).gain.static_value(), db(-600));
    assert_eq!(*settings(&fx, a).gain.static_value(), db(-600));
    assert_eq!(sound(&fx, t), EmbeddedAudio::None);

    set(&mut fx, &[a], AudioEdit::Muted(true)).unwrap();
    set(&mut fx, &[a], AudioEdit::FadeIn(f(3))).unwrap();
    // Longer than the clip: stored as given, clamped only when played.
    set(&mut fx, &[a], AudioEdit::FadeOut(f(40))).unwrap();
    let s = settings(&fx, a);
    assert_eq!((s.muted, s.fade_in, s.fade_out), (true, f(3), f(40)));
    assert_eq!(s.effective_fades(f(10)), (f(3), f(7)));
    // The gain survived the other edits, and trims keep everything.
    assert_eq!(*s.gain.static_value(), db(-600));
}

#[test]
fn clip_settings_refuse_negative_fades_and_selections_without_sound() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    let t = insert_one(&mut fx, InsertTarget::NewTrack, 0, text(5));
    assert_eq!(
        set(&mut fx, &[v], AudioEdit::FadeIn(f(-1))).unwrap_err(),
        EditError::NegativeFade
    );
    assert_eq!(
        set(&mut fx, &[t], AudioEdit::Muted(true)).unwrap_err(),
        EditError::NoClipAudio
    );
    assert_eq!(
        set(&mut fx, &[], AudioEdit::Muted(true)).unwrap_err(),
        EditError::NothingSelected
    );
    // Setting what is already there changes nothing.
    assert_eq!(
        set(&mut fx, &[v], AudioEdit::Gain(GainDb::ZERO)).unwrap(),
        Committed::NoOp
    );
}

#[test]
fn clip_settings_skip_video_whose_sound_is_detached() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v, w] = fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 0, 10), (VIDEO_50, 0, 5)],
    )[..] else {
        panic!()
    };
    detach(&mut fx, &[v]).unwrap();
    set(&mut fx, &[v, w], AudioEdit::Muted(true)).unwrap();
    assert!(matches!(sound(&fx, v), EmbeddedAudio::Detached(_)));
    assert!(settings(&fx, w).muted);
    assert_eq!(
        set(&mut fx, &[v], AudioEdit::Muted(true)).unwrap_err(),
        EditError::NoClipAudio
    );
}

#[test]
fn split_drops_the_fades_on_the_new_inner_edges() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    set(&mut fx, &[v], AudioEdit::FadeIn(f(2))).unwrap();
    set(&mut fx, &[v], AudioEdit::FadeOut(f(3))).unwrap();
    let seq = fx.seq_id();
    let c = fx
        .apply(EditRequest::Split(SplitReq {
            seq,
            at: f(4),
            targets: SplitTargets::Clips(vec![v]),
        }))
        .unwrap();
    let right = applied(c).1[0];
    let (l, r) = (settings(&fx, v), settings(&fx, right));
    assert_eq!((l.fade_in, l.fade_out), (f(2), Tick::ZERO));
    assert_eq!((r.fade_in, r.fade_out), (Tick::ZERO, f(3)));
}

// ---------------------------------------------------------------- detach

#[test]
fn detach_moves_the_sound_to_a_linked_audio_clip() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let mut new = media_clip(VIDEO_100, MediaKind::Video, 20, 10);
    new.name = Some("Interview".into());
    let v = insert_one(&mut fx, InsertTarget::Track(main), 0, new);
    set(&mut fx, &[v], AudioEdit::Gain(db(-350))).unwrap();
    set(&mut fx, &[v], AudioEdit::FadeIn(f(2))).unwrap();
    let before = settings(&fx, v);

    let (notices, selection) = applied(detach(&mut fx, &[v]).unwrap());
    assert_eq!(fx.render(), "V0*: 6@0+10[20]\nA0: 8@0+10[20]");
    let a = selection[0];
    assert_eq!(selection, vec![a], "the new audio clip is selected");
    let a0 = fx.track_at(TrackSlot::audio(0));
    assert_eq!(notices, vec![Notice::CreatedTrack(a0)]);

    let (video, audio) = (clip(&fx, v), clip(&fx, a));
    let EmbeddedAudio::Detached(link) = video.audio else {
        panic!("video still has its sound: {:?}", video.audio)
    };
    assert_eq!((video.link, audio.link), (Some(link), Some(link)));
    assert_eq!(audio.audio, EmbeddedAudio::Active(before));
    assert_eq!(audio.name.as_deref(), Some("Interview"));
    assert_eq!(audio.range, video.range);
    assert_eq!(
        audio.content,
        ClipContent::Media {
            asset: VIDEO_100,
            kind: MediaKind::Audio,
            source: video.content.source().unwrap(),
            stream: None,
        }
    );
    assert_eq!(linked_audio(fx.doc(), fx.seq_id(), v), vec![a]);
}

#[test]
fn detaching_several_clips_is_one_undo_entry() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let ids = fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 0, 10), (VIDEO_50, 0, 5)],
    );
    let entries = fx.editor.history().len();
    let (_, selection) = applied(detach(&mut fx, &ids).unwrap());
    assert_eq!(fx.editor.history().len(), entries + 1);
    assert_eq!(selection.len(), 2);
    // Both fit on one new audio track.
    assert_eq!(
        fx.render(),
        "V0*: 6@0+10[0] 7@10+5[0]\nA0: 9@0+10[0] 12@10+5[0]"
    );
    let links: Vec<_> = ids.iter().map(|id| sound(&fx, *id).detached()).collect();
    assert!(links[0].is_some() && links[1].is_some() && links[0] != links[1]);
}

#[test]
fn detach_uses_the_first_free_audio_track_else_a_new_one_at_the_top() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(AUDIO_200, 0, 20)]);
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    // A0 is busy over the clip: a new track goes on top of the audio band.
    detach(&mut fx, &[v]).unwrap();
    assert_eq!(fx.render(), "V0*: 8@0+10[0]\nA0: 10@0+10[0]\nA1: 6@0+20[0]");
    let [w] = fx.insert(InsertTarget::Track(main), 10, &[(VIDEO_50, 0, 5)])[..] else {
        panic!()
    };
    detach(&mut fx, &[w]).unwrap();
    assert_eq!(
        fx.render(),
        "V0*: 8@0+10[0] 12@10+5[0]\nA0: 10@0+10[0] 14@10+5[0]\nA1: 6@0+20[0]"
    );
    // A locked track is passed over.
    let a0 = fx.track_at(TrackSlot::audio(0));
    set_flags(
        &mut fx,
        a0,
        TrackFlags {
            locked: true,
            ..TrackFlags::default()
        },
    );
    let [x] = fx.insert(InsertTarget::Track(main), 15, &[(VIDEO_50, 0, 5)])[..] else {
        panic!()
    };
    detach(&mut fx, &[x]).unwrap();
    assert_eq!(
        fx.render(),
        "V0*: 8@0+10[0] 12@10+5[0] 15@15+5[0]\nA0: 17@15+5[0]\nA1L: 10@0+10[0] 14@10+5[0]\nA2: 6@0+20[0]"
    );
}

#[test]
fn detach_skips_clips_without_sound_and_refuses_when_none_has_any() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let silent = fx.import("/media/silent.mov", media_info_with(80, true, false));
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    let s = insert_one(
        &mut fx,
        InsertTarget::FirstFree,
        0,
        media_clip(silent, MediaKind::Video, 0, 10),
    );
    let t = insert_one(&mut fx, InsertTarget::NewTrack, 0, text(5));
    let a = insert_one(
        &mut fx,
        InsertTarget::FirstFree,
        0,
        media_clip(AUDIO_200, MediaKind::Audio, 0, 10),
    );
    assert_eq!(
        detach(&mut fx, &[s, t, a]).unwrap_err(),
        EditError::NothingToDetach
    );
    let (_, selection) = applied(detach(&mut fx, &[s, v, t]).unwrap());
    assert_eq!(selection.len(), 1);
    assert!(sound(&fx, v).detached().is_some());
    assert_eq!(sound(&fx, s), EmbeddedAudio::None);
    // Already detached.
    assert_eq!(
        detach(&mut fx, &[v]).unwrap_err(),
        EditError::NothingToDetach
    );
    assert_eq!(
        detach(&mut fx, &[]).unwrap_err(),
        EditError::NothingSelected
    );
}

#[test]
fn deleting_a_detached_video_keeps_its_audio_clip_and_clears_the_link() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    let a = applied(detach(&mut fx, &[v]).unwrap()).1[0];
    let seq = fx.seq_id();
    fx.apply(EditRequest::Delete(DeleteReq {
        seq,
        clips: vec![v],
        ripple: false,
    }))
    .unwrap();
    assert_eq!(clip(&fx, a).link, None);
    assert!(sound(&fx, a).active().is_some());
}

#[test]
fn deleting_a_detached_video_unlinks_every_piece_of_its_split_sound() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    let a = applied(detach(&mut fx, &[v]).unwrap()).1[0];
    let seq = fx.seq_id();
    let c = fx
        .apply(EditRequest::Split(SplitReq {
            seq,
            at: f(5),
            targets: SplitTargets::Clips(vec![a]),
        }))
        .unwrap();
    let b = applied(c).1[0];
    fx.apply(EditRequest::Delete(DeleteReq {
        seq,
        clips: vec![v],
        ripple: false,
    }))
    .unwrap();
    // Two sound pieces are left, but no video holds the link: it is
    // dangling and cleared (TL-030).
    assert_eq!((clip(&fx, a).link, clip(&fx, b).link), (None, None));
    assert!(sound(&fx, a).active().is_some() && sound(&fx, b).active().is_some());
}

// ---------------------------------------------------------------- restore

#[test]
fn restore_brings_back_the_edited_settings_and_removes_the_audio_clip() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    let a = applied(detach(&mut fx, &[v]).unwrap()).1[0];
    set(&mut fx, &[a], AudioEdit::Gain(db(-1200))).unwrap();
    set(&mut fx, &[a], AudioEdit::Muted(true)).unwrap();
    let edited = settings(&fx, a);
    let (notices, selection) = applied(restore(&mut fx, &[v]).unwrap());
    assert_eq!(selection, vec![v]);
    assert_eq!(notices, vec![Notice::RemovedEmptyTracks(1)]);
    assert_eq!(fx.render(), "V0*: 6@0+10[0]");
    assert_eq!(sound(&fx, v), EmbeddedAudio::Active(edited));
    assert_eq!(clip(&fx, v).link, None);
}

#[test]
fn restore_removes_every_piece_of_a_split_audio_clip_and_keeps_both_fades() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    set(&mut fx, &[v], AudioEdit::Gain(db(-300))).unwrap();
    set(&mut fx, &[v], AudioEdit::FadeIn(f(2))).unwrap();
    set(&mut fx, &[v], AudioEdit::FadeOut(f(3))).unwrap();
    let original = settings(&fx, v);
    let a = applied(detach(&mut fx, &[v]).unwrap()).1[0];
    let seq = fx.seq_id();
    let c = fx
        .apply(EditRequest::Split(SplitReq {
            seq,
            at: f(4),
            targets: SplitTargets::Clips(vec![a]),
        }))
        .unwrap();
    let b = applied(c).1[0];
    assert_eq!(linked_audio(fx.doc(), seq, v), vec![a, b]);
    restore(&mut fx, &[v]).unwrap();
    assert_eq!(fx.render(), "V0*: 6@0+10[0]");
    assert_eq!(settings(&fx, v), original);
}

#[test]
fn restore_after_the_audio_clip_was_deleted_brings_back_default_sound() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    set(&mut fx, &[v], AudioEdit::Muted(true)).unwrap();
    let a = applied(detach(&mut fx, &[v]).unwrap()).1[0];
    let link = sound(&fx, v).detached().unwrap();
    let seq = fx.seq_id();
    fx.apply(EditRequest::Delete(DeleteReq {
        seq,
        clips: vec![a],
        ripple: false,
    }))
    .unwrap();
    // The link dissolved, but the video still knows its sound was detached
    // and the id is never handed out again.
    assert_eq!(clip(&fx, v).link, None);
    assert_eq!(sound(&fx, v), EmbeddedAudio::Detached(link));
    assert!(fx.doc().max_id() >= link.get());
    assert!(linked_audio(fx.doc(), seq, v).is_empty());
    restore(&mut fx, &[v]).unwrap();
    assert_eq!(sound(&fx, v), EmbeddedAudio::Active(ClipAudio::default()));
}

#[test]
fn restore_finds_the_audio_clip_wherever_it_was_moved() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    fx.insert(InsertTarget::NewTrack, 30, &[(AUDIO_200, 0, 10)]);
    let a = applied(detach(&mut fx, &[v]).unwrap()).1[0];
    let seq = fx.seq_id();
    // Down one track and later in time.
    fx.apply(EditRequest::Move(MoveReq {
        seq,
        clips: vec![a],
        delta: f(12),
        track_shift: -1,
        followers: Vec::new(),
    }))
    .unwrap();
    // The detached clip went onto the free start of A0, then moved down.
    assert_eq!(
        fx.render(),
        "V0*: 6@0+10[0]\nA0: 7@30+10[0]\nA1: 10@12+10[0]"
    );
    restore(&mut fx, &[v]).unwrap();
    assert_eq!(fx.render(), "V0*: 6@0+10[0]\nA0: 7@30+10[0]");
    assert!(sound(&fx, v).active().is_some());
}

#[test]
fn restore_gives_every_piece_of_a_split_video_its_sound_back() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    set(&mut fx, &[v], AudioEdit::FadeIn(f(2))).unwrap();
    set(&mut fx, &[v], AudioEdit::FadeOut(f(3))).unwrap();
    let a = applied(detach(&mut fx, &[v]).unwrap()).1[0];
    let seq = fx.seq_id();
    let c = fx
        .apply(EditRequest::Split(SplitReq {
            seq,
            at: f(4),
            targets: SplitTargets::Clips(vec![v]),
        }))
        .unwrap();
    let right = applied(c).1[0];
    let link = sound(&fx, v).detached().unwrap();
    assert_eq!(sound(&fx, right), EmbeddedAudio::Detached(link));
    assert_eq!(linked_audio(fx.doc(), seq, right), vec![a]);

    let (_, selection) = applied(restore(&mut fx, &[right]).unwrap());
    assert_eq!(selection, vec![v, right]);
    assert_eq!(fx.render(), "V0*: 6@0+4[0] 10@4+6[4]");
    let (l, r) = (settings(&fx, v), settings(&fx, right));
    assert_eq!((l.fade_in, l.fade_out), (f(2), Tick::ZERO));
    assert_eq!((r.fade_in, r.fade_out), (Tick::ZERO, f(3)));
    assert_eq!((clip(&fx, v).link, clip(&fx, right).link), (None, None));
}

#[test]
fn restore_refuses_clips_without_detached_sound() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    assert_eq!(
        restore(&mut fx, &[v]).unwrap_err(),
        EditError::NothingToRestore
    );
    let a = applied(detach(&mut fx, &[v]).unwrap()).1[0];
    // The audio clip itself is not a restore target.
    assert_eq!(
        restore(&mut fx, &[a]).unwrap_err(),
        EditError::NothingToRestore
    );
    assert!(linked_audio(fx.doc(), fx.seq_id(), a).is_empty());
}

// ---------------------------------------------------------------- extract

#[test]
fn extracting_audio_adds_a_virtual_item_named_after_the_file() {
    let mut fx = Fixture::new();
    let item = extract(&mut fx, VIDEO_100, None);
    let parent = Arc::clone(fx.doc().library().get(VIDEO_100).unwrap());
    let extracted = Arc::clone(fx.doc().library().get(item).unwrap());
    assert_eq!(
        extracted.source,
        MediaSource::AudioOf {
            source: VIDEO_100,
            stream: 1
        }
    );
    assert_eq!(extracted.display_name.as_deref(), Some("3.mp4 (audio)"));
    assert_eq!(extracted.path, parent.path);
    assert_eq!(extracted.info.duration, parent.info.duration);
    assert!(!extracted.info.has_video() && extracted.info.default_video.is_none());
    assert_eq!(extracted.info.audio, parent.info.audio);
}

#[test]
fn extracting_the_same_stream_again_reuses_the_item() {
    let mut fx = Fixture::new();
    let item = extract(&mut fx, VIDEO_100, None);
    let len = fx.doc().library().len();
    for req in [
        EditRequest::ExtractAudio {
            media: VIDEO_100,
            stream: Some(1),
        },
        EditRequest::ExtractAudio {
            media: item,
            stream: None,
        },
    ] {
        let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
        assert!(plan.is_noop());
        assert_eq!(plan.imported(), &[item]);
        assert_eq!(fx.apply(req).unwrap(), Committed::NoOp);
    }
    assert_eq!(fx.doc().library().len(), len);
}

#[test]
fn extracting_from_a_video_without_sound_is_refused() {
    let mut fx = Fixture::new();
    let silent = fx.import("/media/silent.mov", media_info_with(80, true, false));
    let res = fx.apply(EditRequest::ExtractAudio {
        media: silent,
        stream: None,
    });
    let err = res.unwrap_err();
    assert_eq!(err, EditError::NoSound(silent));
    assert_eq!(err.to_string(), "this video has no sound");
    let res = fx.apply(EditRequest::ExtractAudio {
        media: VIDEO_100,
        stream: Some(7),
    });
    assert_eq!(
        res.unwrap_err(),
        EditError::UnknownStream {
            media: VIDEO_100,
            stream: 7
        }
    );
}

#[test]
fn an_extracted_item_plays_as_audio_and_outlives_its_original() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let item = extract(&mut fx, VIDEO_50, None);
    fx.apply(EditRequest::RemoveMedia {
        media: vec![VIDEO_50],
        with_clips: false,
    })
    .unwrap();
    let a = insert_one(
        &mut fx,
        InsertTarget::Track(main),
        0,
        media_clip(item, MediaKind::Video, 0, 10),
    );
    assert_eq!(fx.render(), format!("V0*:\nA0: {}@0+10[0]", a.get()));
    let window = TimeRange::new(f(0), f(1)).unwrap();
    let mix = mix_sources(fx.doc(), fx.seq_id(), window);
    assert_eq!(
        mix.iter().map(|m| (m.asset, m.stream)).collect::<Vec<_>>(),
        vec![(item, 1)]
    );
}

#[test]
fn relinking_a_video_relinks_its_extracted_audio_in_the_same_step() {
    let mut fx = Fixture::new();
    let item = extract(&mut fx, VIDEO_100, None);
    let mut info = media_info(120, true);
    info.container = "mov".into();
    let path = freecut_timeline::model::StoredPath {
        absolute: "/moved/3.mov".into(),
        relative: None,
    };
    let entries = fx.editor.history().len();
    fx.apply(EditRequest::RelinkMedia {
        media: VIDEO_100,
        path: path.clone(),
        info: info.clone(),
        fingerprint: None,
    })
    .unwrap();
    assert_eq!(fx.editor.history().len(), entries + 1);
    let child = fx.doc().library().get(item).unwrap();
    assert_eq!(child.path, path);
    assert_eq!(child.info, info.audio_only(1).unwrap());
    assert_eq!(child.info.duration, f(120));
    // A relink the extracted item's clips don't fit in is refused whole.
    let a = insert_one(
        &mut fx,
        InsertTarget::FirstFree,
        0,
        media_clip(item, MediaKind::Audio, 100, 20),
    );
    let short = fx.apply(EditRequest::RelinkMedia {
        media: VIDEO_100,
        path,
        info: media_info(110, true),
        fingerprint: None,
    });
    assert_eq!(short.unwrap_err(), EditError::SourceLimit { clip: a });
}

fn relink(fx: &mut Fixture, media: MediaId, path: &str, info: MediaInfo) -> Committed {
    fx.apply(EditRequest::RelinkMedia {
        media,
        path: freecut_timeline::model::StoredPath {
            absolute: path.into(),
            relative: None,
        },
        info,
        fingerprint: None,
    })
    .unwrap()
}

/// `info` with its (only) audio stream at container index `stream`.
fn audio_at(mut info: MediaInfo, stream: u32) -> MediaInfo {
    info.audio[0].index = stream;
    info
}

#[test]
fn relinking_a_video_whose_audio_stream_moved_relinks_its_extracted_audio() {
    let mut fx = Fixture::new();
    let item = extract(&mut fx, VIDEO_100, None);
    // A .mov with a timecode track: the sound is stream 2 now, not 1.
    let info = audio_at(media_info(100, true), 2);
    let (notices, _) = applied(relink(&mut fx, VIDEO_100, "/moved/3.mov", info.clone()));
    assert_eq!(notices, Vec::new());
    let child = fx.doc().library().get(item).unwrap();
    assert_eq!(child.path.absolute, "/moved/3.mov");
    assert_eq!(
        child.source,
        MediaSource::AudioOf {
            source: VIDEO_100,
            stream: 2
        }
    );
    assert_eq!(child.info, info.audio_only(2).unwrap());
}

#[test]
fn a_parent_relink_that_leaves_an_extracted_item_behind_says_so() {
    let mut fx = Fixture::new();
    let item = extract(&mut fx, VIDEO_100, None);
    let (notices, _) = applied(relink(
        &mut fx,
        VIDEO_100,
        "/moved/silent.mp4",
        media_info_with(100, true, false),
    ));
    assert_eq!(notices, vec![Notice::ExtractedAudioNotRelinked(item)]);
    assert_eq!(
        fx.doc().library().get(item).unwrap().path.absolute,
        "/media/3.mp4"
    );
}

#[test]
fn an_extracted_item_whose_stream_is_gone_relinks_on_its_own() {
    let mut fx = Fixture::new();
    let item = extract(&mut fx, VIDEO_100, None);
    let a = insert_one(
        &mut fx,
        InsertTarget::FirstFree,
        0,
        media_clip(item, MediaKind::Audio, 0, 10),
    );
    // A plain WAV of the sound: stream 0, where the item played stream 1.
    let wav = media_info(100, false);
    relink(&mut fx, item, "/moved/3.wav", wav.clone());
    let child = fx.doc().library().get(item).unwrap();
    assert_eq!(child.path.absolute, "/moved/3.wav");
    assert_eq!(
        child.source,
        MediaSource::AudioOf {
            source: VIDEO_100,
            stream: 0
        }
    );
    assert_eq!(child.info, wav.audio_only(0).unwrap());
    let window = TimeRange::new(f(0), f(1)).unwrap();
    let mix = mix_sources(fx.doc(), fx.seq_id(), window);
    assert_eq!(
        mix.iter().map(|m| (m.clip, m.stream)).collect::<Vec<_>>(),
        vec![(a, 0)]
    );
    // A file without sound can't stand in for it.
    let silent = fx.apply(EditRequest::RelinkMedia {
        media: item,
        path: freecut_timeline::model::StoredPath {
            absolute: "/moved/silent.mp4".into(),
            relative: None,
        },
        info: media_info_with(100, true, false),
        fingerprint: None,
    });
    assert_eq!(silent.unwrap_err(), EditError::NoSound(item));
}

#[test]
fn relinking_to_a_file_with_sound_gives_video_clips_their_sound() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let silent = fx.import("/m/silent.mp4", media_info_with(100, true, false));
    let v = insert_one(
        &mut fx,
        InsertTarget::Track(main),
        0,
        media_clip(silent, MediaKind::Video, 0, 10),
    );
    assert_eq!(sound(&fx, v), EmbeddedAudio::None);
    relink(&mut fx, silent, "/m/real.mp4", media_info(100, true));
    assert_eq!(sound(&fx, v), EmbeddedAudio::Active(ClipAudio::default()));
    let window = TimeRange::new(f(0), f(1)).unwrap();
    assert_eq!(mix_sources(fx.doc(), fx.seq_id(), window).len(), 1);
    // And back: a file without sound leaves the clip without sound.
    set(&mut fx, &[v], AudioEdit::Gain(db(-600))).unwrap();
    relink(
        &mut fx,
        silent,
        "/m/silent.mp4",
        media_info_with(100, true, false),
    );
    assert_eq!(sound(&fx, v), EmbeddedAudio::None);
}

#[test]
fn relinking_keeps_detached_sound_detached() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    detach(&mut fx, &[v]).unwrap();
    let link = sound(&fx, v).detached().unwrap();
    relink(
        &mut fx,
        VIDEO_100,
        "/m/silent.mp4",
        media_info_with(100, true, false),
    );
    assert_eq!(sound(&fx, v), EmbeddedAudio::Detached(link));
}

// ---------------------------------------------------------------- mix view

#[test]
fn the_mix_holds_every_audible_clip_in_track_order() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 5, 10)])[..] else {
        panic!()
    };
    let o = insert_one(
        &mut fx,
        InsertTarget::FirstFree,
        0,
        media_clip(VIDEO_50, MediaKind::Video, 2, 10),
    );
    let d = applied(detach(&mut fx, &[o]).unwrap()).1[0];
    let m = insert_one(
        &mut fx,
        InsertTarget::FirstFree,
        0,
        media_clip(AUDIO_200, MediaKind::Audio, 0, 20),
    );
    let seq = fx.seq_id();
    let window = |a, b| TimeRange::from_start_end(f(a), f(b)).unwrap();
    let mix = mix_sources(fx.doc(), seq, window(0, 5));
    let a0 = fx.track_at(TrackSlot::audio(0));
    let a1 = fx.track_at(TrackSlot::audio(1));
    assert_eq!(
        mix,
        vec![
            MixSource {
                clip: v,
                track: main,
                asset: VIDEO_100,
                stream: 1,
                range: window(0, 10),
                source_start: f(5),
                audio: ClipAudio::default(),
                mixed: 1,
                track_audio: TrackAudio::default(),
            },
            MixSource {
                clip: d,
                track: a0,
                asset: VIDEO_50,
                stream: 1,
                range: window(0, 10),
                source_start: f(2),
                audio: ClipAudio::default(),
                mixed: 1,
                track_audio: TrackAudio::default(),
            },
            MixSource {
                clip: m,
                track: a1,
                asset: AUDIO_200,
                stream: 0,
                range: window(0, 20),
                source_start: f(0),
                audio: ClipAudio::default(),
                mixed: 1,
                track_audio: TrackAudio::default(),
            },
        ],
        "the detached video {o} is silent"
    );
    // Only what overlaps the window.
    let late: Vec<ClipId> = mix_sources(fx.doc(), seq, window(12, 15))
        .iter()
        .map(|s| s.clip)
        .collect();
    assert_eq!(late, vec![m]);
}

#[test]
fn muted_clips_muted_tracks_and_disabled_clips_are_left_out_but_hidden_tracks_sound() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    let m = insert_one(
        &mut fx,
        InsertTarget::FirstFree,
        0,
        media_clip(AUDIO_200, MediaKind::Audio, 0, 20),
    );
    let seq = fx.seq_id();
    let window = TimeRange::new(f(0), f(5)).unwrap();
    let clips = |fx: &Fixture| -> Vec<ClipId> {
        mix_sources(fx.doc(), seq, window)
            .iter()
            .map(|s| s.clip)
            .collect()
    };
    assert_eq!(clips(&fx), vec![v, m]);

    set_flags(
        &mut fx,
        main,
        TrackFlags {
            hidden: true,
            ..TrackFlags::default()
        },
    );
    assert_eq!(clips(&fx), vec![v, m], "hiding is a picture flag");

    set_flags(
        &mut fx,
        main,
        TrackFlags {
            muted: true,
            ..TrackFlags::default()
        },
    );
    assert_eq!(clips(&fx), vec![m]);
    // Unmuting the track brings back the clip's own state unchanged.
    set_flags(&mut fx, main, TrackFlags::default());
    set(&mut fx, &[m], AudioEdit::Muted(true)).unwrap();
    assert_eq!(clips(&fx), vec![v]);

    let disabled = rebuilt(fx.doc(), v, |c| c.enabled = false);
    let left: Vec<ClipId> = mix_sources(&disabled, seq, window)
        .iter()
        .map(|s| s.clip)
        .collect();
    assert!(left.is_empty());
}

// ---------------------------------------------------------------- validation

#[test]
fn sound_settings_must_fit_the_clip_content() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    let a = insert_one(
        &mut fx,
        InsertTarget::FirstFree,
        0,
        media_clip(AUDIO_200, MediaKind::Audio, 0, 10),
    );
    let t = insert_one(&mut fx, InsertTarget::NewTrack, 0, text(5));
    let doc = fx.doc().clone();
    let link = freecut_timeline::ids::LinkId::from_raw(core::num::NonZeroU64::new(500).unwrap());
    let mismatch = |id| vec![InvariantViolation::AudioMismatch { clip: id }];

    let text_with_sound = rebuilt(&doc, t, |c| {
        c.audio = EmbeddedAudio::Active(ClipAudio::default());
    });
    assert_eq!(violations(&text_with_sound), mismatch(t));
    let silent_audio_clip = rebuilt(&doc, a, |c| c.audio = EmbeddedAudio::None);
    assert_eq!(violations(&silent_audio_clip), mismatch(a));
    let detached_audio_clip = rebuilt(&doc, a, |c| c.audio = EmbeddedAudio::Detached(link));
    assert_eq!(violations(&detached_audio_clip), mismatch(a));
    let other_link = rebuilt(&doc, v, |c| {
        c.audio = EmbeddedAudio::Detached(link);
        c.link = Some(freecut_timeline::ids::LinkId::from_raw(
            core::num::NonZeroU64::new(501).unwrap(),
        ));
    });
    assert!(violations(&other_link).contains(&InvariantViolation::AudioMismatch { clip: v }));
    let negative = rebuilt(&doc, v, |c| {
        c.audio = EmbeddedAudio::Active(ClipAudio {
            fade_out: Tick::from_raw(-1),
            ..ClipAudio::default()
        });
    });
    assert_eq!(
        violations(&negative),
        vec![InvariantViolation::NegativeFade { clip: v }]
    );
    // A detached video whose link dissolved is fine.
    let dangling = rebuilt(&doc, v, |c| c.audio = EmbeddedAudio::Detached(link));
    assert_eq!(violations(&dangling), Vec::new());
    assert_eq!(dangling.max_id(), 500);
}

#[test]
fn an_extracted_item_holds_only_its_stream() {
    let fx = Fixture::new();
    let doc = fx.doc();
    let parent = doc.library().get(VIDEO_100).unwrap();
    let bad = freecut_timeline::model::MediaItem {
        id: PROBING,
        source: MediaSource::AudioOf {
            source: VIDEO_100,
            stream: 1,
        },
        ..(**parent).clone()
    };
    let mut items: Vec<_> = doc.library().items().cloned().collect();
    items.push(Arc::new(bad));
    let broken = Document::from_parts(
        doc.root(),
        *doc.settings(),
        Library::from_parts(items).unwrap(),
        doc.sequences().map(|s| Arc::new(s.clone())).collect(),
    )
    .unwrap();
    assert_eq!(
        violations(&broken),
        vec![InvariantViolation::DerivedMedia { media: PROBING }]
    );
}

#[test]
fn a_missing_parent_id_stays_reserved() {
    let mut fx = Fixture::new();
    let item = extract(&mut fx, VIDEO_50, None);
    let doc = fx.doc();
    // Point the extracted item at a parent id above every id in use, as a
    // file whose original was removed may.
    let gone = MediaId::from_raw(core::num::NonZeroU64::new(5000).unwrap());
    let items: Vec<_> = doc
        .library()
        .items()
        .map(|m| {
            let mut m = (**m).clone();
            if m.id == item {
                m.source = MediaSource::AudioOf {
                    source: gone,
                    stream: 1,
                };
            }
            Arc::new(m)
        })
        .collect();
    let orphan = Document::from_parts(
        doc.root(),
        *doc.settings(),
        Library::from_parts(items).unwrap(),
        doc.sequences().map(|s| Arc::new(s.clone())).collect(),
    )
    .unwrap();
    assert_eq!(violations(&orphan), Vec::new());
    assert_eq!(orphan.max_id(), 5000);
    let ctx = ValidateCtx {
        limits: Default::default(),
        id_high_water: core::num::NonZeroU64::new(5000),
    };
    assert_eq!(
        orphan.validate(&ctx),
        Err(vec![InvariantViolation::IdAboveHighWater(5000)])
    );
}

// ------------------------------------------------------------ master volume

fn set_master(fx: &mut Fixture, gain: GainDb) -> Committed {
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetMasterGain { seq, gain }).unwrap()
}

#[test]
fn a_new_sequence_plays_its_master_at_0_db() {
    let fx = Fixture::new();
    assert_eq!(fx.seq().master, MasterAudio::default());
    assert_eq!(fx.seq().master.gain, GainDb::ZERO);
}

#[test]
fn the_master_volume_is_an_undoable_edit() {
    let mut fx = Fixture::new();
    let quieter = GainDb::from_hundredths(-600);
    assert!(matches!(
        set_master(&mut fx, quieter),
        Committed::Applied { .. }
    ));
    assert_eq!(fx.seq().master.gain, quieter);
    let label = fx.editor.history().undo_label().unwrap();
    assert_eq!(label.op, OpKind::SetMasterGain);
    fx.editor.undo().unwrap();
    assert_eq!(fx.seq().master.gain, GainDb::ZERO);
    fx.editor.redo().unwrap();
    assert_eq!(fx.seq().master.gain, quieter);
    // Silence is a master volume too.
    set_master(&mut fx, GainDb::SILENT);
    assert!(fx.seq().master.gain.is_silent());
}

#[test]
fn the_master_volume_stops_at_plus_12_db() {
    let mut fx = Fixture::new();
    set_master(&mut fx, GainDb::MAX);
    assert_eq!(fx.seq().master.gain, MasterAudio::MAX_GAIN);
    assert_eq!(MasterAudio::MAX_GAIN, GainDb::from_hundredths(1_200));
    // Asking for more again changes nothing and adds no undo entry.
    let entries = fx.editor.history().len();
    assert_eq!(set_master(&mut fx, GainDb::MAX), Committed::NoOp);
    assert_eq!(fx.editor.history().len(), entries);
}

#[test]
fn a_master_volume_drag_is_one_undo_entry() {
    let mut fx = Fixture::new();
    let seq = fx.seq_id();
    let g = fx.editor.begin_gesture();
    for hundredths in [-100, -250, -400, 300] {
        let req = EditRequest::SetMasterGain {
            seq,
            gain: GainDb::from_hundredths(hundredths),
        };
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
    }
    fx.editor.end_gesture(g);
    fx.assert_valid();
    assert_eq!(fx.editor.history().len(), 1);
    assert_eq!(fx.seq().master.gain, GainDb::from_hundredths(300));
    fx.editor.undo().unwrap();
    assert_eq!(fx.seq().master.gain, GainDb::ZERO);
}
