//! Clip sound (AU-002, AU-005, AU-006, AU-007, TL-030) and extracted audio
//! items (AU-004) in the file: how they are spelled, what is refused with
//! its JSON path, and what opens although its partner is gone.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_project::{Extras, OpenError, OpenWarning, SaveError, decode, encode};
use freecut_timeline::edit::{AudioEdit, EditCtx, EditRequest, Limits};
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::ids::{ClipId, LinkId, MediaId, SeqId};
use freecut_timeline::model::{
    ClipAudio, EmbeddedAudio, GainDb, InvariantViolation, MasterAudio, MediaSource,
};
use freecut_timeline::time::Tick;
use freecut_timeline::{CommitMeta, Editor};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn sink_value() -> Value {
    let (doc, ids) = kitchen_sink();
    to_value(&encode(&doc, ids, &Extras::new()).unwrap())
}

fn open_value(v: &Value) -> Result<freecut_project::Opened, OpenError> {
    decode(&compact(v))
}

fn clip(id: u64) -> ClipId {
    ClipId::from_raw(nz(id))
}

// JSON pointers into the kitchen sink.
/// Clip 40: a detached video, linked to clip 47.
const INTRO: &str = "/document/sequences/0/visual/0/clips/0";
/// Clip 41: an image.
const STILL: &str = "/document/sequences/0/visual/0/clips/1";
/// Clip 47: the audio clip detached from clip 40.
const VOICE: &str = "/document/sequences/0/audio/0/clips/0";
/// Clip 48: an audio clip with gain, mute and fades.
const MUSIC: &str = "/document/sequences/0/audio/1/clips/0";
/// Item 13: stream 5 of item 10, extracted.
const EXTRACTED: &str = "/document/library/3";

/// `pointer` as a JSON path in error reports.
fn path_of(pointer: &str) -> String {
    pointer
        .trim_start_matches('/')
        .split('/')
        .fold(String::new(), |path, part| match part.parse::<usize>() {
            Ok(i) => format!("{path}[{i}]"),
            Err(_) if path.is_empty() => part.to_string(),
            Err(_) => format!("{path}.{part}"),
        })
}

fn with(pointer: &str, value: Value) -> Value {
    let mut v = sink_value();
    *v.pointer_mut(pointer)
        .unwrap_or_else(|| panic!("nothing at {pointer}")) = value;
    v
}

fn without(pointer: &str, key: &str) -> Value {
    let mut v = sink_value();
    v.pointer_mut(pointer)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove(key)
        .unwrap();
    v
}

#[test]
fn sound_and_sources_are_spelled_out_in_integers_and_tags() {
    let v = sink_value();
    assert_eq!(
        v.pointer(&format!("{INTRO}/audio")).unwrap(),
        &json!({"type": "detached", "link": 91})
    );
    assert_eq!(
        v.pointer(&format!("{STILL}/audio")).unwrap(),
        &json!({"type": "none"})
    );
    assert_eq!(
        v.pointer(&format!("{MUSIC}/audio")).unwrap(),
        &json!({
            "type": "active",
            // −12.5 dB in hundredths of a decibel.
            "gain": -1250,
            "gain_keys": [],
            "muted": true,
            "fade_in": fr(30).raw(),
            "fade_in_curve": "fast",
            // Longer than the 120-frame clip: stored as set.
            "fade_out": fr(200).raw(),
            "fade_out_curve": "slow",
        })
    );
    // −∞ dB has no JSON number.
    assert_eq!(
        v.pointer("/document/sequences/0/visual/1/clips/0/audio/gain"),
        Some(&json!("-inf"))
    );
    // A detached video whose audio clip is gone: no link, still detached.
    let orphan = v.pointer("/document/sequences/1/visual/0/clips/0").unwrap();
    assert_eq!(orphan["link"], Value::Null);
    assert_eq!(orphan["audio"], json!({"type": "detached", "link": 92}));

    assert_eq!(
        v["document"]["library"][0]["source"],
        json!({"type": "file"})
    );
    assert_eq!(
        v.pointer(&format!("{EXTRACTED}/source")).unwrap(),
        &json!({"type": "audio-of", "media": 10, "stream": 5})
    );
}

#[test]
fn every_gain_and_fade_round_trips() {
    let gains = [-6000, -5999, -1, 0, 1, 1999, 2000];
    for hundredths in gains {
        let v = with(&format!("{MUSIC}/audio/gain"), json!(hundredths));
        let opened = open_value(&v).unwrap();
        let audio = opened.document.clip(clip(48)).unwrap().audio.clone();
        assert_eq!(
            audio.active().unwrap().gain.static_value().hundredths(),
            hundredths
        );
        let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
        assert_eq!(again, v);
    }
    let v = with(&format!("{MUSIC}/audio/fade_in"), json!(1i64 << 53));
    let opened = open_value(&v).unwrap();
    let audio = opened.document.clip(clip(48)).unwrap().audio.clone();
    assert_eq!(audio.active().unwrap().fade_in, Tick::from_raw(1 << 53));
}

fn json_error(v: &Value) -> (String, String) {
    match open_value(v) {
        Err(OpenError::Json { path, message }) => (path, message),
        other => panic!("expected a JSON error, got {other:?}"),
    }
}

fn bad_value(v: &Value) -> (String, String) {
    match open_value(v) {
        Err(OpenError::BadValue { path, message }) => (path, message),
        other => panic!("expected a bad value, got {other:?}"),
    }
}

#[test]
fn schema_errors_in_sound_and_sources_name_their_path() {
    let music = path_of(MUSIC);
    let intro = path_of(INTRO);
    let item = path_of(EXTRACTED);
    let cases: &[(String, Value, String, &str)] = &[
        (
            format!("{MUSIC}/audio"),
            json!("active"),
            format!("{music}.audio"),
            "expected a clip audio object",
        ),
        (
            format!("{MUSIC}/audio/type"),
            json!("loud"),
            format!("{music}.audio.type"),
            "unknown variant `loud`, expected one of `none`, `active`, `detached`",
        ),
        (
            format!("{MUSIC}/audio/muted"),
            json!("yes"),
            format!("{music}.audio.muted"),
            "invalid type",
        ),
        (
            format!("{MUSIC}/audio/gain"),
            json!("-infinity"),
            format!("{music}.audio.gain"),
            "a gain in hundredths of a decibel",
        ),
        (
            format!("{MUSIC}/audio/gain"),
            json!(null),
            format!("{music}.audio.gain"),
            "invalid type: null",
        ),
        (
            format!("{MUSIC}/audio/gain"),
            json!(-3.5),
            format!("{music}.audio.gain"),
            "invalid",
        ),
        (
            format!("{MUSIC}/audio/fade_out"),
            json!("long"),
            format!("{music}.audio.fade_out"),
            "invalid type",
        ),
        (
            format!("{INTRO}/audio/link"),
            json!(0),
            format!("{intro}.audio.link"),
            "nonzero",
        ),
        (
            format!("{EXTRACTED}/source/type"),
            json!("proxy"),
            format!("{item}.source.type"),
            "unknown variant `proxy`, expected one of `file`, `audio-of`",
        ),
        (
            format!("{EXTRACTED}/source/stream"),
            json!(-1),
            format!("{item}.source.stream"),
            // serde_json's arbitrary-precision numbers report "invalid number".
            "invalid",
        ),
        (
            format!("{EXTRACTED}/source"),
            json!(null),
            format!("{item}.source"),
            "expected a media source object",
        ),
    ];
    for (pointer, value, expected, message_part) in cases {
        let (path, message) = json_error(&with(pointer, value.clone()));
        assert_eq!(&path, expected, "{pointer} = {value}");
        assert!(message.contains(message_part), "{pointer}: {message}");
    }

    // Missing keys are reported at the object that lacks them.
    let (path, message) = json_error(&without(MUSIC, "audio"));
    assert_eq!(path, music);
    assert!(message.contains("missing field `audio`"), "{message}");
    let (path, message) = json_error(&without(&format!("{MUSIC}/audio"), "fade_in"));
    assert_eq!(path, format!("{music}.audio"));
    assert!(message.contains("missing field `fade_in`"), "{message}");
    let (path, message) = json_error(&without(&format!("{INTRO}/audio"), "type"));
    assert_eq!(
        (path, message),
        (format!("{intro}.audio"), "missing field `type`".to_string())
    );
    let (path, message) = json_error(&without(EXTRACTED, "source"));
    assert_eq!(path, item);
    assert!(message.contains("missing field `source`"), "{message}");
}

#[test]
fn values_outside_the_model_are_refused_never_clamped() {
    let music = path_of(MUSIC);
    let cases: &[(&str, Value, String)] = &[
        // Above +20 dB and below −60 dB (AU-005).
        ("audio/gain", json!(2001), format!("{music}.audio.gain")),
        ("audio/gain", json!(-6001), format!("{music}.audio.gain")),
        // −∞ is spelled "-inf", not as the model's sentinel.
        (
            "audio/gain",
            json!(i64::from(i32::MIN)),
            format!("{music}.audio.gain"),
        ),
        ("audio/gain", json!(i64::MAX), format!("{music}.audio.gain")),
        ("audio/fade_in", json!(-1), format!("{music}.audio.fade_in")),
        (
            "audio/fade_out",
            json!((1i64 << 53) + 1),
            format!("{music}.audio.fade_out"),
        ),
    ];
    for (field, value, expected) in cases {
        let (path, message) = bad_value(&with(&format!("{MUSIC}/{field}"), value.clone()));
        assert_eq!(&path, expected, "{field} = {value}");
        assert!(!message.is_empty());
    }
    let (path, _) = bad_value(&with(&format!("{INTRO}/audio/link"), json!(1u64 << 60)));
    assert_eq!(path, format!("{}.audio.link", path_of(INTRO)));
    let (path, _) = bad_value(&with(
        &format!("{EXTRACTED}/source/media"),
        json!(1u64 << 60),
    ));
    assert_eq!(path, format!("{}.source.media", path_of(EXTRACTED)));
}

fn invalid(v: &Value) -> Vec<InvariantViolation> {
    match open_value(v) {
        Err(OpenError::Invalid(violations)) => violations,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn sound_that_doesnt_fit_its_clip_is_refused() {
    let active = json!({
        "type": "active", "gain": 0, "gain_keys": [], "muted": false,
        "fade_in": 0, "fade_in_curve": "linear", "fade_out": 0, "fade_out_curve": "linear"
    });
    let mismatch = |id: u64| InvariantViolation::AudioMismatch { clip: clip(id) };
    // An image can't play sound.
    assert!(invalid(&with(&format!("{STILL}/audio"), active)).contains(&mismatch(41)));
    // An audio clip always plays its own.
    assert!(
        invalid(&with(&format!("{MUSIC}/audio"), json!({"type": "none"}))).contains(&mismatch(48))
    );
    assert!(
        invalid(&with(
            &format!("{VOICE}/audio"),
            json!({"type": "detached", "link": 91})
        ))
        .contains(&mismatch(47))
    );
    // A detached video names the link it carries.
    assert!(invalid(&with(&format!("{INTRO}/audio/link"), json!(93))).contains(&mismatch(40)));
}

#[test]
fn an_extracted_item_holds_exactly_its_one_stream() {
    let derived = InvariantViolation::DerivedMedia {
        media: MediaId::from_raw(nz(13)),
    };
    // A stream it doesn't have.
    assert!(invalid(&with(&format!("{EXTRACTED}/source/stream"), json!(4))).contains(&derived));
    // A picture.
    let v = with(&format!("{EXTRACTED}/info/default_video"), json!(0));
    assert!(invalid(&v).contains(&derived));
    // Itself as the parent.
    assert!(invalid(&with(&format!("{EXTRACTED}/source/media"), json!(13))).contains(&derived));
}

#[test]
fn an_extracted_item_whose_parent_was_removed_still_opens() {
    // The parent, item 95, was removed from the library after the
    // extraction; the extracted item has its own copy of the path.
    let mut v = with(&format!("{EXTRACTED}/source/media"), json!(95));
    let opened = open_value(&v).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    let item = opened
        .document
        .library()
        .get(MediaId::from_raw(nz(13)))
        .unwrap();
    assert_eq!(
        item.source,
        MediaSource::AudioOf {
            source: MediaId::from_raw(nz(95)),
            stream: 5
        }
    );

    // The parent's id is never handed out again: a new item with it would
    // look like the parent.
    v["ids_next"] = json!(50);
    let opened = open_value(&v).unwrap();
    assert_eq!(opened.ids.next(), nz(96));
    assert!(opened.warnings.contains(&OpenWarning::IdsNextRaised {
        stored: 50,
        raised_to: 96
    }));
}

#[test]
fn a_detached_video_left_alone_in_its_link_stays_detached() {
    // The detached audio clip 47 is gone, but the video still carries the
    // link (written by another tool, or by hand): the link dissolves on
    // open (TL-011), the sound stays detached.
    let mut v = sink_value();
    v["document"]["sequences"][0]["audio"][0]["clips"] = json!([]);
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::RelationsDissolved {
            seq: SeqId::from_raw(nz(1)),
            groups: 0,
            links: 1,
        }]
    );
    assert!(opened.needs_save && opened.backup_on_save());
    let intro = opened.document.clip(clip(40)).unwrap();
    assert_eq!(intro.link, None);
    assert_eq!(
        intro.group.map(|g| g.get()),
        Some(90),
        "the group is intact"
    );
    assert_eq!(
        intro.audio,
        EmbeddedAudio::Detached(LinkId::from_raw(nz(91)))
    );

    // A group left with one member dissolves the same way.
    let mut v = sink_value();
    v["document"]["sequences"][0]["visual"][0]["clips"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::RelationsDissolved {
            seq: SeqId::from_raw(nz(1)),
            groups: 1,
            links: 0,
        }]
    );
    assert_eq!(opened.document.clip(clip(40)).unwrap().group, None);
}

#[test]
fn restoring_a_detached_video_after_open_gives_the_sound_back() {
    let (doc, ids) = kitchen_sink();
    let opened = decode(&encode(&doc, ids, &Extras::new()).unwrap()).unwrap();
    let mut editor = Editor::open(
        opened.document.clone(),
        opened.ids.clone(),
        HistoryLimits::default(),
    )
    .unwrap();
    let ctx = EditCtx {
        limits: Limits::default(),
    };
    // Clip 49 lost its audio clip before the save; restoring re-enables
    // its sound with default settings (TL-030).
    let restore = EditRequest::RestoreAudio {
        seq: SeqId::from_raw(nz(2)),
        clips: vec![clip(49)],
    };
    let plan = editor.plan(&ctx, &restore).unwrap();
    editor.commit(plan, CommitMeta::default()).unwrap();
    assert_eq!(
        editor.document().clip(clip(49)).unwrap().audio,
        EmbeddedAudio::Active(ClipAudio::default())
    );
    // And the result saves and opens again.
    let bytes = encode(editor.document(), editor.ids().next(), &opened.extras).unwrap();
    assert_eq!(&decode(&bytes).unwrap().document, editor.document());
}

#[test]
fn a_fade_too_long_for_the_file_is_refused_on_save() {
    let (doc, ids) = kitchen_sink();
    let mut editor = Editor::open(
        doc,
        freecut_timeline::ids::IdAlloc::starting_at(ids),
        HistoryLimits::default(),
    )
    .unwrap();
    let ctx = EditCtx {
        limits: Limits::default(),
    };
    let fade = EditRequest::SetClipAudio {
        seq: SeqId::from_raw(nz(1)),
        clips: vec![clip(48)],
        edit: AudioEdit::FadeOut(Tick::from_raw((1 << 53) + 1)),
    };
    let plan = editor.plan(&ctx, &fade).unwrap();
    editor.commit(plan, CommitMeta::default()).unwrap();
    match encode(editor.document(), editor.ids().next(), &Extras::new()) {
        Err(SaveError::OutOfRange { path, message }) => {
            assert_eq!(path, format!("{}.audio.fade_out", path_of(MUSIC)));
            assert!(message.contains("9007199254740993"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

/// The JSON object of clip `id`, wherever it is.
fn clip_json(v: &mut Value, id: u64) -> &mut Value {
    v["document"]["sequences"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .flat_map(|seq| {
            seq.as_object_mut()
                .unwrap()
                .iter_mut()
                .filter(|(k, _)| *k == "visual" || *k == "audio")
                .flat_map(|(_, tracks)| tracks.as_array_mut().unwrap().iter_mut())
        })
        .flat_map(|track| track["clips"].as_array_mut().unwrap().iter_mut())
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no clip {id}"))
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

/// How often `key` appears as a key of the `audio` object of clip `id` in
/// the file text (a JSON value can't show a duplicate key).
fn audio_key_count(bytes: &[u8], id: u64, key: &str) -> usize {
    let text = std::str::from_utf8(bytes).unwrap();
    let at = text.find(&format!("\"id\": {id},")).unwrap();
    let open = at + text[at..].find("\"audio\": {").unwrap() + "\"audio\": ".len();
    // The object's own keys: those at depth 1 (keyframes nest objects).
    let (mut depth, mut count) = (0usize, 0);
    let needle = format!("\"{key}\":");
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            '"' if depth == 1 && text[open + i..].starts_with(&needle) => count += 1,
            _ => {}
        }
    }
    count
}

#[test]
fn unknown_keys_of_another_sound_variant_never_override_the_sound() {
    // Restore: the detached object of clip 40 carried an unknown `muted`.
    let mut v = sink_value();
    clip_json(&mut v, 40)["audio"]["muted"] = json!(true);
    let opened = open_value(&v).unwrap();
    let mut editor = editor_for(&opened);
    commit(
        &mut editor,
        EditRequest::RestoreAudio {
            seq: SeqId::from_raw(nz(1)),
            clips: vec![clip(40)],
        },
    );
    let restored = editor.document().clip(clip(40)).unwrap().audio.clone();
    assert!(!restored.active().unwrap().muted);
    let bytes = encode(editor.document(), editor.ids().next(), &opened.extras).unwrap();
    assert_eq!(audio_key_count(&bytes, 40, "muted"), 1);
    let reopened = decode(&bytes).unwrap();
    assert_eq!(reopened.document.clip(clip(40)).unwrap().audio, restored);

    // Detach: the active object of clip 42 carried an unknown `link`, and
    // unknown keys named like every other field of either variant.
    let mut v = sink_value();
    clip_json(&mut v, 42)["audio"]["link"] = json!(3);
    for (id, keys) in [
        (40, ["gain", "muted", "fade_in", "fade_out"].as_slice()),
        (42, ["link"].as_slice()),
    ] {
        for key in keys {
            clip_json(&mut v, id)["audio"][*key] = json!("stale");
        }
    }
    let opened = open_value(&v).unwrap();
    let mut editor = editor_for(&opened);
    commit(
        &mut editor,
        EditRequest::DetachAudio {
            seq: SeqId::from_raw(nz(1)),
            clips: vec![clip(42)],
        },
    );
    commit(
        &mut editor,
        EditRequest::RestoreAudio {
            seq: SeqId::from_raw(nz(1)),
            clips: vec![clip(40)],
        },
    );
    let bytes = encode(editor.document(), editor.ids().next(), &opened.extras).unwrap();
    for (id, key) in [(42, "link"), (40, "gain"), (40, "fade_in")] {
        assert_eq!(audio_key_count(&bytes, id, key), 1, "clip {id} {key}");
    }
    assert_eq!(&decode(&bytes).unwrap().document, editor.document());
}

#[test]
fn a_clip_that_cant_belong_to_a_link_loses_it_on_open_and_survives_restore() {
    // The image (clip 41) carries the link of the detached video 40 and its
    // audio clip 47.
    let mut v = sink_value();
    clip_json(&mut v, 41)["link"] = json!(91);
    let opened = open_value(&v).unwrap();
    assert!(opened.warnings.contains(&OpenWarning::StrayLinksCleared {
        seq: SeqId::from_raw(nz(1)),
        clips: 1,
    }));
    assert!(opened.needs_save);
    assert_eq!(opened.document.clip(clip(41)).unwrap().link, None);
    let mut editor = editor_for(&opened);
    commit(
        &mut editor,
        EditRequest::RestoreAudio {
            seq: SeqId::from_raw(nz(1)),
            clips: vec![clip(40)],
        },
    );
    assert!(editor.document().clip(clip(41)).is_some());
    assert!(editor.document().clip(clip(47)).is_none());
}

// ------------------------------------------------------------ master volume

const SEQ: &str = "/document/sequences/0";

#[test]
fn a_default_master_volume_is_left_out_of_the_file() {
    let v = sink_value();
    assert_eq!(v.pointer(&format!("{SEQ}/master")), None);
    let opened = open_value(&v).unwrap();
    let root = opened.document.root();
    assert_eq!(
        opened.document.sequence(root).unwrap().master,
        MasterAudio::default()
    );
}

#[test]
fn the_master_volume_is_saved_and_reopened() {
    let opened = open_value(&sink_value()).unwrap();
    let mut editor = editor_for(&opened);
    let seq = editor.document().root();
    for gain in [
        GainDb::from_hundredths(-650),
        GainDb::SILENT,
        MasterAudio::MAX_GAIN,
    ] {
        commit(&mut editor, EditRequest::SetMasterGain { seq, gain });
        let bytes = encode(editor.document(), editor.ids().next(), &opened.extras).unwrap();
        let v = to_value(&bytes);
        let expected = if gain.is_silent() {
            json!("-inf")
        } else {
            json!(gain.hundredths())
        };
        assert_eq!(v.pointer(&format!("{SEQ}/master/gain")), Some(&expected));
        let again = decode(&bytes).unwrap();
        assert_eq!(again.document.sequence(seq).unwrap().master.gain, gain);
    }
}

#[test]
fn a_master_volume_above_plus_12_db_is_refused() {
    let mut v = sink_value();
    v.pointer_mut(SEQ)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("master".to_owned(), json!({"gain": 1201}));
    let (path, message) = bad_value(&v);
    assert_eq!(path, format!("{}.master.gain", path_of(SEQ)));
    assert!(message.contains("1200"), "{message}");
    // Unknown keys next to the gain survive a round trip.
    let mut v = sink_value();
    v.pointer_mut(SEQ)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("master".to_owned(), json!({"gain": 1200, "later": true}));
    let opened = open_value(&v).unwrap();
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(again, v);
}
