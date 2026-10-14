//! Save → open round trips: random valid documents come back equal, saving
//! is a byte-level fixpoint, and the file never contains a float.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_project::{CURRENT_FORMAT_VERSION, Extras, SaveRequest, decode, encode, open, save};
use pretty_assertions::assert_eq;
use proptest::prelude::*;

fn save_open_save(doc: &freecut_timeline::model::Document, ids: core::num::NonZeroU64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Random.freecut");
    let extras = Extras::new();
    let request = SaveRequest {
        document: doc,
        ids_next: ids,
        extras: &extras,
        backup: false,
        relative_base: Some(dir.path()),
        export: None,
        ui: None,
    };
    save(&path, &request).unwrap();
    let first = std::fs::read(&path).unwrap();

    let opened = open(&path).unwrap();
    assert_eq!(&opened.document, doc, "save → open gives an equal document");
    assert_eq!(opened.ids.next(), ids, "the id counter survives");
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    assert!(!opened.needs_save);
    assert!(opened.extras.is_empty());

    let request = SaveRequest {
        document: &opened.document,
        ids_next: opened.ids.next(),
        extras: &opened.extras,
        backup: false,
        relative_base: Some(dir.path()),
        export: None,
        ui: None,
    };
    save(&path, &request).unwrap();
    let second = std::fs::read(&path).unwrap();
    assert!(
        first == second,
        "save(open(save(d))) == save(d) byte for byte"
    );

    let mut found = Vec::new();
    floats(&to_value(&first), "", &mut found);
    assert!(found.is_empty(), "floats in the file at {found:?}");
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    #[test]
    fn random_documents_survive_save_and_open(plan in doc_plan()) {
        let (doc, ids) = build(&plan);
        save_open_save(&doc, ids);
    }

    /// The reader doesn't depend on our formatting or key order.
    #[test]
    fn compact_json_reads_back_the_same(plan in doc_plan()) {
        let (doc, ids) = build(&plan);
        let bytes = encode(&doc, ids, &Extras::new()).unwrap();
        let compacted = compact(&to_value(&bytes));
        let opened = decode(&compacted).unwrap();
        prop_assert_eq!(&opened.document, &doc);
        prop_assert_eq!(encode(&opened.document, opened.ids.next(), &opened.extras).unwrap(), bytes);
    }
}

#[test]
fn kitchen_sink_round_trips() {
    let (doc, ids) = kitchen_sink();
    save_open_save(&doc, ids);
}

#[test]
fn canonical_bytes_use_two_space_indent_lf_and_a_trailing_newline() {
    let (doc, ids) = kitchen_sink();
    let bytes = encode(&doc, ids, &Extras::new()).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.ends_with("}\n") && !text.ends_with("\n\n"));
    assert!(!text.contains('\r'));
    assert!(text.starts_with(&format!(
        "{{\n  \"format\": \"freecut-project\",\n  \"format_version\": {CURRENT_FORMAT_VERSION},\n  \"app\": \"freecut "
    )));
    for line in text.lines() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        assert_eq!(indent % 2, 0, "odd indent in {line:?}");
        assert!(!line.starts_with('\t'));
    }
}

#[test]
fn a_bom_is_accepted() {
    let (doc, ids) = kitchen_sink();
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend(encode(&doc, ids, &Extras::new()).unwrap());
    assert_eq!(decode(&bytes).unwrap().document, doc);
}

#[test]
fn hashes_are_hex_strings_and_ticks_are_integers() {
    let (doc, ids) = kitchen_sink();
    let v = to_value(&encode(&doc, ids, &Extras::new()).unwrap());
    let beach = &v["document"]["library"][0];
    assert_eq!(beach["fingerprint"]["head_tail"], "fedcba9876543210");
    assert_eq!(beach["info"]["duration"], fr(100).raw());
    assert_eq!(v["document"]["settings"]["frame_rate"]["num"], 30_000);
    assert_eq!(v["document"]["settings"]["frame_rate"]["den"], 1001);
    assert_eq!(v["ids_next"], 100);
}

/// Text clips and transforms made through the editor's planners survive a
/// save and open (format 3).
#[test]
fn edited_text_and_transforms_round_trip() {
    let video = MediaSpec {
        kind: 0,
        frames: 120,
        absolute: "/m/a.mp4".into(),
        relative: None,
        fingerprint: None,
        display_name: None,
        rotation: 0,
        start: 0,
        tagged: false,
    };
    let ops = vec![
        Op::Insert {
            asset: 0,
            src: 0,
            len: 60,
            at: 0,
            track: 200,
        },
        Op::InsertText {
            at: 5,
            len: 40,
            text: "Hello\n“world” 🎬".into(),
            // `FirstFree`: past the existing tracks and the new-track slot.
            target: 3,
        },
        Op::TextStyle {
            pick: 0,
            edit: 0,
            value: 0,
            family: "Inter".into(),
        },
        Op::TextStyle {
            pick: 0,
            edit: 1,
            value: 12_345,
            family: String::new(),
        },
        Op::TextStyle {
            pick: 0,
            edit: 2,
            value: 0x7f10_20ff,
            family: String::new(),
        },
        Op::TextLayout {
            pick: 0,
            edit: 1,
            value: 50_000,
        },
        Op::TextLayout {
            pick: 0,
            edit: 2,
            value: 1_500,
        },
        Op::Transform {
            pick: 0,
            edit: 0,
            x: -400_000,
            y: 9_999_999,
        },
        Op::Transform {
            pick: 1,
            edit: 2,
            x: 45_000,
            y: 0,
        },
        Op::Transform {
            pick: 1,
            edit: 5,
            x: 600,
            y: 0,
        },
    ];
    let plan = DocPlan {
        settings: freecut_timeline::model::ProjectSettings::new_default().unwrap(),
        media: vec![video],
        ops,
    };
    let (doc, ids) = build(&plan);
    let clips: Vec<_> = doc
        .sequences()
        .flat_map(|s| s.tracks().map(|(_, t)| t))
        .flat_map(|t| t.clips())
        .collect();
    let text = clips
        .iter()
        .find_map(|c| c.content.text())
        .expect("the text clip was inserted");
    assert_eq!(text.content.as_str(), "Hello\n“world” 🎬");
    assert_eq!(text.style.font.family, "Inter");
    assert_eq!(text.style.size.hundredths(), 12_345);
    assert_ne!(text.layout.box_width, None);
    assert!(
        clips
            .iter()
            .filter(|c| c.transform != freecut_timeline::model::Transform::IDENTITY)
            .count()
            >= 2,
        "{clips:#?}"
    );
    save_open_save(&doc, ids);
}

/// The random plans above really exercise the format-3 data: some of their
/// documents hold styled text and non-identity transforms.
#[test]
fn random_plans_reach_text_and_transforms() {
    use freecut_timeline::model::{TextClipData, Transform};
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRng, TestRunner};
    let mut runner = TestRunner::new_with_rng(
        Config::default(),
        TestRng::deterministic_rng(proptest::test_runner::RngAlgorithm::ChaCha),
    );
    let (mut styled, mut transformed) = (0, 0);
    for _ in 0..64 {
        let plan = doc_plan().new_tree(&mut runner).unwrap().current();
        let (doc, _) = build(&plan);
        for clip in doc
            .sequences()
            .flat_map(|s| s.tracks().map(|(_, t)| t))
            .flat_map(|t| t.clips())
        {
            if clip
                .content
                .text()
                .is_some_and(|d| d.style != TextClipData::default().style)
            {
                styled += 1;
            }
            if clip.transform != Transform::IDENTITY {
                transformed += 1;
            }
        }
    }
    assert!(styled > 0 && transformed > 0, "{styled} {transformed}");
}

/// The random plans above really exercise the format-6 data: their
/// documents hold transform and text keys with eased segments, all three
/// animation slots, and eased volume keys; format 10's colour stacks and
/// colour keys; format 11's HSL, curves, wheels and puck keys; format
/// 12's LUT slots, skin protection and their keys; format 13's auto
/// corrections, matches and their keys; and format 14's colour masks of
/// every shape and mode and their keys.
#[test]
fn random_plans_reach_keys_and_animations() {
    use freecut_timeline::model::{Interp, PropPath, Slot};
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRng, TestRunner};
    let mut runner = TestRunner::new_with_rng(
        Config::default(),
        TestRng::deterministic_rng(proptest::test_runner::RngAlgorithm::ChaCha),
    );
    let eased = |i: Interp| !matches!(i, Interp::Linear | Interp::Hold);
    let (mut transform_keys, mut text_keys, mut eased_keys, mut eased_gain) = (0, 0, 0, 0);
    let (mut graded, mut color_keys, mut puck_keys) = (0, 0, 0);
    let (mut hsl, mut curves, mut wheels) = (0, 0, 0);
    let (mut luts, mut skins, mut k3_keys) = (0, 0, 0);
    let (mut autos, mut matches, mut k5_keys) = (0, 0, 0);
    let (mut shapes, mut modes, mut k6_keys) = (
        std::collections::BTreeSet::new(),
        std::collections::BTreeSet::new(),
        0,
    );
    let mut slots = [0; 3];
    for _ in 0..960 {
        let plan = doc_plan().new_tree(&mut runner).unwrap().current();
        let (doc, _) = build(&plan);
        for clip in doc
            .sequences()
            .flat_map(|s| s.tracks().map(|(_, t)| t))
            .flat_map(|t| t.clips())
        {
            for track in clip.keys.tracks() {
                match track.prop() {
                    PropPath::Transform(_) => transform_keys += 1,
                    PropPath::Color(
                        freecut_timeline::model::ColorParamId::LutIntensity
                        | freecut_timeline::model::ColorParamId::SkinAmount(_),
                    ) => k3_keys += 1,
                    PropPath::Color(
                        freecut_timeline::model::ColorParamId::AutoStrength
                        | freecut_timeline::model::ColorParamId::MatchIntensity,
                    ) => k5_keys += 1,
                    PropPath::Color(_) => color_keys += 1,
                    PropPath::ColorPuck(_) => puck_keys += 1,
                    PropPath::ColorMask { .. } => k6_keys += 1,
                    _ => text_keys += 1,
                }
                eased_keys += track.keys().iter().filter(|k| eased(k.interp)).count();
            }
            graded += usize::from(clip.color.is_some());
            if let Some(c) = &clip.color {
                hsl += usize::from(!c.hsl.is_default());
                curves += usize::from(!c.curves.is_identity());
                wheels += usize::from(c.wheels.any_set());
                luts += usize::from(c.lut.is_some());
                skins += usize::from(c.basic.skin.on || c.lut.as_ref().is_some_and(|l| l.skin.on));
                autos += usize::from(c.auto.is_some());
                matches += usize::from(c.match_xf.is_some());
                for m in &c.masks {
                    shapes.insert(m.mask.kind());
                    modes.insert(m.mode);
                }
            }
            for (n, slot) in slots.iter_mut().zip(Slot::ALL) {
                *n += usize::from(clip.animations.get(slot).is_some());
            }
            if let Some(audio) = clip.audio.active() {
                eased_gain += audio.gain.keys().iter().filter(|k| eased(k.interp)).count();
            }
        }
    }
    assert!(
        transform_keys > 0 && text_keys > 0 && eased_keys > 0 && eased_gain > 0,
        "{transform_keys} {text_keys} {eased_keys} {eased_gain}"
    );
    assert!(slots.iter().all(|n| *n > 0), "{slots:?}");
    // Format 10: colour stacks and colour keys.
    assert!(graded > 0 && color_keys > 0, "{graded} {color_keys}");
    // Format 11: HSL, curves, wheels and keyed pucks.
    assert!(
        hsl > 0 && curves > 0 && wheels > 0 && puck_keys > 0,
        "{hsl} {curves} {wheels} {puck_keys}"
    );
    // Format 12: LUT slots, skin protection and their keys.
    assert!(
        luts > 0 && skins > 0 && k3_keys > 0,
        "{luts} {skins} {k3_keys}"
    );
    // Format 13: auto corrections, matches and their keys.
    assert!(
        autos > 0 && matches > 0 && k5_keys > 0,
        "{autos} {matches} {k5_keys}"
    );
    // Format 14: masks of every shape and mode, and their keys.
    assert!(
        shapes.len() == 4 && modes.len() == 3 && k6_keys > 0,
        "{shapes:?} {modes:?} {k6_keys}"
    );
}

/// The random plans above really exercise the format-7 to 9 data: their
/// documents hold timeline and clip markers (ranges and points, several
/// colours), attachments, sequences that don't follow or follow only some
/// kinds, and recorded voiceover takes.
#[test]
fn random_plans_reach_markers_following_and_recorded_takes() {
    use freecut_timeline::model::{FollowKinds, MediaOrigin};
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRng, TestRunner};
    let mut runner = TestRunner::new_with_rng(
        Config::default(),
        TestRng::deterministic_rng(proptest::test_runner::RngAlgorithm::ChaCha),
    );
    let (mut timeline_markers, mut clip_markers, mut ranges) = (0, 0, 0);
    let (mut attached, mut not_following, mut some_kinds, mut takes) = (0, 0, 0, 0);
    let mut colors = std::collections::BTreeSet::new();
    for _ in 0..64 {
        let plan = doc_plan().new_tree(&mut runner).unwrap().current();
        let (doc, _) = build(&plan);
        takes += doc
            .library()
            .items()
            .filter(|m| m.origin != MediaOrigin::Imported)
            .count();
        for seq in doc.sequences() {
            not_following += usize::from(!seq.flags.follow_main);
            some_kinds += usize::from(seq.flags.follow_kinds != FollowKinds::ALL);
            timeline_markers += seq.markers.len();
            let clips = seq.tracks().flat_map(|(_, t)| t.clips());
            let markers = seq
                .markers
                .iter()
                .chain(clips.flat_map(|c| c.markers.iter()));
            for m in markers {
                ranges += usize::from(!m.at.is_point());
                colors.insert(m.color);
            }
            for clip in seq.tracks().flat_map(|(_, t)| t.clips()) {
                clip_markers += clip.markers.len();
                attached += usize::from(clip.attach.is_some());
            }
        }
    }
    let counts = [
        timeline_markers,
        clip_markers,
        ranges,
        attached,
        not_following,
        some_kinds,
        takes,
    ];
    assert!(counts.iter().all(|n| *n > 0), "{counts:?}");
    assert!(colors.len() > 4, "{colors:?}");
}
