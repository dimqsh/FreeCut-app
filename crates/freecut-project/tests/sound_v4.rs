//! Format 4's sound (AU-006, AU-008, AU-012, AU-013, AU-038) in the file:
//! volume keyframes, fade curves, track stages, stream choices, stream tags
//! and the solo set. How each is spelled, what a hand-edited file gets
//! refused or repaired for, and what survives open → edit → save.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_project::export::ExportSettings;
use freecut_project::{
    Extras, OpenError, OpenWarning, Owner, SaveError, SaveRequest, UiState, decode, encode,
    encode_with_ui, open, save,
};
use freecut_timeline::edit::{EditCtx, EditRequest, GainKeyEdit, Limits};
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::ids::{ClipId, SeqId, TrackId};
use freecut_timeline::model::{
    Animatable, ClipContent, EmbeddedAudio, GainDb, Interp, Keyframe, StreamSel, TrackAudio,
};
use freecut_timeline::time::Tick;
use freecut_timeline::{CommitMeta, Editor};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn sink_value() -> Value {
    let (doc, ids) = kitchen_sink();
    to_value(&encode_with_ui(&doc, ids, None, &kitchen_ui(), &Extras::new()).unwrap())
}

fn open_value(v: &Value) -> Result<freecut_project::Opened, OpenError> {
    decode(&compact(v))
}

fn clip(id: u64) -> ClipId {
    ClipId::from_raw(nz(id))
}

fn track(id: u64) -> TrackId {
    TrackId::from_raw(nz(id))
}

fn bad_value(v: &Value) -> (String, String) {
    match open_value(v) {
        Err(OpenError::BadValue { path, message }) => (path, message),
        other => panic!("expected a bad value, got {other:?}"),
    }
}

fn json_error(v: &Value) -> (String, String) {
    match open_value(v) {
        Err(OpenError::Json { path, message }) => (path, message),
        other => panic!("expected a JSON error, got {other:?}"),
    }
}

/// Track 26 (audio, −3 dB, 40 % left) and its clip 47 with four keys.
const VOICE_TRACK: &str = "/document/sequences/0/audio/0";
const VOICE_KEYS: &str = "/document/sequences/0/audio/0/clips/0/audio/gain_keys";
const VOICE_KEYS_PATH: &str = "document.sequences[0].audio[0].clips[0].audio.gain_keys";
/// Clip 42: a video playing all streams mixed.
const OVERLAY_CONTENT: &str = "/document/sequences/0/visual/1/clips/0/content";
/// Track 22: a text track.
const TEXT_TRACK: &str = "/document/sequences/0/visual/2";

fn keys(v: &mut Value) -> &mut Vec<Value> {
    v.pointer_mut(VOICE_KEYS).unwrap().as_array_mut().unwrap()
}

fn editor_for(opened: &freecut_project::Opened) -> Editor {
    Editor::open(
        opened.document.clone(),
        opened.ids.clone(),
        HistoryLimits::default(),
    )
    .unwrap()
}

fn commit(editor: &mut Editor, req: EditRequest) {
    let ctx = EditCtx {
        limits: Limits::default(),
    };
    let plan = editor.plan(&ctx, &req).unwrap();
    editor.commit(plan, CommitMeta::default()).unwrap();
}

#[test]
fn format_4_sound_is_spelled_out_in_integers_and_tags() {
    let v = sink_value();
    assert_eq!(
        v.pointer(&format!("{VOICE_KEYS}/1")).unwrap(),
        &json!({"at": fr(20).raw(), "gain": "-inf", "interp": "hold"})
    );
    assert_eq!(
        v.pointer(&format!("{VOICE_TRACK}/mix")).unwrap(),
        &json!({"gain": -300, "pan": -40})
    );
    assert_eq!(
        v.pointer(&format!("{OVERLAY_CONTENT}/stream")).unwrap(),
        &json!({"type": "all-mixed"})
    );
    assert_eq!(
        v.pointer("/document/sequences/0/visual/0/clips/0/content/stream")
            .unwrap(),
        &json!({"type": "index", "index": 5})
    );
    assert_eq!(v["ui"], json!({"solo": [20, 27]}));
    // Defaults are left out: the text track has no stage.
    assert!(v.pointer(&format!("{TEXT_TRACK}/mix")).is_none());

    let opened = open_value(&v).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    assert_eq!(opened.ui, kitchen_ui());
}

#[test]
fn keyframes_out_of_order_are_sorted_with_their_unknown_keys() {
    let mut v = sink_value();
    keys(&mut v)[1]["x_ease"] = json!("in");
    keys(&mut v).reverse();
    let opened = open_value(&v).unwrap();
    assert!(
        opened
            .warnings
            .contains(&OpenWarning::KeyframesSorted { clips: 1 })
    );
    assert!(opened.needs_save, "the document differs from the file");
    let (sink, _) = kitchen_sink();
    assert_eq!(opened.document, sink);
    // The unknown key went with its key, filed by the key's time.
    let owner = Owner::Clip(clip(47));
    assert!(
        opened
            .extras
            .get(owner, &format!("audio.gain_keys@{}", fr(20).raw()))
            .unwrap()
            .contains_key("x_ease")
    );
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(again.pointer(VOICE_KEYS).unwrap()[1]["x_ease"], "in");
    assert_eq!(
        again.pointer(VOICE_KEYS).unwrap()[0]["at"],
        json!(fr(10).raw())
    );
}

#[test]
fn unknown_keys_of_a_keyframe_stay_with_it_when_an_earlier_one_goes() {
    let mut v = sink_value();
    keys(&mut v)[1]["x_ease"] = json!("in");
    let first = Tick::from_raw(keys(&mut v)[0]["at"].as_i64().unwrap());
    let opened = open_value(&v).unwrap();
    let mut editor = editor_for(&opened);
    commit(
        &mut editor,
        EditRequest::EditGainKeys {
            seq: SeqId::from_raw(nz(1)),
            clips: vec![clip(47)],
            edit: GainKeyEdit::Remove(vec![first]),
        },
    );
    let again = to_value(&encode(editor.document(), editor.ids().next(), &opened.extras).unwrap());
    let ease: Vec<(i64, Option<&Value>)> = again
        .pointer(VOICE_KEYS)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|k| (k["at"].as_i64().unwrap(), k.get("x_ease")))
        .collect();
    assert_eq!(ease.len(), 3);
    let eased = json!("in");
    for (at, x) in ease {
        let expected = (at == fr(20).raw()).then_some(&eased);
        assert_eq!(x, expected, "key at {at}");
    }
}

#[test]
fn repairs_on_open_keep_the_track_stage() {
    // A clip of track 26 (−3 dB, 40 % left) in a group no other clip has.
    let mut v = sink_value();
    v.pointer_mut(&format!("{VOICE_TRACK}/clips/0"))
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("group".to_owned(), json!(9000));
    let opened = open_value(&v).unwrap();
    assert!(
        opened
            .warnings
            .iter()
            .any(|w| matches!(w, OpenWarning::RelationsDissolved { groups: 1, .. })),
        "{:?}",
        opened.warnings
    );
    let voice = opened
        .document
        .sequence(SeqId::from_raw(nz(1)))
        .unwrap()
        .tracks()
        .find(|(_, t)| t.id == track(26))
        .unwrap()
        .1
        .mix;
    assert_eq!(
        voice,
        TrackAudio {
            gain: GainDb::from_hundredths(-300),
            pan: freecut_timeline::model::Pan::new(-40),
        }
    );
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(
        again.pointer(&format!("{VOICE_TRACK}/mix")).unwrap(),
        &json!({"gain": -300, "pan": -40})
    );
}

#[test]
fn two_keyframes_at_one_time_are_refused() {
    let mut v = sink_value();
    keys(&mut v)[3]["at"] = json!(fr(20).raw());
    let (path, message) = bad_value(&v);
    assert_eq!(path, format!("{VOICE_KEYS_PATH}[3].at"));
    assert!(
        message.contains(&format!("{VOICE_KEYS_PATH}[1]")),
        "{message}"
    );
}

#[test]
fn keyframe_values_outside_the_model_are_refused() {
    let cases = [
        ("gain", json!(2001)),
        ("gain", json!(-6001)),
        ("gain", json!("loud")),
        ("at", json!((1i64 << 53) + 1)),
        ("interp", json!("bezier")),
    ];
    for (key, value) in cases {
        let mut v = sink_value();
        keys(&mut v)[2][key] = value.clone();
        let path = match open_value(&v) {
            Err(OpenError::BadValue { path, .. } | OpenError::Json { path, .. }) => path,
            other => panic!("{key} = {value}: {other:?}"),
        };
        assert_eq!(path, format!("{VOICE_KEYS_PATH}[2].{key}"), "{value}");
    }
}

#[test]
fn a_keyframe_too_late_for_the_file_is_refused_on_save() {
    let (mut doc, ids) = kitchen_sink();
    let mut editor = Editor::open(
        doc.clone(),
        freecut_timeline::ids::IdAlloc::starting_at(ids),
        HistoryLimits::default(),
    )
    .unwrap();
    commit(
        &mut editor,
        EditRequest::EditGainKeys {
            seq: SeqId::from_raw(nz(1)),
            clips: vec![clip(48)],
            edit: GainKeyEdit::Set {
                at: Tick::from_raw((1 << 53) + 1),
                gain: Some(GainDb::ZERO),
            },
        },
    );
    doc = editor.document().clone();
    match encode(&doc, ids, &Extras::new()) {
        Err(SaveError::OutOfRange { path, .. }) => assert_eq!(
            path,
            "document.sequences[0].audio[1].clips[0].audio.gain_keys[0].at"
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn track_stages_outside_their_range_are_refused() {
    let mix = format!("{VOICE_TRACK}/mix");
    let mix_path = "document.sequences[0].audio[0].mix";
    let cases = [
        ("pan", json!(101)),
        ("pan", json!(-101)),
        ("pan", json!(i64::MIN)),
        ("gain", json!(1201)),
        ("gain", json!(-6001)),
    ];
    for (key, value) in cases {
        let mut v = sink_value();
        v.pointer_mut(&mix).unwrap()[key] = value.clone();
        let (path, _) = bad_value(&v);
        assert_eq!(path, format!("{mix_path}.{key}"), "{value}");
    }
    // The extremes are fine.
    let mut v = sink_value();
    v.pointer_mut(&mix).unwrap()["pan"] = json!(100);
    v.pointer_mut(&mix).unwrap()["gain"] = json!(1200);
    open_value(&v).unwrap();
    // A pan that isn't an integer is a schema error.
    let mut v = sink_value();
    v.pointer_mut(&mix).unwrap()["pan"] = json!("left");
    assert_eq!(json_error(&v).0, format!("{mix_path}.pan"));
}

#[test]
fn a_track_without_sound_keeps_the_default_stage() {
    let mut v = sink_value();
    v.pointer_mut(TEXT_TRACK).unwrap()["mix"] = json!({"gain": 0, "pan": 0, "x": 1});
    let opened = open_value(&v).unwrap();
    // Spelled-out defaults (with an unknown key) are kept and written back.
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(
        again.pointer(&format!("{TEXT_TRACK}/mix")).unwrap(),
        &json!({"gain": 0, "pan": 0, "x": 1})
    );
    v.pointer_mut(TEXT_TRACK).unwrap()["mix"] = json!({"gain": -600, "pan": 0});
    let (path, _) = bad_value(&v);
    assert_eq!(path, "document.sequences[0].visual[2].mix");
}

#[test]
fn track_stages_survive_an_edit_and_a_save() {
    let opened = open_value(&sink_value()).unwrap();
    let mut editor = editor_for(&opened);
    commit(
        &mut editor,
        EditRequest::SetTrackPan {
            seq: SeqId::from_raw(nz(1)),
            track: track(26),
            pan: freecut_timeline::model::Pan::new(75),
        },
    );
    let bytes = encode(editor.document(), editor.ids().next(), &opened.extras).unwrap();
    let reopened = decode(&bytes).unwrap();
    let voice = reopened
        .document
        .sequence(SeqId::from_raw(nz(1)))
        .unwrap()
        .tracks()
        .find(|(_, t)| t.id == track(26))
        .unwrap()
        .1
        .mix;
    assert_eq!(
        voice,
        TrackAudio {
            gain: GainDb::from_hundredths(-300),
            pan: freecut_timeline::model::Pan::new(75),
        }
    );
}

#[test]
fn a_stream_the_media_lacks_is_kept_and_reported() {
    let mut v = sink_value();
    v.pointer_mut(OVERLAY_CONTENT).unwrap()["stream"] = json!({"type": "index", "index": 9});
    let opened = open_value(&v).unwrap();
    assert!(opened.warnings.contains(&OpenWarning::MissingStreams {
        clips: vec![clip(42)]
    }));
    assert!(!opened.needs_save, "nothing was changed");
    match &opened.document.clip(clip(42)).unwrap().content {
        ClipContent::Media { stream, .. } => assert_eq!(*stream, Some(StreamSel::Index(9))),
        other => panic!("{other:?}"),
    }
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(
        again.pointer(&format!("{OVERLAY_CONTENT}/stream")).unwrap(),
        &json!({"type": "index", "index": 9})
    );
}

#[test]
fn malformed_stream_choices_and_curves_name_their_path() {
    let overlay = "document.sequences[0].visual[1].clips[0].content.stream";
    let cases = [
        (json!({"type": "first"}), format!("{overlay}.type")),
        (json!({"type": "index"}), overlay.to_string()),
        (
            json!({"type": "index", "index": -1}),
            format!("{overlay}.index"),
        ),
        (json!(3), overlay.to_string()),
    ];
    for (stream, want) in cases {
        let mut v = sink_value();
        v.pointer_mut(OVERLAY_CONTENT).unwrap()["stream"] = stream.clone();
        assert_eq!(json_error(&v).0, want, "{stream}");
    }
    let mut v = sink_value();
    v.pointer_mut("/document/sequences/0/audio/1/clips/0/audio")
        .unwrap()["fade_out_curve"] = json!("exponential");
    assert_eq!(
        json_error(&v).0,
        "document.sequences[0].audio[1].clips[0].audio.fade_out_curve"
    );
    // A format-4 key the step always writes is required.
    let mut v = sink_value();
    v.pointer_mut("/document/sequences/0/audio/1/clips/0/audio")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("gain_keys");
    let (path, message) = json_error(&v);
    assert_eq!(path, "document.sequences[0].audio[1].clips[0].audio");
    assert!(message.contains("gain_keys"), "{message}");
}

#[test]
fn the_solo_set_keeps_only_sound_carrying_tracks_of_the_document() {
    let mut v = sink_value();
    // 22 is a text track, 77 no track at all.
    v["ui"]["solo"] = json!([77, 22, 26]);
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.ui.solo.iter().copied().collect::<Vec<_>>(),
        vec![track(26)]
    );
    assert!(!opened.needs_save, "UI state is no repair");
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    // An id no file can hold is refused like any other.
    v["ui"]["solo"] = json!([1u64 << 60]);
    assert_eq!(bad_value(&v).0, "ui.solo[0]");
}

#[test]
fn the_solo_set_survives_save_and_reopen_but_not_its_tracks_removal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.freecut");
    let (doc, ids) = kitchen_sink();
    let ui = kitchen_ui();
    let request = SaveRequest {
        document: &doc,
        ids_next: ids,
        extras: &Extras::new(),
        backup: false,
        relative_base: None,
        export: None,
        ui: Some(&ui),
    };
    save(&path, &request).unwrap();
    let opened = open(&path).unwrap();
    assert_eq!(opened.ui, ui);
    assert!(!opened.needs_save);

    let mut editor = editor_for(&opened);
    // Removing a soloed track clears its solo on the next save (AU-012).
    // Deleting its last clips removes the emptied audio track.
    commit(
        &mut editor,
        EditRequest::Delete(freecut_timeline::edit::DeleteReq {
            seq: SeqId::from_raw(nz(1)),
            clips: vec![clip(48), clip(50)],
            ripple: false,
        }),
    );
    let root = editor.document().sequence(SeqId::from_raw(nz(1))).unwrap();
    if root.tracks().any(|(_, t)| t.id == track(27)) {
        commit(
            &mut editor,
            EditRequest::RemoveTrack {
                seq: SeqId::from_raw(nz(1)),
                track: track(27),
            },
        );
    }
    let request = SaveRequest {
        document: editor.document(),
        ids_next: editor.ids().next(),
        extras: &opened.extras,
        backup: false,
        relative_base: None,
        export: None,
        ui: Some(&opened.ui),
    };
    save(&path, &request).unwrap();
    let v = to_value(&std::fs::read(&path).unwrap());
    assert_eq!(v["ui"], json!({"solo": [20]}));
    assert_eq!(
        open(&path).unwrap().ui.solo.into_iter().collect::<Vec<_>>(),
        vec![track(20)]
    );

    // An empty solo set leaves the key out.
    let request = SaveRequest {
        ui: Some(&UiState::new()),
        ..request
    };
    save(&path, &request).unwrap();
    assert!(to_value(&std::fs::read(&path).unwrap()).get("ui").is_none());
}

#[test]
fn keyframes_and_curves_round_trip_through_edits() {
    let opened = open_value(&sink_value()).unwrap();
    let mut editor = editor_for(&opened);
    let seq = SeqId::from_raw(nz(1));
    commit(
        &mut editor,
        EditRequest::EditGainKeys {
            seq,
            clips: vec![clip(50)],
            edit: GainKeyEdit::Set {
                at: fr(30),
                gain: Some(GainDb::SILENT),
            },
        },
    );
    commit(
        &mut editor,
        EditRequest::EditGainKeys {
            seq,
            clips: vec![clip(50)],
            edit: GainKeyEdit::Set {
                at: fr(25),
                gain: None,
            },
        },
    );
    let bytes = encode(editor.document(), editor.ids().next(), &opened.extras).unwrap();
    let reopened = decode(&bytes).unwrap();
    assert_eq!(&reopened.document, editor.document());
    let audio = reopened.document.clip(clip(50)).unwrap().audio.clone();
    let EmbeddedAudio::Active(audio) = audio else {
        panic!("{audio:?}")
    };
    assert_eq!(
        audio.gain,
        Animatable::from_parts(
            GainDb::MIN,
            vec![
                Keyframe {
                    // Before the only key, the volume is that key's.
                    at: fr(25),
                    value: GainDb::SILENT,
                    interp: Interp::Linear
                },
                Keyframe {
                    at: fr(30),
                    value: GainDb::SILENT,
                    interp: Interp::Linear
                },
            ]
        )
        .unwrap()
    );
}

#[test]
fn audio_export_follows_keyframes_and_track_volume() {
    let (doc, ids) = kitchen_sink();
    let mut editor = Editor::open(
        doc,
        freecut_timeline::ids::IdAlloc::starting_at(ids),
        HistoryLimits::default(),
    )
    .unwrap();
    let seq = SeqId::from_raw(nz(1));
    let include = |editor: &Editor| {
        ExportSettings::defaults("p", None, editor.document())
            .audio
            .include
    };
    assert!(include(&editor), "clip 50 plays on track 27");
    // Track 27 at −∞: every clip with sound is muted, silent or on a silent
    // track (EX-031).
    commit(
        &mut editor,
        EditRequest::SetTrackGain {
            seq,
            track: track(27),
            gain: GainDb::SILENT,
        },
    );
    assert!(!include(&editor), "the timeline is silent");
    // A keyframe above −∞ on the silent clip 42 makes it audible.
    commit(
        &mut editor,
        EditRequest::EditGainKeys {
            seq,
            clips: vec![clip(42)],
            edit: GainKeyEdit::Set {
                at: fr(5),
                gain: Some(GainDb::ZERO),
            },
        },
    );
    assert!(include(&editor));
}
