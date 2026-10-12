//! The open pipeline refuses damaged, foreign, newer and invalid files with
//! precise errors, and repairs what it safely can.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use core::num::NonZeroU64;
use std::sync::Arc;

use common::*;
use freecut_project::migrate::MigrationError;
use freecut_project::{
    CURRENT_FORMAT_VERSION, Extras, OpenError, OpenWarning, decode, encode, open,
};
use freecut_timeline::ids::{ClipId, MediaId, SeqId, TrackId};
use freecut_timeline::model::{
    Angle, CanvasPos, Clip, ClipContent, Document, InvariantViolation, Library, MediaInfo,
    MediaItem, MediaSource, PerMille, ProjectSettings, Scale, Sequence, SequenceFlags, StoredPath,
    TextClipData, TextLayout, TextStyle, Track, TrackHeader, TrackKind,
};
use freecut_timeline::time::{Tick, TimeRange};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn sink_value() -> Value {
    let (doc, ids) = kitchen_sink();
    to_value(&encode(&doc, ids, &Extras::new()).unwrap())
}

fn open_value(v: &Value) -> Result<freecut_project::Opened, OpenError> {
    decode(&compact(v))
}

fn json_error(result: Result<freecut_project::Opened, OpenError>) -> (String, String) {
    match result {
        Err(OpenError::Json { path, message }) => (path, message),
        other => panic!("expected a JSON error, got {other:?}"),
    }
}

fn bad_value(result: Result<freecut_project::Opened, OpenError>) -> (String, String) {
    match result {
        Err(OpenError::BadValue { path, message }) => (path, message),
        other => panic!("expected a bad value, got {other:?}"),
    }
}

// ---- damaged and foreign files ----

#[test]
fn a_truncated_file_names_the_json_path_where_it_broke() {
    let (doc, ids) = kitchen_sink();
    let bytes = encode(&doc, ids, &Extras::new()).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    // Cut inside the title clip on the text track.
    let cut = text.find("\"name\": \"Title\"").unwrap();
    let (path, message) = json_error(decode(&bytes[..cut]));
    assert!(
        path.starts_with("document.sequences[0].visual[2].clips[0]"),
        "path {path:?}"
    );
    assert!(message.contains("EOF"), "{message}");
}

#[test]
fn every_truncation_is_an_error_never_a_panic() {
    let (doc, ids) = kitchen_sink();
    let bytes = encode(&doc, ids, &Extras::new()).unwrap();
    for len in (0..bytes.len() - 2).step_by(3) {
        assert!(decode(&bytes[..len]).is_err(), "truncated at {len}");
    }
    // Only the trailing newline is optional.
    assert!(decode(&bytes[..bytes.len() - 1]).is_ok());
}

#[test]
fn a_type_error_names_the_value() {
    let mut v = sink_value();
    v["document"]["sequences"][0]["visual"][0]["clips"][1]["range"]["start"] = json!("soon");
    let (path, message) = json_error(open_value(&v));
    assert_eq!(path, "document.sequences[0].visual[0].clips[1].range.start");
    assert!(message.contains("expected i64"), "{message}");

    let mut v = sink_value();
    v["document"]["sequences"][0]["visual"][1]["kind"] = json!("hologram");
    let (path, message) = json_error(open_value(&v));
    assert_eq!(path, "document.sequences[0].visual[1].kind");
    assert!(message.contains("hologram"), "{message}");

    let mut v = sink_value();
    v.as_object_mut().unwrap().remove("ids_next");
    let (path, message) = json_error(open_value(&v));
    assert_eq!(path, ".");
    assert!(message.contains("ids_next"), "{message}");

    let mut v = sink_value();
    v["document"]["library"][0]["id"] = json!(0);
    let (path, _) = json_error(open_value(&v));
    assert_eq!(path, "document.library[0].id");
}

#[test]
fn trailing_garbage_is_an_error() {
    let (doc, ids) = kitchen_sink();
    let mut bytes = encode(&doc, ids, &Extras::new()).unwrap();
    bytes.extend_from_slice(b"{}");
    let (path, message) = json_error(decode(&bytes));
    assert_eq!(path, ".");
    assert!(message.contains("trailing"), "{message}");
}

#[test]
fn foreign_files_are_not_projects() {
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    for bytes in [
        &png[..],
        b"[1, 2]",
        b"<mlt version=\"7\"/>",
        br#"{"OTIO_SCHEMA": "Timeline.1", "tracks": {}}"#,
        br#"{"format": "something-else", "format_version": 1}"#,
        br#"{"format": 1}"#,
    ] {
        assert!(
            matches!(decode(bytes), Err(OpenError::NotAProject)),
            "{:?}",
            String::from_utf8_lossy(bytes)
        );
    }
}

#[test]
fn blank_files_are_damaged_projects_not_foreign_files() {
    // What a lost write leaves: nothing, blanks or zero bytes.
    for bytes in [
        &b""[..],
        b"   \n",
        &[0u8; 64][..],
        b"\0\0\n\0",
        b"\xEF\xBB\xBF",
        b"\xEF\xBB\xBF\0\0",
    ] {
        assert!(
            matches!(decode(bytes), Err(OpenError::Empty)),
            "{bytes:?}: {:?}",
            decode(bytes)
        );
    }
    // Zeros where the start of the file should be: damaged, at the start.
    let (doc, ids) = kitchen_sink();
    let mut bytes = vec![0u8; 16];
    bytes.extend(encode(&doc, ids, &Extras::new()).unwrap());
    assert_eq!(json_error(decode(&bytes)).0, ".");
    assert_eq!(
        OpenError::Empty.to_string(),
        "the project file is damaged: it is empty"
    );
}

#[test]
fn a_missing_file_is_an_io_error_with_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nope.freecut");
    match open(&path) {
        Err(OpenError::Io { path: p, source }) => {
            assert_eq!(p, path);
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_oversized_file_is_refused_before_reading_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.freecut");
    // Sparse: no data is actually written.
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(freecut_project::MAX_FILE_BYTES + 1).unwrap();
    drop(file);
    assert!(matches!(
        open(&path),
        Err(OpenError::TooLarge { len, limit })
            if len == limit + 1 && limit == freecut_project::MAX_FILE_BYTES
    ));
}

// ---- versions ----

#[test]
fn a_newer_format_is_refused_with_both_versions() {
    let mut v = sink_value();
    let newer = CURRENT_FORMAT_VERSION + 1;
    v["format_version"] = json!(newer);
    // A newer file may look completely different; the version check comes
    // before any typed decoding.
    v["document"] = json!({"totally": "new"});
    v["app"] = json!("freecut 9.0.0");
    match open_value(&v) {
        Err(OpenError::NewerVersion {
            found,
            supported,
            written_by,
        }) => {
            assert_eq!((found, supported), (newer, CURRENT_FORMAT_VERSION));
            assert_eq!(written_by.as_deref(), Some("freecut 9.0.0"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn bad_format_versions() {
    let mut v = sink_value();
    v["format_version"] = json!(0);
    assert!(matches!(
        open_value(&v),
        Err(OpenError::Migration(MigrationError::TooOld {
            found: 0,
            oldest: 1
        }))
    ));
    v["format_version"] = json!("1");
    assert_eq!(json_error(open_value(&v)).0, "format_version");
    v["format_version"] = json!(1.5);
    assert_eq!(json_error(open_value(&v)).0, "format_version");
    v["format_version"] = json!(u64::from(u32::MAX) + 1);
    assert_eq!(json_error(open_value(&v)).0, "format_version");
}

// ---- refused values ----

#[test]
fn an_off_grid_frame_rate_is_refused_never_rounded() {
    let mut v = sink_value();
    v["document"]["settings"]["frame_rate"] = json!({"num": 11, "den": 1});
    let (path, message) = bad_value(open_value(&v));
    assert_eq!(path, "document.settings.frame_rate");
    assert!(message.contains("11/1"), "{message}");

    v["document"]["settings"]["frame_rate"] = json!({"num": 30, "den": 0});
    assert_eq!(bad_value(open_value(&v)).0, "document.settings.frame_rate");

    v["document"]["settings"]["sample_rate"] = json!(384_000);
    v["document"]["settings"]["frame_rate"] = json!({"num": 25, "den": 1});
    assert_eq!(bad_value(open_value(&v)).0, "document.settings.sample_rate");
}

#[test]
fn an_unreduced_rate_is_accepted_and_written_reduced() {
    let mut v = sink_value();
    v["document"]["settings"]["frame_rate"] = json!({"num": 60_000, "den": 2002});
    let opened = open_value(&v).unwrap();
    assert_eq!(opened.document.settings().frame_rate, ntsc());
    let again = to_value(&encode(&opened.document, opened.ids.next(), &opened.extras).unwrap());
    assert_eq!(
        again["document"]["settings"]["frame_rate"],
        json!({"num": 30_000, "den": 1001})
    );
}

#[test]
fn impossible_values_are_refused_with_their_path() {
    let clip = "/document/sequences/0/visual/0/clips/0";
    let cases: &[(&str, Value, &str)] = &[
        (
            "/range/duration",
            json!(0),
            "document.sequences[0].visual[0].clips[0].range",
        ),
        (
            "/range/duration",
            json!(-5),
            "document.sequences[0].visual[0].clips[0].range",
        ),
        (
            "/range/start",
            json!(i64::MAX),
            "document.sequences[0].visual[0].clips[0].range.start",
        ),
        (
            "/range/duration",
            json!((1i64 << 53) + 1),
            "document.sequences[0].visual[0].clips[0].range.duration",
        ),
        (
            "/content/source/start",
            json!(-(1i64 << 53) - 1),
            "document.sequences[0].visual[0].clips[0].content.source.start",
        ),
        (
            "/content/source/duration",
            json!(0),
            "document.sequences[0].visual[0].clips[0].content.source",
        ),
        (
            "/group",
            json!(1u64 << 60),
            "document.sequences[0].visual[0].clips[0].group",
        ),
    ];
    for (field, value, expected) in cases {
        let mut v = sink_value();
        *v.pointer_mut(&format!("{clip}{field}")).unwrap() = value.clone();
        assert_eq!(bad_value(open_value(&v)).0, *expected, "{field} = {value}");
    }

    let mut v = sink_value();
    v["document"]["library"][0]["fingerprint"]["head_tail"] = json!("12345");
    assert_eq!(
        bad_value(open_value(&v)).0,
        "document.library[0].fingerprint.head_tail"
    );
    let mut v = sink_value();
    v["document"]["library"][0]["info"]["video"][0]["rate"] = json!({"num": 1, "den": 0});
    assert_eq!(
        bad_value(open_value(&v)).0,
        "document.library[0].info.video[0].rate"
    );
}

// ---- invariants ----

fn invalid(result: Result<freecut_project::Opened, OpenError>) -> Vec<InvariantViolation> {
    match result {
        Err(OpenError::Invalid(v)) => v,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn documents_that_break_invariants_are_refused() {
    // A clip id used twice.
    let mut v = sink_value();
    v["document"]["sequences"][1]["visual"][0]["clips"][0]["id"] = json!(40);
    assert!(invalid(open_value(&v)).contains(&InvariantViolation::DuplicateId(40)));

    // A clip playing media that isn't in the library.
    let mut v = sink_value();
    v["document"]["sequences"][1]["visual"][0]["clips"][0]["content"]["asset"] = json!(77);
    assert!(
        invalid(open_value(&v))
            .iter()
            .any(|e| matches!(e, InvariantViolation::MissingMedia { .. }))
    );

    // Two media items with one id.
    let mut v = sink_value();
    v["document"]["library"][1]["id"] = json!(10);
    assert!(invalid(open_value(&v)).contains(&InvariantViolation::DuplicateId(10)));

    // A missing root sequence.
    let mut v = sink_value();
    v["document"]["root"] = json!(3);
    assert!(
        invalid(open_value(&v))
            .iter()
            .any(|e| matches!(e, InvariantViolation::RootMissing(_)))
    );

    // An odd canvas.
    let mut v = sink_value();
    v["document"]["settings"]["canvas"]["width"] = json!(1081);
    assert!(
        invalid(open_value(&v))
            .iter()
            .any(|e| matches!(e, InvariantViolation::InvalidCanvas { .. }))
    );
}

// ---- repairs ----

fn id64(n: u64) -> NonZeroU64 {
    nz(n)
}

/// Root sequence 1 with main track 2 (one media clip) and a text track 3,
/// media 4; ids_next 10.
fn small() -> (Document, NonZeroU64) {
    let settings = ProjectSettings::new_default().unwrap();
    let rate = settings.frame_rate;
    let len = rate.frames(30).unwrap();
    let media = MediaItem {
        id: MediaId::from_raw(id64(4)),
        source: MediaSource::File,
        origin: Default::default(),
        path: StoredPath {
            absolute: "/m/a.mp4".into(),
            relative: None,
        },
        info: MediaInfo {
            container: "mp4".into(),
            default_video: None,
            duration: len,
            video: Vec::new(),
            audio: Vec::new(),
            still: None,
        },
        fingerprint: None,
        display_name: None,
    };
    let clip = Clip::new(
        ClipId::from_raw(id64(5)),
        TimeRange::new(Tick::ZERO, len).unwrap(),
        ClipContent::Image {
            asset: MediaId::from_raw(id64(4)),
        },
    );
    let title = Clip::new(
        ClipId::from_raw(id64(6)),
        TimeRange::new(Tick::ZERO, len).unwrap(),
        ClipContent::Text(Arc::new(TextClipData::default())),
    );
    let main = Track::from_parts(
        TrackId::from_raw(id64(2)),
        TrackKind::Video,
        TrackHeader::default(),
        vec![Arc::new(clip)],
    );
    let text = Track::from_parts(
        TrackId::from_raw(id64(3)),
        TrackKind::Text,
        TrackHeader::default(),
        vec![Arc::new(title)],
    );
    let seq = Sequence::from_parts(
        SeqId::from_raw(id64(1)),
        SequenceFlags::default(),
        vec![Arc::new(main), Arc::new(text)],
        Vec::new(),
    );
    let doc = Document::from_parts(
        SeqId::from_raw(id64(1)),
        settings,
        Library::from_parts(vec![Arc::new(media)]).unwrap(),
        vec![Arc::new(seq)],
    )
    .unwrap();
    (doc, id64(10))
}

fn small_value() -> Value {
    let (doc, ids) = small();
    to_value(&encode(&doc, ids, &Extras::new()).unwrap())
}

#[test]
fn a_main_track_out_of_place_is_promoted() {
    let mut v = small_value();
    v["document"]["sequences"][0]["visual"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::MainTrackPromoted {
            seq: SeqId::from_raw(id64(1)),
            track: TrackId::from_raw(id64(2)),
            from_index: 1,
        }]
    );
    assert!(opened.needs_save);
    assert_eq!(opened.document, small().0, "the repair restores the order");
    assert_eq!(opened.ids.next(), id64(10));
}

#[test]
fn a_missing_main_track_is_created_with_a_fresh_id() {
    let mut v = small_value();
    // Drop the main track (and with it the only media clip).
    v["document"]["sequences"][0]["visual"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    let opened = open_value(&v).unwrap();
    let seq = opened.document.sequence(opened.document.root()).unwrap();
    let main = seq.main_track().unwrap();
    assert_eq!(main.kind, TrackKind::Video);
    assert!(main.is_empty());
    assert_eq!(main.id.get(), 10, "the first free id");
    assert_eq!(opened.ids.next(), id64(11), "and the counter moved past it");
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::MainTrackCreated {
            seq: SeqId::from_raw(id64(1)),
            track: main.id,
        }]
    );
    assert!(opened.needs_save);
    assert_eq!(seq.visual_tracks()[1].id.get(), 3);

    // An empty visual lane gets one too.
    let mut v = small_value();
    v["document"]["sequences"][0]["visual"] = json!([]);
    let opened = open_value(&v).unwrap();
    let seq = opened.document.sequence(opened.document.root()).unwrap();
    assert_eq!(seq.visual_tracks().len(), 1);
}

#[test]
fn a_low_id_counter_is_raised_above_every_id() {
    let mut v = small_value();
    v["ids_next"] = json!(3);
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::IdsNextRaised {
            stored: 3,
            raised_to: 7
        }]
    );
    assert_eq!(opened.ids.next(), id64(7));
    assert!(opened.needs_save);

    // Group and link ids count too, and so does the link a detached video
    // keeps after its audio clips are gone (clip 49's 92).
    let mut v = sink_value();
    v["ids_next"] = json!(50);
    let opened = open_value(&v).unwrap();
    assert_eq!(opened.ids.next(), id64(93));
}

#[test]
fn a_raised_counter_is_used_for_the_repair_track() {
    let mut v = small_value();
    v["ids_next"] = json!(1);
    v["document"]["sequences"][0]["visual"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    let opened = open_value(&v).unwrap();
    let seq = opened.document.sequence(opened.document.root()).unwrap();
    // Remaining ids: 1 (seq), 3 (text track), 4 (media), 6 (title) → 7.
    assert_eq!(seq.main_track().unwrap().id.get(), 7);
    assert_eq!(opened.ids.next(), id64(8));
}

// ---- review regressions ----

/// F1: a lost write (an empty, blank or zero-filled file) is a damaged
/// project, not a foreign file.
#[test]
fn f1_empty_or_zeroed_project_file_is_damaged_not_foreign() {
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in [
        ("empty.freecut", vec![]),
        ("zeros.freecut", vec![0u8; 4096]),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, &bytes).unwrap();
        let result = open(&path);
        assert!(
            !matches!(result, Err(OpenError::NotAProject)),
            "{name}: {result:?}"
        );
    }
}

/// F3: an id counter beyond the id space is refused, not opened into an
/// editor that can't allocate.
#[test]
fn f3_ids_next_above_the_id_space_is_refused() {
    use freecut_timeline::Editor;
    use freecut_timeline::edit::{AddTrackReq, EditCtx, EditRequest, TrackPosition};
    use freecut_timeline::history::HistoryLimits;

    let mut v = sink_value();
    v["ids_next"] = json!(1u64 << 60);
    let result = decode(&compact(&v));
    if let Ok(opened) = &result {
        let editor = Editor::open(
            opened.document.clone(),
            opened.ids.clone(),
            HistoryLimits::default(),
        )
        .unwrap();
        let seq = editor.document().root();
        let add = editor.plan(
            &EditCtx::default(),
            &EditRequest::AddTrack(AddTrackReq {
                seq,
                kind: TrackKind::Text,
                position: TrackPosition::Default,
            }),
        );
        panic!(
            "ids_next {} accepted; AddTrack -> {:?}",
            opened.ids.next(),
            add.err()
        );
    }
}

/// F5: type errors inside a clip's content name the value, not just the
/// content object.
#[test]
fn f5_type_errors_inside_content_name_the_value() {
    let mut v = sink_value();
    v["document"]["sequences"][0]["visual"][0]["clips"][0]["content"]["source"]["start"] =
        json!("soon");
    match decode(&compact(&v)) {
        Err(OpenError::Json { path, .. }) => assert_eq!(
            path,
            "document.sequences[0].visual[0].clips[0].content.source.start"
        ),
        other => panic!("{other:?}"),
    }
}

/// F6: promoting the main track keeps the video band together.
#[test]
fn f6_main_track_repair_keeps_the_video_band_together() {
    let mut v = sink_value();
    let visual = v["document"]["sequences"][0]["visual"]
        .as_array_mut()
        .unwrap();
    let text = visual.remove(2);
    visual.insert(0, text);
    match decode(&compact(&v)) {
        Ok(opened) => assert_eq!(opened.document, kitchen_sink().0),
        Err(e) => panic!("{e:?}"),
    }
}

/// F10: ticks beyond ±2^53 can't be read exactly by every JSON tool and are
/// refused.
#[test]
fn f10_ticks_beyond_2_pow_53_are_refused_on_open() {
    let mut v = sink_value();
    v["document"]["sequences"][0]["visual"][1]["clips"][0]["range"]["start"] = json!(1i64 << 60);
    assert!(decode(&compact(&v)).is_err());
}

/// F5: every schema error inside a clip's content names its own path.
#[test]
fn content_errors_name_their_own_path() {
    let clip = "/document/sequences/0/visual/0/clips/0/content";
    let image = "/document/sequences/0/visual/0/clips/1/content";
    let at = "document.sequences[0].visual[0].clips[0].content";
    let image_at = "document.sequences[0].visual[0].clips[1].content";
    let cases: &[(&str, Value, String, &str)] = &[
        (
            clip,
            json!("media"),
            at.into(),
            "expected a clip content object",
        ),
        (
            &format!("{clip}/type"),
            json!("hologram"),
            format!("{at}.type"),
            "unknown variant `hologram`",
        ),
        (
            &format!("{clip}/type"),
            json!(3),
            format!("{at}.type"),
            "invalid type",
        ),
        (
            &format!("{clip}/kind"),
            json!("smell"),
            format!("{at}.kind"),
            "smell",
        ),
        (
            &format!("{clip}/source/duration"),
            json!(1.5),
            format!("{at}.source.duration"),
            // serde_json's arbitrary-precision numbers report "invalid number".
            "invalid",
        ),
        (
            &format!("{clip}/asset"),
            json!(0),
            format!("{at}.asset"),
            "nonzero",
        ),
        (
            &format!("{image}/asset"),
            json!(null),
            format!("{image_at}.asset"),
            "invalid type: null",
        ),
    ];
    for (pointer, value, expected, message_part) in cases {
        let mut v = sink_value();
        *v.pointer_mut(pointer).unwrap() = value.clone();
        let (path, message) = json_error(open_value(&v));
        assert_eq!(&path, expected, "{pointer} = {value}");
        assert!(message.contains(message_part), "{pointer}: {message}");
    }

    // A missing tag or field is reported at the content object.
    let mut v = sink_value();
    v.pointer_mut(clip)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("type");
    let (path, message) = json_error(open_value(&v));
    assert_eq!(
        (path.as_str(), message.as_str()),
        (at, "missing field `type`")
    );
    let mut v = sink_value();
    v.pointer_mut(clip)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("source");
    let (path, message) = json_error(open_value(&v));
    assert_eq!(path, at);
    assert!(message.contains("missing field `source`"), "{message}");
}

/// F3, F10: the bounds hold exactly at their limits, on both sides.
#[test]
fn id_counter_and_tick_bounds_are_inclusive() {
    use freecut_timeline::ids::MAX_ID;

    // An exhausted allocator's position is the largest counter allowed.
    let mut v = sink_value();
    v["ids_next"] = json!(MAX_ID + 1);
    let opened = open_value(&v).unwrap();
    assert_eq!(opened.ids.next().get(), MAX_ID + 1);
    assert!(encode(&opened.document, opened.ids.next(), &opened.extras).is_ok());
    v["ids_next"] = json!(MAX_ID + 2);
    assert_eq!(bad_value(open_value(&v)).0, "ids_next");

    // Ticks reach ±2^53 and no further.
    let mut v = sink_value();
    v["document"]["library"][1]["info"]["duration"] = json!(1i64 << 53);
    assert!(open_value(&v).is_ok());
    for (pointer, value) in [
        ("/document/library/1/info/duration", json!((1i64 << 53) + 1)),
        (
            "/document/library/0/info/video/1/start",
            json!(-(1i64 << 53) - 1),
        ),
        ("/document/library/0/info/audio/0/duration", json!(i64::MIN)),
        ("/document/library/0/fingerprint/duration", json!(i64::MAX)),
    ] {
        let mut v = sink_value();
        *v.pointer_mut(pointer).unwrap() = value;
        let expected =
            pointer
                .trim_start_matches('/')
                .split('/')
                .fold(String::new(), |path, part| match part.parse::<usize>() {
                    Ok(i) => format!("{path}[{i}]"),
                    Err(_) if path.is_empty() => part.to_string(),
                    Err(_) => format!("{path}.{part}"),
                });
        assert_eq!(bad_value(open_value(&v)).0, expected);
    }
}

// ---- text, transforms and export settings (format 3) ----

/// The kitchen sink with its export settings, as JSON.
fn sink_with_export() -> Value {
    let (doc, ids) = kitchen_sink();
    let export = kitchen_export();
    to_value(
        &freecut_project::encode_with_export(&doc, ids, Some(&export), &Extras::new()).unwrap(),
    )
}

const TITLE: &str = "/document/sequences/0/visual/2/clips/0";
const OVERLAY: &str = "/document/sequences/0/visual/1/clips/0";

#[test]
fn out_of_range_transform_and_text_values_are_clamped_as_a_repair() {
    let mut v = sink_value();
    let title = v.pointer_mut(TITLE).unwrap();
    title["content"]["style"]["size"] = json!(-5);
    title["content"]["layout"]["line_spacing"] = json!(60_000);
    title["transform"]["rotation"] = json!(9_000_000);
    let overlay = v.pointer_mut(OVERLAY).unwrap();
    overlay["transform"]["scale"] = json!(0);
    overlay["transform"]["opacity"] = json!(4000);
    overlay["transform"]["position"]["x"] = json!(i32::MIN);
    // An audio clip may carry a transform too; the identity is fine.
    let opened = open_value(&v).unwrap();
    assert_eq!(
        opened.warnings,
        vec![OpenWarning::ValuesClamped { clips: 2 }]
    );
    assert!(opened.needs_save, "a clamp is a repair");
    assert!(opened.backup_on_save());
    let doc = &opened.document;
    let title = doc.clip(ClipId::from_raw(id64(43))).unwrap();
    let data = title.content.text().unwrap();
    assert_eq!(data.style.size, TextStyle::MIN_SIZE);
    assert_eq!(data.layout.line_spacing, TextLayout::MAX_LINE_SPACING);
    assert_eq!(title.transform.rotation, Angle::LIMIT);
    let overlay = doc.clip(ClipId::from_raw(id64(42))).unwrap();
    assert_eq!(overlay.transform.scale, Scale::MIN);
    assert_eq!(overlay.transform.opacity, PerMille::ONE);
    assert_eq!(overlay.transform.position.x, -CanvasPos::LIMIT);
    // The repaired document saves and opens without warnings.
    let bytes = encode(doc, opened.ids.next(), &opened.extras).unwrap();
    assert!(decode(&bytes).unwrap().warnings.is_empty());
}

#[test]
fn values_beyond_the_models_integer_width_are_clamped_too() {
    let cases: Vec<(&str, Value)> = vec![
        ("/content/layout/line_spacing", json!(-1)),
        ("/content/layout/line_spacing", json!(70_000)),
        ("/content/layout/box_width", json!(-10_000_000_000_i64)),
        ("/content/style/size", json!(3_000_000_000_i64)),
        ("/transform/opacity", json!(-1)),
        ("/transform/opacity", json!(70_000)),
        ("/transform/scale", json!(5_000_000_000_i64)),
        ("/transform/rotation", json!(i64::MIN)),
        ("/transform/position/y", json!(1_000_000_000_000_i64)),
    ];
    for (pointer, value) in cases {
        let mut v = sink_value();
        *v.pointer_mut(&format!("{TITLE}{pointer}")).unwrap() = value.clone();
        let opened = open_value(&v).unwrap_or_else(|e| panic!("{pointer} = {value}: {e}"));
        assert_eq!(
            opened.warnings,
            vec![OpenWarning::ValuesClamped { clips: 1 }],
            "{pointer} = {value}"
        );
        let title = opened.document.clip(ClipId::from_raw(id64(43))).unwrap();
        let data = title.content.text().unwrap();
        let t = title.transform;
        let got = match pointer {
            "/content/layout/line_spacing" => i64::from(data.layout.line_spacing.get()),
            "/content/layout/box_width" => i64::from(data.layout.box_width.unwrap().hundredths()),
            "/content/style/size" => i64::from(data.style.size.hundredths()),
            "/transform/opacity" => i64::from(t.opacity.get()),
            "/transform/scale" => i64::from(t.scale.basis_points()),
            "/transform/rotation" => i64::from(t.rotation.hundredths()),
            _ => i64::from(t.position.y),
        };
        let expected = match (pointer, value.as_i64().unwrap() < 0) {
            ("/content/layout/line_spacing", true) => TextLayout::MIN_LINE_SPACING.get().into(),
            ("/content/layout/line_spacing", false) => TextLayout::MAX_LINE_SPACING.get().into(),
            ("/content/layout/box_width", _) => TextLayout::MIN_BOX_WIDTH.hundredths().into(),
            ("/content/style/size", _) => TextStyle::MAX_SIZE.hundredths().into(),
            ("/transform/opacity", true) => 0,
            ("/transform/opacity", false) => PerMille::ONE.get().into(),
            ("/transform/scale", _) => Scale::MAX.basis_points().into(),
            ("/transform/rotation", _) => (-Angle::LIMIT.hundredths()).into(),
            _ => CanvasPos::LIMIT.into(),
        };
        assert_eq!(got, expected, "{pointer} = {value}");
    }
}

#[test]
fn malformed_text_and_transform_values_are_refused_with_their_path() {
    let title = "document.sequences[0].visual[2].clips[0]";
    for (fill, ok) in [
        ("#ff8000c0", true),
        ("#FF8000", false),
        ("FF8000C0", false),
        ("#FF8000CG", false),
        ("#FF8000C0FF", false),
        ("red", false),
    ] {
        let mut v = sink_value();
        v.pointer_mut(TITLE).unwrap()["content"]["style"]["fill"] =
            json!({"type": "solid", "color": fill});
        if ok {
            assert!(open_value(&v).is_ok(), "{fill}");
        } else {
            let (path, message) = bad_value(open_value(&v));
            assert_eq!(path, format!("{title}.content.style.fill.color"), "{fill}");
            assert!(message.contains("#RRGGBBAA"), "{message}");
        }
    }
    // A format-4 colour string in a format-5 file is not a fill.
    let mut v = sink_value();
    v.pointer_mut(TITLE).unwrap()["content"]["style"]["fill"] = json!("#FF8000C0");
    let (path, message) = json_error(open_value(&v));
    assert_eq!(path, format!("{title}.content.style.fill"));
    assert!(message.contains("expected a fill object"), "{message}");

    let cases: Vec<(&str, Value, &str)> = vec![
        ("/content/origin", json!("template"), "content.origin"),
        (
            "/content/layout/align",
            json!("justify"),
            "content.layout.align",
        ),
        ("/content/style/size", json!(1.5), "content.style.size"),
        ("/content/text", json!(null), "content.text"),
        ("/transform/opacity", json!(0.5), "transform.opacity"),
        ("/transform/scale", json!(u64::MAX), "transform.scale"),
        ("/transform/position/y", json!("0"), "transform.position.y"),
    ];
    for (pointer, value, path) in cases {
        let mut v = sink_value();
        *v.pointer_mut(&format!("{TITLE}{pointer}")).unwrap() = value;
        let (got, _) = json_error(open_value(&v));
        assert_eq!(got, format!("{title}.{path}"), "{pointer}");
    }
    // Text written the format-2 way is an unknown generated kind now.
    let mut v = sink_value();
    v.pointer_mut(TITLE).unwrap()["content"] = json!({"type": "generated", "kind": "text"});
    let (path, message) = json_error(open_value(&v));
    assert_eq!(path, format!("{title}.content.kind"));
    assert!(message.contains("unknown variant `text`"), "{message}");
}

#[test]
fn export_settings_values_are_checked_with_their_path() {
    assert!(open_value(&sink_with_export()).is_ok());
    let bad: Vec<(&str, Value, &str)> = vec![
        (
            "/export/video/size/short_side",
            json!(1000),
            "export.video.size.short_side",
        ),
        (
            "/export/video/frame_rate/rate/num",
            json!(11),
            "export.video.frame_rate.rate",
        ),
        ("/export/audio/kbps", json!(200), "export.audio.kbps"),
        (
            "/export/audio/sample_rate",
            json!(44_101),
            "export.audio.sample_rate",
        ),
    ];
    for (pointer, value, path) in bad {
        let mut v = sink_with_export();
        *v.pointer_mut(pointer).unwrap() = value;
        let (got, _) = bad_value(open_value(&v));
        assert_eq!(got, path, "{pointer}");
    }
    let schema: Vec<(&str, Value, &str)> = vec![
        (
            "/export/video/size/type",
            json!("custom"),
            "export.video.size.type",
        ),
        (
            "/export/video/quality/tier",
            json!("best"),
            "export.video.quality.tier",
        ),
        ("/export/video/codec", json!("hevc"), "export.video.codec"),
        (
            "/export/video/container",
            json!("mkv"),
            "export.video.container",
        ),
        ("/export/file/name", json!(3), "export.file.name"),
        (
            "/export/audio/include",
            json!("yes"),
            "export.audio.include",
        ),
    ];
    for (pointer, value, path) in schema {
        let mut v = sink_with_export();
        *v.pointer_mut(pointer).unwrap() = value;
        let (got, _) = json_error(open_value(&v));
        assert_eq!(got, path, "{pointer}");
    }
    // `null` means no settings were saved yet.
    let mut v = sink_with_export();
    v["export"] = Value::Null;
    assert_eq!(open_value(&v).unwrap().export, None);
}
