//! Offline export mixing against synthetic sources (no FFmpeg, no device).

use std::cell::Cell;

use freecut_timeline::edit::{EditCtx, EditRequest, InsertReq, InsertTarget, NewClip, NewMedia};
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::model::{AudioStreamInfo, GainDb, MediaInfo, MediaKind, StoredPath};
use freecut_timeline::time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};
use freecut_timeline::{CommitMeta, Editor};

use super::*;
use crate::testing::{List, Opener, Wave, input, rate, secs};

fn ntsc() -> FrameRate {
    FrameRate::new(30_000, 1001).unwrap()
}

fn clock(start: Tick, duration: Tick, fps: FrameRate) -> ExportClock {
    ExportClock::new(TimeRange::new(start, duration).unwrap(), fps).unwrap()
}

fn never() -> bool {
    false
}

/// One quiet sine (below the limiter's threshold) over the first minute.
fn quiet_sine() -> (List, Opener) {
    (
        List(vec![input(1, 7, Tick::ZERO, secs(60.0), Tick::ZERO)]),
        Opener::with(&[(
            7,
            Wave::Sine {
                amp: 0.5,
                hz: 440.0,
            },
        )]),
    )
}

fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|v| v.to_bits()).collect()
}

#[test]
fn the_range_has_exactly_the_clocks_sample_count() {
    let (schedule, opener) = quiet_sine();
    let f = ntsc().frame_ticks();
    let frames = |n: i64| Tick::from_raw(f.raw() * n);
    for hz in [44_100, 48_000] {
        let r = rate(hz);
        for (start, len) in [(0, 1), (7, 333), (340, 667), (1, 30)] {
            let c = clock(frames(start), frames(len), ntsc());
            let mut mix = OfflineMix::new(&c, r).unwrap().with_block_frames(1000);
            assert_eq!(mix.total_frames(), c.sample_count(r).unwrap());
            assert_eq!(mix.first_sample(), sample_at(frames(start), r));
            let out = mix.render_to_vec(&schedule, &opener, &never).unwrap();
            assert_eq!(out.len() as u64, 2 * c.sample_count(r).unwrap());
            assert!(mix.is_done());
            assert!(mix.next_block(&schedule, &opener).is_none());
        }
    }
}

#[test]
fn adjacent_ranges_tile_into_the_whole() {
    let (schedule, opener) = quiet_sine();
    let f = ntsc().frame_ticks().raw();
    let r = rate(44_100);
    let range =
        |start: i64, len: i64| clock(Tick::from_raw(start * f), Tick::from_raw(len * f), ntsc());
    let (whole, a, b) = (range(7, 1000), range(7, 333), range(340, 667));
    let mix = |c: &ExportClock| {
        let mut m = OfflineMix::new(c, r).unwrap();
        let first = m.first_sample();
        (first, m.render_to_vec(&schedule, &opener, &never).unwrap())
    };
    let ((w0, w), (a0, av), (b0, bv)) = (mix(&whole), mix(&a), mix(&b));
    assert_eq!(a0, w0);
    assert_eq!(b0, a0 + (av.len() / 2) as i64, "no gap, no overlap");
    assert_eq!(av.len() + bv.len(), w.len());
    let joined: Vec<f32> = av.iter().chain(&bv).copied().collect();
    assert_eq!(bits(&joined), bits(&w));
}

#[test]
fn a_clip_at_one_second_starts_at_sample_rate_times_one() {
    let schedule = List(vec![input(1, 7, secs(1.0), secs(1.0), Tick::ZERO)]);
    let opener = Opener::with(&[(7, Wave::Dc(0.5))]);
    for hz in [44_100, 48_000] {
        let r = rate(hz);
        let sr = hz as usize;
        let c = clock(Tick::ZERO, secs(3.0), ntsc());
        let out = OfflineMix::new(&c, r)
            .unwrap()
            .with_block_frames(777)
            .render_to_vec(&schedule, &opener, &never)
            .unwrap();
        assert_eq!(out.len(), 2 * 3 * sr);
        assert_eq!(out[2 * (sr - 1)], 0.0, "{hz} Hz: silent just before 1 s");
        assert_eq!(
            (out[2 * sr], out[2 * sr + 1]),
            (0.5, 0.5),
            "{hz} Hz: on at 1 s"
        );
        assert_eq!(out[2 * (2 * sr - 1)], 0.5, "{hz} Hz: still on before 2 s");
        assert_eq!(out[2 * 2 * sr], 0.0, "{hz} Hz: off at 2 s");
        // Exported from 0.25 s, the clip starts 0.75 s into the output.
        let late = clock(secs(0.25), secs(2.0), ntsc());
        let out = OfflineMix::new(&late, r)
            .unwrap()
            .render_to_vec(&schedule, &opener, &never)
            .unwrap();
        let k = sr * 3 / 4;
        assert_eq!((out[2 * (k - 1)], out[2 * k]), (0.0, 0.5), "{hz} Hz");
    }
}

#[test]
fn blocks_give_the_same_samples_as_one_mixer_render() {
    let r = rate(48_000);
    let mut fading = input(2, 8, secs(0.7), secs(1.5), secs(3.0));
    fading.gain = 0.7.into();
    fading.fade_in = secs(0.3).into();
    fading.fade_out = secs(0.2).into();
    let c = clock(secs(0.1), secs(2.5), FrameRate::new(25, 1).unwrap());
    for (limiter, amp) in [(true, 1.6_f32), (false, 0.9)] {
        let schedule = List(vec![
            input(1, 7, Tick::ZERO, secs(2.0), secs(0.5)),
            fading.clone(),
        ]);
        let opener = Opener::with(&[
            (7, Wave::Sine { amp, hz: 440.0 }),
            (8, Wave::Sine { amp: 0.8, hz: 97.0 }),
        ]);
        let mut mix = OfflineMix::new(&c, r).unwrap().with_block_frames(1000);
        mix.set_limiter(limiter);
        mix.set_master_gain(0.8);
        let blocks = mix.render_to_vec(&schedule, &opener, &never).unwrap();

        let mut mixer = Mixer::new(r);
        mixer.set_limiter(limiter);
        mixer.set_master_gain(0.8);
        let total = usize::try_from(c.sample_count(r).unwrap()).unwrap();
        let mut once = vec![0.0; 2 * total];
        mixer.render(&schedule, &opener, mix.first_sample(), &mut once);
        assert_eq!(bits(&blocks), bits(&once), "limiter {limiter}");
        assert!(blocks.iter().any(|v| *v != 0.0));
    }
}

#[test]
fn sources_are_read_sequentially() {
    let (schedule, opener) = quiet_sine();
    let c = clock(secs(1.0), secs(1.0), FrameRate::new(25, 1).unwrap());
    let mut mix = OfflineMix::new(&c, rate(48_000))
        .unwrap()
        .with_block_frames(4800);
    mix.render_to_vec(&schedule, &opener, &never).unwrap();
    assert_eq!(opener.opens(), 1, "one reader for the whole range");
    // Every read continues where the previous one ended.
    let reads = opener.reads();
    for pair in reads.windows(2) {
        let (_, at, frames) = pair[0];
        let next = at.raw() + frames as i64 * (TICKS_PER_SECOND / 48_000);
        assert_eq!(pair[1].1.raw(), next, "{reads:?}");
    }
}

#[test]
fn cancelling_stops_between_blocks() {
    let (schedule, opener) = quiet_sine();
    let c = clock(Tick::ZERO, secs(2.0), FrameRate::new(25, 1).unwrap());
    let mut mix = OfflineMix::new(&c, rate(48_000))
        .unwrap()
        .with_block_frames(1000);
    let checks = Cell::new(0);
    let cancelled = || {
        checks.set(checks.get() + 1);
        checks.get() > 3
    };
    let mut handed = 0;
    let result = mix.render_all(&schedule, &opener, &cancelled, &mut |b| handed += b.len());
    assert_eq!(result, Err(OfflineMixError::Cancelled));
    assert_eq!(mix.rendered_frames(), 3000);
    assert_eq!(handed, 6000);
    assert!(!mix.is_done());
}

#[test]
fn a_range_shorter_than_a_sample_is_empty() {
    let (schedule, opener) = quiet_sine();
    let c = clock(
        Tick::from_raw(1),
        Tick::from_raw(1),
        FrameRate::new(25, 1).unwrap(),
    );
    let mut mix = OfflineMix::new(&c, rate(48_000)).unwrap();
    assert_eq!(mix.total_frames(), 0);
    assert!(mix.next_block(&schedule, &opener).is_none());
    assert!(
        mix.render_to_vec(&schedule, &opener, &never)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_documents_master_volume_applies() {
    let mut editor = Editor::new(HistoryLimits::default()).unwrap();
    let commit = |editor: &mut Editor, req: EditRequest| {
        let plan = editor.plan(&EditCtx::default(), &req).unwrap();
        let imported = plan.imported().first().copied();
        editor.commit(plan, CommitMeta::default()).unwrap();
        imported
    };
    let info = MediaInfo {
        container: "wav".into(),
        default_video: None,
        duration: secs(10.0),
        video: Vec::new(),
        audio: vec![AudioStreamInfo {
            index: 0,
            codec: "pcm_s16le".into(),
            sample_rate: 48_000,
            channels: 2,
            start: Tick::ZERO,
            duration: secs(10.0),
            duration_exact: true,
            default: true,
            language: None,
            title: None,
            layout: None,
        }],
        still: None,
    };
    let music = commit(
        &mut editor,
        EditRequest::ImportMedia(vec![NewMedia {
            path: StoredPath {
                absolute: "/media/music.wav".into(),
                relative: None,
            },
            info,
            fingerprint: None,
            display_name: None,
        }]),
    )
    .unwrap();
    let seq = editor.document().root();
    commit(
        &mut editor,
        EditRequest::Insert(InsertReq {
            seq,
            target: InsertTarget::FirstFree,
            at: secs(1.0),
            clips: vec![NewClip::media(
                music,
                MediaKind::Audio,
                TimeRange::new(Tick::ZERO, secs(2.0)).unwrap(),
            )],
        }),
    );
    // −6.02 dB ≈ half amplitude.
    commit(
        &mut editor,
        EditRequest::SetMasterGain {
            seq,
            gain: GainDb::from_hundredths(-602),
        },
    );
    let schedule = DocSchedule::root(editor.document().clone());
    assert!((schedule.master_amplitude() - 0.5).abs() < 1e-3);
    let opener = Opener::with(&[(music.get(), Wave::Dc(0.5))]);
    let c = clock(Tick::ZERO, secs(4.0), FrameRate::new(30, 1).unwrap());
    let out = OfflineMix::for_document(&schedule, &c, rate(48_000))
        .unwrap()
        .render_to_vec(&schedule, &opener, &never)
        .unwrap();
    assert_eq!(out.len(), 2 * 4 * 48_000);
    assert_eq!(out[2 * 47_999], 0.0);
    assert!((out[2 * 48_000] - 0.25).abs() < 1e-3, "{}", out[2 * 48_000]);
}
