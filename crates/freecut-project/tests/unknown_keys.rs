//! Keys this build doesn't know survive open → save at every object level,
//! follow their entity through edits, and leave with it when it is deleted.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_project::{
    Extras, OpenWarning, Opened, Owner, SaveRequest, decode, encode, open, save,
};
use freecut_timeline::edit::{
    AddTrackReq, DeleteReq, EditCtx, EditRequest, MoveReq, SplitReq, SplitTargets, TrackPosition,
};
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::ids::ClipId;
use freecut_timeline::ids::TrackId;
use freecut_timeline::model::TrackKind;
use freecut_timeline::{CommitMeta, Committed, Editor};
use pretty_assertions::assert_eq;
use proptest::prelude::*;
use serde_json::{Value, json};

/// The kitchen sink as JSON, without unknown keys.
fn sink_value() -> Value {
    let (doc, ids) = kitchen_sink();
    to_value(&encode(&doc, ids, &Extras::new()).unwrap())
}

fn reencode(opened: &Opened) -> Value {
    to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap())
}

const MAIN_CLIPS: &str = "/document/sequences/0/visual/0/clips";

#[test]
fn unknown_keys_survive_at_top_level_in_a_clip_and_in_a_library_item() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Trip.freecut");
    let mut v = sink_value();
    v["future_top"] = json!({"a": [1, 2, 3]});
    v["document"]["library"][1]["future_media"] = json!("kept");
    v.pointer_mut(&format!("{MAIN_CLIPS}/0"))
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("future_clip".into(), json!({"k": null}));
    std::fs::write(&path, compact(&v)).unwrap();

    let opened = open(&path).unwrap();
    assert_eq!(opened.extras.len(), 3);
    assert!(
        opened
            .warnings
            .contains(&OpenWarning::UnknownFields { objects: 3 })
    );
    assert!(
        !opened.needs_save,
        "unknown keys alone don't dirty the project"
    );
    let (doc, _) = kitchen_sink();
    assert_eq!(opened.document, doc, "the known data is unaffected");

    save(
        &path,
        &SaveRequest {
            document: &opened.document,
            ids_next: opened.ids.next(),
            extras: &opened.extras,
            backup: false,
            relative_base: None,
            export: None,
            ui: None,
        },
    )
    .unwrap();
    let saved = to_value(&std::fs::read(&path).unwrap());
    assert_eq!(saved["future_top"], json!({"a": [1, 2, 3]}));
    assert_eq!(saved["document"]["library"][1]["future_media"], "kept");
    assert_eq!(
        saved.pointer(&format!("{MAIN_CLIPS}/0/future_clip")),
        Some(&json!({"k": null}))
    );
}

/// Every object of the schema, as a JSON pointer into the kitchen sink.
const OBJECTS: &[&str] = &[
    "",
    "/document",
    "/document/settings",
    "/document/settings/canvas",
    "/document/settings/frame_rate",
    "/document/library/0",
    "/document/library/0/source",
    "/document/library/3/source",
    "/document/library/0/path",
    "/document/library/0/info",
    "/document/library/0/info/video/2",
    "/document/library/0/info/video/0/sar",
    "/document/library/0/info/video/0/rate",
    "/document/library/0/info/audio/1",
    "/document/library/0/fingerprint",
    "/document/library/2/info/still",
    "/document/sequences/1",
    "/document/sequences/1/flags",
    "/document/sequences/0/visual/3",
    "/document/sequences/0/visual/3/header",
    "/document/sequences/0/audio/0/clips/0",
    "/document/sequences/0/audio/0/clips/0/range",
    "/document/sequences/0/audio/0/clips/0/content",
    "/document/sequences/0/audio/0/clips/0/content/source",
    "/document/sequences/0/visual/0/clips/1/content",
    "/document/sequences/0/visual/2/clips/0/content",
    "/document/sequences/0/visual/0/clips/0/audio",
    "/document/sequences/0/visual/0/clips/1/audio",
    "/document/sequences/0/audio/1/clips/0/audio",
    "/document/sequences/0/visual/0/clips/0/color",
    "/document/sequences/0/visual/0/clips/0/color/light",
    "/document/sequences/0/visual/0/clips/0/color/color",
];

#[test]
fn unknown_keys_survive_at_every_object_level() {
    let mut v = sink_value();
    for (i, pointer) in OBJECTS.iter().enumerate() {
        v.pointer_mut(pointer)
            .unwrap_or_else(|| panic!("no object at {pointer}"))
            .as_object_mut()
            .unwrap()
            .insert(format!("zz_unknown_{i}"), json!({"i": i}));
    }
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(opened.extras.len(), OBJECTS.len());
    let saved = reencode(&opened);
    for (i, pointer) in OBJECTS.iter().enumerate() {
        assert_eq!(
            saved.pointer(&format!("{pointer}/zz_unknown_{i}")),
            Some(&json!({"i": i})),
            "lost the unknown key at {pointer:?}"
        );
    }
    assert_eq!(saved, v, "nothing else changed either");
}

proptest! {
    /// Random unknown values in random objects come back unchanged.
    #[test]
    fn random_unknown_values_round_trip(
        picks in proptest::collection::vec((0..OBJECTS.len(), "[a-z_]{1,8}", any::<i64>(), text()), 1..8)
    ) {
        let mut v = sink_value();
        for (object, key, number, string) in &picks {
            let key = format!("x_{key}");
            v.pointer_mut(OBJECTS[*object]).unwrap().as_object_mut().unwrap()
                .insert(key, json!({"n": number, "s": string}));
        }
        let opened = decode(&compact(&v)).unwrap();
        prop_assert_eq!(reencode(&opened), v);
    }
}

fn editor_for(opened: &Opened) -> Editor {
    Editor::open(
        opened.document.clone(),
        opened.ids.clone(),
        HistoryLimits::default(),
    )
    .unwrap()
}

fn commit(editor: &mut Editor, req: EditRequest) -> Committed {
    let plan = editor.plan(&EditCtx::default(), &req).unwrap();
    editor.commit(plan, CommitMeta::default()).unwrap()
}

fn save_editor(editor: &Editor, extras: &Extras) -> Value {
    to_value(&encode(editor.document(), editor.ids().next(), extras).unwrap())
}

fn clip_object(v: &Value, id: u64) -> Option<Value> {
    let seqs = v["document"]["sequences"].as_array()?;
    seqs.iter()
        .flat_map(|s| {
            s["visual"]
                .as_array()
                .unwrap()
                .iter()
                .chain(s["audio"].as_array().unwrap())
        })
        .flat_map(|t| t["clips"].as_array().unwrap())
        .find(|c| c["id"] == id)
        .cloned()
}

#[test]
fn unknown_keys_follow_their_clip_through_edits_and_leave_with_it() {
    let mut v = sink_value();
    // Clip 42 (overlay) and clip 48 (music) carry unknown keys.
    let tag = |v: &mut Value, pointer: &str, key: &str| {
        v.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(key.into(), json!(key));
    };
    tag(
        &mut v,
        "/document/sequences/0/visual/1/clips/0",
        "x_overlay",
    );
    tag(
        &mut v,
        "/document/sequences/0/visual/1/clips/0/range",
        "x_overlay_range",
    );
    tag(&mut v, "/document/sequences/0/audio/1/clips/0", "x_music");
    let opened = decode(&compact(&v)).unwrap();
    let mut editor = editor_for(&opened);
    let seq = editor.document().root();
    let id = |n: u64| ClipId::from_raw(nz(n));

    // Move the overlay up onto a new video track: the keys move with it.
    commit(
        &mut editor,
        EditRequest::AddTrack(AddTrackReq {
            seq,
            kind: TrackKind::Video,
            position: TrackPosition::Above(TrackId::from_raw(nz(21))),
        }),
    );
    commit(
        &mut editor,
        EditRequest::Move(MoveReq {
            seq,
            clips: vec![id(42)],
            delta: fr(10),
            track_shift: 1,
        }),
    );
    let new_track = editor.document().find_clip(id(42)).unwrap().track;
    assert_ne!(
        new_track,
        TrackId::from_raw(nz(21)),
        "the clip changed tracks"
    );
    let moved = clip_object(&save_editor(&editor, &opened.extras), 42).unwrap();
    assert_eq!(moved["x_overlay"], "x_overlay");
    assert_eq!(moved["range"]["x_overlay_range"], "x_overlay_range");

    // Split it: the left part keeps the id and the keys, the new right part
    // starts clean.
    let at = editor.document().clip(id(42)).unwrap().start();
    let split = commit(
        &mut editor,
        EditRequest::Split(SplitReq {
            seq,
            at: at.checked_add(fr(10)).unwrap(),
            targets: SplitTargets::Clips(vec![id(42)]),
        }),
    );
    let Committed::Applied {
        selection: Some(right_ids),
        ..
    } = split
    else {
        panic!("split did nothing: {split:?}");
    };
    let saved = save_editor(&editor, &opened.extras);
    assert_eq!(clip_object(&saved, 42).unwrap()["x_overlay"], "x_overlay");
    let right = clip_object(&saved, right_ids[0].get()).expect("the split's right part");
    assert!(right.get("x_overlay").is_none());

    // Delete the music clip: its keys are gone, the others stay.
    commit(
        &mut editor,
        EditRequest::Delete(DeleteReq {
            seq,
            clips: vec![id(48)],
            ripple: false,
        }),
    );
    let saved = save_editor(&editor, &opened.extras);
    assert!(clip_object(&saved, 48).is_none());
    assert!(
        !String::from_utf8(compact(&saved))
            .unwrap()
            .contains("x_music")
    );
    assert_eq!(clip_object(&saved, 42).unwrap()["x_overlay"], "x_overlay");

    // Reopening the saved file gives exactly the surviving keys.
    let reopened = decode(&compact(&saved)).unwrap();
    let owners: Vec<_> = reopened
        .extras
        .iter()
        .map(|(o, at, _)| (o, at.to_string()))
        .collect();
    assert_eq!(
        owners,
        vec![
            (Owner::Clip(id(42)), String::new()),
            (Owner::Clip(id(42)), "range".to_string()),
        ]
    );
}

// ---- review regressions ----

/// F7: an unknown integer too large for 64 bits is written back verbatim.
#[test]
fn f7_big_unknown_integers_survive_verbatim() {
    let mut bytes = compact(&sink_value());
    bytes.splice(
        1..1,
        br#""x_big":123456789012345678901234567890,"#.iter().copied(),
    );
    let opened = decode(&bytes).unwrap();
    let text =
        String::from_utf8(encode(&opened.document, opened.ids.next(), &opened.extras).unwrap())
            .unwrap();
    let line = text.lines().find(|l| l.contains("x_big")).unwrap();
    assert!(line.contains("123456789012345678901234567890"), "{line}");
}

/// F7: wide integers survive at any depth, negative ones too, while one in a
/// known field is still refused with its path.
#[test]
fn wide_unknown_integers_survive_anywhere() {
    let mut v = sink_value();
    let clip = v.pointer_mut(&format!("{MAIN_CLIPS}/0")).unwrap();
    clip["content"]["x_wide"] = json!("placeholder");
    clip["content"]["source"]["x_list"] = json!(["placeholder", 1]);
    let mut text = String::from_utf8(compact(&v)).unwrap();
    text = text.replace(
        "\"x_wide\":\"placeholder\"",
        "\"x_wide\":-98765432109876543210987654321",
    );
    text = text.replace(
        "\"x_list\":[\"placeholder\",1]",
        "\"x_list\":[340282366920938463463374607431768211457,1]",
    );
    let opened = decode(text.as_bytes()).unwrap();
    let out =
        String::from_utf8(encode(&opened.document, opened.ids.next(), &opened.extras).unwrap())
            .unwrap();
    assert!(
        out.contains("\"x_wide\": -98765432109876543210987654321"),
        "{out}"
    );
    assert!(
        out.contains("340282366920938463463374607431768211457"),
        "{out}"
    );

    let too_big = String::from_utf8(compact(&sink_value())).unwrap().replace(
        "\"ids_next\":100",
        "\"ids_next\":123456789012345678901234567890",
    );
    match decode(too_big.as_bytes()) {
        Err(freecut_project::OpenError::Json { path, .. }) => assert_eq!(path, "ids_next"),
        other => panic!("{other:?}"),
    }
}
