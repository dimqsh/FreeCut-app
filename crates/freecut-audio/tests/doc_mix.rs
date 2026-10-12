//! Real files, probed and imported into a document through the editor, then
//! mixed through `DocSchedule` and `FileOpener::from_paths` (AU-010).
#![cfg(feature = "media")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use freecut_audio::{DocSchedule, FileOpener, Mixer};
use freecut_timeline::edit::{
    AudioEdit, EditCtx, EditRequest, InsertReq, InsertTarget, NewClip, NewMedia,
};
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::ids::{ClipId, MediaId, TrackId};
use freecut_timeline::model::{MediaKind, StoredPath, TrackFlags, TrackHeader};
use freecut_timeline::time::{SampleRate, TICKS_PER_SECOND, Tick, TimeRange};
use freecut_timeline::{CommitMeta, Committed, Editor};

const RATE: u32 = 48_000;

/// Runs the `ffmpeg` CLI writing `out`; `false` when it isn't installed.
fn ffmpeg(args: &[&str], out: &Path) -> bool {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(out)
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

/// Two files of 2 s each, both with a stereo sine of amplitude 1/8: a WAV
/// at 440 Hz and an MP4 (picture plus AAC sound) at 880 Hz. `None` when
/// ffmpeg isn't installed.
fn make_media(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let tone = dir.join("tone.wav");
    let movie = dir.join("movie.mp4");
    let stereo = "pan=stereo|c0=c0|c1=c0";
    let ok = ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
            "-af",
            stereo,
            "-c:a",
            "pcm_s16le",
        ],
        &tone,
    ) && ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:s=320x240:r=25:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000:duration=2",
            "-af",
            stereo,
            "-c:v",
            "mpeg4",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-shortest",
        ],
        &movie,
    );
    ok.then_some((tone, movie))
}

fn ms(n: i64) -> Tick {
    Tick::from_raw(TICKS_PER_SECOND / 1000 * n)
}

fn span_ms(start: i64, len: i64) -> TimeRange {
    TimeRange::new(ms(start), ms(len)).unwrap()
}

/// An editor holding the two files, each as an imported library item.
struct Project {
    editor: Editor,
    tone: MediaId,
    movie: MediaId,
    paths: BTreeMap<MediaId, PathBuf>,
}

impl Project {
    fn import(tone: PathBuf, movie: PathBuf) -> Project {
        let mut editor = Editor::new(HistoryLimits::default()).unwrap();
        let files = [&tone, &movie];
        let items = files
            .iter()
            .map(|path| {
                let probed = freecut_media::probe(path).unwrap();
                NewMedia {
                    path: StoredPath {
                        absolute: path.display().to_string(),
                        relative: None,
                    },
                    info: probed.info,
                    fingerprint: Some(probed.fingerprint),
                    display_name: None,
                }
            })
            .collect();
        let plan = editor
            .plan(&EditCtx::default(), &EditRequest::ImportMedia(items))
            .unwrap();
        let [tone_id, movie_id] = plan.imported()[..] else {
            panic!("expected two items")
        };
        editor.commit(plan, CommitMeta::default()).unwrap();
        Project {
            editor,
            tone: tone_id,
            movie: movie_id,
            paths: BTreeMap::from([(tone_id, tone), (movie_id, movie)]),
        }
    }

    fn apply(&mut self, req: EditRequest) -> Vec<ClipId> {
        let plan = self.editor.plan(&EditCtx::default(), &req).unwrap();
        match self.editor.commit(plan, CommitMeta::default()).unwrap() {
            Committed::Applied { selection, .. } => selection.unwrap_or_default(),
            Committed::NoOp => panic!("{req:?} changed nothing"),
        }
    }

    fn place(&mut self, target: InsertTarget, at: Tick, clip: NewClip) -> ClipId {
        let seq = self.editor.document().root();
        self.apply(EditRequest::Insert(InsertReq {
            seq,
            target,
            at,
            clips: vec![clip],
        }))[0]
    }

    fn main(&self) -> TrackId {
        let doc = self.editor.document();
        doc.sequence(doc.root()).unwrap().main_track().unwrap().id
    }

    /// The movie's first second at [0 s, 1 s) on the main track, and the
    /// tone from 0.5 s at [1.5 s, 2.5 s) on an audio track.
    fn place_clips(&mut self) -> (ClipId, ClipId) {
        let main = self.main();
        let v = self.place(
            InsertTarget::Track(main),
            Tick::ZERO,
            NewClip::media(self.movie, MediaKind::Video, span_ms(0, 1000)),
        );
        let t = self.place(
            InsertTarget::FirstFree,
            ms(1500),
            NewClip::media(self.tone, MediaKind::Audio, span_ms(500, 1000)),
        );
        (v, t)
    }

    fn schedule(&self) -> DocSchedule {
        DocSchedule::root(self.editor.document().clone())
    }
}

/// Mixes the first 3 s in 10 ms blocks, as playback pulls them, with the
/// limiter on. Returns (left, right).
fn mix(sched: &DocSchedule, opener: &FileOpener) -> (Vec<f32>, Vec<f32>) {
    let mut mixer = Mixer::new(SampleRate::new(RATE).unwrap());
    let block = RATE as usize / 100;
    let mut out = Vec::new();
    for k in 0..300 {
        let mut buf = vec![f32::NAN; block * 2];
        mixer.render(sched, opener, (k * block) as i64, &mut buf);
        out.extend(buf);
    }
    let left = out.iter().step_by(2).copied().collect();
    let right = out.iter().skip(1).step_by(2).copied().collect();
    (left, right)
}

/// Samples `[from, to)` in milliseconds.
fn part(samples: &[f32], from: usize, to: usize) -> &[f32] {
    let per_ms = RATE as usize / 1000;
    &samples[from * per_ms..to * per_ms]
}

fn rms(v: &[f32]) -> f64 {
    (v.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>() / v.len() as f64).sqrt()
}

/// Upward zero crossings per second.
fn frequency(v: &[f32]) -> f64 {
    let ups = v.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    ups as f64 * f64::from(RATE) / v.len() as f64
}

/// A sine of amplitude 1/8.
const SINE_RMS: f64 = 0.125 / std::f64::consts::SQRT_2;

fn assert_tone(samples: &[f32], from: usize, to: usize, hz: f64, what: &str) {
    // Leave 50 ms at each edge for codec ramps.
    let body = part(samples, from + 50, to - 50);
    let level = rms(body);
    assert!(
        (level - SINE_RMS).abs() < 0.05 * SINE_RMS,
        "{what}: RMS {level}"
    );
    let f = frequency(body);
    assert!((f - hz).abs() < 0.01 * hz, "{what}: frequency {f}");
}

fn max_difference(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

fn assert_silent(samples: &[f32], from: usize, to: usize, what: &str) {
    let span = part(samples, from, to);
    assert!(span.iter().all(|&v| v == 0.0), "{what}: not silent");
}

#[test]
fn real_files_sound_exactly_where_their_clips_are() {
    let dir = tempfile::tempdir().unwrap();
    let Some((tone, movie)) = make_media(dir.path()) else {
        return;
    };
    let mut p = Project::import(tone, movie);
    p.place_clips();
    let opener = FileOpener::from_paths(&p.paths, &BTreeSet::new());
    let (left, right) = mix(&p.schedule(), &opener);

    assert_tone(&left, 0, 1000, 880.0, "movie sound");
    assert_silent(&left, 1000, 1500, "gap");
    assert_tone(&left, 1500, 2500, 440.0, "tone");
    assert_silent(&left, 2500, 3000, "after the end");
    // Both channels carry the same sine (AAC codes them separately, so
    // the movie's differ by a little coding noise).
    let worst = max_difference(&left, &right);
    assert!(worst < 1e-3, "channels differ by {worst}");
    // The tone clip starts on sample 72 000 (1.5 s at 48 kHz): silence up
    // to it, sound right after (the WAV plays from 0.5 s, mid-wave).
    assert_eq!(left[71_999], 0.0);
    assert!(left[72_000..72_004].iter().any(|&v| v != 0.0));
}

#[test]
fn detached_offline_and_muted_sources() {
    let dir = tempfile::tempdir().unwrap();
    let Some((tone, movie)) = make_media(dir.path()) else {
        return;
    };
    let mut p = Project::import(tone, movie);
    let (v, t) = p.place_clips();
    let opener = FileOpener::from_paths(&p.paths, &BTreeSet::new());
    let (before, _) = mix(&p.schedule(), &opener);

    // Detaching moves the movie's sound to an audio clip; it sounds the same.
    let seq = p.editor.document().root();
    let a = p.apply(EditRequest::DetachAudio {
        seq,
        clips: vec![v],
    })[0];
    let (after, _) = mix(&p.schedule(), &opener);
    let worst = max_difference(&before, &after);
    assert!(worst < 1e-6, "detaching changed the sound by {worst}");

    // The tone's file is offline: only its clip goes silent.
    let offline = FileOpener::from_paths(&p.paths, &BTreeSet::from([p.tone]));
    let (left, _) = mix(&p.schedule(), &offline);
    assert_tone(&left, 0, 1000, 880.0, "movie sound");
    assert_silent(&left, 1000, 3000, "offline tone");

    // Muting the detached clip silences only the movie's sound.
    p.apply(EditRequest::SetClipAudio {
        seq,
        clips: vec![a],
        edit: AudioEdit::Muted(true),
    });
    let (left, _) = mix(&p.schedule(), &opener);
    assert_silent(&left, 0, 1500, "muted clip");
    assert_tone(&left, 1500, 2500, 440.0, "tone");

    // The detached clip went onto the tone's track, which was free over
    // [0 s, 1 s); muting that track silences both.
    let track_of = |clip| p.editor.document().find_clip(clip).unwrap().track;
    let track = track_of(a);
    assert_eq!(track, track_of(t));
    p.apply(EditRequest::SetClipAudio {
        seq,
        clips: vec![a],
        edit: AudioEdit::Muted(false),
    });
    p.apply(EditRequest::SetTrackHeader {
        seq,
        track,
        header: TrackHeader {
            name: None,
            flags: TrackFlags {
                muted: true,
                ..TrackFlags::default()
            },
        },
    });
    let (left, _) = mix(&p.schedule(), &opener);
    assert_silent(&left, 0, 3000, "muted track");
}
