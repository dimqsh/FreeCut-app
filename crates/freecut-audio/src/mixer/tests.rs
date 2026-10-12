use freecut_timeline::time::Tick;

use super::*;
use crate::envelope::{GainEnvelope, GainKey, GainKeys, KeyInterp};
use crate::gain::{Fade, FadeCurve};
use crate::source::{MixInput, Silence, TrackStage};
use crate::testing::{List, Opener, Wave, input, rate, secs};

/// Renders `[start, start + frames)` in one call.
fn render(
    mixer: &mut Mixer,
    schedule: &List,
    opener: &Opener,
    start: i64,
    frames: usize,
) -> Vec<f32> {
    let mut out = vec![f32::NAN; frames * 2];
    mixer.render(schedule, opener, start, &mut out);
    out
}

/// A mixer with the limiter off, so test signals come through bit-exact.
fn raw_mixer(hz: u32) -> Mixer {
    let mut m = Mixer::new(rate(hz));
    m.set_limiter(false);
    m
}

fn left(out: &[f32], frame: usize) -> f32 {
    out[frame * 2]
}

/// `n` samples at 48 kHz.
fn samples(n: i64) -> Tick {
    Tick::from_raw(n * 14_700)
}

#[test]
fn empty_timeline_is_silent() {
    let mut m = Mixer::new(rate(48_000));
    let mut out = vec![1.0; 2001];
    m.render(&Silence, &Silence, 0, &mut out);
    assert!(out.iter().all(|&v| v == 0.0));
}

#[test]
fn clip_lands_on_the_exact_samples() {
    for hz in [48_000, 44_100] {
        let r = i64::from(hz);
        let mut m = raw_mixer(hz);
        // Clip at [1 s, 2 s) playing media from 0.5 s.
        let sched = List(vec![input(1, 7, secs(1.0), secs(1.0), secs(0.5))]);
        let opener = Opener::with(&[(7, Wave::Counter)]);
        let out = render(&mut m, &sched, &opener, 0, 3 * hz as usize);
        for k in 0..3 * r {
            let expected = if (r..2 * r).contains(&k) {
                (r / 2 + k - r) as f32
            } else {
                0.0
            };
            assert_eq!(left(&out, k as usize), expected, "{hz} Hz, sample {k}");
            assert_eq!(out[k as usize * 2 + 1], -expected);
        }
    }
}

#[test]
fn off_grid_edges_round_down_to_samples() {
    let mut m = raw_mixer(48_000);
    let st = 14_700;
    // Starts 1 tick after sample 10 begins, ends 1 tick into sample 20.
    let start = Tick::from_raw(10 * st + 1);
    let dur = Tick::from_raw(10 * st);
    let sched = List(vec![input(1, 7, start, dur, Tick::from_raw(100 * st))]);
    let opener = Opener::with(&[(7, Wave::Counter)]);
    let out = render(&mut m, &sched, &opener, 0, 30);
    let lefts: Vec<f32> = (0..30).map(|k| left(&out, k)).collect();
    let mut expected = vec![0.0; 30];
    for (k, e) in expected.iter_mut().enumerate().take(20).skip(10) {
        *e = (100 + k - 10) as f32;
    }
    assert_eq!(lefts, expected);
}

#[test]
fn odd_block_sizes_match_one_block() {
    // Overlapping sines with fades, loud enough to drive the limiter, and a
    // clip starting mid-way through, at a start that isn't sample 0.
    let mut a = input(1, 1, secs(0.0), secs(1.0), secs(0.0));
    a.gain = 2.5.into();
    a.fade_in = secs(0.1).into();
    a.fade_out = secs(0.3).into();
    let mut b = input(2, 2, secs(0.37), secs(0.5), secs(2.0));
    b.fade_in = secs(0.05).into();
    let sched = List(vec![a, b]);
    let opener = Opener::with(&[
        (
            1,
            Wave::Sine {
                amp: 0.8,
                hz: 440.0,
            },
        ),
        (
            2,
            Wave::Sine {
                amp: 0.6,
                hz: 1234.5,
            },
        ),
    ]);
    let start = 1_000;
    let total = 48_000;

    let mut whole = Mixer::new(rate(48_000));
    let expected = render(&mut whole, &sched, &opener, start, total);

    let mut pieces = Mixer::new(rate(48_000));
    let mut got = Vec::new();
    let sizes = [1, 7, 13, 240, 241, 997, 4096, 3];
    let mut pos = 0;
    let mut i = 0;
    while pos < total {
        let n = sizes[i % sizes.len()].min(total - pos);
        got.extend(render(&mut pieces, &sched, &opener, start + pos as i64, n));
        pos += n;
        i += 1;
    }
    assert_eq!(got.len(), expected.len());
    let first_diff = got.iter().zip(&expected).position(|(a, b)| a != b);
    assert_eq!(first_diff, None);
    // The limiter actually worked on this signal.
    let peak = expected.iter().fold(0f32, |m, v| m.max(v.abs()));
    assert!(peak <= 0.891_251 && peak > 0.85, "peak {peak}");
}

#[test]
fn overlapping_inputs_sum() {
    let mut m = raw_mixer(48_000);
    let sched = List(vec![
        input(1, 1, secs(0.0), secs(1.0), Tick::ZERO),
        input(2, 2, secs(0.5), secs(1.0), Tick::ZERO),
    ]);
    let opener = Opener::with(&[(1, Wave::Dc(0.25)), (2, Wave::Dc(0.5))]);
    let out = render(&mut m, &sched, &opener, 0, 96_000);
    assert_eq!(left(&out, 0), 0.25);
    assert_eq!(left(&out, 23_999), 0.25);
    assert_eq!(left(&out, 24_000), 0.75);
    assert_eq!(left(&out, 47_999), 0.75);
    assert_eq!(left(&out, 48_000), 0.5);
    assert_eq!(left(&out, 71_999), 0.5);
    assert_eq!(left(&out, 72_000), 0.0);
}

#[test]
fn gain_and_fades_shape_the_clip() {
    let mut m = raw_mixer(48_000);
    let mut i = input(1, 1, secs(0.0), secs(1.0), Tick::ZERO);
    i.gain = 0.5.into();
    i.fade_in = secs(0.5).into();
    i.fade_out = secs(0.25).into();
    let sched = List(vec![i]);
    let opener = Opener::with(&[(1, Wave::Dc(1.0))]);
    let out = render(&mut m, &sched, &opener, 0, 48_000);
    assert_eq!(left(&out, 0), 0.0);
    assert_eq!(left(&out, 12_000), 0.25);
    assert_eq!(left(&out, 24_000), 0.5);
    assert_eq!(left(&out, 30_000), 0.5);
    assert_eq!(left(&out, 41_999), 0.25);
    assert_eq!(left(&out, 47_999), 0.0);
}

#[test]
fn readers_are_reused_sequentially() {
    let mut m = raw_mixer(48_000);
    let sched = List(vec![input(1, 7, secs(0.0), secs(10.0), secs(3.0))]);
    let opener = Opener::with(&[(7, Wave::Counter)]);
    let mut next = 0;
    for n in [480, 512, 100, 1, 4000] {
        render(&mut m, &sched, &opener, next, n);
        next += n as i64;
    }
    assert_eq!(opener.opens(), 1);
    // Every read continues where the previous one ended.
    let reads = opener.reads();
    for w in reads.windows(2) {
        let (_, at, frames) = w[0];
        assert_eq!(w[1].1.raw(), at.raw() + frames as i64 * 14_700);
    }
    assert_eq!(reads[0].1, secs(3.0));

    // A jump seeks the same reader.
    render(&mut m, &sched, &opener, 48_000 * 5, 100);
    assert_eq!(opener.opens(), 1);
    let (_, at, _) = opener.reads()[reads.len()];
    assert_eq!(at, secs(8.0));
}

#[test]
fn readers_are_dropped_when_unscheduled() {
    let mut m = raw_mixer(48_000);
    let sched = List(vec![input(1, 7, secs(0.0), secs(0.01), Tick::ZERO)]);
    let opener = Opener::with(&[(7, Wave::Dc(0.1))]);
    render(&mut m, &sched, &opener, 0, 240);
    assert_eq!((opener.opens(), opener.drops()), (1, 0));
    render(&mut m, &sched, &opener, 240, 960);
    assert_eq!(opener.drops(), 1);
}

#[test]
fn missing_sources_are_silent() {
    let mut m = raw_mixer(48_000);
    let sched = List(vec![
        input(1, 404, secs(0.0), secs(1.0), Tick::ZERO),
        input(2, 2, secs(0.0), secs(1.0), Tick::ZERO),
    ]);
    let opener = Opener::with(&[(2, Wave::Dc(0.5))]);
    let mut out = Vec::new();
    for k in 0..10 {
        out.extend(render(&mut m, &sched, &opener, k * 480, 480));
    }
    assert!(out.iter().all(|&v| v == 0.5));
    // The offline source was tried once (plus the real one), not every block.
    assert_eq!(opener.opens(), 2);
    // A jump tries it again.
    render(&mut m, &sched, &opener, 20_000, 480);
    assert_eq!(opener.opens(), 3);
}

#[test]
fn read_errors_play_silence_and_retry_after_a_jump() {
    let mut m = raw_mixer(48_000);
    let sched = List(vec![
        input(1, 1, secs(0.0), secs(1.0), Tick::ZERO),
        input(2, 2, secs(0.0), secs(1.0), Tick::ZERO),
    ]);
    let opener = Opener::with(&[(1, Wave::Broken), (2, Wave::Dc(0.25))]);
    let out = render(&mut m, &sched, &opener, 0, 480);
    assert!(out.iter().all(|&v| v == 0.25));
    render(&mut m, &sched, &opener, 480, 480);
    assert_eq!(opener.opens(), 2);
    render(&mut m, &sched, &opener, 10_000, 480);
    assert_eq!(opener.opens(), 3);
}

#[test]
fn silent_inputs_are_not_read() {
    let mut m = raw_mixer(48_000);
    let mut i = input(1, 1, secs(0.0), secs(1.0), Tick::ZERO);
    i.gain = 0.0.into();
    let opener = Opener::with(&[(1, Wave::Dc(1.0))]);
    let out = render(&mut m, &List(vec![i]), &opener, 0, 480);
    assert!(out.iter().all(|&v| v == 0.0));
    assert_eq!(opener.opens(), 0);
}

#[test]
fn a_changed_source_reopens_the_reader() {
    let mut m = raw_mixer(48_000);
    let opener = Opener::with(&[(1, Wave::Dc(0.25)), (2, Wave::Dc(0.5))]);
    let a = List(vec![input(9, 1, secs(0.0), secs(1.0), Tick::ZERO)]);
    let b = List(vec![input(9, 2, secs(0.0), secs(1.0), Tick::ZERO)]);
    assert_eq!(left(&render(&mut m, &a, &opener, 0, 480), 0), 0.25);
    // Mixing runs 240 samples (the limiter's look-ahead) ahead of the output.
    assert_eq!(left(&render(&mut m, &b, &opener, 480, 480), 240), 0.5);
    assert_eq!((opener.opens(), opener.drops()), (2, 1));
}

#[test]
fn limiter_holds_the_ceiling_on_overs() {
    let mut m = Mixer::new(rate(48_000));
    let sched = List(
        (0..3)
            .map(|k| input(k, 1, secs(0.0), secs(1.0), Tick::ZERO))
            .collect(),
    );
    let opener = Opener::with(&[(
        1,
        Wave::Sine {
            amp: 0.7,
            hz: 220.0,
        },
    )]);
    let out = render(&mut m, &sched, &opener, 0, 48_000);
    let peak = out.iter().fold(0f32, |m, v| m.max(v.abs()));
    assert!(peak <= 0.891_251, "peak {peak}");
    assert!(peak > 0.85);
}

#[test]
fn master_gain_scales_and_ramps() {
    let mut m = raw_mixer(48_000);
    let sched = List(vec![input(1, 1, secs(0.0), secs(10.0), Tick::ZERO)]);
    let opener = Opener::with(&[(1, Wave::Dc(0.5))]);
    // Set before rendering: applies at once (no ramp from a seek).
    m.set_master_gain(0.5);
    let out = render(&mut m, &sched, &opener, 0, 480);
    assert_eq!(left(&out, 0), 0.25);
    // Changed mid-stream: ramps by 1/480 per sample (full scale in 10 ms).
    // The first 240 samples (the limiter's look-ahead) were already mixed.
    m.set_master_gain(1.0);
    let out = render(&mut m, &sched, &opener, 480, 960);
    let l: Vec<f32> = (0..960).map(|k| left(&out, k)).collect();
    assert_eq!(l[239], 0.25);
    assert!(l[240] > 0.25 && l[240] < 0.26);
    assert!(l.windows(2).all(|w| w[1] >= w[0]));
    assert_eq!(l[600], 0.5);
    m.set_master_gain(f32::NAN);
    let out = render(&mut m, &sched, &opener, 99_999, 10);
    assert!(out.iter().all(|&v| v == 0.0));
}

/// The left channel of `out`.
fn lefts(out: &[f32]) -> Vec<f32> {
    out.iter().step_by(2).copied().collect()
}

/// The largest change between two neighbouring samples.
fn largest_step(l: &[f32]) -> f32 {
    l.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

#[test]
fn a_clip_volume_change_ramps_over_10_ms() {
    let mut m = raw_mixer(48_000);
    let opener = Opener::with(&[(1, Wave::Dc(0.5))]);
    let loud = input(1, 1, secs(0.0), secs(10.0), Tick::ZERO);
    let quiet = MixInput {
        gain: 0.1.into(),
        ..loud.clone()
    };
    assert_eq!(
        left(&render(&mut m, &List(vec![loud]), &opener, 0, 480), 0),
        0.5
    );
    let l = lefts(&render(&mut m, &List(vec![quiet]), &opener, 480, 1440));
    // The look-ahead (240 samples) was mixed at the old volume.
    assert_eq!(l[239], 0.5);
    assert!(l[240] < 0.5 && l[240] > 0.49, "{}", l[240]);
    // 0.45 over 480 samples, never a step.
    assert!(largest_step(&l) < 0.002, "{}", largest_step(&l));
    assert!(l.windows(2).all(|w| w[1] <= w[0]));
    assert!((l[240 + 480] - 0.05).abs() < 1e-6, "{}", l[240 + 480]);
    assert_eq!(opener.opens(), 1);
}

#[test]
fn a_clip_leaving_the_mix_mid_way_ramps_out_and_one_joining_ramps_in() {
    let mut m = raw_mixer(48_000);
    let opener = Opener::with(&[(1, Wave::Dc(0.5))]);
    let clip = List(vec![input(1, 1, secs(0.0), secs(10.0), Tick::ZERO)]);
    let muted = List(Vec::new());
    render(&mut m, &clip, &opener, 0, 480);
    // Muted (gone from the schedule) while sounding: ramps out, then stops
    // reading.
    let l = lefts(&render(&mut m, &muted, &opener, 480, 1440));
    assert_eq!(l[239], 0.5);
    assert!(largest_step(&l) < 0.002, "{}", largest_step(&l));
    assert_eq!(l[240 + 480], 0.0);
    assert_eq!(*l.last().unwrap(), 0.0);
    assert_eq!(opener.drops(), 1);
    // Unmuted mid-clip: a new reader ramps in.
    let l = lefts(&render(&mut m, &clip, &opener, 1920, 1440));
    assert_eq!(l[239], 0.0);
    assert!(largest_step(&l) < 0.002, "{}", largest_step(&l));
    assert!((l[240 + 480] - 0.5).abs() < 1e-6, "{}", l[240 + 480]);
    // A clip that ends where it is placed stops on its last sample.
    let short = List(vec![input(2, 1, secs(0.0), secs(0.1), Tick::ZERO)]);
    let l = lefts(&render(&mut raw_mixer(48_000), &short, &opener, 0, 9_600));
    assert_eq!((l[4_799], l[4_800]), (0.5, 0.0));
}

#[test]
fn a_silenced_clip_ramps_down_then_stops_reading() {
    let mut m = raw_mixer(48_000);
    let opener = Opener::with(&[(1, Wave::Dc(0.5))]);
    let on = input(1, 1, secs(0.0), secs(10.0), Tick::ZERO);
    let off = MixInput {
        gain: 0.0.into(),
        ..on.clone()
    };
    render(&mut m, &List(vec![on]), &opener, 0, 480);
    let l = lefts(&render(
        &mut m,
        &List(vec![off.clone()]),
        &opener,
        480,
        1440,
    ));
    assert!(largest_step(&l) < 0.002, "{}", largest_step(&l));
    assert_eq!(l[240 + 480], 0.0);
    let reads = opener.reads().len();
    render(&mut m, &List(vec![off]), &opener, 1920, 480);
    assert_eq!(opener.reads().len(), reads, "a silent clip isn't read");
}

#[test]
fn deferred_inputs_never_open_and_play_once_a_reader_is_handed_in() {
    let mut m = raw_mixer(48_000);
    m.defer_opens();
    let opener = Opener::with(&[(1, Wave::Counter)]);
    // A clip from sample 1000, playing media from sample 0.
    let sched = List(vec![input(1, 1, samples(1_000), secs(10.0), Tick::ZERO)]);
    let key = SourceKey {
        asset: 1,
        stream: 0,
    };
    assert_eq!(m.reader_state(1, &key), ReaderState::Missing);
    let out = render(&mut m, &sched, &opener, 0, 2_000);
    assert!(out.iter().all(|&v| v == 0.0));
    assert_eq!(opener.opens(), 0, "the mixer never opens a reader itself");

    // Handed in positioned at media sample 2000 (timeline sample 3000):
    // silent until there, then exact.
    let reader = opener.open(&key, rate(48_000)).unwrap();
    m.offer(1, key, samples(2_000), Some(reader));
    assert_eq!(
        m.reader_state(1, &key),
        ReaderState::Prepared(samples(2_000))
    );
    let l = lefts(&render(&mut m, &sched, &opener, 2_000, 2_000));
    assert_eq!(l[999], 0.0);
    assert!(
        l[1_000] > 0.0 && l[1_000] < 2_000.0,
        "ramps in: {}",
        l[1_000]
    );
    assert_eq!(l[1_000 + 480], 2_480.0);
    assert_eq!(m.reader_state(1, &key), ReaderState::Held);
    // Every read after the hand-over is sequential.
    let reads = opener.reads();
    for w in reads.windows(2) {
        assert_eq!(w[1].1.raw(), w[0].1.raw() + w[0].2 as i64 * 14_700);
    }
}

#[test]
fn a_reader_handed_in_at_the_clip_start_plays_its_first_sample_exactly() {
    let mut m = raw_mixer(48_000);
    m.defer_opens();
    let opener = Opener::with(&[(1, Wave::Counter)]);
    let sched = List(vec![input(1, 1, samples(1_000), secs(10.0), samples(50))]);
    let key = SourceKey {
        asset: 1,
        stream: 0,
    };
    let mut reader = opener.open(&key, rate(48_000)).unwrap();
    reader.prepare(samples(50)).unwrap();
    m.offer(1, key, samples(50), Some(reader));
    let l = lefts(&render(&mut m, &sched, &opener, 0, 2_000));
    assert_eq!((l[999], l[1_000], l[1_001]), (0.0, 50.0, 51.0));
}

#[test]
fn a_replaced_opener_keeps_old_readers_until_new_ones_take_over() {
    let mut m = raw_mixer(48_000);
    m.defer_opens();
    let old = Opener::with(&[(1, Wave::Dc(0.25))]);
    let new = Opener::with(&[(1, Wave::Dc(0.5))]);
    let sched = List(vec![input(1, 1, Tick::ZERO, secs(10.0), Tick::ZERO)]);
    let key = SourceKey {
        asset: 1,
        stream: 0,
    };
    m.offer(1, key, Tick::ZERO, old.open(&key, rate(48_000)));
    assert_eq!(left(&render(&mut m, &sched, &old, 0, 480), 0), 0.25);
    m.invalidate_readers();
    assert_eq!(m.reader_state(1, &key), ReaderState::Missing);
    // The new reader is positioned at sample 2400; the old one plays on
    // until the block that reaches it.
    m.offer(1, key, samples(2_400), new.open(&key, rate(48_000)));
    let mut l = Vec::new();
    for k in 1..10 {
        l.extend(lefts(&render(&mut m, &sched, &old, k * 480, 480)));
    }
    assert!(l.iter().all(|&v| v == 0.25 || v == 0.5), "no gap, no ramp");
    assert_eq!(l[0], 0.25);
    assert_eq!(*l.last().unwrap(), 0.5);
    assert_eq!(old.drops(), 1);
}

#[test]
fn a_source_switched_during_playback_plays_on_until_the_new_reader_takes_over() {
    let mut m = raw_mixer(48_000);
    m.defer_opens();
    let opener = Opener::with(&[(1, Wave::Dc(0.25)), (2, Wave::Dc(0.5))]);
    let a = List(vec![input(9, 1, Tick::ZERO, secs(10.0), Tick::ZERO)]);
    let b = List(vec![input(9, 2, Tick::ZERO, secs(10.0), Tick::ZERO)]);
    let (one, two) = (
        SourceKey {
            asset: 1,
            stream: 0,
        },
        SourceKey {
            asset: 2,
            stream: 0,
        },
    );
    m.offer(9, one, Tick::ZERO, opener.open(&one, rate(48_000)));
    assert_eq!(left(&render(&mut m, &a, &opener, 0, 480), 0), 0.25);
    // The other stream is picked: its reader is wanted, and comes
    // positioned ahead of the mix, as the playback planner asks for it.
    let mut l = lefts(&render(&mut m, &b, &opener, 480, 480));
    assert_eq!(m.reader_state(9, &two), ReaderState::Missing);
    m.offer(9, two, samples(3_600), opener.open(&two, rate(48_000)));
    for k in 2..12 {
        l.extend(lefts(&render(&mut m, &b, &opener, k * 480, 480)));
    }
    assert!(l.iter().all(|&v| v == 0.25 || v == 0.5), "no gap");
    assert_eq!(l[0], 0.25);
    assert_eq!(*l.last().unwrap(), 0.5);
    assert_eq!(m.reader_state(9, &two), ReaderState::Held);
    assert_eq!(opener.drops(), 1, "the old reader is closed after the swap");
}

/// Every read in `opener`'s log continues where the one before ended.
fn all_sequential(opener: &Opener) -> bool {
    opener
        .reads()
        .windows(2)
        .all(|w| w[1].1.raw() == w[0].1.raw() + w[0].2 as i64 * 14_700)
}

#[test]
fn a_rekeyed_clip_keeps_its_reader_and_ramps_to_its_new_level() {
    // Switching a clip to "All streams mixed" re-keys its stream (AU-038):
    // the same source on the same media axis, at 1/√2.
    let mut m = raw_mixer(48_000);
    m.defer_opens();
    let opener = Opener::with(&[(1, Wave::Dc(0.5))]);
    let old = input(1, 1, Tick::ZERO, secs(10.0), Tick::ZERO);
    let new = MixInput {
        key: 2,
        stream_mix: crate::stream_mix_factor(2),
        ..old.clone()
    };
    m.offer(
        1,
        old.source,
        Tick::ZERO,
        opener.open(&old.source, rate(48_000)),
    );
    assert_eq!(
        left(
            &render(&mut m, &List(vec![old.clone()]), &opener, 0, 480),
            0
        ),
        0.5
    );
    // While key 1 is still scheduled nothing is handed over; once it has
    // left, key 2 needs no reader of its own.
    assert!(!m.can_adopt(&new, |k| k == 1 || k == 2));
    assert!(m.can_adopt(&new, |k| k == 2));
    let mut l = Vec::new();
    for k in 1..6 {
        l.extend(lefts(&render(
            &mut m,
            &List(vec![new.clone()]),
            &opener,
            k * 480,
            480,
        )));
    }
    assert_eq!(l[239], 0.5, "the look-ahead was mixed before the switch");
    assert!(l.iter().all(|&v| v > 0.35), "no gap");
    assert!(largest_step(&l) < 0.002, "{}", largest_step(&l));
    let want = 0.5 * crate::stream_mix_factor(2);
    assert!(
        (l.last().unwrap() - want).abs() < 1e-6,
        "{}",
        l.last().unwrap()
    );
    assert_eq!(m.reader_state(2, &new.source), ReaderState::Held);
    assert_eq!((opener.opens(), opener.drops()), (1, 0));
    assert!(all_sequential(&opener));

    // A reader opened for key 2 before the hand-over (a request already
    // in flight) arrives late: the current reader keeps playing.
    m.offer(
        2,
        new.source,
        samples(2_400),
        opener.open(&new.source, rate(48_000)),
    );
    let l = lefts(&render(
        &mut m,
        &List(vec![new.clone()]),
        &opener,
        2_880,
        480,
    ));
    assert!(l.iter().all(|&v| (v - want).abs() < 1e-6));
    assert_eq!(opener.drops(), 0, "the adopted reader wasn't replaced");
    assert!(all_sequential(&opener));

    // And back to one stream, under the first key again (undo).
    let l = lefts(&render(&mut m, &List(vec![old]), &opener, 3_360, 1_440));
    assert!(l.iter().all(|&v| v > 0.35), "no gap");
    assert!(largest_step(&l) < 0.002, "{}", largest_step(&l));
    assert_eq!(*l.last().unwrap(), 0.5);
    assert_eq!(opener.drops(), 0);
}

/// Stream `stream` of clip 1 (asset 1 on `[0, 10 s)`), keyed `key`, as one
/// of `mixed` streams.
fn clip_stream(key: u64, stream: u32, mixed: usize) -> MixInput {
    MixInput {
        clip: 1,
        source: SourceKey { asset: 1, stream },
        stream_mix: crate::stream_mix_factor(mixed),
        ..input(key, 1, Tick::ZERO, secs(10.0), Tick::ZERO)
    }
}

/// A deferred mixer that has played stream 1 of clip 1 alone (key 1) over
/// `[0, 480)`, with an opener of stream 1 at 0.5 and `stream2`.
fn playing_one_stream(stream2: Option<Wave>) -> (Mixer, Opener) {
    let mut m = raw_mixer(48_000);
    m.defer_opens();
    let mut opener = Opener::default();
    opener.streams.insert((1, 1), Wave::Dc(0.5));
    if let Some(wave) = stream2 {
        opener.streams.insert((1, 2), wave);
    }
    let one = clip_stream(1, 1, 1);
    m.offer(
        1,
        one.source,
        Tick::ZERO,
        opener.open(&one.source, rate(48_000)),
    );
    render(&mut m, &List(vec![one]), &opener, 0, 480);
    (m, opener)
}

#[test]
fn a_clip_switched_to_all_streams_keeps_its_level_until_the_others_sound() {
    // AU-038 during playback: stream 1 carries on under its new key at its
    // old level while stream 2's reader is on its way, then both ramp from
    // the very sample stream 2 joins at, so the clip never dips. Stream 2
    // is listed first: the order doesn't matter.
    let (mut m, opener) = playing_one_stream(Some(Wave::Dc(0.3)));
    let all = List(vec![clip_stream(3, 2, 2), clip_stream(2, 1, 2)]);
    let mut l = lefts(&render(&mut m, &all, &opener, 480, 960));
    assert!(l.iter().all(|&v| v == 0.5), "held while stream 2 has none");
    let two = all.0[0].source;
    m.offer(3, two, samples(2_000), opener.open(&two, rate(48_000)));
    for k in 3..7 {
        l.extend(lefts(&render(&mut m, &all, &opener, k * 480, 480)));
    }
    // `l[i]` is sample 480 + i; stream 2 joins mid-block, at 2 000.
    assert_eq!(l[1_999 - 480], 0.5);
    assert!(l[2_000 - 480] > 0.5, "both ramp from where stream 2 joins");
    assert!(l.windows(2).all(|w| w[1] >= w[0] - 1e-5), "never dips");
    assert!(largest_step(&l) < 0.002, "{}", largest_step(&l));
    let want = 0.8 * crate::stream_mix_factor(2);
    assert!((l[l.len() - 1] - want).abs() < 1e-6, "{}", l[l.len() - 1]);
    assert_eq!((opener.opens(), opener.drops()), (2, 0));
}

#[test]
fn an_offline_stream_does_not_hold_the_clips_level_back() {
    // Stream 2 can't be opened: there is nothing to wait for, and stream 1
    // goes to its share of the mix, as in an export.
    let (mut m, opener) = playing_one_stream(None);
    let all = List(vec![clip_stream(2, 1, 2), clip_stream(3, 2, 2)]);
    let mut l = lefts(&render(&mut m, &all, &opener, 480, 480));
    assert!(l.iter().all(|&v| v == 0.5), "held while stream 2 is opened");
    m.offer(3, all.0[1].source, samples(1_200), None);
    for k in 2..6 {
        l.extend(lefts(&render(&mut m, &all, &opener, k * 480, 480)));
    }
    assert!(largest_step(&l) < 0.002, "ramps: {}", largest_step(&l));
    let want = 0.5 * crate::stream_mix_factor(2);
    assert!((l[l.len() - 1] - want).abs() < 1e-6, "{}", l[l.len() - 1]);
}

#[test]
fn a_new_key_reading_elsewhere_in_the_source_does_not_take_a_reader() {
    // The same source, but a clip that plays it from another point: not a
    // re-key. The old clip ramps out and the new one waits for its reader.
    let mut m = raw_mixer(48_000);
    m.defer_opens();
    let opener = Opener::with(&[(1, Wave::Dc(0.5))]);
    let old = input(1, 1, Tick::ZERO, secs(10.0), Tick::ZERO);
    let elsewhere = input(2, 1, Tick::ZERO, secs(10.0), secs(3.0));
    m.offer(
        1,
        old.source,
        Tick::ZERO,
        opener.open(&old.source, rate(48_000)),
    );
    render(&mut m, &List(vec![old]), &opener, 0, 480);
    assert!(!m.can_adopt(&elsewhere, |k| k == 2));
    let l = lefts(&render(
        &mut m,
        &List(vec![elsewhere.clone()]),
        &opener,
        480,
        1_440,
    ));
    assert!(largest_step(&l) < 0.002, "ramps out: {}", largest_step(&l));
    assert_eq!(*l.last().unwrap(), 0.0);
    assert_eq!(m.reader_state(2, &elsewhere.source), ReaderState::Missing);
    assert_eq!(opener.drops(), 1);
}

#[test]
fn an_export_reuses_the_reader_of_a_rekeyed_clip() {
    // Without deferred opens (export) the hand-over works the same way.
    let mut m = raw_mixer(48_000);
    let opener = Opener::with(&[(1, Wave::Counter)]);
    let old = input(1, 1, Tick::ZERO, secs(10.0), Tick::ZERO);
    let new = MixInput {
        key: 2,
        ..old.clone()
    };
    render(&mut m, &List(vec![old]), &opener, 0, 480);
    let l = lefts(&render(&mut m, &List(vec![new]), &opener, 480, 480));
    assert_eq!((l[0], l[479]), (480.0, 959.0), "exact, at full volume");
    assert_eq!(opener.opens(), 1);
    assert!(all_sequential(&opener));
}

// ------------------------------------------------- keyframes, pan, track stage

const SILENT_DB: f32 = f32::NEG_INFINITY;

/// Volume keyframes at `(source sample, dB, interpolation)` (48 kHz).
fn keyed(list: &[(i64, f32, KeyInterp)]) -> GainEnvelope {
    let keys: Vec<GainKey> = list
        .iter()
        .map(|&(k, db, interp)| GainKey::new(samples(k), db, interp))
        .collect();
    GainEnvelope::Keyframes(GainKeys::new(keys).unwrap())
}

fn rights(out: &[f32]) -> Vec<f32> {
    out.iter().skip(1).step_by(2).copied().collect()
}

#[test]
fn volume_keyframes_shape_the_clip_at_sample_resolution() {
    use KeyInterp::{Hold, Linear};
    let mut m = raw_mixer(48_000);
    // A clip at timeline 1 s playing media from 3 s; keyframes are in media
    // time: silence at 3 s rising (in amplitude) to 0 dB at 3.5 s, held
    // there until 3.75 s, then −20 dB.
    let mut i = input(1, 1, secs(1.0), secs(1.0), secs(3.0));
    i.gain = keyed(&[
        (144_000, SILENT_DB, Linear),
        (168_000, 0.0, Hold),
        (180_000, -20.0, Linear),
    ]);
    let opener = Opener::with(&[(1, Wave::Dc(1.0))]);
    let out = render(&mut m, &List(vec![i]), &opener, 0, 96_000);
    let l = lefts(&out);
    assert_eq!(l, rights(&out), "both channels alike");
    assert!(l[..48_000].iter().all(|&v| v == 0.0), "before the clip");
    assert!(l[96_000 - 1] > 0.0);
    // The rise is linear in amplitude and on time (no lag).
    for k in [240, 6_000, 12_000, 18_000, 23_759] {
        let expected = k as f32 / 24_000.0;
        assert!(
            (l[48_000 + k] - expected).abs() < 1e-6,
            "sample {k}: {}",
            l[48_000 + k]
        );
    }
    // Held at exactly unity, then −20 dB.
    assert!(
        l[48_000 + 24_240..48_000 + 35_760]
            .iter()
            .all(|&v| v == 1.0)
    );
    assert!(
        l[48_000 + 36_241..96_000]
            .iter()
            .all(|&v| (v - 0.1).abs() < 1e-7)
    );
    // The Hold step is spread over 10 ms around its keyframe, without
    // clicks; half-way exactly on the keyframe.
    let step = &l[48_000 + 35_000..48_000 + 37_000];
    assert!(
        largest_step(step) <= 0.9 / 481.0 + 1e-6,
        "{}",
        largest_step(step)
    );
    assert!((l[48_000 + 36_000] - (240.0 + 241.0 * 0.1) / 481.0).abs() < 1e-6);
}

#[test]
fn a_decibel_ramp_is_smooth_and_on_time() {
    use KeyInterp::Linear;
    let mut m = raw_mixer(48_000);
    let mut i = input(1, 1, secs(0.0), secs(1.0), Tick::ZERO);
    i.gain = keyed(&[(0, 0.0, Linear), (48_000, -40.0, Linear)]);
    let env = i.gain.clone();
    let opener = Opener::with(&[(1, Wave::Dc(1.0))]);
    let l = lefts(&render(&mut m, &List(vec![i]), &opener, 0, 48_000));
    for k in (1_000..47_000).step_by(997) {
        let exact = env.amplitude_at(samples(k as i64));
        let db = 20.0 * (f64::from(l[k]) / exact).log10();
        assert!(db.abs() < 0.01, "sample {k}: {db} dB off");
    }
    assert!(l.windows(2).all(|w| w[1] <= w[0]));
}

#[test]
fn dense_keyframes_are_click_free() {
    let mut m = raw_mixer(48_000);
    // Full level and silence alternating every millisecond (48 samples).
    let list: Vec<(i64, f32, KeyInterp)> = (0..1_000)
        .map(|k| {
            (
                k * 48,
                if k % 2 == 0 { 0.0 } else { SILENT_DB },
                KeyInterp::Hold,
            )
        })
        .collect();
    let mut i = input(1, 1, secs(0.0), secs(1.0), Tick::ZERO);
    i.gain = keyed(&list);
    let opener = Opener::with(&[(1, Wave::Dc(1.0))]);
    let mut l = Vec::new();
    for k in 0..40 {
        l.extend(lefts(&render(
            &mut m,
            &List(vec![i.clone()]),
            &opener,
            k * 480,
            480,
        )));
    }
    let middle = &l[1_000..18_000];
    assert!(
        largest_step(middle) <= 1.0 / 481.0 + 1e-6,
        "{}",
        largest_step(middle)
    );
    assert!(middle.iter().all(|&v| v > 0.45 && v < 0.55));
}

#[test]
fn pan_follows_the_balance_law() {
    let opener = Opener::with(&[(1, Wave::Dc(0.5))]);
    let at = |pan: f32, track_pan: f32| {
        let mut i = input(1, 1, secs(0.0), secs(1.0), Tick::ZERO);
        i.pan = pan;
        i.track.pan = track_pan;
        let out = render(&mut raw_mixer(48_000), &List(vec![i]), &opener, 0, 10);
        (out[0], out[1])
    };
    assert_eq!(at(0.0, 0.0), (0.5, 0.5));
    // Towards the right the left channel falls; the right stays at unity.
    assert_eq!(at(0.5, 0.0), (0.25, 0.5));
    assert_eq!(at(1.0, 0.0), (0.0, 0.5));
    assert_eq!(at(-1.0, 0.0), (0.5, 0.0));
    assert_eq!(at(0.0, -0.5), (0.5, 0.25));
    // Clip pan then track pan.
    assert_eq!(at(0.5, -0.5), (0.25, 0.25));
    // A clip panned hard right on a track panned hard left keeps nothing.
    assert_eq!(at(1.0, -1.0), (0.0, 0.0));
}

#[test]
fn streams_mixed_together_keep_the_level_of_one() {
    let mut m = raw_mixer(48_000);
    let opener = Opener::with(&[(1, Wave::Dc(0.5))]);
    let factor = crate::stream_mix_factor(2);
    let streams: Vec<MixInput> = (0..2)
        .map(|s| MixInput {
            key: 10 + s,
            source: SourceKey {
                asset: 1,
                stream: s as u32,
            },
            stream_mix: factor,
            ..input(0, 1, secs(0.0), secs(1.0), Tick::ZERO)
        })
        .collect();
    let out = render(&mut m, &List(streams), &opener, 0, 10);
    // Two correlated streams at −3 dB each sum to +3 dB of one.
    assert!((out[0] - 2.0 * 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    assert_eq!(out[0], out[1]);
}

#[test]
fn track_volume_and_pan_changes_ramp_over_10_ms() {
    let mut m = raw_mixer(48_000);
    let opener = Opener::with(&[(1, Wave::Dc(0.5))]);
    let before = input(1, 1, secs(0.0), secs(10.0), Tick::ZERO);
    let mut after = before.clone();
    after.track = TrackStage {
        id: 4,
        gain: 0.1,
        pan: 1.0,
    };
    render(&mut m, &List(vec![before]), &opener, 0, 480);
    let out = render(&mut m, &List(vec![after]), &opener, 480, 1_440);
    let (l, r) = (lefts(&out), rights(&out));
    // The look-ahead was mixed before the change.
    assert_eq!((l[239], r[239]), (0.5, 0.5));
    for ch in [&l, &r] {
        assert!(largest_step(ch) < 0.002, "{}", largest_step(ch));
        assert!(ch.windows(2).all(|w| w[1] <= w[0]));
    }
    // After 10 ms: right at 0.5 × 0.1, left panned away to silence.
    assert!((r[240 + 480] - 0.05).abs() < 1e-6, "{}", r[240 + 480]);
    assert!(l[240 + 480].abs() < 1e-6, "{}", l[240 + 480]);
    // After a seek the new values apply at once.
    let mut i = input(1, 1, secs(0.0), secs(10.0), Tick::ZERO);
    i.track.gain = 0.1;
    let out = render(&mut m, &List(vec![i]), &opener, 99_000, 10);
    assert!((out[0] - 0.05).abs() < 1e-7);
}

#[test]
fn a_clip_pan_change_ramps() {
    let mut m = raw_mixer(48_000);
    let opener = Opener::with(&[(1, Wave::Dc(0.5))]);
    let centre = input(1, 1, secs(0.0), secs(10.0), Tick::ZERO);
    let left_only = MixInput {
        pan: -1.0,
        ..centre.clone()
    };
    render(&mut m, &List(vec![centre]), &opener, 0, 480);
    let r = rights(&render(&mut m, &List(vec![left_only]), &opener, 480, 1_440));
    assert_eq!(r[239], 0.5);
    assert!(largest_step(&r) < 0.002, "{}", largest_step(&r));
    assert_eq!(r[240 + 480], 0.0);
}

#[test]
fn keyframes_added_or_moved_mid_way_do_not_step() {
    use KeyInterp::Linear;
    let mut m = raw_mixer(48_000);
    let opener = Opener::with(&[(1, Wave::Dc(1.0))]);
    let mut i = input(1, 1, secs(0.0), secs(10.0), Tick::ZERO);
    let half_db = (20.0 * 0.5f64.log10()) as f32;
    i.gain = 0.5.into();
    let mut l = lefts(&render(&mut m, &List(vec![i.clone()]), &opener, 0, 480));
    // The first keyframe captures the static volume: nothing changes.
    i.gain = keyed(&[(0, half_db, Linear)]);
    l.extend(lefts(&render(
        &mut m,
        &List(vec![i.clone()]),
        &opener,
        480,
        960,
    )));
    assert!(
        l.iter().all(|&v| (v - 0.5).abs() < 1e-6),
        "{:?}",
        largest_step(&l)
    );
    // Dragging the line down to −20 dB ramps over 10 ms instead of stepping.
    i.gain = keyed(&[(0, -20.0, Linear), (480_000, -20.0, Linear)]);
    let l = lefts(&render(
        &mut m,
        &List(vec![i.clone()]),
        &opener,
        1_440,
        1_440,
    ));
    assert!((l[239] - 0.5).abs() < 1e-6);
    assert!(largest_step(&l) < 0.002, "{}", largest_step(&l));
    assert!((l[240 + 480] - 0.1).abs() < 1e-6, "{}", l[240 + 480]);
    // And back to a static volume.
    i.gain = 1.0.into();
    let l = lefts(&render(&mut m, &List(vec![i]), &opener, 2_880, 1_440));
    assert!(largest_step(&l) < 0.002, "{}", largest_step(&l));
    assert_eq!(l[240 + 480], 1.0);
}

#[test]
fn keyframes_dragged_to_silence_mid_way_ramp_down() {
    use KeyInterp::Linear;
    // −∞ and the lowest finite volume (−60 dB), which dragging the volume
    // line to the bottom gives, and a value above the old rescaling floor.
    for db in [SILENT_DB, -60.0, -59.0] {
        let mut m = raw_mixer(48_000);
        let opener = Opener::with(&[(1, Wave::Dc(1.0))]);
        let mut i = input(1, 1, secs(0.0), secs(10.0), Tick::ZERO);
        i.gain = keyed(&[(0, 0.0, Linear)]);
        let mut l = lefts(&render(&mut m, &List(vec![i.clone()]), &opener, 0, 960));
        i.gain = keyed(&[(0, db, Linear)]);
        l.extend(lefts(&render(&mut m, &List(vec![i]), &opener, 960, 960)));
        assert!(largest_step(&l) < 0.003, "{db} dB: {}", largest_step(&l));
        let settled = 10f32.powf(db / 20.0);
        assert!((l[1_919] - settled).abs() < 1e-6, "{db} dB: {}", l[1_919]);
    }
}

#[test]
fn keyframes_changed_again_during_a_crossfade_do_not_step() {
    use KeyInterp::Linear;
    let mut m = raw_mixer(48_000);
    let opener = Opener::with(&[(1, Wave::Dc(1.0))]);
    let mut i = input(1, 1, secs(0.0), secs(10.0), Tick::ZERO);
    let mut l = Vec::new();
    // A drag: a new key set every 64 samples, alternating loud and silent.
    for (n, db) in [0.0, SILENT_DB, -6.0, SILENT_DB, 0.0, -60.0, 0.0]
        .into_iter()
        .enumerate()
    {
        i.gain = keyed(&[(0, db, Linear)]);
        l.extend(lefts(&render(
            &mut m,
            &List(vec![i.clone()]),
            &opener,
            n as i64 * 64,
            64,
        )));
    }
    assert!(largest_step(&l) < 0.003, "{}", largest_step(&l));
}

#[test]
fn keyframes_pan_and_track_stage_match_across_block_sizes() {
    use KeyInterp::{Hold, Linear};
    let mut a = input(1, 1, secs(0.0), secs(1.0), secs(0.2));
    a.gain = keyed(&[
        (9_600, 0.0, Linear),
        (20_000, SILENT_DB, Linear),
        (30_000, -6.0, Hold),
        (40_000, 3.0, Linear),
    ]);
    a.fade_in = Fade::new(secs(0.1), FadeCurve::Smooth);
    a.fade_out = Fade::new(secs(0.2), FadeCurve::Fast);
    a.pan = 0.3;
    a.track = TrackStage {
        id: 2,
        gain: 0.7,
        pan: -0.2,
    };
    let mut b = input(2, 2, secs(0.37), secs(0.5), secs(2.0));
    b.stream_mix = crate::stream_mix_factor(3);
    b.fade_out = Fade::new(secs(0.05), FadeCurve::Slow);
    let sched = List(vec![a, b]);
    let opener = Opener::with(&[
        (
            1,
            Wave::Sine {
                amp: 0.8,
                hz: 440.0,
            },
        ),
        (
            2,
            Wave::Sine {
                amp: 0.6,
                hz: 1234.5,
            },
        ),
    ]);
    let total = 48_000;
    let expected = render(&mut Mixer::new(rate(48_000)), &sched, &opener, 500, total);
    let mut pieces = Mixer::new(rate(48_000));
    let mut got = Vec::new();
    let sizes = [1, 7, 13, 240, 241, 997, 4096, 3];
    let (mut pos, mut k) = (0, 0);
    while pos < total {
        let n = sizes[k % sizes.len()].min(total - pos);
        got.extend(render(&mut pieces, &sched, &opener, 500 + pos as i64, n));
        pos += n;
        k += 1;
    }
    assert_eq!(got.iter().zip(&expected).position(|(a, b)| a != b), None);
}
