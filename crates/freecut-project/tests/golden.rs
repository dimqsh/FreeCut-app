//! The committed golden file locks the format: the kitchen-sink document,
//! plus unknown keys at several levels, must encode to exactly the bytes in
//! `tests/golden/kitchen_sink.freecut`, and reading that file must give the
//! same document back.
//!
//! To regenerate after a deliberate format change:
//!
//! 1. Freeze the old golden file first: copy it to
//!    `tests/fixtures/format/v<old version>/kitchen_sink.freecut`, never to
//!    be regenerated (`tests/migration.rs` opens it).
//! 2. Bump `CURRENT_FORMAT_VERSION` and add a migration step.
//! 3. Re-bless the golden file and the migration expectations:
//!
//! ```text
//! FREECUT_BLESS_GOLDEN=1 cargo test -p freecut-project --test golden --test migration
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::path::PathBuf;

use common::*;
use freecut_project::{Extras, Owner, decode, encode_with_ui};
use pretty_assertions::assert_eq;
use serde_json::json;

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/kitchen_sink.freecut")
}

/// The kitchen sink, its export settings and its UI state with unknown keys
/// injected the way a newer build might write them, read and written back
/// by this build.
fn generate() -> Vec<u8> {
    let (doc, ids) = kitchen_sink();
    let export = kitchen_export();
    let ui = kitchen_ui();
    let mut v = to_value(&encode_with_ui(&doc, ids, Some(&export), &ui, &Extras::new()).unwrap());
    // Keys in sorted order, so the output doesn't depend on serde_json's
    // `preserve_order` feature being enabled somewhere in the workspace.
    v["x_future_root"] = json!({"n": -5, "nested": [1, "two", null, true]});
    v["document"]["x_doc"] = json!("document level");
    v["document"]["settings"]["frame_rate"]["x_rate"] = json!(1);
    let media = &mut v["document"]["library"][0];
    media["x_media"] = json!({"proxy": "none"});
    media["info"]["video"][1]["x_stream"] = json!("rotated");
    v["document"]["library"][3]["source"]["x_source"] = json!("left channel");
    let main = &mut v["document"]["sequences"][0]["visual"][0];
    main["clips"][0]["x_clip"] = json!({"keyframes": []});
    main["clips"][0]["content"]["source"]["x_src"] = json!(7);
    main["clips"][0]["audio"]["x_detached"] = json!("resync later");
    v["document"]["sequences"][0]["audio"][1]["clips"][0]["audio"]["x_pan"] = json!(-30);
    v["document"]["sequences"][0]["audio"][1]["header"]["x_header"] = json!(true);
    let overlay = &mut v["document"]["sequences"][0]["visual"][1]["clips"][0];
    overlay["transform"]["x_anchor"] = json!([0, 0]);
    overlay["transform"]["position"]["x_unit"] = json!("canvas");
    let title = &mut v["document"]["sequences"][0]["visual"][2]["clips"][0]["content"];
    title["x_spans"] = json!([]);
    title["style"]["x_outline"] = json!({"width": 300});
    title["style"]["font"]["x_weight"] = json!(700);
    title["layout"]["x_anchor"] = json!("top");
    // Format 5's text style objects, down to a gradient stop and the tail
    // tip, keep theirs too.
    title["style"]["fill"]["x_space"] = json!("oklab");
    title["style"]["fill"]["stops"][1]["x_mid"] = json!(50);
    title["style"]["glow"]["x_blend"] = json!("screen");
    title["style"]["background"]["border"]["x_dash"] = json!([4, 2]);
    title["style"]["bubble"]["kind"]["x_notch"] = json!(1);
    title["style"]["bubble"]["tail"]["tip"]["x_track"] = json!("speaker 1");
    // A factory-default layer is left out, unless it carries unknown keys.
    let caption = &mut v["document"]["sequences"][0]["visual"][2]["clips"][1]["content"];
    assert!(caption["style"].get("outline").is_none());
    caption["style"]["outline"] = json!({
        "on": false, "color": "#000000FF", "width": 60, "opacity": 1000,
        "position": "outside", "x_dash": true
    });
    // Format 6's keyframes and animations keep theirs: on the objects, on
    // a key track, a key and a point value, and an animation slot.
    let title_clip = &mut v["document"]["sequences"][0]["visual"][2]["clips"][0];
    title_clip["keys"]["x_space"] = json!("content");
    let position = &mut title_clip["keys"]["tracks"][0];
    assert_eq!(position["prop"], "transform.position");
    position["x_label"] = json!("path 1");
    position["keys"][1]["x_handles"] = json!([[1, 2], [3, 4]]);
    position["keys"][1]["value"]["x_unit"] = json!("canvas");
    title_clip["animations"]["x_mode"] = json!("auto");
    title_clip["animations"]["entrance"]["x_caret"] = json!(true);
    v["export"]["x_preset"] = json!(null);
    v["export"]["video"]["size"]["x_fit"] = json!("fill");
    v["export"]["audio"]["x_channels"] = json!(2);
    v["ui"]["x_zoom"] = json!(3);
    let voice_track = &mut v["document"]["sequences"][0]["audio"][0];
    voice_track["mix"]["x_meter"] = json!("peak");
    voice_track["clips"][0]["audio"]["gain_keys"][1]["x_ease"] = json!("in");
    v["document"]["sequences"][0]["visual"][1]["clips"][0]["content"]["stream"]["x_label"] =
        json!("both mics");
    // Formats 7 to 9: markers (a timeline marker, its range and a clip
    // marker), the follow flags, an attachment and a generated origin.
    let root = &mut v["document"]["sequences"][0];
    assert_eq!(root["markers"][1]["id"], 61);
    root["markers"][1]["x_marker"] = json!(["lane 2"]);
    root["markers"][1]["at"]["x_unit"] = json!("frames");
    root["flags"]["x_follow"] = json!("beta");
    let intro = &mut root["visual"][0]["clips"][0];
    assert_eq!(intro["markers"][1]["id"], 62);
    intro["markers"][1]["x_bpm"] = json!(120);
    root["visual"][2]["clips"][0]["attach"]["x_lock"] = json!(true);
    // Format 10: the colour stack and its two sections.
    let graded = &mut v["document"]["sequences"][0]["visual"][0]["clips"][0]["color"];
    graded["x_lut"] = json!({"hash": "00", "intensity": 60});
    graded["light"]["x_dehaze"] = json!(5);
    graded["color"]["x_skin"] = json!(true);
    let take = &mut v["document"]["library"][4];
    assert_eq!(take["origin"]["kind"], "voiceover");
    take["origin"]["x_mic"] = json!({"name": "USB mic", "rate": 48000});
    let opened = decode(&compact(&v)).unwrap();
    encode_with_ui(
        &opened.document,
        opened.ids.next(),
        opened.export.as_ref(),
        &opened.ui,
        &opened.extras,
    )
    .unwrap()
}

/// The `app` value records the writer's version; a version bump is not a
/// format change, so that one value is compared by shape only.
fn without_app_version(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if line.starts_with("  \"app\": \"freecut ") {
            out.push_str("  \"app\": \"freecut <version>\",\n");
        } else {
            out.push_str(line);
        }
    }
    out
}

#[test]
fn kitchen_sink_matches_the_golden_file() {
    let generated = generate();
    let path = golden_path();
    if std::env::var_os("FREECUT_BLESS_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &generated).unwrap();
    }
    let golden = std::fs::read(&path).expect(
        "missing golden file; run with FREECUT_BLESS_GOLDEN=1 to create it, then commit it",
    );
    let golden_text = String::from_utf8(golden.clone()).unwrap();
    let generated_text = String::from_utf8(generated).unwrap();
    assert!(
        !golden_text.contains('\r'),
        "the golden file must keep LF endings"
    );
    assert_eq!(
        without_app_version(&generated_text),
        without_app_version(&golden_text),
        "the format changed; bump CURRENT_FORMAT_VERSION, add a migration and re-bless"
    );
    assert_eq!(
        golden_text
            .lines()
            .nth(3)
            .map(|l| l.starts_with("  \"app\": \"freecut ")),
        Some(true)
    );
}

#[test]
fn the_golden_file_reads_back_to_the_kitchen_sink() {
    let golden = std::fs::read(golden_path()).unwrap();
    let opened = decode(&golden).unwrap();
    let (doc, ids) = kitchen_sink();
    assert_eq!(opened.document, doc);
    assert_eq!(opened.ids.next(), ids);
    assert_eq!(opened.export, Some(kitchen_export()));
    assert_eq!(opened.ui, kitchen_ui());
    assert!(!opened.needs_save);
    assert_eq!(opened.extras.len(), 46);
    let marker = |n: u64| Owner::Marker(freecut_timeline::ids::MarkerId::from_raw(nz(n)));
    for (owner, at, key) in [
        (marker(61), "", "x_marker"),
        (marker(61), "at", "x_unit"),
        (marker(62), "", "x_bpm"),
        (
            Owner::Sequence(freecut_timeline::ids::SeqId::from_raw(nz(1))),
            "flags",
            "x_follow",
        ),
        (
            Owner::Clip(freecut_timeline::ids::ClipId::from_raw(nz(43))),
            "attach",
            "x_lock",
        ),
        (
            Owner::Media(freecut_timeline::ids::MediaId::from_raw(nz(14))),
            "origin",
            "x_mic",
        ),
        (
            Owner::Clip(freecut_timeline::ids::ClipId::from_raw(nz(40))),
            "color",
            "x_lut",
        ),
        (
            Owner::Clip(freecut_timeline::ids::ClipId::from_raw(nz(40))),
            "color.light",
            "x_dehaze",
        ),
        (
            Owner::Clip(freecut_timeline::ids::ClipId::from_raw(nz(40))),
            "color.color",
            "x_skin",
        ),
    ] {
        assert!(
            opened.extras.get(owner, at).unwrap().contains_key(key),
            "{owner:?} {at}"
        );
    }
    assert!(
        opened
            .extras
            .get(
                Owner::Clip(freecut_timeline::ids::ClipId::from_raw(nz(47))),
                &format!("audio.gain_keys@{}", fr(20).raw())
            )
            .unwrap()
            .contains_key("x_ease")
    );
    let title = Owner::Clip(freecut_timeline::ids::ClipId::from_raw(nz(43)));
    let moved = format!("keys[transform.position]@{}", fr(20).raw());
    for (at, key) in [
        ("keys", "x_space"),
        ("keys[transform.position]", "x_label"),
        (moved.as_str(), "x_handles"),
        (&format!("{moved}.value"), "x_unit"),
        ("animations", "x_mode"),
        ("animations.entrance", "x_caret"),
        ("content.style.fill", "x_space"),
        ("content.style.fill.stops[1]", "x_mid"),
        ("content.style.bubble.kind", "x_notch"),
        ("content.style.bubble.tail.tip", "x_track"),
    ] {
        assert!(
            opened.extras.get(title, at).unwrap().contains_key(key),
            "{at}"
        );
    }
    assert!(
        opened
            .extras
            .get(Owner::Ui, "")
            .unwrap()
            .contains_key("x_zoom")
    );
    assert!(
        opened
            .extras
            .get(Owner::Export, "video.size")
            .unwrap()
            .contains_key("x_fit")
    );
    assert!(
        opened
            .extras
            .get(Owner::Root, "")
            .unwrap()
            .contains_key("x_future_root")
    );

    // Reading and writing it again is a byte-level fixpoint.
    let again = encode_with_ui(
        &opened.document,
        opened.ids.next(),
        opened.export.as_ref(),
        &opened.ui,
        &opened.extras,
    )
    .unwrap();
    assert!(
        again == golden
            || without_app_version(&String::from_utf8(again).unwrap())
                == without_app_version(&String::from_utf8(golden.clone()).unwrap())
    );

    let mut found = Vec::new();
    floats(&to_value(&golden), "", &mut found);
    assert!(found.is_empty(), "floats at {found:?}");
}
