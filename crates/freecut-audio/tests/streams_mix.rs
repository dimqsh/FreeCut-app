//! A real file with two audio streams, probed, imported and played through
//! `DocSchedule` and `FileOpener`: the stream picker, "All streams mixed"
//! (AU-038) and mono at unity on both channels (AU-010).
#![cfg(feature = "media")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use freecut_audio::{DocSchedule, FileOpener, Mixer, stream_mix_factor};
use freecut_timeline::edit::{EditCtx, EditRequest, InsertReq, InsertTarget, NewClip, NewMedia};
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{MediaKind, StoredPath, StreamSel};
use freecut_timeline::time::{SampleRate, Tick, TimeRange};
use freecut_timeline::{CommitMeta, Editor};

const RATE: u32 = 48_000;

/// 2 s of grey picture (stream 0), a mono 0.4 (stream 1) and a stereo
/// 0.2 | 0.1 (stream 2), as float PCM in Matroska. `false` without the
/// `ffmpeg` CLI.
fn make_dual(path: &Path) -> bool {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "color=c=gray:s=160x120:r=25:d=2"])
        .args(["-f", "lavfi", "-i", "aevalsrc=0.4:c=mono:s=48000:d=2"])
        .args(["-f", "lavfi", "-i", "aevalsrc=0.2|0.1:c=stereo:s=48000:d=2"])
        .args(["-map", "0:v", "-map", "1:a", "-map", "2:a"])
        .args(["-c:v", "mpeg4", "-c:a", "pcm_f32le"])
        .arg(path)
        .status();
    match status {
        Ok(s) if s.success() => true,
        Ok(s) => panic!("ffmpeg failed: {s}"),
        Err(e) => {
            eprintln!("skipping: ffmpeg CLI not available ({e})");
            false
        }
    }
}

fn commit(editor: &mut Editor, req: EditRequest) -> Option<Vec<ClipId>> {
    let plan = editor.plan(&EditCtx::default(), &req).unwrap();
    match editor.commit(plan, CommitMeta::default()).unwrap() {
        freecut_timeline::Committed::Applied { selection, .. } => selection,
        freecut_timeline::Committed::NoOp => panic!("{req:?} changed nothing"),
    }
}

/// The stereo frame at 1 s, limiter off.
fn frame_at_one_second(sched: &DocSchedule, opener: &FileOpener) -> [f32; 2] {
    let mut mixer = Mixer::new(SampleRate::new(RATE).unwrap());
    mixer.set_limiter(false);
    let mut out = vec![f32::NAN; 2 * 4_800];
    mixer.render(sched, opener, i64::from(RATE), &mut out);
    [out[2 * 2_400], out[2 * 2_400 + 1]]
}

#[test]
fn the_chosen_stream_plays_and_all_streams_mix_at_one_over_root_n() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dual.mkv");
    if !make_dual(&path) {
        return;
    }
    let probed = freecut_media::probe(&path).unwrap();
    let streams: Vec<(u32, u16)> = probed
        .info
        .audio
        .iter()
        .map(|a| (a.index, a.channels))
        .collect();
    assert_eq!(streams, [(1, 1), (2, 2)]);

    let mut editor = Editor::new(HistoryLimits::default()).unwrap();
    let plan = editor
        .plan(
            &EditCtx::default(),
            &EditRequest::ImportMedia(vec![NewMedia {
                path: StoredPath {
                    absolute: path.display().to_string(),
                    relative: None,
                },
                info: probed.info,
                fingerprint: Some(probed.fingerprint),
                display_name: None,
            }]),
        )
        .unwrap();
    let media = plan.imported()[0];
    editor.commit(plan, CommitMeta::default()).unwrap();
    let seq = editor.document().root();
    let main = editor
        .document()
        .sequence(seq)
        .unwrap()
        .main_track()
        .unwrap()
        .id;
    let two = TimeRange::new(Tick::ZERO, Tick::from_secs(2).unwrap()).unwrap();
    let clip = commit(
        &mut editor,
        EditRequest::Insert(InsertReq {
            seq,
            target: InsertTarget::Track(main),
            at: Tick::ZERO,
            clips: vec![NewClip::media(media, MediaKind::Video, two)],
        }),
    )
    .unwrap()[0];
    let opener = FileOpener::from_paths(&BTreeMap::from([(media, path)]), &BTreeSet::new());
    let level = |editor: &Editor| {
        frame_at_one_second(&DocSchedule::root(editor.document().clone()), &opener)
    };
    let close = |got: [f32; 2], want: [f32; 2]| {
        assert!(
            (got[0] - want[0]).abs() < 1e-5 && (got[1] - want[1]).abs() < 1e-5,
            "{got:?} vs {want:?}"
        );
    };

    // The default stream is mono: on both channels at unity.
    close(level(&editor), [0.4, 0.4]);

    let mut pick = |sel| {
        commit(
            &mut editor,
            EditRequest::SetAudioStream {
                seq,
                clips: vec![clip],
                sel: Some(sel),
            },
        );
        level(&editor)
    };
    close(pick(StreamSel::Index(2)), [0.2, 0.1]);
    let f = stream_mix_factor(2);
    close(
        pick(StreamSel::AllMixed),
        [(0.4 + 0.2) * f, (0.4 + 0.1) * f],
    );
}
