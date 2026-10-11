//! Media library and project settings edits (PM-005, PM-011, PM-030,
//! PM-058, PM-091).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::*;
use freecut_timeline::edit::{EditError, EditRequest, InsertTarget, NewMedia};
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::ids::IdAlloc;
use freecut_timeline::model::{InvariantViolation, MediaInfo, ProjectSettings, StoredPath};
use freecut_timeline::time::FrameRate;
use freecut_timeline::{CommitMeta, Editor};
use pretty_assertions::assert_eq;

fn new_media(path: &str, frames: i64) -> NewMedia {
    NewMedia {
        path: StoredPath {
            absolute: path.into(),
            relative: None,
        },
        info: MediaInfo {
            container: "wav".into(),
            default_video: None,
            duration: f(frames),
            video: Vec::new(),
            audio: Vec::new(),
            still: None,
        },
        fingerprint: None,
        display_name: None,
    }
}

#[test]
fn import_adds_items_with_fresh_ids_in_one_undo_step() {
    let mut fx = Fixture::new();
    let before = fx.doc().library().len();
    let req = EditRequest::ImportMedia(vec![new_media("/a.wav", 10), new_media("/b.wav", 20)]);
    let plan = fx.editor.plan(&fx.ctx(), &req).unwrap();
    let ids = plan.imported().to_vec();
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().all(|id| fx.doc().library().get(*id).is_none()));
    fx.editor.commit(plan, CommitMeta::default()).unwrap();
    assert_eq!(fx.doc().library().len(), before + 2);
    assert_eq!(fx.doc().library().duration(ids[1]), Some(f(20)));
    fx.editor.undo().unwrap();
    assert_eq!(fx.doc().library().len(), before);
    fx.editor.redo().unwrap();
    assert_eq!(fx.doc().library().len(), before + 2);
}

#[test]
fn media_in_use_is_removed_only_together_with_its_clips() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_50, 0, 5), (VIDEO_100, 0, 5)],
    );
    let refused = fx.apply(EditRequest::RemoveMedia {
        media: vec![VIDEO_50],
        with_clips: false,
    });
    assert_eq!(refused.unwrap_err(), EditError::MediaInUse(VIDEO_50));
    fx.apply(EditRequest::RemoveMedia {
        media: vec![VIDEO_50],
        with_clips: true,
    })
    .unwrap();
    assert!(fx.doc().library().get(VIDEO_50).is_none());
    // The main track closed up around the removed clip.
    assert_eq!(fx.render(), "V0*: 7@0+5[0]");
}

#[test]
fn unused_media_can_be_removed_and_unknown_media_is_refused() {
    let mut fx = Fixture::new();
    fx.apply(EditRequest::RemoveMedia {
        media: vec![AUDIO_200],
        with_clips: false,
    })
    .unwrap();
    assert!(fx.doc().library().get(AUDIO_200).is_none());
    let unknown = fx.apply(EditRequest::RemoveMedia {
        media: vec![PROBING],
        with_clips: false,
    });
    assert_eq!(unknown.unwrap_err(), EditError::UnknownMedia(PROBING));
}

#[test]
fn relinking_to_a_file_too_short_for_the_clips_is_refused() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 50, 20)]);
    let mut info = fx.doc().library().get(VIDEO_100).unwrap().info.clone();
    let path = StoredPath {
        absolute: "/moved/a.mp4".into(),
        relative: None,
    };
    info.duration = f(60);
    let short = fx.apply(EditRequest::RelinkMedia {
        media: VIDEO_100,
        path: path.clone(),
        info: info.clone(),
        fingerprint: None,
    });
    assert!(matches!(short, Err(EditError::SourceLimit { .. })));
    info.duration = f(120);
    fx.apply(EditRequest::RelinkMedia {
        media: VIDEO_100,
        path: path.clone(),
        info,
        fingerprint: None,
    })
    .unwrap();
    let item = fx.doc().library().get(VIDEO_100).unwrap();
    assert_eq!((item.path.clone(), item.info.duration), (path, f(120)));
}

#[test]
fn an_exact_length_correction_is_one_undoable_entry() {
    let mut fx = Fixture::new();
    let mut info = fx.doc().library().get(AUDIO_200).unwrap().info.clone();
    info.duration = f(199);
    fx.apply(EditRequest::UpdateMediaInfo {
        media: AUDIO_200,
        info,
    })
    .unwrap();
    assert_eq!(fx.doc().library().duration(AUDIO_200), Some(f(199)));
}

#[test]
fn settings_refuse_invalid_canvases() {
    let mut fx = Fixture::new();
    let settings = ProjectSettings {
        width: 1921,
        ..*fx.doc().settings()
    };
    let res = fx.apply(EditRequest::SetProjectSettings {
        settings,
        conform: false,
    });
    assert_eq!(res.unwrap_err(), EditError::InvalidCanvas);
}

#[test]
fn a_rate_change_with_conform_puts_every_clip_on_the_new_grid_in_one_step() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(
        InsertTarget::Track(main),
        0,
        &[(VIDEO_100, 0, 7), (VIDEO_50, 0, 3)],
    );
    fx.insert(InsertTarget::NewTrack, 1, &[(VIDEO_50, 0, 1)]);
    let r30 = FrameRate::new(30, 1).unwrap();
    let entries = fx.editor.history().len();
    let settings = ProjectSettings {
        frame_rate: r30,
        ..*fx.doc().settings()
    };
    fx.apply(EditRequest::SetProjectSettings {
        settings,
        conform: true,
    })
    .unwrap();
    assert_eq!(fx.editor.history().len(), entries + 1);
    for (_, track) in fx.seq().tracks() {
        for clip in track.clips() {
            assert!(r30.is_on_grid(clip.start()) && r30.is_on_grid(clip.end()));
            assert!(clip.duration() >= r30.frame_ticks());
        }
    }
}

#[test]
fn a_rate_change_without_conform_keeps_positions() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_100, 0, 7)]);
    let before = fx.seq().main_track().unwrap().clips()[0].range;
    fx.set_rate(FrameRate::new(30, 1).unwrap());
    assert_eq!(fx.seq().main_track().unwrap().clips()[0].range, before);
}

#[test]
fn opening_a_document_whose_clips_play_missing_media_is_refused() {
    let mut fx = Fixture::new();
    let main = fx.main();
    fx.insert(InsertTarget::Track(main), 0, &[(VIDEO_50, 0, 5)]);
    // Rebuild the same sequences without that media in the library.
    let doc = fx.doc();
    let library = freecut_timeline::model::Library::from_parts(
        doc.library()
            .items()
            .filter(|m| m.id != VIDEO_50)
            .cloned()
            .collect(),
    )
    .unwrap();
    let broken = freecut_timeline::model::Document::from_parts(
        doc.root(),
        *doc.settings(),
        library,
        doc.sequences()
            .map(|s| std::sync::Arc::new(s.clone()))
            .collect(),
    )
    .unwrap();
    let err = Editor::open(broken, IdAlloc::default(), HistoryLimits::default()).unwrap_err();
    assert!(matches!(
        err.as_slice(),
        [InvariantViolation::MissingMedia { .. }]
    ));
}
