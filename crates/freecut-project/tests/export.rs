//! Export settings: first-export defaults (EX-021), persistence with the
//! project, and the preflight checks (EX-036).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use common::*;
use freecut_project::export::{
    ExportSettings, FileTarget, FolderStatus, FrameRateChoice, IssueCode, IssueTarget,
    MIN_FREE_BYTES, OutputSize, PreflightEnv, QualityTier, Severity, ShortSide, has_problems,
    validate,
};
use freecut_project::{Extras, SaveRequest, decode, encode_with_export, open, save};
use freecut_timeline::ids::{ClipId, MediaId, SeqId, TrackId};
use freecut_timeline::model::{Document, ProjectSettings};
use freecut_timeline::time::{FrameRate, SampleRate};
use pretty_assertions::assert_eq;

fn root(doc: &Document) -> SeqId {
    doc.root()
}

/// A healthy environment: writable folder, plenty of space, an encoder.
fn env() -> PreflightEnv {
    PreflightEnv {
        folder: FolderStatus::Writable,
        free_bytes: Some(500_000_000_000),
        volume: Some("Data".into()),
        max_texture_side: Some(16_384),
        encoder_available: true,
        ..PreflightEnv::default()
    }
}

/// The kitchen sink's defaults, with a folder.
fn sink_settings(doc: &Document) -> ExportSettings {
    ExportSettings::defaults("Trip", Some(PathBuf::from("/Users/me/Movies")), doc)
}

fn codes(issues: &[freecut_project::export::Issue]) -> Vec<IssueCode> {
    issues.iter().map(|i| i.code.clone()).collect()
}

#[test]
fn first_export_defaults_follow_ex_021() {
    let (doc, _) = kitchen_sink();
    let s = sink_settings(&doc);
    assert_eq!(
        s.file,
        FileTarget {
            name: "Trip".into(),
            folder: Some(PathBuf::from("/Users/me/Movies")),
        }
    );
    assert_eq!(s.video.size, OutputSize::Canvas);
    assert_eq!(s.video.frame_rate, FrameRateChoice::Project);
    assert_eq!(s.video.quality, QualityTier::Recommended);
    // The kitchen sink is 44.1 kHz, which exports keep; it has sound.
    assert!(s.audio.include);
    assert_eq!(s.audio.kbps, 192);
    assert_eq!(s.audio.sample_rate.hz(), 44_100);
    // The canvas is 1080×1920 at 29.97: Recommended is 10 Mbps.
    assert_eq!(s.output_dimensions((1080, 1920)), (1080, 1920));
    assert_eq!(s.video_kbps(doc.settings()), 10_000);
    assert_eq!(s.output_rate(doc.settings().frame_rate), ntsc());
}

#[test]
fn a_silent_timeline_defaults_to_no_audio_and_96_khz_projects_export_at_48() {
    let plan = DocPlan {
        settings: ProjectSettings {
            sample_rate: SampleRate::new(96_000).unwrap(),
            ..ProjectSettings::new_default().unwrap()
        },
        media: Vec::new(),
        ops: Vec::new(),
    };
    let (doc, _) = build(&plan);
    let s = ExportSettings::defaults("x", None, &doc);
    assert!(!s.audio.include, "Timeline is silent");
    assert_eq!(s.audio.sample_rate.hz(), 48_000);
}

#[test]
fn export_settings_are_saved_with_the_project_and_opened_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Trip.freecut");
    let (doc, ids) = kitchen_sink();
    for export in [Some(kitchen_export()), Some(sink_settings(&doc)), None] {
        let request = SaveRequest {
            document: &doc,
            ids_next: ids,
            extras: &Extras::new(),
            backup: false,
            relative_base: None,
            export: export.as_ref(),
            ui: None,
        };
        save(&path, &request).unwrap();
        let opened = open(&path).unwrap();
        assert_eq!(opened.export, export);
        assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
        assert!(!opened.needs_save, "export settings are not a repair");
        // Saving what was opened is a byte-level fixpoint.
        let first = std::fs::read(&path).unwrap();
        let again = encode_with_export(
            &opened.document,
            opened.ids.next(),
            opened.export.as_ref(),
            &opened.extras,
        )
        .unwrap();
        assert_eq!(first, again);
    }
}

#[test]
fn the_export_section_is_written_in_schema_order_without_floats() {
    let (doc, ids) = kitchen_sink();
    let bytes = encode_with_export(&doc, ids, Some(&kitchen_export()), &Extras::new()).unwrap();
    let v = to_value(&bytes);
    assert_eq!(
        v["export"],
        serde_json::json!({
            "file": {"name": "Trip \"final\" / v2 ✂", "folder": r"C:\Users\me\Videos\Exports"},
            "video": {
                "size": {"type": "preset", "short_side": 720},
                "frame_rate": {"type": "fixed", "rate": {"num": 25, "den": 1}},
                "codec": "h264",
                "container": "mp4",
                "quality": {"type": "tier", "tier": "higher"}
            },
            "audio": {"include": false, "kbps": 256, "sample_rate": 44100}
        })
    );
    let text = String::from_utf8(bytes).unwrap();
    // `export` follows `document`; inside it, file, video, audio.
    let document = text.find("\n  \"document\": {").unwrap();
    let export = text.find("\n  \"export\": {").unwrap();
    assert!(document < export);
    let section = &text[export..];
    let keys: Vec<usize> = ["\"file\"", "\"video\"", "\"audio\""]
        .iter()
        .map(|k| section.find(k).unwrap())
        .collect();
    assert!(keys.windows(2).all(|w| w[0] < w[1]), "{keys:?}");
    let mut found = Vec::new();
    floats(&v, "", &mut found);
    assert!(found.is_empty(), "{found:?}");
}

#[cfg(unix)]
#[test]
fn a_folder_that_is_not_unicode_is_forgotten_not_mangled() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let (doc, ids) = kitchen_sink();
    let mut export = kitchen_export();
    export.file.folder = Some(PathBuf::from(OsStr::from_bytes(b"/tmp/caf\xe9")));
    let bytes = encode_with_export(&doc, ids, Some(&export), &Extras::new()).unwrap();
    let opened = decode(&bytes).unwrap();
    export.file.folder = None;
    assert_eq!(opened.export, Some(export));
}

#[test]
fn a_healthy_export_has_no_problems() {
    let (doc, _) = kitchen_sink();
    let issues = validate(&sink_settings(&doc), &doc, root(&doc), &env());
    assert!(!has_problems(&issues), "{issues:?}");
    // The kitchen sink has hidden and muted tracks with clips (21, 26), and
    // its 1080 px short side is not above its largest source (1080 px).
    assert_eq!(
        issues,
        vec![freecut_project::export::Issue {
            severity: Severity::Info,
            code: IssueCode::HiddenOrMutedTracks { tracks: 2 },
            targets: vec![
                IssueTarget::Track(TrackId::from_raw(nz(21))),
                IssueTarget::Track(TrackId::from_raw(nz(26))),
            ],
        }]
    );
}

#[test]
fn name_folder_and_target_problems() {
    let (doc, _) = kitchen_sink();
    let check = |edit: &dyn Fn(&mut ExportSettings), env: &PreflightEnv| {
        let mut s = sink_settings(&doc);
        edit(&mut s);
        codes(&validate(&s, &doc, root(&doc), env))
            .into_iter()
            .filter(|c| c.severity() == Severity::Problem)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        check(&|s| s.file.name = "  ".into(), &env()),
        vec![IssueCode::EmptyFileName]
    );
    // Only invalid characters: "Untitled", which is fine.
    assert_eq!(check(&|s| s.file.name = "???".into(), &env()), vec![]);
    assert_eq!(
        check(&|s| s.file.name = "x".repeat(201), &env()),
        vec![IssueCode::FileNameTooLong {
            chars: 201,
            bytes: 205
        }]
    );
    assert_eq!(check(&|s| s.file.name = "x".repeat(200), &env()), vec![]);
    // 70 four-byte characters fit 200 characters but not 255 bytes.
    assert_eq!(
        check(&|s| s.file.name = "🎬".repeat(70), &env()),
        vec![IssueCode::FileNameTooLong {
            chars: 70,
            bytes: 284
        }]
    );
    assert_eq!(
        check(&|s| s.file.folder = None, &env()),
        vec![IssueCode::NoFolder]
    );
    for (status, code) in [
        (FolderStatus::Missing, IssueCode::FolderMissing),
        (FolderStatus::NotWritable, IssueCode::FolderNotWritable),
        (
            FolderStatus::Disconnected {
                drive: "Card A".into(),
            },
            IssueCode::DriveDisconnected {
                drive: "Card A".into(),
            },
        ),
    ] {
        let env = PreflightEnv {
            folder: status,
            ..env()
        };
        assert_eq!(check(&|_| {}, &env), vec![code]);
    }
    // An unchecked folder reports nothing yet.
    let unknown = PreflightEnv {
        folder: FolderStatus::Unknown,
        ..env()
    };
    assert_eq!(check(&|_| {}, &unknown), vec![]);

    // Next to a source with another extension: a different file.
    let mut s = sink_settings(&doc);
    s.file.folder = Some(PathBuf::from("/Users/me/Pictures"));
    s.file.name = "sunset".into();
    assert!(!has_problems(&validate(&s, &doc, root(&doc), &env())));
    // Over a project source, whatever the case of the name.
    let (doc2, ids2) = kitchen_sink();
    let mut v = to_value(&freecut_project::encode(&doc2, ids2, &Extras::new()).unwrap());
    v["document"]["library"][2]["path"]["absolute"] =
        serde_json::json!("/Users/me/Movies/Trip.MP4");
    let doc3 = decode(&compact(&v)).unwrap().document;
    let issues = validate(&sink_settings(&doc3), &doc3, root(&doc3), &env());
    let target = issues
        .iter()
        .find(|i| matches!(i.code, IssueCode::TargetIsSource { .. }))
        .expect("the photo's file is the target");
    assert_eq!(
        target.code,
        IssueCode::TargetIsSource {
            media: MediaId::from_raw(nz(12))
        }
    );
    assert_eq!(
        target.targets,
        vec![IssueTarget::Media(MediaId::from_raw(nz(12)))]
    );
}

#[test]
fn size_encoder_audio_and_space_problems() {
    let (doc, _) = kitchen_sink();
    let problems = |s: &ExportSettings, env: &PreflightEnv| {
        codes(&validate(s, &doc, root(&doc), env))
            .into_iter()
            .filter(|c| c.severity() == Severity::Problem)
            .collect::<Vec<_>>()
    };
    let mut s = sink_settings(&doc);
    s.video.size = OutputSize::Preset(ShortSide::P2160);
    // 2160 × 3840 is above a 2048 px GPU.
    let small_gpu = PreflightEnv {
        max_texture_side: Some(2048),
        ..env()
    };
    assert_eq!(
        problems(&s, &small_gpu),
        vec![IssueCode::OutputTooLarge {
            width: 2160,
            height: 3840,
            max: 2048
        }]
    );
    assert_eq!(problems(&s, &env()), vec![]);
    let no_encoder = PreflightEnv {
        encoder_available: false,
        ..env()
    };
    assert_eq!(problems(&s, &no_encoder), vec![IssueCode::NoEncoder]);

    let mut s = sink_settings(&doc);
    s.audio.kbps = 200;
    s.audio.sample_rate = SampleRate::new(96_000).unwrap();
    s.video.frame_rate = FrameRateChoice::Fixed(FrameRate::new(15, 1).unwrap());
    assert_eq!(
        problems(&s, &env()),
        vec![
            IssueCode::UnsupportedFrameRate {
                rate: FrameRate::new(15, 1).unwrap()
            },
            IssueCode::AudioBitrateUnsupported { kbps: 200 },
            IssueCode::AudioSampleRateUnsupported { hz: 96_000 },
        ]
    );
    // Audio values don't matter when audio is off.
    s.audio.include = false;
    assert_eq!(problems(&s, &env()).len(), 1);

    // Less than 100 MB free blocks, naming how much to free.
    let s = sink_settings(&doc);
    let low = PreflightEnv {
        free_bytes: Some(30_000_000),
        ..env()
    };
    let estimate = freecut_project::export::estimate_size(
        &s,
        doc.settings(),
        doc.sequence(root(&doc)).unwrap().duration(),
    )
    .bytes;
    assert!(estimate < MIN_FREE_BYTES);
    assert_eq!(
        problems(&s, &low),
        vec![IssueCode::LowDiskSpace {
            free_bytes: 30_000_000,
            to_free: MIN_FREE_BYTES - 30_000_000,
            volume: Some("Data".into()),
        }]
    );
    // Unknown free space skips the check.
    let unknown = PreflightEnv {
        free_bytes: None,
        ..env()
    };
    assert_eq!(problems(&s, &unknown), vec![]);
}

#[test]
fn an_empty_timeline_is_a_problem() {
    let plan = DocPlan {
        settings: ProjectSettings::new_default().unwrap(),
        media: Vec::new(),
        ops: Vec::new(),
    };
    let (doc, _) = build(&plan);
    let s = ExportSettings::defaults("Empty", Some(PathBuf::from("/out")), &doc);
    let issues = validate(&s, &doc, doc.root(), &env());
    assert_eq!(codes(&issues), vec![IssueCode::EmptyTimeline]);
    // A sequence that doesn't exist is empty too.
    let issues = validate(&s, &doc, SeqId::from_raw(nz(999)), &env());
    assert_eq!(codes(&issues), vec![IssueCode::EmptyTimeline]);
}

#[test]
fn warnings_and_info_come_after_problems_in_a_stable_order() {
    // The kitchen sink with its adjustment clip (46) one hour long, so the
    // export is far larger than the free space below.
    let (sink, ids) = kitchen_sink();
    let mut v = to_value(&freecut_project::encode(&sink, ids, &Extras::new()).unwrap());
    v["document"]["sequences"][0]["visual"][5]["clips"][0]["range"]["duration"] =
        serde_json::json!(fr(107_892).raw());
    let doc = decode(&compact(&v)).unwrap().document;
    let mut s = sink_settings(&doc);
    s.file.name = String::new();
    s.video.frame_rate = FrameRateChoice::Fixed(FrameRate::new(60, 1).unwrap());
    s.video.size = OutputSize::Preset(ShortSide::P1440);
    let env = PreflightEnv {
        free_bytes: Some(MIN_FREE_BYTES + 10),
        offline_media: BTreeSet::from([MediaId::from_raw(nz(10))]),
        substituted_fonts: vec!["DejaVu Serif".into()],
        ..env()
    };
    let issues = validate(&s, &doc, root(&doc), &env);
    let severities: Vec<Severity> = issues.iter().map(|i| i.severity).collect();
    let mut sorted = severities.clone();
    sorted.sort();
    assert_eq!(severities, sorted);
    let offline = issues
        .iter()
        .find(|i| matches!(i.code, IssueCode::OfflineMedia { .. }))
        .unwrap();
    // Clips of item 10 in the root sequence: 40, 42 and 47.
    assert_eq!(offline.code, IssueCode::OfflineMedia { clips: 3 });
    let mut targets = offline.targets.clone();
    targets.sort();
    assert_eq!(
        targets,
        [40, 42, 47]
            .map(|n| IssueTarget::Clip(ClipId::from_raw(nz(n))))
            .to_vec()
    );
    let codes = codes(&issues);
    assert!(codes.contains(&IssueCode::EmptyFileName));
    assert!(codes.contains(&IssueCode::FontsSubstituted {
        families: vec!["DejaVu Serif".into()]
    }));
    assert!(
        codes
            .iter()
            .any(|c| matches!(c, IssueCode::SizeAboveFreeSpace { .. }))
    );
    assert!(codes.contains(&IssueCode::FrameRateConversion {
        from: ntsc(),
        to: FrameRate::new(60, 1).unwrap()
    }));
    // The 4000×3000 photo is larger than a 1440 px output.
    assert!(
        !codes
            .iter()
            .any(|c| matches!(c, IssueCode::Upscaling { .. }))
    );
    // Same input, same list.
    assert_eq!(validate(&s, &doc, root(&doc), &env), issues);
    for issue in &issues {
        assert!(issue.code.id().starts_with("export-"));
        assert_eq!(issue.severity, issue.code.severity());
    }
}

#[test]
fn hiding_every_visual_track_warns() {
    let (doc, ids) = kitchen_sink();
    let mut v = to_value(&freecut_project::encode(&doc, ids, &Extras::new()).unwrap());
    for track in v["document"]["sequences"][0]["visual"]
        .as_array_mut()
        .unwrap()
    {
        track["header"]["hidden"] = serde_json::json!(true);
    }
    let hidden = decode(&compact(&v)).unwrap().document;
    let issues = validate(&sink_settings(&hidden), &hidden, hidden.root(), &env());
    assert!(codes(&issues).contains(&IssueCode::AllVisualTracksHidden));
    assert!(!has_problems(&issues));
}

#[test]
fn an_output_larger_than_every_source_is_noted() {
    // One 1200×720 video on a 1920×1080 canvas.
    let video = MediaSpec {
        kind: 0,
        frames: 400,
        absolute: "/m/clip.mp4".into(),
        relative: None,
        fingerprint: None,
        display_name: None,
        rotation: 0,
        start: 0,
        tagged: false,
    };
    let plan = DocPlan {
        settings: ProjectSettings {
            width: 1920,
            height: 1080,
            ..ProjectSettings::new_default().unwrap()
        },
        media: vec![video],
        ops: vec![Op::Insert {
            asset: 0,
            src: 0,
            len: 30,
            at: 0,
            track: 200,
        }],
    };
    let (doc, _) = build(&plan);
    let mut s = ExportSettings::defaults("Clip", Some(PathBuf::from("/out")), &doc);
    let issues = validate(&s, &doc, doc.root(), &env());
    assert_eq!(
        codes(&issues),
        vec![IssueCode::Upscaling {
            output: 1080,
            source: 720
        }]
    );
    s.video.size = OutputSize::Preset(ShortSide::P720);
    assert!(validate(&s, &doc, doc.root(), &env()).is_empty());
}
