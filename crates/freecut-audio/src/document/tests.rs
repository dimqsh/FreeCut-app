//! A document built through the editor, read through [`DocSchedule`] and
//! rendered with synthetic sources.

use core::num::NonZeroU64;
use std::collections::HashMap;

use freecut_timeline::edit::{
    AudioEdit, Edge, EditCtx, EditRequest, GainKeyEdit, InsertReq, InsertTarget, NewClip, NewMedia,
    SplitReq, SplitTargets, TrimReq,
};
use freecut_timeline::export::ExportClock;
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::ids::MediaId;
use freecut_timeline::model::{
    AudioStreamInfo, EmbeddedAudio, MediaInfo, MediaKind, StoredPath, StreamSel, TrackFlags,
    TrackHeader, VideoStreamInfo,
};
use freecut_timeline::time::{FrameRate, Tick};
use freecut_timeline::{CommitMeta, Committed, Editor};

use super::*;
use crate::mixer::Mixer;
use crate::offline::OfflineMix;
use crate::source::SourceOpener;
use crate::testing::{Opener, Wave, rate, secs};

/// Probe-like info for `len` of media with a picture and/or sound. The
/// sound is container stream 1 next to a picture, else stream 0.
fn info(len: Tick, picture: bool, sound: bool) -> MediaInfo {
    let first = u32::from(picture);
    let streams: &[u32] = if sound { &[first] } else { &[] };
    info_with(len, picture, streams)
}

/// Probe-like info with an audio stream at each container index of
/// `streams` (none flagged default: the first plays by default).
fn info_with(len: Tick, picture: bool, streams: &[u32]) -> MediaInfo {
    let video = picture.then(|| VideoStreamInfo {
        index: 0,
        codec: "h264".into(),
        width: 1920,
        height: 1080,
        sar: (1, 1),
        rotation: Default::default(),
        mirrored: false,
        rate: None,
        variable_rate: false,
        has_alpha: false,
        bit_depth: 8,
        start: Tick::ZERO,
        duration: len,
    });
    let audio = streams
        .iter()
        .map(|&index| AudioStreamInfo {
            index,
            codec: "aac".into(),
            sample_rate: 48_000,
            channels: 2,
            start: Tick::ZERO,
            duration: len,
            duration_exact: true,
            default: false,
            language: None,
            title: None,
            layout: None,
        })
        .collect();
    MediaInfo {
        container: "mp4".into(),
        default_video: picture.then_some(0),
        duration: len,
        video: video.into_iter().collect(),
        audio,
        still: None,
    }
}

fn span(start: f64, len: f64) -> TimeRange {
    TimeRange::new(secs(start), secs(len)).unwrap()
}

/// An editor whose edits all go through plan and commit.
struct Project {
    editor: Editor,
}

impl Project {
    fn new() -> Project {
        Project {
            editor: Editor::new(HistoryLimits::default()).unwrap(),
        }
    }

    fn doc(&self) -> &Document {
        self.editor.document()
    }

    fn schedule(&self) -> DocSchedule {
        DocSchedule::root(self.doc().clone())
    }

    /// Commits `req` and returns the selection it leaves.
    fn apply(&mut self, req: EditRequest) -> Vec<ClipId> {
        let plan = self.editor.plan(&EditCtx::default(), &req).unwrap();
        match self.editor.commit(plan, CommitMeta::default()).unwrap() {
            Committed::Applied { selection, .. } => selection.unwrap_or_default(),
            Committed::NoOp => panic!("{req:?} changed nothing"),
        }
    }

    /// Commits a library request and returns the item it names.
    fn add_media(&mut self, req: EditRequest) -> MediaId {
        let plan = self.editor.plan(&EditCtx::default(), &req).unwrap();
        let id = plan.imported()[0];
        self.editor.commit(plan, CommitMeta::default()).unwrap();
        id
    }

    fn import(&mut self, name: &str, info: MediaInfo) -> MediaId {
        self.add_media(EditRequest::ImportMedia(vec![NewMedia {
            path: StoredPath {
                absolute: format!("/media/{name}"),
                relative: None,
            },
            info,
            fingerprint: None,
            display_name: None,
        }]))
    }

    /// Places `source` of `media` at `at` seconds.
    fn place(
        &mut self,
        target: InsertTarget,
        at: f64,
        media: MediaId,
        kind: MediaKind,
        source: TimeRange,
    ) -> ClipId {
        let seq = self.doc().root();
        self.apply(EditRequest::Insert(InsertReq {
            seq,
            target,
            at: secs(at),
            clips: vec![NewClip::media(media, kind, source)],
        }))[0]
    }

    fn main(&self) -> TrackId {
        let seq = self.doc().sequence(self.doc().root()).unwrap();
        seq.main_track().unwrap().id
    }

    fn track_of(&self, clip: ClipId) -> TrackId {
        self.doc().find_clip(clip).unwrap().track
    }

    fn set_track(&mut self, track: TrackId, flags: TrackFlags) {
        let seq = self.doc().root();
        self.apply(EditRequest::SetTrackHeader {
            seq,
            track,
            header: TrackHeader { name: None, flags },
        });
    }

    fn mute_track(&mut self, track: TrackId) {
        self.set_track(
            track,
            TrackFlags {
                muted: true,
                ..TrackFlags::default()
            },
        );
    }

    fn set(&mut self, clip: ClipId, edit: AudioEdit) {
        let seq = self.doc().root();
        self.apply(EditRequest::SetClipAudio {
            seq,
            clips: vec![clip],
            edit,
        });
    }

    /// Detaches the sound of `clip`; returns the new audio clip.
    fn detach(&mut self, clip: ClipId) -> ClipId {
        let seq = self.doc().root();
        let selected = self.apply(EditRequest::DetachAudio {
            seq,
            clips: vec![clip],
        });
        assert_eq!(selected.len(), 1);
        selected[0]
    }
}

/// The library items and clips of [`standard`].
struct Ids {
    /// 10 s with picture and sound (stream 1).
    video: MediaId,
    /// 10 s of sound only (stream 0).
    music: MediaId,
    /// `video` from 0 s, at [0 s, 4 s) on the main track.
    v: ClipId,
    /// `music` from 2 s, at [1 s, 4 s) on an audio track.
    m: ClipId,
    /// A video without sound at [5 s, 7 s) on an overlay track.
    s: ClipId,
}

fn standard() -> (Project, Ids) {
    let mut p = Project::new();
    let video = p.import("video.mp4", info(secs(10.0), true, true));
    let music = p.import("music.wav", info(secs(10.0), false, true));
    let silent = p.import("silent.mp4", info(secs(10.0), true, false));
    let main = p.main();
    let v = p.place(
        InsertTarget::Track(main),
        0.0,
        video,
        MediaKind::Video,
        span(0.0, 4.0),
    );
    let m = p.place(
        InsertTarget::FirstFree,
        1.0,
        music,
        MediaKind::Audio,
        span(2.0, 3.0),
    );
    let s = p.place(
        InsertTarget::FirstFree,
        5.0,
        silent,
        MediaKind::Video,
        span(0.0, 2.0),
    );
    (
        p,
        Ids {
            video,
            music,
            v,
            m,
            s,
        },
    )
}

fn input_of(inputs: &[MixInput], clip: ClipId) -> Option<&MixInput> {
    inputs.iter().find(|i| i.key == clip.get())
}

fn keys(inputs: &[MixInput]) -> Vec<u64> {
    inputs.iter().map(|i| i.key).collect()
}

// ---------------------------------------------------------------- inputs

#[test]
fn every_clip_with_sound_becomes_an_input() {
    let (p, ids) = standard();
    let inputs = p.schedule().inputs(span(0.0, 10.0));
    let stage = |clip: ClipId| TrackStage {
        id: p.track_of(clip).get(),
        ..TrackStage::default()
    };
    assert_eq!(
        inputs,
        vec![
            MixInput {
                key: ids.v.get(),
                clip: ids.v.get(),
                source: SourceKey {
                    asset: ids.video.get(),
                    stream: 1,
                },
                range: span(0.0, 4.0),
                source_start: Tick::ZERO,
                gain: 1.0.into(),
                fade_in: Fade::NONE,
                fade_out: Fade::NONE,
                pan: 0.0,
                stream_mix: 1.0,
                track: stage(ids.v),
            },
            MixInput {
                key: ids.m.get(),
                clip: ids.m.get(),
                source: SourceKey {
                    asset: ids.music.get(),
                    stream: 0,
                },
                range: span(1.0, 3.0),
                source_start: secs(2.0),
                gain: 1.0.into(),
                fade_in: Fade::NONE,
                fade_out: Fade::NONE,
                pan: 0.0,
                stream_mix: 1.0,
                track: stage(ids.m),
            },
        ]
    );
    // The soundless video is placed but never scheduled.
    assert_eq!(p.doc().clip(ids.s).unwrap().audio, EmbeddedAudio::None);
}

#[test]
fn only_clips_overlapping_the_window_are_listed() {
    let (p, ids) = standard();
    let sched = p.schedule();
    assert_eq!(keys(&sched.inputs(span(0.0, 1.0))), [ids.v.get()]);
    assert_eq!(
        keys(&sched.inputs(span(3.5, 1.0))),
        [ids.v.get(), ids.m.get()]
    );
    // Clip ends are exclusive; the soundless clip at 5 s doesn't count.
    assert!(sched.inputs(span(4.0, 3.0)).is_empty());
}

#[test]
fn stored_gain_and_fades_carry_over() {
    let (mut p, ids) = standard();
    p.set(ids.m, AudioEdit::Gain(GainDb::from_hundredths(-2_000)));
    p.set(ids.m, AudioEdit::FadeIn(secs(0.5)));
    // Longer than the clip: passed on as stored, the mixer clamps it.
    p.set(ids.m, AudioEdit::FadeOut(secs(5.0)));
    p.set(ids.v, AudioEdit::Gain(GainDb::SILENT));
    let inputs = p.schedule().inputs(span(0.0, 10.0));

    let m = input_of(&inputs, ids.m).unwrap();
    assert!((static_gain(m) - 0.1).abs() < 1e-7, "gain {:?}", m.gain);
    assert_eq!((m.fade_in.len, m.fade_out.len), (secs(0.5), secs(5.0)));
    // −∞ is still listed, at amplitude 0.
    assert_eq!(static_gain(input_of(&inputs, ids.v).unwrap()), 0.0);
}

#[test]
fn muted_clips_and_tracks_are_left_out_but_hidden_tracks_sound() {
    let (mut p, ids) = standard();
    let main = p.main();
    p.set_track(
        main,
        TrackFlags {
            hidden: true,
            ..TrackFlags::default()
        },
    );
    assert_eq!(
        keys(&p.schedule().inputs(span(0.0, 10.0))),
        [ids.v.get(), ids.m.get()]
    );

    p.set(ids.v, AudioEdit::Muted(true));
    assert_eq!(keys(&p.schedule().inputs(span(0.0, 10.0))), [ids.m.get()]);

    p.mute_track(p.track_of(ids.m));
    assert!(p.schedule().inputs(span(0.0, 10.0)).is_empty());
}

#[test]
fn detached_sound_plays_from_its_audio_clip() {
    let (mut p, ids) = standard();
    p.set(ids.v, AudioEdit::Gain(GainDb::from_hundredths(-2_000)));
    p.set(ids.v, AudioEdit::FadeIn(secs(1.0)));
    let a = p.detach(ids.v);
    let inputs = p.schedule().inputs(span(0.0, 10.0));

    assert!(input_of(&inputs, ids.v).is_none(), "the video is silent");
    let audio = input_of(&inputs, a).unwrap();
    assert_eq!(
        audio.source,
        SourceKey {
            asset: ids.video.get(),
            stream: 1,
        }
    );
    assert_eq!(
        (audio.range, audio.source_start, audio.fade_in.len),
        (span(0.0, 4.0), Tick::ZERO, secs(1.0))
    );
    assert!((static_gain(audio) - 0.1).abs() < 1e-7);
    assert!(input_of(&inputs, ids.m).is_some());

    p.mute_track(p.track_of(a));
    assert_eq!(keys(&p.schedule().inputs(span(0.0, 10.0))), [ids.m.get()]);
}

#[test]
fn extracted_audio_reads_its_own_item_at_the_parent_stream() {
    let (mut p, ids) = standard();
    let extracted = p.add_media(EditRequest::ExtractAudio {
        media: ids.video,
        stream: None,
    });
    let x = p.place(
        InsertTarget::FirstFree,
        5.0,
        extracted,
        MediaKind::Audio,
        span(3.0, 1.0),
    );
    let inputs = p.schedule().inputs(span(5.0, 1.0));
    let input = input_of(&inputs, x).unwrap();
    assert_eq!(
        (input.source, input.source_start),
        (
            SourceKey {
                asset: extracted.get(),
                stream: 1,
            },
            secs(3.0)
        )
    );
}

#[test]
fn an_unknown_sequence_is_silent() {
    let (p, _) = standard();
    let other = freecut_timeline::ids::SeqId::from_raw(NonZeroU64::new(999_999).unwrap());
    let sched = DocSchedule::new(p.doc().clone(), other);
    assert_eq!(sched.sequence(), other);
    assert!(sched.inputs(span(0.0, 10.0)).is_empty());
    assert_eq!(
        DocSchedule::root(p.doc().clone()).sequence(),
        p.doc().root()
    );
}

// ---------------------------------------------------------------- render

/// Renders `seconds` from 0 with the limiter off (bit-exact values) and
/// returns the left channel, checking the right one matches it.
fn render(sched: &DocSchedule, opener: &dyn SourceOpener, hz: u32, seconds: usize) -> Vec<f32> {
    let mut mixer = Mixer::new(rate(hz));
    mixer.set_limiter(false);
    let mut out = vec![f32::NAN; hz as usize * seconds * 2];
    mixer.render(sched, opener, 0, &mut out);
    let (left, right): (Vec<f32>, Vec<f32>) =
        out.as_chunks::<2>().0.iter().map(|f| (f[0], f[1])).unzip();
    let negated: Vec<f32> = right.iter().map(|r| -r).collect();
    assert!(right == left || negated == left, "channels disagree");
    left
}

#[test]
fn clips_land_on_their_exact_samples() {
    let (p, ids) = standard();
    // The video's file is offline: only the music sounds.
    let opener = Opener::with(&[(ids.music.get(), Wave::Counter)]);
    for hz in [48_000u32, 44_100] {
        let r = i64::from(hz);
        let left = render(&p.schedule(), &opener, hz, 5);
        for (k, &v) in left.iter().enumerate() {
            let k = k as i64;
            // [1 s, 4 s) on the timeline plays the music from 2 s.
            let expected = if (r..4 * r).contains(&k) {
                (2 * r + k - r) as f32
            } else {
                0.0
            };
            assert_eq!(v, expected, "{hz} Hz, sample {k}");
        }
    }
}

#[test]
fn detaching_keeps_the_sound_and_muting_silences_it() {
    let (mut p, ids) = standard();
    let opener = Opener::with(&[
        (ids.video.get(), Wave::Dc(0.5)),
        (ids.music.get(), Wave::Dc(0.25)),
    ]);
    let r = 48_000;
    let levels = |left: &[f32]| {
        [0.5, 2.5, 4.5].map(|s: f64| {
            let k = (s * f64::from(r)) as usize;
            left[k]
        })
    };
    let before = render(&p.schedule(), &opener, r, 5);
    assert_eq!(levels(&before), [0.5, 0.75, 0.0]);
    // Both clips start on their exact first samples.
    assert_eq!(before[0], 0.5);
    assert_eq!((before[47_999], before[48_000]), (0.5, 0.75));
    assert_eq!((before[191_999], before[192_000]), (0.75, 0.0));

    let a = p.detach(ids.v);
    let after = render(&p.schedule(), &opener, r, 5);
    assert!(after == before, "detaching changed the sound");

    p.mute_track(p.track_of(a));
    assert_eq!(
        levels(&render(&p.schedule(), &opener, r, 5)),
        [0.0, 0.25, 0.0]
    );

    p.set(ids.m, AudioEdit::Muted(true));
    assert!(
        render(&p.schedule(), &opener, r, 5)
            .iter()
            .all(|&v| v == 0.0)
    );
}

#[test]
fn clip_gain_and_fades_shape_the_output() {
    let (mut p, ids) = standard();
    p.set(ids.m, AudioEdit::Gain(GainDb::from_hundredths(-2_000)));
    p.set(ids.m, AudioEdit::FadeIn(secs(1.0)));
    let opener = Opener::with(&[(ids.music.get(), Wave::Dc(1.0))]);
    let left = render(&p.schedule(), &opener, 48_000, 5);
    let at = |s: f64| left[(s * 48_000.0) as usize];
    let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
    // The fade starts from silence on the clip's first sample.
    assert_eq!(at(1.0), 0.0);
    assert!(close(at(1.5), 0.05), "{}", at(1.5));
    assert!(close(at(3.0), 0.1), "{}", at(3.0));
    assert_eq!(at(4.0), 0.0);
}

/// The static volume of an input listed without keyframes.
fn static_gain(input: &MixInput) -> f32 {
    match input.gain {
        crate::GainEnvelope::Static(g) => g,
        crate::GainEnvelope::Keyframes(_) => panic!("keyframes on {input:?}"),
    }
}

// ---------------------------------------------------------------- v1 chain

impl Project {
    fn seq(&self) -> SeqId {
        self.doc().root()
    }

    fn keys(&mut self, clip: ClipId, edit: GainKeyEdit) {
        let seq = self.seq();
        self.apply(EditRequest::EditGainKeys {
            seq,
            clips: vec![clip],
            edit,
        });
    }

    fn stream(&mut self, clip: ClipId, sel: Option<StreamSel>) {
        let seq = self.seq();
        self.apply(EditRequest::SetAudioStream {
            seq,
            clips: vec![clip],
            sel,
        });
    }

    fn solo(&self, tracks: &[TrackId]) -> DocSchedule {
        self.schedule()
            .with_solo(Some(tracks.iter().copied().collect()))
    }
}

/// Volume keys on `clip` at source 3 s (0 dB, held) and 4 s (−20 dB): the
/// level steps from 1 to 0.1 at source time 4 s.
fn add_step_keys(p: &mut Project, clip: ClipId) {
    p.keys(
        clip,
        GainKeyEdit::Set {
            at: secs(3.0),
            gain: Some(GainDb::ZERO),
        },
    );
    p.keys(
        clip,
        GainKeyEdit::Set {
            at: secs(4.0),
            gain: Some(GainDb::from_hundredths(-2_000)),
        },
    );
    p.keys(
        clip,
        GainKeyEdit::Interp {
            at: secs(3.0),
            interp: Interp::Hold,
        },
    );
}

/// Renders `seconds` from 0 with the limiter off; interleaved stereo.
fn render_stereo(
    sched: &dyn Schedule,
    opener: &dyn SourceOpener,
    hz: u32,
    seconds: usize,
) -> Vec<[f32; 2]> {
    let mut mixer = Mixer::new(rate(hz));
    mixer.set_limiter(false);
    let mut out = vec![f32::NAN; hz as usize * seconds * 2];
    mixer.render(sched, opener, 0, &mut out);
    out.as_chunks::<2>().0.to_vec()
}

#[test]
fn keyframes_become_an_envelope_at_their_source_times() {
    let (mut p, ids) = standard();
    add_step_keys(&mut p, ids.m);
    p.keys(
        ids.m,
        GainKeyEdit::Set {
            at: secs(4.5),
            gain: Some(GainDb::SILENT),
        },
    );
    let inputs = p.schedule().inputs(span(0.0, 10.0));
    let m = input_of(&inputs, ids.m).unwrap();
    let GainEnvelope::Keyframes(keys) = &m.gain else {
        panic!("static gain on {m:?}");
    };
    assert_eq!(
        keys.keys(),
        [
            GainKey::new(secs(3.0), 0.0, KeyInterp::Hold),
            GainKey::new(secs(4.0), -20.0, KeyInterp::Linear),
            GainKey::new(secs(4.5), f32::NEG_INFINITY, KeyInterp::Linear),
        ]
    );
    // The clip without keys keeps a static volume.
    assert_eq!(static_gain(input_of(&inputs, ids.v).unwrap()), 1.0);
}

#[test]
fn keyframes_stay_on_their_samples_through_head_trims_and_splits() {
    let (mut p, ids) = standard();
    add_step_keys(&mut p, ids.m);
    let opener = Opener::with(&[(ids.music.get(), Wave::Dc(1.0))]);
    let r = 48_000usize;
    // The clip plays source 2 s at timeline 1 s: source 4 s is at 3 s.
    let k0 = 3 * r;
    let around = |p: &Project, what: &str| -> Vec<f32> {
        let left = render(&p.schedule(), &opener, r as u32, 5);
        let near = &left[k0 - 2_400..k0 + 2_400];
        let step = k0 - 2_400 + near.iter().position(|&v| v < 0.55).unwrap();
        assert!(
            step.abs_diff(k0) <= 1,
            "{what}: the step is at sample {step}, not {k0}"
        );
        // Outside the 10 ms smoothing the levels are the keys' own.
        assert!((left[k0 - 600] - 1.0).abs() < 1e-6, "{what}");
        assert!((left[k0 + 600] - 0.1).abs() < 1e-6, "{what}");
        near.to_vec()
    };
    let placed = around(&p, "as placed");

    // A head trim: the clip starts 0.5 s later, reading from 2.5 s.
    let seq = p.seq();
    p.apply(EditRequest::Trim(TrimReq {
        seq,
        clip: ids.m,
        edge: Edge::In,
        delta: secs(0.5),
        ripple: false,
        partners: Vec::new(),
    }));
    let m = p.doc().clip(ids.m).unwrap();
    assert_eq!(m.range.start(), secs(1.5));
    let trimmed = around(&p, "after a head trim");

    // A split before the key: the right piece plays it.
    let right = p.apply(EditRequest::Split(SplitReq {
        seq,
        at: secs(2.0),
        targets: SplitTargets::Clips(vec![ids.m]),
    }))[0];
    let inputs = p.schedule().inputs(span(2.0, 2.0));
    let piece = input_of(&inputs, right).unwrap();
    assert_eq!(
        (piece.range.start(), piece.source_start),
        (secs(2.0), secs(3.0))
    );
    let split = around(&p, "after a split");
    assert!(trimmed == placed, "a head trim moved the keys");
    assert!(split == placed, "a split moved the keys");
}

#[test]
fn fade_curves_carry_over() {
    let (mut p, ids) = standard();
    p.set(ids.m, AudioEdit::FadeIn(secs(0.5)));
    p.set(ids.m, AudioEdit::FadeInCurve(ModelCurve::Smooth));
    p.set(ids.m, AudioEdit::FadeOut(secs(1.0)));
    p.set(ids.m, AudioEdit::FadeOutCurve(ModelCurve::Fast));
    let inputs = p.schedule().inputs(span(0.0, 10.0));
    let m = input_of(&inputs, ids.m).unwrap();
    assert_eq!(
        (m.fade_in, m.fade_out),
        (
            Fade::new(secs(0.5), FadeCurve::Smooth),
            Fade::new(secs(1.0), FadeCurve::Fast)
        )
    );
    // Every model curve has its engine twin.
    for c in ModelCurve::ALL {
        assert_eq!(format!("{:?}", curve(c)), format!("{c:?}"));
    }
}

#[test]
fn stream_selection_and_all_streams_mixed() {
    let mut p = Project::new();
    let dual = p.import("dual.mp4", info_with(secs(10.0), true, &[1, 2]));
    let main = p.main();
    let v = p.place(
        InsertTarget::Track(main),
        0.0,
        dual,
        MediaKind::Video,
        span(0.0, 2.0),
    );
    let opener = Opener {
        streams: HashMap::from([
            ((dual.get(), 1), Wave::Dc(0.5)),
            ((dual.get(), 2), Wave::Dc(0.25)),
        ]),
        ..Opener::default()
    };
    let listed = |p: &Project| -> Vec<(u64, u32, f32)> {
        p.schedule()
            .inputs(span(0.0, 2.0))
            .iter()
            .map(|i| (i.key, i.source.stream, i.stream_mix))
            .collect()
    };
    let level = |p: &Project| render(&p.schedule(), &opener, 48_000, 1)[24_000];

    // The media default: the first stream, keyed by the clip.
    assert_eq!(listed(&p), [(v.get(), 1, 1.0)]);
    assert_eq!(level(&p), 0.5);

    // A chosen stream keeps the clip's key.
    p.stream(v, Some(StreamSel::Index(2)));
    assert_eq!(listed(&p), [(v.get(), 2, 1.0)]);
    assert_eq!(level(&p), 0.25);

    // All streams mixed: one input per stream at 1/√2, with its own key.
    p.stream(v, Some(StreamSel::AllMixed));
    let f = stream_mix_factor(2);
    let both = [
        (stream_input_key(v, 1), 1, f),
        (stream_input_key(v, 2), 2, f),
    ];
    assert_eq!(listed(&p), both);
    assert!(
        both.iter().all(|(k, ..)| *k != v.get()) && both[0].0 != both[1].0,
        "keys must be distinct"
    );
    let want = (0.5 + 0.25) * f;
    assert!((level(&p) - want).abs() < 1e-6, "{} vs {want}", level(&p));

    // Keys stay put through edits.
    p.set(v, AudioEdit::FadeOut(secs(0.5)));
    assert_eq!(listed(&p), both);
}

#[test]
fn stream_keys_never_collide_with_clip_ids() {
    let clip = |raw: u64| ClipId::from_raw(NonZeroU64::new(raw).unwrap());
    let largest = clip(freecut_timeline::ids::MAX_ID);
    let keys = [
        stream_input_key(clip(1), 0),
        stream_input_key(clip(1), 1),
        stream_input_key(clip(2), 0),
        stream_input_key(largest, 0),
        stream_input_key(largest, 511),
    ];
    for (i, a) in keys.iter().enumerate() {
        assert!(
            *a > freecut_timeline::ids::MAX_ID,
            "{a:#x} looks like a clip id"
        );
        assert!(keys[i + 1..].iter().all(|b| b != a), "{a:#x} repeats");
    }
}

#[test]
fn track_volume_and_pan_are_the_track_stage() {
    let (mut p, ids) = standard();
    let track = p.track_of(ids.m);
    let seq = p.seq();
    p.apply(EditRequest::SetTrackGain {
        seq,
        track,
        gain: GainDb::from_hundredths(-602),
    });
    p.apply(EditRequest::SetTrackPan {
        seq,
        track,
        pan: Pan::new(50),
    });
    let inputs = p.schedule().inputs(span(0.0, 10.0));
    let m = input_of(&inputs, ids.m).unwrap();
    assert_eq!((m.track.id, m.track.pan), (track.get(), 0.5));
    assert!((m.track.gain - 0.5).abs() < 1e-3, "{}", m.track.gain);
    // Other tracks keep unity.
    let v = input_of(&inputs, ids.v).unwrap();
    assert_eq!((v.track.gain, v.track.pan), (1.0, 0.0));

    let opener = Opener::with(&[(ids.music.get(), Wave::Dc(1.0))]);
    let [l, r] = render_stereo(&p.schedule(), &opener, 48_000, 3)[2 * 48_000];
    // Balance law at +50: the left side at half, the right at unity.
    assert!((l - 0.25).abs() < 1e-3 && (r - 0.5).abs() < 1e-3, "{l} {r}");
}

#[test]
fn solo_hears_only_soloed_tracks() {
    let (mut p, ids) = standard();
    let (vt, mt) = (p.track_of(ids.v), p.track_of(ids.m));
    let all = span(0.0, 10.0);
    let both = [ids.v.get(), ids.m.get()];
    assert_eq!(keys(&p.solo(&[mt]).inputs(all)), [ids.m.get()]);
    assert_eq!(keys(&p.solo(&[vt]).inputs(all)), [ids.v.get()]);
    assert_eq!(keys(&p.solo(&[vt, mt]).inputs(all)), both);
    assert_eq!(p.solo(&[mt]).solo(), Some(&BTreeSet::from([mt])));

    // Nothing soloed: no set, an empty one, or only tracks that are gone.
    let gone = TrackId::from_raw(NonZeroU64::new(999_999).unwrap());
    for sched in [
        p.schedule(),
        p.schedule().with_solo(None),
        p.solo(&[]),
        p.solo(&[gone]),
    ] {
        assert!(sched.solo().is_none());
        assert_eq!(keys(&sched.inputs(all)), both);
    }
    assert_eq!(p.solo(&[mt, gone]).solo(), Some(&BTreeSet::from([mt])));
    assert_eq!(keys(&p.solo(&[mt]).without_solo().inputs(all)), both);

    // Heard as such: only the music at 2.5 s.
    let opener = Opener::with(&[
        (ids.video.get(), Wave::Dc(0.5)),
        (ids.music.get(), Wave::Dc(0.25)),
    ]);
    let at = |sched: &DocSchedule| render(sched, &opener, 48_000, 3)[120_000];
    assert_eq!(at(&p.schedule()), 0.75);
    assert_eq!(at(&p.solo(&[mt])), 0.25);

    // A soloed track that is muted is silent.
    p.mute_track(mt);
    assert!(p.solo(&[mt]).inputs(all).is_empty());
    assert_eq!(keys(&p.solo(&[mt, vt]).inputs(all)), [ids.v.get()]);
}

#[test]
fn export_ignores_solo_unless_exported_as_heard() {
    let (p, ids) = standard();
    let sched = p.solo(&[p.track_of(ids.m)]);
    let opener = Opener::with(&[
        (ids.video.get(), Wave::Dc(0.5)),
        (ids.music.get(), Wave::Dc(0.25)),
    ]);
    let fps = FrameRate::new(30, 1).unwrap();
    let clock = ExportClock::new(span(0.0, 3.0), fps).unwrap();
    let never = || false;
    let export = |as_heard: bool| {
        let mut mix = OfflineMix::for_document(&sched, &clock, rate(48_000)).unwrap();
        mix.set_limiter(false);
        mix.set_export_as_heard(as_heard);
        assert_eq!(mix.exports_as_heard(), as_heard);
        mix.render_to_vec(&sched, &opener, &never).unwrap()
    };
    // Frame 2.5 s, left channel.
    assert_eq!(export(false)[2 * 120_000], 0.75, "solo is ignored");
    assert_eq!(export(true)[2 * 120_000], 0.25, "exported as heard");
    // Preview hears the solo.
    assert_eq!(render(&sched, &opener, 48_000, 3)[120_000], 0.25);
}
