//! Formats 7 to 9: markers (TL-037), *Follow main track* (TL-029) and media
//! origins (PM-041). What the reader refuses, what it repairs, and what
//! survives open → edit → save.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_project::{Extras, OpenError, OpenWarning, Owner, SaveError, decode, encode};
use freecut_timeline::edit::{EditCtx, EditRequest, Limits, RecordTarget, SplitReq, SplitTargets};
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::ids::{ClipId, MarkerId, MediaId, SeqId};
use freecut_timeline::model::{
    GeneratedMediaKind, MarkerOrigin, MediaOrigin, PaletteColor, Sequence, SequenceFlags,
};
use freecut_timeline::time::{MarkRange, Tick};
use freecut_timeline::{CommitMeta, Editor};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

/// A change to a file's JSON.
type EditJson = dyn Fn(&mut Value);

fn sink_value() -> Value {
    let (doc, ids) = kitchen_sink();
    to_value(&encode(&doc, ids, &Extras::new()).unwrap())
}

fn clip_id(n: u64) -> ClipId {
    ClipId::from_raw(nz(n))
}

fn marker_id(n: u64) -> MarkerId {
    MarkerId::from_raw(nz(n))
}

/// The kitchen sink's intro clip (40) as JSON.
fn intro(v: &mut Value) -> &mut Value {
    let intro = &mut v["document"]["sequences"][0]["visual"][0]["clips"][0];
    assert_eq!(intro["id"], 40);
    intro
}

fn bad_value_at(err: OpenError) -> String {
    match err {
        OpenError::BadValue { path, .. } | OpenError::Json { path, .. } => path,
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_beat_on_the_timeline_is_refused() {
    let mut v = sink_value();
    v["document"]["sequences"][0]["markers"][0]["origin"] = json!("detected-beat");
    let err = decode(&compact(&v)).unwrap_err();
    assert!(matches!(err, OpenError::BadValue { .. }), "{err:?}");
    assert_eq!(bad_value_at(err), "document.sequences[0].markers[0].origin");
}

#[test]
fn unknown_colours_kinds_and_origins_are_refused_with_their_path() {
    let cases: [(&EditJson, &str); 5] = [
        (
            &|v| v["document"]["sequences"][0]["markers"][1]["color"] = json!("chartreuse"),
            "document.sequences[0].markers[1].color",
        ),
        (
            &|v| intro(v)["markers"][0]["origin"] = json!("guess"),
            "document.sequences[0].visual[0].clips[0].markers[0].origin",
        ),
        (
            &|v| v["document"]["sequences"][0]["flags"]["follow_kinds"][0] = json!("captions"),
            "document.sequences[0].flags.follow_kinds[0]",
        ),
        (
            &|v| v["document"]["library"][4]["origin"]["kind"] = json!("hologram"),
            "document.library[4].origin.kind",
        ),
        (
            &|v| v["document"]["library"][4]["origin"]["type"] = json!("found"),
            "document.library[4].origin.type",
        ),
    ];
    for (edit, path) in cases {
        let mut v = sink_value();
        edit(&mut v);
        assert_eq!(bad_value_at(decode(&compact(&v)).unwrap_err()), path);
    }
}

#[test]
fn marker_ranges_and_anchors_out_of_bounds_are_refused() {
    let cases: [(&EditJson, &str); 4] = [
        (
            &|v| v["document"]["sequences"][0]["markers"][1]["at"]["duration"] = json!(-1),
            "document.sequences[0].markers[1].at",
        ),
        (
            &|v| intro(v)["markers"][0]["at"]["start"] = json!(1_u64 << 60),
            "document.sequences[0].visual[0].clips[0].markers[0].at.start",
        ),
        (
            &|v| intro(v)["markers"][0]["id"] = json!(1_u64 << 60),
            "document.sequences[0].visual[0].clips[0].markers[0].id",
        ),
        (
            &|v| {
                let title = &mut v["document"]["sequences"][0]["visual"][2]["clips"][0];
                title["attach"]["owner_source_time"] = json!(-(1_i64 << 60));
            },
            "document.sequences[0].visual[2].clips[0].attach.owner_source_time",
        ),
    ];
    for (edit, path) in cases {
        let mut v = sink_value();
        edit(&mut v);
        assert_eq!(bad_value_at(decode(&compact(&v)).unwrap_err()), path);
    }
}

#[test]
fn markers_out_of_order_or_too_long_are_fixed_and_reported() {
    let mut v = sink_value();
    // The timeline list reversed, and a clip marker's name and note over
    // their limits.
    let list = v["document"]["sequences"][0]["markers"]
        .as_array_mut()
        .unwrap();
    list.reverse();
    intro(&mut v)["markers"][2]["name"] = json!("n".repeat(130));
    intro(&mut v)["markers"][2]["note"] = json!("é".repeat(2_001));
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::MarkersFixed { lists: 2 }]
    );
    assert!(opened.needs_save);
    let doc = &opened.document;
    let (sink, _) = kitchen_sink();
    let root = SeqId::from_raw(nz(1));
    assert_eq!(
        doc.sequence(root).unwrap().markers,
        sink.sequence(root).unwrap().markers
    );
    let fixed = doc
        .clip(clip_id(40))
        .unwrap()
        .markers
        .get(marker_id(63))
        .unwrap();
    assert_eq!(fixed.name, "n".repeat(120));
    assert_eq!(fixed.note, "é".repeat(2_000));
    assert_eq!(fixed.origin, MarkerOrigin::Beat { auto: false });
    assert_eq!(fixed.color, PaletteColor::Yellow);
}

#[test]
fn marker_ids_count_toward_the_id_counter() {
    let mut v = sink_value();
    v["ids_next"] = json!(99);
    intro(&mut v)["markers"][0]["id"] = json!(150);
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::IdsNextRaised {
            stored: 99,
            raised_to: 151
        }]
    );
    assert!(
        opened
            .document
            .clip(clip_id(40))
            .unwrap()
            .markers
            .get(marker_id(150))
            .is_some()
    );
}

#[test]
fn attachments_that_cannot_hold_are_cleared_and_reported() {
    // The title follows a clip that is not on the main track, the sticker
    // a clip that doesn't exist, and the intro (on the main track) the
    // still image.
    let mut v = sink_value();
    let root = &mut v["document"]["sequences"][0];
    root["visual"][2]["clips"][0]["attach"]["owner"] = json!(42);
    root["visual"][3]["clips"][0]["attach"]["owner"] = json!(98);
    root["visual"][0]["clips"][0]["attach"] = json!({"owner": 41, "owner_source_time": 0});
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::AttachmentsCleared {
            seq: SeqId::from_raw(nz(1)),
            clips: 3
        }]
    );
    assert!(opened.needs_save);
    let doc = &opened.document;
    let (sink, _) = kitchen_sink();
    for id in [40, 43, 44] {
        assert_eq!(doc.clip(clip_id(id)).unwrap().attach, None, "clip {id}");
        // Nothing moved.
        assert_eq!(
            doc.clip(clip_id(id)).unwrap().range,
            sink.clip(clip_id(id)).unwrap().range
        );
    }
    // The others keep theirs.
    for id in [46, 47, 48] {
        assert_eq!(
            doc.clip(clip_id(id)).unwrap().attach,
            sink.clip(clip_id(id)).unwrap().attach
        );
    }

    // With following off, nothing stays attached.
    let mut v = sink_value();
    v["document"]["sequences"][0]["flags"]["follow_main"] = json!(false);
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::AttachmentsCleared {
            seq: SeqId::from_raw(nz(1)),
            clips: 5
        }]
    );
    assert!(
        !opened
            .document
            .sequences()
            .next()
            .unwrap()
            .flags
            .follow_main
    );
}

#[test]
fn follow_flags_default_to_every_kind_and_are_written_in_kind_order() {
    let mut v = sink_value();
    let flags = &mut v["document"]["sequences"][1]["flags"];
    flags.as_object_mut().unwrap().remove("follow_main");
    flags["follow_kinds"] = json!(["sound-effect", "effect", "effect"]);
    let opened = decode(&compact(&v)).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    let nested = opened.document.sequence(SeqId::from_raw(nz(2))).unwrap();
    assert!(nested.flags.follow_main, "a missing key reads as on");
    let written = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(
        written["document"]["sequences"][1]["flags"],
        json!({
            "main_magnet": false,
            "follow_main": true,
            "follow_kinds": ["effect", "sound-effect"]
        })
    );
    // No kind checked is a valid set of its own.
    let mut v = sink_value();
    v["document"]["sequences"][1]["flags"]["follow_kinds"] = json!([]);
    let opened = decode(&compact(&v)).unwrap();
    let nested = opened.document.sequence(SeqId::from_raw(nz(2))).unwrap();
    assert!(nested.flags.follow_kinds.is_empty());
    assert!(!nested.flags.follows());
}

#[test]
fn an_imported_origin_is_left_out_unless_it_carries_unknown_keys() {
    let mut v = sink_value();
    let beach = &mut v["document"]["library"][0];
    assert!(beach.get("origin").is_none());
    beach["origin"] = json!({"type": "imported"});
    v["document"]["library"][1]["origin"] = json!({"type": "imported", "x_from": "camera"});
    let opened = decode(&compact(&v)).unwrap();
    for item in opened.document.library().items().take(2) {
        assert_eq!(item.origin, MediaOrigin::Imported);
    }
    let written = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert!(written["document"]["library"][0].get("origin").is_none());
    assert_eq!(
        written["document"]["library"][1]["origin"],
        json!({"type": "imported", "x_from": "camera"})
    );
}

#[test]
fn a_clip_marker_keeps_its_unknown_keys_through_a_split() {
    let mut v = sink_value();
    // Marker 62 sits at source frame 30 of the intro, sequence frame 20.
    intro(&mut v)["markers"][1]["x_bpm"] = json!(120);
    let opened = decode(&compact(&v)).unwrap();
    let mut editor = Editor::open(
        opened.document.clone(),
        opened.ids.clone(),
        HistoryLimits::default(),
    )
    .unwrap();
    let ctx = EditCtx {
        limits: Limits::default(),
    };
    let split = EditRequest::Split(SplitReq {
        seq: SeqId::from_raw(nz(1)),
        at: fr(15),
        targets: SplitTargets::Clips(vec![clip_id(40)]),
    });
    let plan = editor.plan(&ctx, &split).unwrap();
    editor.commit(plan, CommitMeta::default()).unwrap();
    let doc = editor.document();
    let right = doc
        .sequences()
        .flat_map(|s| s.tracks().map(|(_, t)| t))
        .flat_map(|t| t.clips())
        .find(|c| c.markers.get(marker_id(62)).is_some())
        .unwrap();
    assert_ne!(right.id, clip_id(40), "the right piece shows it");

    let bytes = encode(doc, editor.ids().next(), &opened.extras).unwrap();
    let reopened = decode(&bytes).unwrap();
    assert_eq!(&reopened.document, doc);
    assert!(
        reopened
            .extras
            .get(Owner::Marker(marker_id(62)), "")
            .unwrap()
            .contains_key("x_bpm")
    );
}

#[test]
fn a_recorded_take_saves_with_its_voiceover_origin() {
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
    // A 2-second take after a 1-frame pre-roll, recorded at frame 20.
    let item = editor
        .document()
        .library()
        .get(MediaId::from_raw(nz(14)))
        .unwrap()
        .clone();
    let media = freecut_timeline::edit::NewMedia {
        path: freecut_timeline::model::StoredPath {
            absolute: "/Users/me/Movies/Trip Media/Recordings/Voiceover 2.wav".into(),
            relative: Some("Trip Media/Recordings/Voiceover 2.wav".into()),
        },
        info: item.info.clone(),
        fingerprint: None,
        display_name: None,
    };
    let record = EditRequest::InsertRecordedClip {
        seq: SeqId::from_raw(nz(1)),
        media,
        at: fr(20),
        handle_in: fr(1),
        target: RecordTarget::NewTrack,
        replace: None,
    };
    let plan = editor.plan(&ctx, &record).unwrap();
    editor.commit(plan, CommitMeta::default()).unwrap();
    let doc = editor.document();
    let take = doc
        .library()
        .items()
        .find(|m| m.path.absolute.ends_with("Voiceover 2.wav"))
        .unwrap();
    assert_eq!(
        take.origin,
        MediaOrigin::Generated {
            kind: GeneratedMediaKind::Voiceover
        }
    );
    let bytes = encode(doc, editor.ids().next(), &Extras::new()).unwrap();
    let reopened = decode(&bytes).unwrap();
    assert_eq!(&reopened.document, doc);
    assert!(reopened.warnings.is_empty(), "{:?}", reopened.warnings);
}

#[test]
fn markers_beyond_the_file_tick_range_are_refused_on_save() {
    let (doc, ids) = kitchen_sink();
    let root = doc.sequence(SeqId::from_raw(nz(1))).unwrap();
    let mut far = root.clone();
    let mut m = far.markers.iter().next().unwrap().clone();
    m.at = MarkRange::new(Tick::from_raw((1 << 53) + 1), Tick::ZERO).unwrap();
    far.markers = freecut_timeline::model::Markers::new(vec![m]);
    let nested = std::sync::Arc::new(doc.sequence(SeqId::from_raw(nz(2))).unwrap().clone());
    let doc = freecut_timeline::model::Document::from_parts(
        doc.root(),
        *doc.settings(),
        doc.library().clone(),
        vec![std::sync::Arc::new(far), nested],
    )
    .unwrap();
    match encode(&doc, ids, &Extras::new()) {
        Err(SaveError::OutOfRange { path, .. }) => {
            assert_eq!(path, "document.sequences[0].markers[0].at.start");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_sequence_made_today_follows_every_kind() {
    // What the toolbar round made the default; a format-7 file reads the
    // same way (tests/migration.rs).
    let seq = Sequence::new(
        SeqId::from_raw(nz(1)),
        freecut_timeline::ids::TrackId::from_raw(nz(2)),
    );
    assert_eq!(seq.flags, SequenceFlags::default());
    assert!(seq.flags.follow_main && seq.flags.follows());
}
