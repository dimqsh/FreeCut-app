//! Files from older formats open in the current one (PM-021).
//!
//! Each superseded format keeps a file its writer produced, frozen under
//! `tests/fixtures/format/v<N>/` and never regenerated. It must open, and
//! re-encoding it must give `<name>.expected.freecut`: what this build
//! writes for it. After a later format change, re-bless the expectations
//! (not the frozen files) with
//!
//! ```text
//! FREECUT_BLESS_GOLDEN=1 cargo test -p freecut-project --test migration
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::path::PathBuf;

use common::*;
use freecut_project::{CURRENT_FORMAT_VERSION, Extras, OpenWarning, Owner, decode, encode};
use freecut_timeline::ids::{ClipId, MediaId};
use freecut_timeline::model::{
    ClipAnimations, ClipAudio, ClipContent, ClipKeys, EmbeddedAudio, Fill, Markers, MediaOrigin,
    MediaSource, PropPath, Rgba8, SequenceFlags, TextClipData, TrackAudio, Transform,
};
use pretty_assertions::assert_eq;
use proptest::prelude::*;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/format")
        .join(name)
}

/// The `app` line records the writer's version, which is not a format change.
fn without_app_version(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            if line.starts_with("  \"app\": \"freecut ") {
                "  \"app\": \"freecut <version>\",\n"
            } else {
                line
            }
        })
        .collect()
}

#[test]
fn the_frozen_v1_kitchen_sink_opens_with_the_sound_of_its_time() {
    let v1 = std::fs::read(fixture("v1/kitchen_sink.freecut")).unwrap();
    assert_eq!(to_value(&v1)["format_version"], 1, "the fixture is frozen");
    let opened = decode(&v1).unwrap();
    assert_eq!(
        opened.warnings,
        vec![
            OpenWarning::Migrated {
                from: 1,
                to: CURRENT_FORMAT_VERSION
            },
            OpenWarning::UnknownFields { objects: 8 },
        ]
    );
    assert!(!opened.needs_save, "an upgrade alone is not a change");
    assert!(opened.backup_on_save(), "but the original is kept once");
    assert_eq!(opened.ids.next(), nz(100));
    assert!(
        opened
            .extras
            .get(Owner::Root, "")
            .unwrap()
            .contains_key("x_future_root")
    );

    let doc = &opened.document;
    for item in doc.library().items() {
        assert_eq!(item.source, MediaSource::File, "format 1 had only files");
    }
    let sound = |id: u64| doc.clip(ClipId::from_raw(nz(id))).unwrap().audio.clone();
    let unity = EmbeddedAudio::Active(ClipAudio::default());
    // Video and audio clips of media with sound play it at unity gain, with
    // no fades; 40 and 47 were linked, but format 1 had no detached sound.
    for id in [40, 42, 47, 48, 49] {
        assert_eq!(sound(id), unity, "clip {id}");
    }
    // The image and the generated items have none.
    for id in [41, 43, 44, 45, 46] {
        assert_eq!(sound(id), EmbeddedAudio::None, "clip {id}");
    }

    // Format 1's generated text opens as a factory-default text clip.
    assert_eq!(
        doc.clip(ClipId::from_raw(nz(43))).unwrap().content,
        ClipContent::Text(std::sync::Arc::new(TextClipData::default()))
    );

    // What this build writes for it; it opens as is, with nothing left to
    // migrate.
    check_expected(&opened, "v1/kitchen_sink.expected.freecut");
}

#[test]
fn a_video_without_an_audio_stream_has_no_sound_after_migration() {
    // One silent video, one video with sound, inserted by today's editor.
    let video = |kind| MediaSpec {
        kind,
        frames: 60,
        absolute: "/m/clip.mp4".into(),
        relative: None,
        fingerprint: None,
        display_name: None,
        rotation: 0,
        start: 0,
        tagged: false,
    };
    let insert = |asset| Op::Insert {
        asset,
        src: 0,
        len: 30,
        at: 0,
        track: 200,
    };
    let plan = DocPlan {
        settings: freecut_timeline::model::ProjectSettings::new_default().unwrap(),
        media: vec![video(3), video(0)],
        ops: vec![insert(0), insert(1)],
    };
    let (doc, ids) = build(&plan);
    let v1 = to_v1(&to_value(&encode(&doc, ids, &Extras::new()).unwrap()));
    let opened = decode(&compact(&v1)).unwrap();
    assert_eq!(opened.document, doc);
    let sounds: Vec<_> = doc
        .library()
        .items()
        .map(|item| {
            let clip = doc
                .sequences()
                .flat_map(|s| s.tracks().map(|(_, t)| t))
                .flat_map(|t| t.clips())
                .find(|c| c.content.asset() == Some(item.id))
                .unwrap();
            (item.info.has_audio(), clip.audio.clone())
        })
        .collect();
    assert_eq!(
        sounds,
        vec![
            (false, EmbeddedAudio::None),
            (true, EmbeddedAudio::Active(ClipAudio::default())),
        ]
    );
}

#[test]
fn keys_a_development_build_wrote_under_format_1_are_kept() {
    // A build that already wrote sound but still called the format 1.
    let (doc, ids) = kitchen_sink();
    let mut v = to_value(&encode(&doc, ids, &Extras::new()).unwrap());
    v["format_version"] = 1.into();
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(opened.document, doc);
    let extracted = opened
        .document
        .library()
        .get(MediaId::from_raw(nz(13)))
        .unwrap();
    assert!(matches!(extracted.source, MediaSource::AudioOf { .. }));
}

/// Opens a frozen fixture, checks that encoding it gives
/// `<name>.expected.freecut` (re-blessed with `FREECUT_BLESS_GOLDEN=1`), and
/// that the result opens with nothing left to migrate.
fn check_expected(opened: &freecut_project::Opened, expected_name: &str) {
    let doc = &opened.document;
    let written = encode(doc, opened.ids.next(), &opened.extras).unwrap();
    let expected_path = fixture(expected_name);
    if std::env::var_os("FREECUT_BLESS_GOLDEN").is_some() {
        std::fs::write(&expected_path, &written).unwrap();
    }
    let expected = std::fs::read_to_string(&expected_path)
        .expect("missing expectation; run with FREECUT_BLESS_GOLDEN=1 to create it");
    let written = String::from_utf8(written).unwrap();
    assert_eq!(
        without_app_version(&written),
        without_app_version(&expected)
    );
    let again = decode(written.as_bytes()).unwrap();
    assert_eq!(&again.document, doc);
    assert!(
        !again
            .warnings
            .iter()
            .any(|w| matches!(w, OpenWarning::Migrated { .. }))
    );
    assert!(!again.backup_on_save());
}

#[test]
fn the_frozen_v2_kitchen_sink_opens_with_factory_text_and_no_transforms() {
    let v2 = std::fs::read(fixture("v2/kitchen_sink.freecut")).unwrap();
    assert_eq!(to_value(&v2)["format_version"], 2, "the fixture is frozen");
    let opened = decode(&v2).unwrap();
    assert_eq!(
        opened.warnings,
        vec![
            OpenWarning::Migrated {
                from: 2,
                to: CURRENT_FORMAT_VERSION
            },
            OpenWarning::UnknownFields { objects: 11 },
        ]
    );
    assert!(!opened.needs_save, "an upgrade alone is not a change");
    assert!(opened.backup_on_save(), "but the original is kept once");
    assert_eq!(opened.export, None, "format 2 saved no export settings");
    let doc = &opened.document;
    // The generated text clip became a factory-default text clip, keeping
    // its name; nothing else changed kind.
    let title = doc.clip(ClipId::from_raw(nz(43))).unwrap();
    assert_eq!(
        title.content,
        ClipContent::Text(std::sync::Arc::new(TextClipData::default()))
    );
    assert_eq!(title.name.as_deref(), Some("Title"));
    let clips: Vec<_> = doc
        .sequences()
        .flat_map(|s| s.tracks().map(|(_, t)| t))
        .flat_map(|t| t.clips())
        .collect();
    assert_eq!(clips.len(), 11);
    for clip in &clips {
        assert_eq!(clip.transform, Transform::IDENTITY, "clip {}", clip.id);
    }
    assert_eq!(
        clips.iter().filter(|c| c.content.text().is_some()).count(),
        1
    );
    // Everything else is what format 2 held: the kitchen sink of its time.
    let (sink, _) = kitchen_sink();
    for clip in clips.iter().filter(|c| c.content.text().is_none()) {
        let today = sink.clip(clip.id).unwrap();
        assert_eq!(clip.range, today.range);
        assert_eq!((clip.content.clone(), clip.audio.clone()), v3_sound(today));
    }
    check_expected(&opened, "v2/kitchen_sink.expected.freecut");
}

#[test]
fn the_frozen_v3_kitchen_sink_opens_with_the_sound_of_its_time() {
    let v3 = std::fs::read(fixture("v3/kitchen_sink.freecut")).unwrap();
    assert_eq!(to_value(&v3)["format_version"], 3, "the fixture is frozen");
    let opened = decode(&v3).unwrap();
    assert_eq!(
        opened.warnings,
        vec![
            OpenWarning::Migrated {
                from: 3,
                to: CURRENT_FORMAT_VERSION
            },
            OpenWarning::UnknownFields { objects: 20 },
        ]
    );
    assert!(!opened.needs_save, "an upgrade alone is not a change");
    assert!(opened.backup_on_save(), "but the original is kept once");
    assert_eq!(opened.export, Some(kitchen_export()));
    assert_eq!(opened.ui, freecut_project::UiState::default());
    let doc = &opened.document;
    let (sink, _) = kitchen_sink();
    // Clips are today's, with the sound format 3 could hold: the default
    // stream, static volume, linear fades.
    for seq in doc.sequences() {
        for (_, track) in seq.tracks() {
            assert_eq!(track.mix, TrackAudio::default(), "track {}", track.id);
            for clip in track.clips() {
                let today = sink.clip(clip.id).unwrap();
                assert_eq!(clip.range, today.range);
                assert_eq!(clip.transform, today.transform);
                assert_eq!(
                    (clip.content.clone(), clip.audio.clone()),
                    v3_sound(today),
                    "clip {}",
                    clip.id
                );
            }
        }
    }
    // Streams carry no probe tags; with none flagged, the first plays, as
    // in format 3.
    let beach = doc.library().get(MediaId::from_raw(nz(10))).unwrap();
    for stream in &beach.info.audio {
        assert!(!stream.default && stream.language.is_none() && stream.layout.is_none());
    }
    assert_eq!(beach.info.default_audio().unwrap().index, 4);
    check_expected(&opened, "v3/kitchen_sink.expected.freecut");
}

#[test]
fn the_frozen_v4_kitchen_sink_opens_with_the_text_of_its_time() {
    let v4 = std::fs::read(fixture("v4/kitchen_sink.freecut")).unwrap();
    assert_eq!(to_value(&v4)["format_version"], 4, "the fixture is frozen");
    let opened = decode(&v4).unwrap();
    assert_eq!(
        opened.warnings,
        vec![
            OpenWarning::Migrated {
                from: 4,
                to: CURRENT_FORMAT_VERSION
            },
            OpenWarning::UnknownFields { objects: 24 },
        ]
    );
    assert!(!opened.needs_save, "an upgrade alone is not a change");
    assert!(opened.backup_on_save(), "but the original is kept once");
    assert_eq!(opened.export, Some(kitchen_export()));
    assert_eq!(opened.ui, kitchen_ui());
    let doc = &opened.document;
    let (sink, _) = kitchen_sink();
    // Everything is today's kitchen sink, with the text format 4 could
    // hold: solid fills and no format-5 style.
    for seq in doc.sequences() {
        for (_, track) in seq.tracks() {
            let same = sink
                .sequences()
                .flat_map(|s| s.tracks().map(|(_, t)| t))
                .find(|t| t.id == track.id)
                .unwrap();
            assert_eq!(track.mix, same.mix);
            for clip in track.clips() {
                let today = sink.clip(clip.id).unwrap();
                assert_eq!(clip.range, today.range);
                assert_eq!(clip.transform, today.transform);
                assert_eq!(clip.audio, today.audio, "clip {}", clip.id);
                assert_eq!(clip.content, v4_text(&today.content), "clip {}", clip.id);
            }
        }
    }
    let title = doc.clip(ClipId::from_raw(nz(43))).unwrap();
    let data = title.content.text().unwrap();
    assert_eq!(data.style.fill, Fill::Solid(Rgba8::new(255, 128, 0, 192)));
    // Its unknown keys stay unknown: `x_outline` is not `outline`.
    let owner = Owner::Clip(ClipId::from_raw(nz(43)));
    assert!(
        opened
            .extras
            .get(owner, "content.style")
            .unwrap()
            .contains_key("x_outline")
    );
    assert!(
        opened
            .extras
            .get(owner, "content.layout")
            .unwrap()
            .contains_key("x_anchor")
    );
    check_expected(&opened, "v4/kitchen_sink.expected.freecut");
}

#[test]
fn the_frozen_v5_kitchen_sink_opens_without_keys_or_animations() {
    let v5 = std::fs::read(fixture("v5/kitchen_sink.freecut")).unwrap();
    assert_eq!(to_value(&v5)["format_version"], 5, "the fixture is frozen");
    let opened = decode(&v5).unwrap();
    assert_eq!(
        opened.warnings,
        vec![
            OpenWarning::Migrated {
                from: 5,
                to: CURRENT_FORMAT_VERSION
            },
            OpenWarning::UnknownFields { objects: 31 },
        ]
    );
    assert!(!opened.needs_save, "an upgrade alone is not a change");
    assert!(opened.backup_on_save(), "but the original is kept once");
    assert_eq!(opened.export, Some(kitchen_export()));
    assert_eq!(opened.ui, kitchen_ui());
    // Everything is today's kitchen sink without format 6's keys and
    // animations.
    let (sink, ids) = kitchen_sink();
    assert_eq!(opened.ids.next(), ids);
    let doc = &opened.document;
    let mut clips = 0;
    for seq in doc.sequences() {
        for (_, track) in seq.tracks() {
            for clip in track.clips() {
                let mut today = (**sink.clip(clip.id).unwrap()).clone();
                today.keys = ClipKeys::default();
                today.animations = ClipAnimations::default();
                today.markers = Markers::default();
                today.attach = None;
                today.color = None;
                assert_eq!(&**clip, &today, "clip {}", clip.id);
                clips += 1;
            }
        }
    }
    assert_eq!(
        clips,
        sink.sequences()
            .flat_map(|s| s.tracks())
            .map(|(_, t)| t.clips().len())
            .sum::<usize>()
    );
    check_expected(&opened, "v5/kitchen_sink.expected.freecut");
}

/// The kitchen sink as a file of today's format, as a JSON value.
fn kitchen_value() -> serde_json::Value {
    let (doc, ids) = kitchen_sink();
    to_value(&encode(&doc, ids, &Extras::new()).unwrap())
}

#[test]
fn the_frozen_v6_kitchen_sink_opens_without_markers_or_attachments() {
    let v6 = std::fs::read(fixture("v6/kitchen_sink.freecut")).unwrap();
    assert_eq!(to_value(&v6)["format_version"], 6, "the fixture is frozen");
    let opened = decode(&v6).unwrap();
    assert_eq!(
        opened.warnings,
        vec![
            OpenWarning::Migrated {
                from: 6,
                to: CURRENT_FORMAT_VERSION
            },
            OpenWarning::UnknownFields { objects: 37 },
        ]
    );
    assert!(!opened.needs_save, "an upgrade alone is not a change");
    assert!(opened.backup_on_save(), "but the original is kept once");
    assert_eq!(opened.export, Some(kitchen_export()));
    assert_eq!(opened.ui, kitchen_ui());
    let (sink, ids) = kitchen_sink();
    assert_eq!(opened.ids.next(), ids);
    let doc = &opened.document;
    // Every sequence follows the main track with every kind, as new ones
    // do, and has no markers.
    for seq in doc.sequences() {
        let today = sink.sequence(seq.id).unwrap();
        assert_eq!(
            seq.flags,
            SequenceFlags {
                main_magnet: today.flags.main_magnet,
                ..SequenceFlags::default()
            }
        );
        assert!(seq.markers.is_empty());
    }
    // Clips are today's without markers, attachments and colour.
    let plain = kitchen_sink_without_color();
    let mut clips = 0;
    for seq in doc.sequences() {
        for (_, track) in seq.tracks() {
            for clip in track.clips() {
                let mut today = (**plain.clip(clip.id).unwrap()).clone();
                today.markers = Markers::default();
                today.attach = None;
                assert_eq!(&**clip, &today, "clip {}", clip.id);
                clips += 1;
            }
        }
    }
    assert_eq!(clips, 12);
    // Format 6 knew only imported files.
    let items: Vec<_> = doc.library().items().collect();
    assert_eq!(items.len(), 4);
    for item in items {
        assert_eq!(item.origin, MediaOrigin::Imported);
        assert_eq!(**item, **sink.library().get(item.id).unwrap());
    }
    check_expected(&opened, "v6/kitchen_sink.expected.freecut");
}

#[test]
fn a_format_7_file_opens_following_every_kind_with_nothing_attached() {
    // Format 7 is today's kitchen sink without origins, follow flags and
    // attachments (no build wrote format 7 alone, so there is no frozen
    // file).
    let mut v = kitchen_value();
    strip_color(&mut v);
    v["format_version"] = 7.into();
    for item in v["document"]["library"].as_array_mut().unwrap() {
        item.as_object_mut().unwrap().remove("origin");
    }
    for seq in v["document"]["sequences"].as_array_mut().unwrap() {
        let flags = seq["flags"].as_object_mut().unwrap();
        flags.remove("follow_main");
        flags.remove("follow_kinds");
        for lane in ["visual", "audio"] {
            for track in seq[lane].as_array_mut().unwrap() {
                for clip in track["clips"].as_array_mut().unwrap() {
                    clip.as_object_mut().unwrap().remove("attach");
                }
            }
        }
    }
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::Migrated {
            from: 7,
            to: CURRENT_FORMAT_VERSION
        }]
    );
    let sink = kitchen_sink_without_color();
    for seq in opened.document.sequences() {
        let today = sink.sequence(seq.id).unwrap();
        assert!(seq.flags.follow_main);
        assert_eq!(
            seq.flags.follow_kinds,
            freecut_timeline::model::FollowKinds::ALL
        );
        assert_eq!(seq.flags.main_magnet, today.flags.main_magnet);
        // Markers came with format 7 and stay.
        assert_eq!(seq.markers, today.markers);
        for (_, track) in seq.tracks() {
            for clip in track.clips() {
                assert_eq!(clip.attach, None, "clip {}", clip.id);
                assert_eq!(clip.markers, sink.clip(clip.id).unwrap().markers);
            }
        }
    }
    for item in opened.document.library().items() {
        assert_eq!(item.origin, MediaOrigin::Imported);
    }
}

#[test]
fn a_format_8_file_opens_with_imported_items_only() {
    let mut v = kitchen_value();
    strip_color(&mut v);
    v["format_version"] = 8.into();
    for item in v["document"]["library"].as_array_mut().unwrap() {
        item.as_object_mut().unwrap().remove("origin");
    }
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::Migrated {
            from: 8,
            to: CURRENT_FORMAT_VERSION
        }]
    );
    let sink = kitchen_sink_without_color();
    // Everything but the origins is today's.
    let doc = &opened.document;
    assert!(doc.sequences().eq(sink.sequences()));
    for item in doc.library().items() {
        assert_eq!(item.origin, MediaOrigin::Imported);
        let mut today = (**sink.library().get(item.id).unwrap()).clone();
        today.origin = MediaOrigin::Imported;
        assert_eq!(**item, today);
    }
}

/// Today's kitchen sink without its colour (what formats before 10 hold).
fn kitchen_sink_without_color() -> freecut_timeline::model::Document {
    let mut v = kitchen_value();
    strip_color(&mut v);
    decode(&compact(&v)).unwrap().document
}

#[test]
fn a_format_9_file_opens_with_no_colour() {
    let mut v = kitchen_value();
    strip_color(&mut v);
    v["format_version"] = 9.into();
    let opened = decode(&compact(&v)).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::Migrated {
            from: 9,
            to: CURRENT_FORMAT_VERSION
        }]
    );
    let (sink, _) = kitchen_sink();
    let mut graded = 0;
    for seq in opened.document.sequences() {
        for (_, track) in seq.tracks() {
            for clip in track.clips() {
                assert_eq!(clip.color, None, "clip {}", clip.id);
                assert!(
                    !clip
                        .keys
                        .tracks()
                        .iter()
                        .any(|t| matches!(t.prop(), PropPath::Color(_)))
                );
                let mut today = (**sink.clip(clip.id).unwrap()).clone();
                graded += usize::from(today.color.is_some());
                today.color = None;
                if today
                    .keys
                    .tracks()
                    .iter()
                    .any(|t| matches!(t.prop(), PropPath::Color(_)))
                {
                    assert_eq!(clip.keys.tracks().len() + 1, today.keys.tracks().len());
                } else {
                    assert_eq!(&**clip, &today, "clip {}", clip.id);
                }
            }
        }
    }
    assert_eq!(graded, 3, "the kitchen sink grades three clips");
}

#[test]
fn the_frozen_v9_kitchen_sink_opens_without_colour() {
    let v9 = std::fs::read(fixture("v9/kitchen_sink.freecut")).unwrap();
    assert_eq!(to_value(&v9)["format_version"], 9, "the fixture is frozen");
    let opened = decode(&v9).unwrap();
    assert_eq!(
        opened.warnings[0],
        OpenWarning::Migrated {
            from: 9,
            to: CURRENT_FORMAT_VERSION
        }
    );
    assert!(!opened.needs_save, "an upgrade alone is not a change");
    assert!(opened.backup_on_save(), "but the original is kept once");
    let sink = kitchen_sink_without_color();
    for seq in opened.document.sequences() {
        assert_eq!(Some(seq), sink.sequence(seq.id), "sequence {}", seq.id);
    }
    check_expected(&opened, "v9/kitchen_sink.expected.freecut");
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    /// A format-9 file opens to the document today's editor makes from the
    /// same edits, and saving it again writes the current format.
    #[test]
    fn v9_files_open_to_what_the_editor_makes_today(plan in doc_plan_v9()) {
        let (doc, ids) = build(&plan);
        let current = encode(&doc, ids, &Extras::new()).unwrap();
        let v9 = to_v9(&to_value(&current));
        let opened = decode(&compact(&v9)).unwrap();
        prop_assert_eq!(opened.ids.next(), ids);
        prop_assert_eq!(
            &opened.warnings,
            &vec![OpenWarning::Migrated { from: 9, to: CURRENT_FORMAT_VERSION }]
        );
        prop_assert_eq!(&opened.document, &doc);
    }

    /// A format-8 file opens to the document today's editor makes from the
    /// same edits, and saving it again writes the current format.
    #[test]
    fn v8_files_open_to_what_the_editor_makes_today(plan in doc_plan_v8()) {
        let (doc, ids) = build(&plan);
        let current = encode(&doc, ids, &Extras::new()).unwrap();
        let v8 = to_v8(&to_value(&current));
        let opened = decode(&compact(&v8)).unwrap();
        prop_assert_eq!(opened.ids.next(), ids);
        prop_assert_eq!(
            &opened.warnings,
            &vec![OpenWarning::Migrated { from: 8, to: CURRENT_FORMAT_VERSION }]
        );
        prop_assert_eq!(&opened.document, &doc);
    }

    /// A format-7 file opens to the document today's editor makes from the
    /// same edits, without the attachments format 7 couldn't hold.
    #[test]
    fn v7_files_open_to_what_the_editor_makes_today(plan in doc_plan_v7()) {
        let (doc, ids) = build(&plan);
        let current = encode(&doc, ids, &Extras::new()).unwrap();
        let v7 = to_v7(&to_value(&current));
        let opened = decode(&compact(&v7)).unwrap();
        prop_assert_eq!(opened.ids.next(), ids);
        prop_assert_eq!(
            &opened.warnings,
            &vec![OpenWarning::Migrated { from: 7, to: CURRENT_FORMAT_VERSION }]
        );
        let again = encode(&opened.document, opened.ids.next(), &opened.extras).unwrap();
        prop_assert_eq!(older_view(&again), older_view(&current));
    }

    /// A format-6 file opens to the document today's editor makes from the
    /// same edits, without the attachments format 6 couldn't hold.
    #[test]
    fn v6_files_open_to_what_the_editor_makes_today(plan in doc_plan_v6()) {
        let (doc, ids) = build(&plan);
        let current = encode(&doc, ids, &Extras::new()).unwrap();
        let v6 = to_v6(&to_value(&current));
        let opened = decode(&compact(&v6)).unwrap();
        prop_assert_eq!(opened.ids.next(), ids);
        prop_assert_eq!(
            &opened.warnings,
            &vec![OpenWarning::Migrated { from: 6, to: CURRENT_FORMAT_VERSION }]
        );
        let again = encode(&opened.document, opened.ids.next(), &opened.extras).unwrap();
        prop_assert_eq!(older_view(&again), older_view(&current));
    }

    /// A format-5 file opens to the document today's editor makes from the
    /// same edits, and saving it again writes the current format.
    #[test]
    fn v5_files_open_to_what_the_editor_makes_today(plan in doc_plan_v5()) {
        let (doc, ids) = build(&plan);
        let current = encode(&doc, ids, &Extras::new()).unwrap();
        let v5 = to_v5(&to_value(&current));
        let opened = decode(&compact(&v5)).unwrap();
        prop_assert_eq!(opened.ids.next(), ids);
        prop_assert_eq!(
            &opened.warnings,
            &vec![OpenWarning::Migrated { from: 5, to: CURRENT_FORMAT_VERSION }]
        );
        // The same document, saved in the current format, except for the
        // content-time origins of split clips without keys, which format 5
        // and older couldn't hold, and the attachments today's editor made,
        // which format 7 and older couldn't ([`older_view`]).
        let again = encode(&opened.document, opened.ids.next(), &opened.extras).unwrap();
        prop_assert_eq!(older_view(&again), older_view(&current));
    }

    /// A format-4 file opens to the document today's editor makes from the
    /// same edits, and saving it again writes the current format.
    #[test]
    fn v4_files_open_to_what_the_editor_makes_today(plan in doc_plan_v4()) {
        let (doc, ids) = build(&plan);
        let current = encode(&doc, ids, &Extras::new()).unwrap();
        let v4 = to_v4(&to_value(&current));
        let opened = decode(&compact(&v4)).unwrap();
        prop_assert_eq!(opened.ids.next(), ids);
        prop_assert_eq!(
            &opened.warnings,
            &vec![OpenWarning::Migrated { from: 4, to: CURRENT_FORMAT_VERSION }]
        );
        // The same document, saved in the current format, except for the
        // content-time origins of split clips without keys, which format 5
        // and older couldn't hold, and the attachments today's editor made,
        // which format 7 and older couldn't ([`older_view`]).
        let again = encode(&opened.document, opened.ids.next(), &opened.extras).unwrap();
        prop_assert_eq!(older_view(&again), older_view(&current));
    }

    /// A format-3 file opens to the document today's editor makes from the
    /// same edits, and saving it again writes the current format.
    #[test]
    fn v3_files_open_to_what_the_editor_makes_today(plan in doc_plan_v3()) {
        let (doc, ids) = build(&plan);
        let current = encode(&doc, ids, &Extras::new()).unwrap();
        let v3 = to_v3(&to_value(&current));
        let opened = decode(&compact(&v3)).unwrap();
        prop_assert_eq!(opened.ids.next(), ids);
        prop_assert_eq!(
            &opened.warnings,
            &vec![OpenWarning::Migrated { from: 3, to: CURRENT_FORMAT_VERSION }]
        );
        // The same document, saved in the current format, except for the
        // content-time origins of split clips without keys, which format 5
        // and older couldn't hold, and the attachments today's editor made,
        // which format 7 and older couldn't ([`older_view`]).
        let again = encode(&opened.document, opened.ids.next(), &opened.extras).unwrap();
        prop_assert_eq!(older_view(&again), older_view(&current));
    }

    /// A format-2 file opens to the document today's editor makes from the
    /// same edits, and saving it again writes the current format.
    #[test]
    fn v2_files_open_to_what_the_editor_makes_today(plan in doc_plan_v2()) {
        let (doc, ids) = build(&plan);
        let current = encode(&doc, ids, &Extras::new()).unwrap();
        let v2 = to_v2(&to_value(&current));
        let opened = decode(&compact(&v2)).unwrap();
        prop_assert_eq!(opened.ids.next(), ids);
        prop_assert_eq!(
            &opened.warnings,
            &vec![OpenWarning::Migrated { from: 2, to: CURRENT_FORMAT_VERSION }]
        );
        // The same document, saved in the current format, except for the
        // content-time origins of split clips without keys, which format 5
        // and older couldn't hold, and the attachments today's editor made,
        // which format 7 and older couldn't ([`older_view`]).
        let again = encode(&opened.document, opened.ids.next(), &opened.extras).unwrap();
        prop_assert_eq!(older_view(&again), older_view(&current));
    }

    /// A format-1 file opens to the document today's editor makes from the
    /// same edits, and saving it again writes the current format.
    #[test]
    fn v1_files_open_to_what_the_editor_makes_today(plan in doc_plan_v1()) {
        let (doc, ids) = build(&plan);
        let current = encode(&doc, ids, &Extras::new()).unwrap();
        let v1 = to_v1(&to_value(&current));
        let opened = decode(&compact(&v1)).unwrap();
        prop_assert_eq!(opened.ids.next(), ids);
        prop_assert_eq!(
            &opened.warnings,
            &vec![OpenWarning::Migrated { from: 1, to: CURRENT_FORMAT_VERSION }]
        );
        // The same document, saved in the current format, except for the
        // content-time origins of split clips without keys, which format 5
        // and older couldn't hold, and the attachments today's editor made,
        // which format 7 and older couldn't ([`older_view`]).
        let again = encode(&opened.document, opened.ids.next(), &opened.extras).unwrap();
        prop_assert_eq!(older_view(&again), older_view(&current));
    }
}

#[test]
fn the_first_save_after_migration_keeps_the_original_as_a_versioned_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.freecut");
    let v1 = std::fs::read(fixture("v1/kitchen_sink.freecut")).unwrap();
    std::fs::write(&path, &v1).unwrap();
    // A copy kept by an earlier repair already holds the `.bak` slot.
    let (sink, sink_ids) = kitchen_sink();
    let repaired = encode(&sink, sink_ids, &Extras::new()).unwrap();
    std::fs::write(dir.path().join("p.freecut.bak"), &repaired).unwrap();

    let opened = freecut_project::open(&path).unwrap();
    assert!(opened.backup_on_save());
    let request = freecut_project::SaveRequest {
        document: &opened.document,
        ids_next: opened.ids.next(),
        extras: &opened.extras,
        backup: opened.backup_on_save(),
        relative_base: None,
        export: None,
        ui: None,
    };
    freecut_project::save(&path, &request).unwrap();
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["p.freecut", "p.freecut.bak", "p.freecut.v1.bak"]);
    assert_eq!(
        std::fs::read(dir.path().join("p.freecut.v1.bak")).unwrap(),
        v1
    );
    assert_eq!(
        std::fs::read(dir.path().join("p.freecut.bak")).unwrap(),
        repaired
    );
    // One-time: a second save keeps the first copy.
    freecut_project::save(&path, &request).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("p.freecut.v1.bak")).unwrap(),
        v1
    );
}

#[test]
fn the_first_save_after_a_v2_upgrade_keeps_a_v2_backup_and_the_export_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.freecut");
    let v2 = std::fs::read(fixture("v2/kitchen_sink.freecut")).unwrap();
    std::fs::write(&path, &v2).unwrap();

    let opened = freecut_project::open(&path).unwrap();
    assert!(opened.backup_on_save());
    // An export started: its settings are saved with the project.
    let export = kitchen_export();
    let request = freecut_project::SaveRequest {
        document: &opened.document,
        ids_next: opened.ids.next(),
        extras: &opened.extras,
        backup: opened.backup_on_save(),
        relative_base: None,
        export: Some(&export),
        ui: None,
    };
    freecut_project::save(&path, &request).unwrap();
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["p.freecut", "p.freecut.v2.bak"]);
    assert_eq!(
        std::fs::read(dir.path().join("p.freecut.v2.bak")).unwrap(),
        v2
    );
    let reopened = freecut_project::open(&path).unwrap();
    assert_eq!(reopened.export, Some(export));
    assert!(!reopened.backup_on_save());
    assert_eq!(reopened.document, opened.document);
}
