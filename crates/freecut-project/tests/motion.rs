//! Format 6's keyframes and animations (TR-027, TR-031, TX-021, FX-018,
//! FX-019, TX-022) in the file: how each is spelled, what a hand-edited
//! file gets refused or repaired for, and what survives open → edit → save.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::sync::Arc;

use common::*;
use freecut_project::{
    Extras, OpenError, OpenWarning, Owner, SaveError, decode, encode, encode_with_ui,
};
use freecut_timeline::edit::{EditCtx, EditRequest, KeyEdit, Limits};
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::ids::{ClipId, SeqId, TrackId};
use freecut_timeline::model::{
    AnimationSpec, Clip, ClipContent, ClipKeys, DeciDegrees, Document, EaseCurve, EaseDir, Interp,
    Keyframe, Library, PerMille, PresetId, ProjectSettings, PropPath, PropValue, Sequence,
    SequenceFlags, Slot, TextClipData, Track, TrackHeader, TrackKind, TransformProp,
};
use freecut_timeline::time::{Tick, TimeRange};
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

/// Clip 43, the title: keys on every kind of value, In, Out and Loop.
const TITLE: &str = "/document/sequences/0/visual/2/clips/0";
const TITLE_PATH: &str = "document.sequences[0].visual[2].clips[0]";
/// Clip 42, a video with an In.
const OVERLAY: &str = "/document/sequences/0/visual/1/clips/0";
const OVERLAY_PATH: &str = "document.sequences[0].visual[1].clips[0]";
/// Clip 47, an audio clip.
const VOICE: &str = "/document/sequences/0/audio/0/clips/0";
const VOICE_PATH: &str = "document.sequences[0].audio[0].clips[0]";
/// Clip 44, a sticker with a Loop.
const STICKER: &str = "/document/sequences/0/visual/3/clips/0";

fn at<'a>(v: &'a mut Value, pointer: &str) -> &'a mut Value {
    v.pointer_mut(pointer)
        .unwrap_or_else(|| panic!("nothing at {pointer}"))
}

/// The index of `prop`'s key track on the title.
fn track_index(v: &Value, prop: &str) -> usize {
    v.pointer(&format!("{TITLE}/keys/tracks"))
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .position(|t| t["prop"] == prop)
        .unwrap_or_else(|| panic!("no {prop} track"))
}

/// The key list of `prop` on the title.
fn title_keys<'a>(v: &'a mut Value, prop: &str) -> &'a mut Vec<Value> {
    let i = track_index(v, prop);
    at(v, &format!("{TITLE}/keys/tracks/{i}/keys"))
        .as_array_mut()
        .unwrap()
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
fn keys_and_animations_are_spelled_out_in_integers_and_names() {
    let v = sink_value();
    let title = v.pointer(TITLE).unwrap();
    assert_eq!(title["keys"]["origin"], fr(5).raw());
    let prop = |name: &str| &title["keys"]["tracks"][track_index(&v, name)];
    assert_eq!(
        prop("transform.position")["keys"][0],
        json!({"at": fr(5).raw(), "value": {"x": 0, "y": -350_000}, "interp": "ease-in-out-cubic"})
    );
    assert_eq!(prop("transform.scale")["keys"][0]["value"], 8_000);
    assert_eq!(prop("transform.rotation")["keys"][0]["value"], -1_500);
    assert_eq!(prop("transform.opacity")["keys"][1]["value"], 1_000);
    assert_eq!(prop("text.size")["keys"][1]["value"], 6_500);
    assert_eq!(prop("text.letter_spacing")["keys"][0]["value"], -50);
    assert_eq!(prop("text.outline.color")["keys"][1]["value"], "#FF0080C8");
    assert_eq!(prop("text.shadow.angle")["keys"][0]["value"], 1_350);
    assert_eq!(
        prop("text.bubble.tail.tip")["keys"][0]["value"],
        json!({"x": -4_000, "y": 9_000})
    );
    assert_eq!(
        prop("text.fill.stops.1.color")["keys"][1]["interp"],
        "overshoot-in"
    );
    assert_eq!(prop("text.reveal")["keys"][1]["value"], 1_000);
    // Every TR-031 interpolation has a name.
    let names: Vec<&str> = prop("text.size")["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["interp"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "linear",
            "ease-in-quad",
            "ease-out-quad",
            "ease-in-out-quad",
            "hold",
            "ease-in-sine",
            "ease-out-sine",
            "ease-in-out-sine",
            "ease-in-cubic",
            "ease-out-cubic",
            "ease-in-out-cubic",
            "ease-in-expo",
            "ease-out-expo",
            "ease-in-out-expo",
            "overshoot-out",
            "overshoot-in",
            "elastic-out",
            "bounce-out",
        ]
    );
    assert_eq!(
        title["animations"]["entrance"],
        json!({
            "preset": "fc.in.typewriter", "duration": fr(12).raw(), "easing": "ease-in-expo",
            "direction": null, "unit": "letter", "overlap": 0, "order": "random",
            "seed": 0xDEAD_BEEF_u32, "pace": 35_280_000, "intensity": 750, "travel": "short",
            "phase": 0
        })
    );
    assert_eq!(title["animations"]["continuous"]["phase"], fr(7).raw());
    assert_eq!(
        title["animations"]["continuous"]["direction"],
        "counter-clockwise"
    );
    assert_eq!(title["animations"]["exit"]["order"], "edges-in");
    let overlay = v.pointer(OVERLAY).unwrap();
    assert!(
        overlay.get("keys").is_none(),
        "no keys, no origin: left out"
    );
    assert_eq!(overlay["animations"]["exit"], Value::Null);
    assert_eq!(overlay["animations"]["entrance"]["direction"], "up");
    // Clips without motion leave both out.
    assert!(v.pointer(&format!("{VOICE}/keys")).is_none());
    assert!(v.pointer(&format!("{VOICE}/animations")).is_none());

    let opened = open_value(&v).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    assert_eq!(opened.document, kitchen_sink().0);
}

#[test]
fn eased_volume_keys_open_with_their_curve() {
    let mut v = sink_value();
    *at(&mut v, &format!("{VOICE}/audio/gain_keys/0/interp")) = json!("ease-in-out-sine");
    let opened = open_value(&v).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    let audio = opened.document.clip(clip(47)).unwrap().audio.clone();
    assert_eq!(
        audio.active().unwrap().gain.keys()[0].interp,
        Interp::Ease {
            curve: EaseCurve::Sine,
            dir: EaseDir::InOut
        }
    );
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(
        again.pointer(&format!("{VOICE}/audio/gain_keys/0/interp")),
        Some(&json!("ease-in-out-sine"))
    );
}

#[test]
fn keys_out_of_order_are_sorted_with_their_unknown_keys() {
    let mut v = sink_value();
    let keys = title_keys(&mut v, "transform.position");
    keys[1]["x_handles"] = json!([1, 2]);
    keys[1]["value"]["x_unit"] = json!("canvas");
    // An unknown key too wide for 64 bits survives digit for digit.
    keys[1]["value"]["x_wide"] = serde_json::from_str("123456789012345678901234567890").unwrap();
    keys.reverse();
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.warnings,
        vec![
            OpenWarning::KeyframesSorted { clips: 1 },
            OpenWarning::UnknownFields { objects: 2 },
        ]
    );
    assert!(opened.needs_save, "the document differs from the file");
    assert_eq!(opened.document, kitchen_sink().0);
    let key = format!("keys[transform.position]@{}", fr(20).raw());
    let owner = Owner::Clip(clip(43));
    assert!(
        opened
            .extras
            .get(owner, &key)
            .unwrap()
            .contains_key("x_handles")
    );
    assert!(
        opened
            .extras
            .get(owner, &format!("{key}.value"))
            .unwrap()
            .contains_key("x_unit")
    );
    let again = encode(&opened.document, opened.ids.next(), &opened.extras).unwrap();
    assert!(
        String::from_utf8(again)
            .unwrap()
            .contains("\"x_wide\": 123456789012345678901234567890")
    );
}

#[test]
fn unknown_keys_stay_with_their_key_through_edits_and_go_with_it() {
    let mut v = sink_value();
    title_keys(&mut v, "transform.position")[1]["x_handles"] = json!([1, 2]);
    // The title's track is locked in the kitchen sink.
    *at(&mut v, "/document/sequences/0/visual/2/header/locked") = json!(false);
    let opened = open_value(&v).unwrap();
    let mut editor = editor_for(&opened);
    let seq = SeqId::from_raw(nz(1));
    let position = PropPath::Transform(TransformProp::Position);
    // A key before it (sequence time 1 frame, content time 6 frames).
    commit(
        &mut editor,
        EditRequest::EditKeys {
            seq,
            clips: vec![clip(43)],
            prop: position,
            edit: KeyEdit::Add {
                at: fr(1),
                value: None,
            },
        },
    );
    let saved = to_value(&encode(editor.document(), editor.ids().next(), &opened.extras).unwrap());
    let keys = &saved.pointer(TITLE).unwrap()["keys"]["tracks"][0]["keys"];
    assert_eq!(keys.as_array().unwrap().len(), 4);
    assert_eq!(keys[2]["at"], fr(20).raw());
    assert_eq!(keys[2]["x_handles"], json!([1, 2]));
    assert!(keys[1].get("x_handles").is_none());
    // Moved: the key's unknown keys go, like a volume key's.
    commit(
        &mut editor,
        EditRequest::EditKeys {
            seq,
            clips: vec![clip(43)],
            prop: position,
            edit: KeyEdit::Move {
                from: fr(15),
                to: fr(16),
            },
        },
    );
    let saved = to_value(&encode(editor.document(), editor.ids().next(), &opened.extras).unwrap());
    let keys = &saved.pointer(TITLE).unwrap()["keys"]["tracks"][0]["keys"];
    assert_eq!(keys[2]["at"], fr(21).raw());
    assert!(keys[2].get("x_handles").is_none(), "{keys}");
}

#[test]
fn key_values_out_of_range_are_clamped_as_an_edit_would() {
    let mut v = sink_value();
    title_keys(&mut v, "transform.opacity")[1]["value"] = json!(5_000);
    title_keys(&mut v, "text.shadow.angle")[0]["value"] = json!(-900);
    title_keys(&mut v, "text.size")[0]["value"] = json!(i64::MAX);
    title_keys(&mut v, "text.reveal")[1]["value"] = json!(70_000);
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::ValuesClamped { clips: 1 }]
    );
    assert!(opened.needs_save);
    let title = opened.document.clip(clip(43)).unwrap();
    let first = |prop: PropPath, i: usize| title.keys.keys(prop)[i].value;
    assert_eq!(
        first(PropPath::Transform(TransformProp::Opacity), 1),
        PropValue::Ratio(PerMille::ONE)
    );
    assert_eq!(
        first(PropPath::parse("text.shadow.angle").unwrap(), 0),
        PropValue::Direction(DeciDegrees::new(2_700))
    );
    assert_eq!(
        first(PropPath::parse("text.size").unwrap(), 0),
        PropValue::Length(freecut_timeline::model::TextStyle::MAX_SIZE)
    );
    assert_eq!(first(PropPath::Reveal, 1), PropValue::Ratio(PerMille::ONE));
}

#[test]
fn keys_that_cant_be_read_are_refused_with_their_path() {
    let keys_at =
        |prop: &str, v: &Value| format!("{TITLE_PATH}.keys.tracks[{}]", track_index(v, prop));

    // A property that takes no keys, or that the clip has no place for.
    let mut v = sink_value();
    let i = track_index(&v, "transform.scale");
    *at(&mut v, &format!("{TITLE}/keys/tracks/{i}/prop")) = json!("transform.skew");
    let (path, message) = bad_value(&v);
    assert_eq!(path, format!("{TITLE_PATH}.keys.tracks[{i}].prop"));
    assert!(message.contains("transform.skew"), "{message}");

    let mut v = sink_value();
    at(&mut v, OVERLAY)["keys"] = json!({"origin": 0, "tracks": [
        {"prop": "text.size", "keys": [{"at": 0, "value": 6400, "interp": "linear"}]}
    ]});
    let (path, message) = bad_value(&v);
    assert_eq!(path, format!("{OVERLAY_PATH}.keys.tracks[0].prop"));
    assert!(message.contains("only on text clips"), "{message}");

    let mut v = sink_value();
    at(&mut v, VOICE)["keys"] = json!({"origin": 0, "tracks": [
        {"prop": "transform.opacity", "keys": [{"at": 0, "value": 500, "interp": "linear"}]}
    ]});
    let (path, message) = bad_value(&v);
    assert_eq!(path, format!("{VOICE_PATH}.keys.tracks[0].prop"));
    assert!(message.contains("only on visual clips"), "{message}");

    // Listed twice.
    let mut v = sink_value();
    let tracks = at(&mut v, &format!("{TITLE}/keys/tracks"));
    let first = tracks[0].clone();
    tracks.as_array_mut().unwrap().push(first);
    let n = tracks.as_array().unwrap().len() - 1;
    let (path, message) = bad_value(&v);
    assert_eq!(path, format!("{TITLE_PATH}.keys.tracks[{n}].prop"));
    assert!(message.contains("listed twice"), "{message}");

    // No keys at all.
    let mut v = sink_value();
    title_keys(&mut v, "text.reveal").clear();
    let (path, _) = bad_value(&v);
    assert_eq!(path, format!("{}.keys", keys_at("text.reveal", &v)));

    // Two keys at one time.
    let mut v = sink_value();
    let keys = title_keys(&mut v, "transform.position");
    keys[2]["at"] = keys[0]["at"].clone();
    let (path, message) = bad_value(&v);
    assert_eq!(
        path,
        format!("{}.keys[2].at", keys_at("transform.position", &v))
    );
    assert!(message.contains("must differ"), "{message}");

    // Values of the wrong shape.
    let mut v = sink_value();
    title_keys(&mut v, "text.outline.color")[0]["value"] = json!(255);
    let (path, _) = json_error(&v);
    assert_eq!(
        path,
        format!("{}.keys[0].value", keys_at("text.outline.color", &v))
    );

    let mut v = sink_value();
    title_keys(&mut v, "text.outline.color")[0]["value"] = json!("red");
    let (path, message) = bad_value(&v);
    assert_eq!(
        path,
        format!("{}.keys[0].value", keys_at("text.outline.color", &v))
    );
    assert!(message.contains("#RRGGBBAA"), "{message}");

    let mut v = sink_value();
    title_keys(&mut v, "transform.position")[0]["value"] = json!(12);
    let (path, _) = json_error(&v);
    assert_eq!(
        path,
        format!("{}.keys[0].value", keys_at("transform.position", &v))
    );

    let mut v = sink_value();
    title_keys(&mut v, "text.size")[0]["value"] = json!(64.5);
    let (path, message) = json_error(&v);
    assert_eq!(path, format!("{}.keys[0].value", keys_at("text.size", &v)));
    assert!(message.contains("integer"), "{message}");

    let mut v = sink_value();
    title_keys(&mut v, "text.size")[0]["interp"] = json!("wobbly");
    let (path, _) = json_error(&v);
    assert_eq!(path, format!("{}.keys[0].interp", keys_at("text.size", &v)));

    // Times beyond what a file holds exactly.
    let mut v = sink_value();
    title_keys(&mut v, "text.size")[0]["at"] = json!((1_i64 << 53) + 1);
    let (path, _) = bad_value(&v);
    assert_eq!(path, format!("{}.keys[0].at", keys_at("text.size", &v)));
}

#[test]
fn animation_settings_out_of_range_are_clamped() {
    let mut v = sink_value();
    *at(&mut v, &format!("{TITLE}/animations/entrance/overlap")) = json!(4_000);
    *at(&mut v, &format!("{TITLE}/animations/entrance/intensity")) = json!(-3);
    *at(&mut v, &format!("{TITLE}/animations/exit/easing")) = json!("hold");
    *at(&mut v, &format!("{TITLE}/animations/entrance/pace")) = json!(1);
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::ValuesClamped { clips: 1 }]
    );
    let anims = &opened.document.clip(clip(43)).unwrap().animations;
    let entrance = anims.get(Slot::Entrance).unwrap();
    assert_eq!(entrance.params.overlap, PerMille::ONE);
    assert_eq!(entrance.params.intensity, PerMille::ZERO);
    assert_eq!(
        entrance.params.pace,
        Some(freecut_timeline::model::AnimParams::MIN_PACE)
    );
    assert_eq!(anims.get(Slot::Exit).unwrap().easing, None);
}

#[test]
fn animation_lengths_out_of_range_are_clamped_as_an_edit_would() {
    use freecut_timeline::time::TICKS_PER_SECOND;
    // A Loop cycle of one tick, or an hour; an In of one tick.
    for (cycle, want) in [
        (1, AnimationSpec::MIN_DURATION),
        (3_600 * TICKS_PER_SECOND, AnimationSpec::MAX_CYCLE),
    ] {
        let mut v = sink_value();
        *at(&mut v, &format!("{STICKER}/animations/continuous/duration")) = json!(cycle);
        *at(&mut v, &format!("{TITLE}/animations/exit/duration")) = json!(1);
        let opened = open_value(&v).unwrap();
        assert_eq!(
            opened.warnings,
            vec![OpenWarning::ValuesClamped { clips: 2 }],
            "cycle {cycle}"
        );
        let doc = &opened.document;
        let looped = doc.clip(clip(44)).unwrap().animations.continuous.clone();
        assert_eq!(looped.unwrap().duration, want);
        let out = doc.clip(clip(43)).unwrap().animations.exit.clone();
        assert_eq!(out.unwrap().duration, AnimationSpec::MIN_DURATION);
    }
    // In range: untouched, no repair.
    let v = sink_value();
    assert!(open_value(&v).unwrap().warnings.is_empty());
}

#[test]
fn a_seed_beyond_32_bits_is_saturated_not_refused() {
    for (seed, want) in [(-1, 0), (4_294_967_296_i64, u32::MAX)] {
        let mut v = sink_value();
        *at(&mut v, &format!("{STICKER}/animations/continuous/seed")) = json!(seed);
        let opened = open_value(&v).unwrap();
        assert_eq!(
            opened.warnings,
            vec![OpenWarning::ValuesClamped { clips: 1 }],
            "seed {seed}"
        );
        let looped = opened
            .document
            .clip(clip(44))
            .unwrap()
            .animations
            .continuous
            .clone();
        assert_eq!(looped.unwrap().params.seed, want);
    }
}

#[test]
fn a_split_loops_seams_are_written_and_read_back() {
    use freecut_timeline::model::LoopSeams;
    // Never split: no key, as before the option.
    let v = sink_value();
    assert!(
        v.pointer(&format!("{TITLE}/animations/continuous/seams"))
            .is_none()
    );
    let mut v = sink_value();
    at(&mut v, &format!("{TITLE}/animations/continuous"))["seams"] =
        json!({"start": fr(4).raw(), "end": null, "ramp": fr(1).raw()});
    let opened = open_value(&v).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    let anims = &opened.document.clip(clip(43)).unwrap().animations;
    assert_eq!(
        anims.get(Slot::Continuous).unwrap().seams,
        Some(LoopSeams {
            start: Some(fr(4)),
            end: None,
            ramp: fr(1),
        })
    );
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(
        again.pointer(&format!("{TITLE}/animations/continuous/seams")),
        Some(&json!({"start": fr(4).raw(), "end": null, "ramp": fr(1).raw()}))
    );
    // A ramp past TX-022's 250 ms is brought back.
    let mut v = sink_value();
    at(&mut v, &format!("{TITLE}/animations/continuous"))["seams"] =
        json!({"start": null, "end": 5, "ramp": fr(100).raw()});
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::ValuesClamped { clips: 1 }]
    );
    let anims = &opened.document.clip(clip(43)).unwrap().animations;
    let seams = anims.get(Slot::Continuous).unwrap().seams.unwrap();
    assert_eq!(seams.ramp, LoopSeams::MAX_RAMP);
}

#[test]
fn animations_that_cant_be_played_are_refused_with_their_path() {
    // Audio clips have no slots.
    let mut v = sink_value();
    at(&mut v, VOICE)["animations"] = sink_value()
        .pointer(&format!("{OVERLAY}/animations"))
        .unwrap()
        .clone();
    let (path, message) = bad_value(&v);
    assert_eq!(path, format!("{VOICE_PATH}.animations.entrance"));
    assert!(message.contains("audio clips"), "{message}");

    // A video plays a Loop or In/Out, not both (FX-019).
    let mut v = sink_value();
    let wave = v
        .pointer(&format!("{TITLE}/animations/continuous"))
        .unwrap()
        .clone();
    *at(&mut v, &format!("{OVERLAY}/animations/continuous")) = wave;
    let (path, message) = bad_value(&v);
    assert_eq!(path, format!("{OVERLAY_PATH}.animations.continuous"));
    assert!(message.contains("FX-019"), "{message}");

    // A length that isn't positive, an empty preset id, an unknown name.
    let mut v = sink_value();
    *at(&mut v, &format!("{OVERLAY}/animations/entrance/duration")) = json!(0);
    let (path, _) = bad_value(&v);
    assert_eq!(path, format!("{OVERLAY_PATH}.animations.entrance.duration"));

    let mut v = sink_value();
    *at(&mut v, &format!("{OVERLAY}/animations/entrance/preset")) = json!("");
    let (path, _) = bad_value(&v);
    assert_eq!(path, format!("{OVERLAY_PATH}.animations.entrance.preset"));

    let mut v = sink_value();
    *at(&mut v, &format!("{OVERLAY}/animations/entrance/order")) = json!("sideways");
    let (path, _) = json_error(&v);
    assert_eq!(path, format!("{OVERLAY_PATH}.animations.entrance.order"));
}

#[test]
fn typewriters_caret_is_written_only_when_on() {
    // Off (the kitchen sink's typewriter): no key, as before the option.
    let v = sink_value();
    assert!(
        v.pointer(&format!("{TITLE}/animations/entrance/caret"))
            .is_none()
    );
    // On: written, read back, and kept through save.
    let mut v = sink_value();
    at(&mut v, &format!("{TITLE}/animations/entrance"))["caret"] = json!(true);
    let opened = open_value(&v).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    let anims = &opened.document.clip(clip(43)).unwrap().animations;
    assert!(anims.get(Slot::Entrance).unwrap().params.caret);
    assert!(!anims.get(Slot::Exit).unwrap().params.caret);
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(
        again.pointer(&format!("{TITLE}/animations/entrance/caret")),
        Some(&json!(true))
    );
    assert!(
        again
            .pointer(&format!("{TITLE}/animations/exit/caret"))
            .is_none()
    );
}

#[test]
fn presets_this_build_lacks_are_kept_as_chosen() {
    let mut v = sink_value();
    *at(&mut v, &format!("{STICKER}/animations/continuous/preset")) =
        json!("org.example.loop.glitter");
    let opened = open_value(&v).unwrap();
    assert!(opened.warnings.is_empty());
    let sticker = opened.document.clip(clip(44)).unwrap();
    assert_eq!(
        sticker.animations.get(Slot::Continuous).unwrap().preset,
        PresetId::new("org.example.loop.glitter")
    );
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(
        again.pointer(&format!("{STICKER}/animations/continuous/preset")),
        Some(&json!("org.example.loop.glitter"))
    );
}

#[test]
fn an_origin_without_keys_is_kept() {
    // A text clip split in two: the right piece starts later in its
    // content, so keys added to it later line up with the left piece's.
    let mut v = sink_value();
    at(&mut v, "/document/sequences/0/visual/2/clips/1")["keys"] =
        json!({"origin": fr(30).raw(), "tracks": []});
    let opened = open_value(&v).unwrap();
    assert!(opened.warnings.is_empty());
    assert_eq!(
        opened.document.clip(clip(51)).unwrap().keys.origin(),
        fr(30)
    );
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(
        again.pointer("/document/sequences/0/visual/2/clips/1/keys"),
        Some(&json!({"origin": fr(30).raw(), "tracks": []}))
    );
}

/// A document with one text clip carrying `keys` and `entrance`.
fn one_text_clip(keys: ClipKeys, entrance: Option<AnimationSpec>) -> Document {
    let mut text = Clip::new(
        clip(5),
        TimeRange::new(Tick::ZERO, fr(30)).unwrap(),
        ClipContent::Text(Arc::new(TextClipData::default())),
    );
    text.keys = keys;
    if let Some(spec) = entrance {
        text.animations.entrance = Some(Arc::new(spec));
    }
    let seq = Sequence::from_parts(
        SeqId::from_raw(nz(1)),
        SequenceFlags {
            main_magnet: true,
            ..SequenceFlags::default()
        },
        vec![
            Arc::new(Track::from_parts(
                TrackId::from_raw(nz(2)),
                TrackKind::Video,
                TrackHeader::default(),
                Vec::new(),
            )),
            Arc::new(Track::from_parts(
                TrackId::from_raw(nz(3)),
                TrackKind::Text,
                TrackHeader::default(),
                vec![Arc::new(text)],
            )),
        ],
        Vec::new(),
    );
    Document::from_parts(
        SeqId::from_raw(nz(1)),
        ProjectSettings::new_default().unwrap(),
        Library::from_parts(Vec::new()).unwrap(),
        vec![Arc::new(seq)],
    )
    .unwrap()
}

#[test]
fn ticks_the_file_cant_hold_exactly_are_refused_on_save() {
    let far = Tick::from_raw((1 << 53) + 1);
    let path = "document.sequences[0].visual[1].clips[0]";
    let keys = ClipKeys::from_parts(
        Tick::ZERO,
        vec![(
            PropPath::Reveal,
            vec![Keyframe {
                at: far,
                value: PropValue::Ratio(PerMille::ZERO),
                interp: Interp::Linear,
            }],
        )],
    )
    .unwrap();
    match encode(&one_text_clip(keys, None), nz(10), &Extras::new()) {
        Err(SaveError::OutOfRange { path: at, message }) => {
            assert_eq!(at, format!("{path}.keys.tracks[0].keys[0].at"));
            assert!(message.contains("9007199254740993"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    let mut spec = AnimationSpec::new(PresetId::new("fc.in.fade"), fr(10));
    spec.phase = far;
    match encode(
        &one_text_clip(ClipKeys::default(), Some(spec)),
        nz(10),
        &Extras::new(),
    ) {
        Err(SaveError::OutOfRange { path: at, .. }) => {
            assert_eq!(at, format!("{path}.animations.entrance.phase"));
        }
        other => panic!("{other:?}"),
    }
    let origin = ClipKeys::from_parts(far, Vec::new()).unwrap();
    match encode(&one_text_clip(origin, None), nz(10), &Extras::new()) {
        Err(SaveError::OutOfRange { path: at, .. }) => {
            assert_eq!(at, format!("{path}.keys.origin"));
        }
        other => panic!("{other:?}"),
    }
    // Within range, all of it opens again as saved.
    let fine = ClipKeys::from_parts(fr(3), Vec::new()).unwrap();
    let doc = one_text_clip(
        fine,
        Some(AnimationSpec::new(PresetId::new("fc.in.fade"), fr(10))),
    );
    let opened = decode(&encode(&doc, nz(10), &Extras::new()).unwrap()).unwrap();
    assert_eq!(opened.document, doc);
}
