//! Fade curves (AU-006), volume keyframes (AU-008, TR-027) and the track
//! stage (AU-013). Every edit goes through `Fixture::apply`, which also
//! checks the undo/redo round trip.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::sync::Arc;

use common::*;
use freecut_timeline::edit::{
    AudioEdit, DeleteReq, Edge, EditError, EditRequest, GainKeyEdit, InsertReq, InsertTarget,
    NewClip, OpKind, SplitReq, SplitTargets, TrimReq,
};
use freecut_timeline::history::Coalesce;
use freecut_timeline::ids::{ClipId, TrackId};
use freecut_timeline::mix::mix_sources;
use freecut_timeline::model::{
    Animatable, Clip, ClipAudio, EmbeddedAudio, FadeCurve, GainDb, Interp, Keyframe, MediaKind,
    Pan, TextClipData, TrackAudio, TrackFlags, TrackHeader, TrackKind, TrackSlot,
};
use freecut_timeline::time::{Tick, TimeRange};
use freecut_timeline::{CommitMeta, Committed};
use pretty_assertions::assert_eq;

// ---------------------------------------------------------------- helpers

fn clip(fx: &Fixture, id: ClipId) -> Arc<Clip> {
    Arc::clone(fx.doc().clip(id).unwrap())
}

fn settings(fx: &Fixture, id: ClipId) -> ClipAudio {
    clip(fx, id).audio.active().unwrap().clone()
}

fn db(hundredths: i32) -> GainDb {
    GainDb::from_hundredths(hundredths)
}

fn selection(c: Committed) -> Vec<ClipId> {
    match c {
        Committed::Applied { selection, .. } => selection.unwrap_or_default(),
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

fn keys(fx: &mut Fixture, clips: &[ClipId], edit: GainKeyEdit) -> Result<Committed, EditError> {
    let seq = fx.seq_id();
    fx.apply(EditRequest::EditGainKeys {
        seq,
        clips: clips.to_vec(),
        edit,
    })
}

/// One audio clip of AUDIO_200 at frame 0, source 20..60.
fn audio_clip(fx: &mut Fixture) -> ClipId {
    let seq = fx.seq_id();
    let c = fx
        .apply(EditRequest::Insert(InsertReq {
            seq,
            target: InsertTarget::FirstFree,
            at: f(0),
            clips: vec![NewClip::media(
                AUDIO_200,
                MediaKind::Audio,
                TimeRange::new(f(20), f(40)).unwrap(),
            )],
        }))
        .unwrap();
    selection(c)[0]
}

fn key_at(at: Tick, value: GainDb) -> Keyframe<GainDb> {
    Keyframe {
        at,
        value,
        interp: Interp::Linear,
    }
}

fn set_lock(fx: &mut Fixture, track: TrackId, locked: bool) {
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

// ------------------------------------------------------------ fade curves

#[test]
fn fade_curves_are_set_per_edge_and_default_to_linear() {
    let mut fx = Fixture::new();
    let a = audio_clip(&mut fx);
    assert_eq!(
        (
            settings(&fx, a).fade_in_curve,
            settings(&fx, a).fade_out_curve
        ),
        (FadeCurve::Linear, FadeCurve::Linear)
    );
    set(&mut fx, &[a], AudioEdit::FadeInCurve(FadeCurve::Smooth)).unwrap();
    set(&mut fx, &[a], AudioEdit::FadeOutCurve(FadeCurve::Fast)).unwrap();
    let s = settings(&fx, a);
    assert_eq!(
        (s.fade_in_curve, s.fade_out_curve),
        (FadeCurve::Smooth, FadeCurve::Fast)
    );
    let entries = fx.editor.history().len();
    assert_eq!(
        set(&mut fx, &[a], AudioEdit::FadeOutCurve(FadeCurve::Fast)).unwrap(),
        Committed::NoOp
    );
    assert_eq!(fx.editor.history().len(), entries);
    let label = fx.editor.history().undo_label().unwrap();
    assert_eq!(label.op, OpKind::SetClipAudio);
}

#[test]
fn split_keeps_each_curve_on_its_kept_edge() {
    let mut fx = Fixture::new();
    let a = audio_clip(&mut fx);
    set(&mut fx, &[a], AudioEdit::FadeIn(f(5))).unwrap();
    set(&mut fx, &[a], AudioEdit::FadeOut(f(5))).unwrap();
    set(&mut fx, &[a], AudioEdit::FadeInCurve(FadeCurve::Slow)).unwrap();
    set(&mut fx, &[a], AudioEdit::FadeOutCurve(FadeCurve::Smooth)).unwrap();
    let seq = fx.seq_id();
    let right = selection(
        fx.apply(EditRequest::Split(SplitReq {
            seq,
            at: f(20),
            targets: SplitTargets::Clips(vec![a]),
        }))
        .unwrap(),
    )[0];
    let (l, r) = (settings(&fx, a), settings(&fx, right));
    assert_eq!(
        (l.fade_in, l.fade_in_curve, l.fade_out, l.fade_out_curve),
        (f(5), FadeCurve::Slow, f(0), FadeCurve::Linear)
    );
    assert_eq!(
        (r.fade_in, r.fade_in_curve, r.fade_out, r.fade_out_curve),
        (f(0), FadeCurve::Linear, f(5), FadeCurve::Smooth)
    );
}

// -------------------------------------------------------- volume keyframes

#[test]
fn the_first_key_captures_the_static_volume() {
    let mut fx = Fixture::new();
    let a = audio_clip(&mut fx);
    set(&mut fx, &[a], AudioEdit::Gain(db(-600))).unwrap();
    keys(
        &mut fx,
        &[a],
        GainKeyEdit::Set {
            at: f(30),
            gain: None,
        },
    )
    .unwrap();
    let s = settings(&fx, a);
    assert_eq!(s.gain.keys(), &[key_at(f(30), db(-600))]);
    assert_eq!(s.gain_at(f(0)), db(-600));
    // A second key with a value; the envelope interpolates in dB.
    keys(
        &mut fx,
        &[a],
        GainKeyEdit::Set {
            at: f(40),
            gain: Some(db(0)),
        },
    )
    .unwrap();
    let s = settings(&fx, a);
    assert_eq!(s.gain_at(f(35)), db(-300));
    assert_eq!(s.gain_at(f(50)), db(0));
    // Setting the key where one is replaces its value.
    keys(
        &mut fx,
        &[a],
        GainKeyEdit::Set {
            at: f(40),
            gain: Some(db(-100)),
        },
    )
    .unwrap();
    assert_eq!(settings(&fx, a).gain.keys().len(), 2);
    assert_eq!(settings(&fx, a).gain_at(f(40)), db(-100));
    assert_eq!(
        fx.editor.history().undo_label().unwrap().op,
        OpKind::EditGainKeys
    );
}

#[test]
fn a_keyframed_volume_refuses_a_static_change_and_mixed_selections_skip_it() {
    let mut fx = Fixture::new();
    let a = audio_clip(&mut fx);
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 10)])[..] else {
        panic!()
    };
    keys(
        &mut fx,
        &[a],
        GainKeyEdit::Set {
            at: f(30),
            gain: Some(db(-1_000)),
        },
    )
    .unwrap();
    assert_eq!(
        set(&mut fx, &[a], AudioEdit::Gain(db(-600))),
        Err(EditError::VolumeKeyframed)
    );
    set(&mut fx, &[a, v], AudioEdit::Gain(db(-600))).unwrap();
    assert_eq!(*settings(&fx, v).gain.static_value(), db(-600));
    assert_eq!(
        settings(&fx, a).gain.keys(),
        &[key_at(f(30), db(-1_000))],
        "the keyed clip was skipped"
    );
    // Other settings still apply to keyed clips.
    set(&mut fx, &[a], AudioEdit::Muted(true)).unwrap();
    assert!(settings(&fx, a).muted);
}

#[test]
fn moving_a_key_clamps_between_its_neighbours() {
    let mut fx = Fixture::new();
    let a = audio_clip(&mut fx);
    for (at, g) in [(25, -100), (30, -200), (35, -300)] {
        keys(
            &mut fx,
            &[a],
            GainKeyEdit::Set {
                at: f(at),
                gain: Some(db(g)),
            },
        )
        .unwrap();
    }
    keys(
        &mut fx,
        &[a],
        GainKeyEdit::Move {
            from: f(30),
            to: f(90),
        },
    )
    .unwrap();
    let times: Vec<Tick> = settings(&fx, a).gain.keys().iter().map(|k| k.at).collect();
    let just_before = Tick::from_raw(f(35).raw() - 1);
    assert_eq!(times, vec![f(25), just_before, f(35)], "never reorders");
    assert_eq!(
        keys(
            &mut fx,
            &[a],
            GainKeyEdit::Move {
                from: f(31),
                to: f(32)
            }
        ),
        Err(EditError::NoKeyframe { at: f(31) })
    );
    // Hold interpolation on the first key.
    keys(
        &mut fx,
        &[a],
        GainKeyEdit::Interp {
            at: f(25),
            interp: Interp::Hold,
        },
    )
    .unwrap();
    assert_eq!(settings(&fx, a).gain_at(f(30)), db(-100));
    assert_eq!(
        keys(
            &mut fx,
            &[a],
            GainKeyEdit::Interp {
                at: f(26),
                interp: Interp::Linear
            }
        ),
        Err(EditError::NoKeyframe { at: f(26) })
    );
}

#[test]
fn removing_the_last_key_leaves_its_value_as_the_static_volume() {
    let mut fx = Fixture::new();
    let a = audio_clip(&mut fx);
    for (at, g) in [(25, -100), (30, -200)] {
        keys(
            &mut fx,
            &[a],
            GainKeyEdit::Set {
                at: f(at),
                gain: Some(db(g)),
            },
        )
        .unwrap();
    }
    keys(&mut fx, &[a], GainKeyEdit::Remove(vec![f(25)])).unwrap();
    assert_eq!(
        settings(&fx, a).gain.keys(),
        &[key_at(f(30), db(-200))],
        "one key left"
    );
    keys(&mut fx, &[a], GainKeyEdit::Remove(vec![f(30)])).unwrap();
    assert_eq!(settings(&fx, a).gain, Animatable::new(db(-200)));
    assert_eq!(
        keys(&mut fx, &[a], GainKeyEdit::Remove(vec![f(30)])),
        Err(EditError::NoKeyframe { at: f(30) })
    );
    // Clearing keeps the first key's value; with no keys it changes nothing.
    for (at, g) in [(25, -700), (30, -800)] {
        keys(
            &mut fx,
            &[a],
            GainKeyEdit::Set {
                at: f(at),
                gain: Some(db(g)),
            },
        )
        .unwrap();
    }
    keys(&mut fx, &[a], GainKeyEdit::Clear).unwrap();
    assert_eq!(settings(&fx, a).gain, Animatable::new(db(-700)));
    assert_eq!(
        keys(&mut fx, &[a], GainKeyEdit::Clear).unwrap(),
        Committed::NoOp
    );
}

#[test]
fn key_edits_need_sound_and_unlocked_tracks() {
    let mut fx = Fixture::new();
    let a = audio_clip(&mut fx);
    let seq = fx.seq_id();
    let t = selection(
        fx.apply(EditRequest::Insert(InsertReq {
            seq,
            target: InsertTarget::NewTrack,
            at: f(0),
            clips: vec![NewClip::text(TextClipData::default(), f(5))],
        }))
        .unwrap(),
    )[0];
    let set_key = GainKeyEdit::Set {
        at: f(30),
        gain: None,
    };
    assert_eq!(
        keys(&mut fx, &[t], set_key.clone()),
        Err(EditError::NoClipAudio)
    );
    assert_eq!(
        keys(&mut fx, &[], set_key.clone()),
        Err(EditError::NothingSelected)
    );
    let track = fx.doc().find_clip(a).unwrap().track;
    set_lock(&mut fx, track, true);
    assert_eq!(
        keys(&mut fx, &[a], set_key),
        Err(EditError::TrackLocked { track })
    );
}

#[test]
fn trims_and_splits_keep_every_key_and_the_sound() {
    let mut fx = Fixture::new();
    let a = audio_clip(&mut fx); // source 20..60 at 0..40
    for (at, g) in [(10, -1_200), (30, -600), (50, 0), (70, -300)] {
        keys(
            &mut fx,
            &[a],
            GainKeyEdit::Set {
                at: f(at),
                gain: Some(db(g)),
            },
        )
        .unwrap();
    }
    let envelope = settings(&fx, a).gain;
    let seq = fx.seq_id();
    fx.apply(EditRequest::Trim(TrimReq {
        seq,
        clip: a,
        edge: Edge::In,
        delta: f(15),
        ripple: false,
        partners: Vec::new(),
    }))
    .unwrap();
    assert_eq!(
        settings(&fx, a).gain,
        envelope,
        "keys outside the trim stay"
    );
    let right = selection(
        fx.apply(EditRequest::Split(SplitReq {
            seq,
            at: f(25),
            targets: SplitTargets::Clips(vec![a]),
        }))
        .unwrap(),
    )[0];
    assert_eq!(settings(&fx, a).gain, envelope);
    assert_eq!(settings(&fx, right).gain, envelope);
    // The sound on each side of the cut is unchanged: same source times.
    let (l, r) = (clip(&fx, a), clip(&fx, right));
    for t in [16, 20, 24] {
        let s = l.source_time_at(f(t)).unwrap();
        assert_eq!(settings(&fx, a).gain_at(s), envelope.value_at(s));
    }
    for t in [25, 30, 39] {
        let s = r.source_time_at(f(t)).unwrap();
        assert_eq!(settings(&fx, right).gain_at(s), envelope.value_at(s));
    }
    // Lengthening again shows the hidden keys at the edge.
    fx.apply(EditRequest::Trim(TrimReq {
        seq,
        clip: a,
        edge: Edge::In,
        delta: f(-15),
        ripple: false,
        partners: Vec::new(),
    }))
    .unwrap();
    assert_eq!(settings(&fx, a).gain, envelope);
}

#[test]
fn detach_and_restore_carry_the_keys_and_curves() {
    let mut fx = Fixture::new();
    let main = fx.main();
    let [v] = fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 20)])[..] else {
        panic!()
    };
    keys(
        &mut fx,
        &[v],
        GainKeyEdit::Set {
            at: f(5),
            gain: Some(db(-900)),
        },
    )
    .unwrap();
    set(&mut fx, &[v], AudioEdit::FadeOutCurve(FadeCurve::Slow)).unwrap();
    let before = settings(&fx, v);
    let seq = fx.seq_id();
    let a = selection(
        fx.apply(EditRequest::DetachAudio {
            seq,
            clips: vec![v],
        })
        .unwrap(),
    )[0];
    assert_eq!(settings(&fx, a), before);
    keys(
        &mut fx,
        &[a],
        GainKeyEdit::Set {
            at: f(10),
            gain: Some(db(-100)),
        },
    )
    .unwrap();
    let edited = settings(&fx, a);
    fx.apply(EditRequest::RestoreAudio {
        seq,
        clips: vec![v],
    })
    .unwrap();
    assert_eq!(settings(&fx, v), edited);
    assert_eq!(edited.gain.keys().len(), 2);
}

#[test]
fn a_keyframe_drag_is_one_undo_entry() {
    let mut fx = Fixture::new();
    let a = audio_clip(&mut fx);
    keys(
        &mut fx,
        &[a],
        GainKeyEdit::Set {
            at: f(30),
            gain: Some(db(0)),
        },
    )
    .unwrap();
    let entries = fx.editor.history().len();
    let before = fx.doc().clone();
    let gesture = fx.editor.begin_gesture();
    let seq = fx.seq_id();
    let mut at = f(30);
    for step in 1..=4 {
        let to = f(30 + step);
        let req = EditRequest::EditGainKeys {
            seq,
            clips: vec![a],
            edit: GainKeyEdit::Move { from: at, to },
        };
        let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
        fx.editor
            .commit(
                plan,
                CommitMeta {
                    selection_before: Vec::new(),
                    coalesce: Coalesce::Gesture(gesture),
                },
            )
            .unwrap();
        at = to;
    }
    fx.editor.end_gesture(gesture);
    assert_eq!(fx.editor.history().len(), entries + 1);
    assert_eq!(settings(&fx, a).gain.keys()[0].at, f(34));
    fx.editor.undo().unwrap();
    assert_eq!(fx.doc(), &before);
}

// ------------------------------------------------------------ track stage

fn track_gain(fx: &mut Fixture, track: TrackId, gain: GainDb) -> Result<Committed, EditError> {
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetTrackGain { seq, track, gain })
}

fn track_pan(fx: &mut Fixture, track: TrackId, pan: Pan) -> Result<Committed, EditError> {
    let seq = fx.seq_id();
    fx.apply(EditRequest::SetTrackPan { seq, track, pan })
}

#[test]
fn every_track_starts_at_unity_and_centre() {
    let mut fx = Fixture::new();
    audio_clip(&mut fx);
    for (_, t) in fx.seq().tracks() {
        assert_eq!(t.mix, TrackAudio::default());
    }
}

#[test]
fn track_volume_and_pan_are_undoable_and_clamped() {
    let mut fx = Fixture::new();
    let a = audio_clip(&mut fx);
    let track = fx.doc().find_clip(a).unwrap().track;
    track_gain(&mut fx, track, db(-600)).unwrap();
    track_pan(&mut fx, track, Pan::new(-40)).unwrap();
    let mix = fx.seq().find_track(track).unwrap().1.mix;
    assert_eq!(
        mix,
        TrackAudio {
            gain: db(-600),
            pan: Pan::new(-40)
        }
    );
    assert_eq!(
        fx.editor.history().undo_label().unwrap().op,
        OpKind::SetTrackPan
    );
    track_gain(&mut fx, track, GainDb::MAX).unwrap();
    assert_eq!(
        fx.seq().find_track(track).unwrap().1.mix.gain,
        TrackAudio::MAX_GAIN,
        "+20 dB becomes +12 dB"
    );
    track_gain(&mut fx, track, GainDb::SILENT).unwrap();
    assert!(fx.seq().find_track(track).unwrap().1.mix.gain.is_silent());
    assert_eq!(
        track_gain(&mut fx, track, GainDb::SILENT).unwrap(),
        Committed::NoOp
    );
    assert_eq!(
        track_pan(&mut fx, track, Pan::new(-40)).unwrap(),
        Committed::NoOp
    );
    // The main (video) track carries embedded sound; a locked track still
    // takes the change (locks protect clips).
    let main = fx.main();
    set_lock(&mut fx, main, true);
    track_pan(&mut fx, main, Pan::RIGHT).unwrap();
}

#[test]
fn tracks_without_sound_refuse_a_track_stage() {
    let mut fx = Fixture::new();
    let seq = fx.seq_id();
    fx.apply(EditRequest::Insert(InsertReq {
        seq,
        target: InsertTarget::NewTrack,
        at: f(0),
        clips: vec![NewClip::text(TextClipData::default(), f(5))],
    }))
    .unwrap();
    let text = fx
        .seq()
        .tracks()
        .find(|(_, t)| t.kind == TrackKind::Text)
        .unwrap()
        .1
        .id;
    assert_eq!(
        track_gain(&mut fx, text, db(-100)),
        Err(EditError::TrackHasNoSound { track: text })
    );
    assert_eq!(
        track_pan(&mut fx, text, Pan::LEFT),
        Err(EditError::TrackHasNoSound { track: text })
    );
}

#[test]
fn a_removed_track_comes_back_with_its_stage() {
    let mut fx = Fixture::new();
    let a = audio_clip(&mut fx);
    let track = fx.doc().find_clip(a).unwrap().track;
    track_gain(&mut fx, track, db(-300)).unwrap();
    let seq = fx.seq_id();
    fx.apply(EditRequest::Delete(DeleteReq {
        seq,
        clips: vec![a],
        ripple: false,
    }))
    .unwrap();
    assert!(
        fx.seq().find_track(track).is_none(),
        "emptied track removed"
    );
    fx.editor.undo().unwrap();
    assert_eq!(fx.seq().find_track(track).unwrap().1.mix.gain, db(-300));
}

#[test]
fn the_mix_carries_the_track_stage_and_the_keys() {
    let mut fx = Fixture::new();
    let a = audio_clip(&mut fx);
    let track = fx.track_at(TrackSlot::audio(0));
    track_gain(&mut fx, track, db(-250)).unwrap();
    track_pan(&mut fx, track, Pan::new(30)).unwrap();
    keys(
        &mut fx,
        &[a],
        GainKeyEdit::Set {
            at: f(25),
            gain: Some(db(-50)),
        },
    )
    .unwrap();
    let seq = fx.seq_id();
    let mix = mix_sources(fx.doc(), seq, TimeRange::new(f(0), f(5)).unwrap());
    assert_eq!(mix.len(), 1);
    assert_eq!(
        mix[0].track_audio,
        TrackAudio {
            gain: db(-250),
            pan: Pan::new(30)
        }
    );
    assert_eq!((mix[0].clip, mix[0].mixed), (a, 1));
    assert_eq!(mix[0].audio.gain.keys(), &[key_at(f(25), db(-50))]);
    assert!(matches!(clip(&fx, a).audio, EmbeddedAudio::Active(_)));
}
