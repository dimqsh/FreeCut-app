//! The recorder driven by synthetic sources (no microphone).

use std::path::Path;
use std::time::{Duration, Instant};

use super::wav::{self, WavForm};
use super::*;

const RATE: u32 = 8_000;

/// Exact values, no countdown, an endless disk.
fn config(dir: &Path, name: &str) -> RecordConfig {
    RecordConfig {
        countdown: Duration::ZERO,
        dither: false,
        free_space: SpaceProbe::new(|_| Some(u64::MAX)),
        ..RecordConfig::new(dir.join(name))
    }
}

fn open_driven(config: RecordConfig, input: SyntheticInput) -> (Recorder, SyntheticDriver) {
    let (input, driver) = input.driven();
    let recorder = Recorder::open(config, Box::new(input)).unwrap();
    (recorder, driver)
}

fn ramp_source(channels: u16) -> SyntheticInput {
    SyntheticInput::new(Signal::Ramp, RATE, channels)
}

/// Waits (bounded) until the recorder reaches `state`.
fn wait_for(recorder: &Recorder, state: RecorderState) {
    let start = Instant::now();
    while recorder.state() != state {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "still {:?}, waiting for {state:?}",
            recorder.state()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The 24-bit value the ramp's channel `c` of frame `n` writes at `gain`.
fn ramp_24(n: u64, c: usize, gain: f32) -> i32 {
    let v = Signal::Ramp.value(n, c, RATE) * gain;
    (v * 8_388_608.0).round() as i32
}

#[test]
fn a_mono_take_reads_back_sample_exact() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rec, driver) = open_driven(config(dir.path(), "a.wav"), ramp_source(1));
    assert_eq!(rec.state(), RecorderState::Ready);
    assert_eq!(rec.rate().hz(), RATE);
    rec.start().unwrap();
    assert_eq!(rec.state(), RecorderState::Recording);
    assert_eq!(driver.feed(10_000), 10_000);
    assert_eq!(rec.captured_frames(), 10_000);
    let take = rec.stop().unwrap();
    assert_eq!(take.frames, 10_000);
    assert_eq!(take.ended_by, EndReason::User);
    assert_eq!((take.channels, take.rate.hz()), (Channels::Mono, RATE));
    assert_eq!(take.preroll_frames, 0);
    assert_eq!(rec.state(), RecorderState::Ready);
    assert!(!wav::marker_path(&take.path).exists());
    let (info, samples) = wav::read_samples(&take.path).unwrap();
    assert_eq!(
        (info.form, info.rate, info.channels, info.bits),
        (WavForm::Riff, RATE, 1, 24)
    );
    let want: Vec<i32> = (0..10_000).map(|n| ramp_24(n, 0, 1.0)).collect();
    assert_eq!(samples, want);
}

#[test]
fn a_stereo_take_keeps_both_channels_with_the_input_gain() {
    let dir = tempfile::tempdir().unwrap();
    let gain = GainDb::from_hundredths(600);
    let cfg = RecordConfig {
        channels: Channels::Stereo,
        gain,
        ..config(dir.path(), "s.wav")
    };
    let (mut rec, driver) = open_driven(cfg, ramp_source(2));
    rec.start().unwrap();
    driver.feed(4_000);
    let take = rec.stop().unwrap();
    assert_eq!((take.frames, take.channels), (4_000, Channels::Stereo));
    let (info, samples) = wav::read_samples(&take.path).unwrap();
    assert_eq!(info.channels, 2);
    let amp = gain_amplitude(gain);
    let want: Vec<i32> = (0..4_000u64)
        .flat_map(|n| [ramp_24(n, 0, amp), ramp_24(n, 1, amp)])
        .collect();
    assert_eq!(samples, want);
}

#[test]
fn channels_are_mapped_to_the_take() {
    let dir = tempfile::tempdir().unwrap();
    // Two device channels into a mono take: their average.
    let source = SyntheticInput::new(Signal::Dc(0.25), RATE, 2);
    let (mut rec, driver) = open_driven(config(dir.path(), "m.wav"), source);
    rec.start().unwrap();
    driver.feed(100);
    let take = rec.stop().unwrap();
    let (_, samples) = wav::read_samples(&take.path).unwrap();
    assert_eq!(samples, vec![2_097_152; 100]);
    // One device channel into a stereo take: on both sides.
    let cfg = RecordConfig {
        channels: Channels::Stereo,
        ..config(dir.path(), "st.wav")
    };
    let (mut rec, driver) = open_driven(cfg, SyntheticInput::new(Signal::Dc(-0.5), RATE, 1));
    rec.start().unwrap();
    driver.feed(50);
    let take = rec.stop().unwrap();
    let (info, samples) = wav::read_samples(&take.path).unwrap();
    assert_eq!(info.channels, 2);
    assert_eq!(samples, vec![-4_194_304; 100]);
}

#[test]
fn countdown_and_preroll_are_counted_in_frames_and_the_range_end_stops_the_take() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = RecordConfig {
        // 400 frames at 8 kHz.
        countdown: Duration::from_millis(50),
        preroll_frames: 100,
        stop_after_frames: Some(1_000),
        ..config(dir.path(), "punch.wav")
    };
    let (mut rec, driver) = open_driven(cfg, ramp_source(1));
    rec.arm().unwrap();
    assert_eq!(rec.state(), RecorderState::CountingDown);
    driver.feed(300);
    assert_eq!(rec.state(), RecorderState::CountingDown);
    assert_eq!(rec.countdown_left(), Some(Duration::from_micros(12_500)));
    // Nothing is written while counting down.
    assert!(!dir.path().join("punch.wav").exists());
    driver.feed(150);
    assert_eq!(rec.state(), RecorderState::PreRoll);
    assert_eq!(rec.captured_frames(), 50);
    assert_eq!(rec.countdown_left(), None);
    driver.feed(60);
    assert_eq!(rec.state(), RecorderState::Recording);
    // The source keeps delivering; the take ends at exactly 1 000 frames.
    driver.feed(5_000);
    let take = rec.stop().unwrap();
    assert_eq!(take.ended_by, EndReason::RangeEnd);
    assert_eq!((take.frames, take.preroll_frames), (1_000, 100));
    let (_, samples) = wav::read_samples(&take.path).unwrap();
    // The first frame written is the one right after the countdown.
    let want: Vec<i32> = (400..1_400).map(|n| ramp_24(n, 0, 1.0)).collect();
    assert_eq!(samples, want);
    assert_eq!(rec.state(), RecorderState::Ready);
}

#[test]
fn start_cuts_a_countdown_short() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = RecordConfig {
        countdown: Duration::from_secs(3),
        ..config(dir.path(), "short.wav")
    };
    let (mut rec, driver) = open_driven(cfg, ramp_source(1));
    rec.arm().unwrap();
    driver.feed(100);
    rec.start().unwrap();
    assert_eq!(rec.state(), RecorderState::Recording);
    driver.feed(2_400);
    let take = rec.stop().unwrap();
    assert_eq!(take.frames, 2_400);
    let (_, samples) = wav::read_samples(&take.path).unwrap();
    assert_eq!(samples[0], ramp_24(100, 0, 1.0));
}

#[test]
fn a_lost_device_ends_the_take_and_keeps_the_audio() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rec, driver) = open_driven(
        config(dir.path(), "lost.wav"),
        ramp_source(1).fail_after(3_000),
    );
    let events = rec.events();
    rec.start().unwrap();
    assert_eq!(driver.feed(5_000), 3_000);
    wait_for(&rec, RecorderState::Waiting);
    let take = rec.stop().unwrap();
    assert_eq!((take.frames, take.ended_by), (3_000, EndReason::DeviceLost));
    let (_, samples) = wav::read_samples(&take.path).unwrap();
    assert_eq!(samples.len(), 3_000);
    // The recorder waits for another device.
    assert!(matches!(
        rec.arm(),
        Err(RecordError::WrongState(RecorderState::Waiting))
    ));
    assert!(matches!(rec.stop(), Err(RecordError::NotRecording)));
    let finished: Vec<TakeInfo> = events
        .try_iter()
        .filter_map(|e| match e {
            RecorderEvent::Finished(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(finished, [take]);
}

#[test]
fn a_device_lost_while_ready_waits_without_a_take() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rec, driver) = open_driven(config(dir.path(), "w.wav"), ramp_source(1).fail_after(10));
    driver.feed(20);
    assert_eq!(rec.state(), RecorderState::Waiting);
    assert!(matches!(rec.cancel(), Ok(None)));
    assert!(matches!(rec.stop(), Err(RecordError::NotRecording)));
    assert!(!dir.path().join("w.wav").exists());
}

#[test]
fn cancel_works_from_every_state() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = RecordConfig {
        countdown: Duration::from_millis(100),
        ..config(dir.path(), "c1.wav")
    };
    let (mut rec, driver) = open_driven(cfg.clone(), ramp_source(1));

    // Ready: nothing to cancel.
    assert!(matches!(rec.cancel(), Ok(None)));
    assert_eq!(rec.state(), RecorderState::Ready);

    // Counting down: back to Ready, nothing written.
    rec.arm().unwrap();
    driver.feed(10);
    assert!(matches!(rec.cancel(), Ok(None)));
    assert_eq!(rec.state(), RecorderState::Ready);
    driver.feed(2_000);
    assert_eq!(rec.state(), RecorderState::Ready);
    assert!(!cfg.path.exists());

    // Pre-roll, before the start position: nothing kept.
    rec.update(RecordConfig {
        countdown: Duration::ZERO,
        preroll_frames: 1_000,
        ..cfg.clone()
    })
    .unwrap();
    rec.start().unwrap();
    driver.feed(500);
    assert_eq!(rec.state(), RecorderState::PreRoll);
    assert!(matches!(rec.cancel(), Ok(None)));
    assert_eq!(rec.state(), RecorderState::Ready);
    assert!(!cfg.path.exists());
    assert!(!wav::marker_path(&cfg.path).exists());

    // Recording: the take is finished and kept, marked cancelled.
    rec.start().unwrap();
    driver.feed(3_000);
    assert_eq!(rec.state(), RecorderState::Recording);
    let take = rec.cancel().unwrap().unwrap();
    assert_eq!(
        (take.frames, take.preroll_frames, take.ended_by),
        (3_000, 1_000, EndReason::Cancelled)
    );
    assert!(!wav::marker_path(&take.path).exists());
    assert_eq!(wav::read_info(&take.path).unwrap().frames(), 3_000);
    assert_eq!(rec.state(), RecorderState::Ready);

    // Stopping (the take ended by itself): cancel returns that take.
    rec.update(RecordConfig {
        path: dir.path().join("c2.wav"),
        countdown: Duration::ZERO,
        stop_after_frames: Some(100),
        ..cfg.clone()
    })
    .unwrap();
    rec.start().unwrap();
    driver.feed(200);
    let take = rec.cancel().unwrap().unwrap();
    assert_eq!((take.frames, take.ended_by), (100, EndReason::RangeEnd));
    // Returned once.
    assert!(matches!(rec.cancel(), Ok(None)));
    assert!(matches!(rec.stop(), Err(RecordError::NotRecording)));
}

#[test]
fn commands_are_refused_in_the_wrong_state() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rec, driver) = open_driven(config(dir.path(), "x.wav"), ramp_source(1));
    assert!(matches!(rec.stop(), Err(RecordError::NotRecording)));
    assert!(!rec.request_stop());
    rec.start().unwrap();
    assert!(matches!(
        rec.arm(),
        Err(RecordError::WrongState(RecorderState::Recording))
    ));
    assert!(matches!(
        rec.start(),
        Err(RecordError::WrongState(RecorderState::Recording))
    ));
    assert!(matches!(
        rec.update(config(dir.path(), "y.wav")),
        Err(RecordError::WrongState(RecorderState::Recording))
    ));
    driver.feed(10);
    assert!(rec.request_stop());
    let take = rec.stop().unwrap();
    assert_eq!(take.frames, 10);
    assert!(matches!(rec.stop(), Err(RecordError::NotRecording)));
}

#[test]
fn takes_follow_one_another_and_never_replace_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), "one.wav");
    let (mut rec, driver) = open_driven(cfg.clone(), ramp_source(1));
    let events = rec.events();
    rec.start().unwrap();
    driver.feed(100);
    let first = rec.stop().unwrap();
    // Same path again: the writer refuses to replace the first take.
    rec.start().unwrap();
    driver.feed(100);
    let err = rec.stop();
    assert!(matches!(err, Err(RecordError::Io { .. })), "{err:?}");
    assert!(
        events
            .try_iter()
            .any(|e| matches!(e, RecorderEvent::Failed(RecordError::Io { .. })))
    );
    assert_eq!(rec.state(), RecorderState::Ready);
    assert_eq!(wav::read_info(&first.path).unwrap().frames(), 100);
    // A new path records again.
    rec.update(config(dir.path(), "two.wav")).unwrap();
    rec.start().unwrap();
    driver.feed(300);
    let second = rec.stop().unwrap();
    assert_eq!(second.frames, 300);
    let (_, samples) = wav::read_samples(&second.path).unwrap();
    // The ramp continued through the failed take.
    assert_eq!(samples[0], ramp_24(200, 0, 1.0));
}

#[test]
fn a_full_disk_ends_the_take_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = RecordConfig {
        free_space: SpaceProbe::new(|_| Some(1_000)),
        ..config(dir.path(), "full.wav")
    };
    let (mut rec, driver) = open_driven(cfg, ramp_source(1));
    rec.start().unwrap();
    driver.feed(500);
    wait_for(&rec, RecorderState::Ready);
    driver.feed(500);
    let take = rec.stop().unwrap();
    assert_eq!(take.ended_by, EndReason::DiskLow);
    assert!(take.frames > 0 && take.frames <= 500, "{}", take.frames);
    assert_eq!(wav::read_info(&take.path).unwrap().frames(), take.frames);
}

#[test]
fn a_low_disk_warns_once() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = RecordConfig {
        free_space: SpaceProbe::new(|_| Some(100_000_000)),
        ..config(dir.path(), "low.wav")
    };
    let (mut rec, driver) = open_driven(cfg, ramp_source(1));
    let events = rec.events();
    rec.start().unwrap();
    for _ in 0..4 {
        driver.feed(5_000);
        std::thread::sleep(Duration::from_millis(20));
    }
    let take = rec.stop().unwrap();
    assert_eq!(take.ended_by, EndReason::User);
    let warnings = events
        .try_iter()
        .filter(|e| {
            matches!(
                e,
                RecorderEvent::LowDisk {
                    free_bytes: 100_000_000
                }
            )
        })
        .count();
    assert_eq!(warnings, 1);
}

#[test]
fn a_stalled_writer_ends_the_take_with_an_overrun() {
    let dir = tempfile::tempdir().unwrap();
    let (gate, wait) = crossbeam_channel::bounded::<()>(0);
    let (entered_tx, entered) = crossbeam_channel::bounded::<()>(1);
    let cfg = RecordConfig {
        free_space: SpaceProbe::new(move |_| {
            let _ = entered_tx.try_send(());
            // Blocks until the test lets go (drops `gate`).
            let _ = wait.recv();
            Some(u64::MAX)
        }),
        ..config(dir.path(), "over.wav")
    };
    let (mut rec, driver) = open_driven(cfg, ramp_source(1));
    rec.start().unwrap();
    driver.feed(100);
    // The writer wrote the first block and is now stuck checking the disk.
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    // The ring holds 2 s of stereo: 32 000 mono frames at 8 kHz.
    driver.feed(40_000);
    assert_eq!(rec.state(), RecorderState::Stopping);
    drop(gate);
    let take = rec.stop().unwrap();
    assert_eq!(take.ended_by, EndReason::Overrun);
    assert_eq!(take.frames, 100 + 32_000);
    let (_, samples) = wav::read_samples(&take.path).unwrap();
    let want: Vec<i32> = (0..32_100).map(|n| ramp_24(n, 0, 1.0)).collect();
    assert_eq!(samples, want);
}

#[test]
fn a_long_take_switches_to_rf64_on_the_fly() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = RecordConfig {
        rf64_threshold: 10_000,
        header_refresh: Duration::ZERO,
        ..config(dir.path(), "long.wav")
    };
    let (mut rec, driver) = open_driven(cfg.clone(), ramp_source(1));
    rec.start().unwrap();
    driver.feed(1_000);
    // 3 000 bytes: still RIFF on disk.
    wait_until(|| {
        wav::read_info(&cfg.path).is_ok_and(|i| i.frames() == 1_000 && i.form == WavForm::Riff)
    });
    driver.feed(4_000);
    // 15 000 bytes: RF64, with every frame readable mid-take.
    wait_until(|| {
        wav::read_info(&cfg.path).is_ok_and(|i| i.frames() == 5_000 && i.form == WavForm::Rf64)
    });
    let take = rec.stop().unwrap();
    let (info, samples) = wav::read_samples(&take.path).unwrap();
    assert_eq!((info.form, info.frames()), (WavForm::Rf64, 5_000));
    let want: Vec<i32> = (0..5_000).map(|n| ramp_24(n, 0, 1.0)).collect();
    assert_eq!(samples, want);
}

/// Polls `f` (bounded).
fn wait_until(f: impl Fn() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn the_file_on_disk_is_readable_while_recording() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = RecordConfig {
        header_refresh: Duration::ZERO,
        ..config(dir.path(), "live.wav")
    };
    let (mut rec, driver) = open_driven(cfg.clone(), ramp_source(1));
    rec.start().unwrap();
    driver.feed(4_000);
    // A crash now would leave this: a valid file stating every frame
    // written at the last refresh, and the marker for recovery.
    wait_until(|| wav::read_info(&cfg.path).is_ok_and(|i| i.frames() == 4_000));
    assert!(wav::marker_path(&cfg.path).exists());
    let (_, samples) = wav::read_samples(&cfg.path).unwrap();
    assert_eq!(samples.len(), 4_000);
    let take = rec.stop().unwrap();
    assert_eq!(take.frames, 4_000);
}

#[test]
fn dropping_the_recorder_mid_take_keeps_a_finished_file() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), "drop.wav");
    let (mut rec, driver) = open_driven(cfg.clone(), ramp_source(1));
    rec.start().unwrap();
    driver.feed(1_234);
    drop(rec);
    assert!(!driver.is_running());
    assert!(!wav::marker_path(&cfg.path).exists());
    assert_eq!(wav::read_info(&cfg.path).unwrap().frames(), 1_234);
}

#[test]
fn the_meter_is_live_before_recording_and_follows_the_gain() {
    let dir = tempfile::tempdir().unwrap();
    let (mut rec, driver) = open_driven(
        config(dir.path(), "meter.wav"),
        SyntheticInput::new(Signal::Dc(0.5), RATE, 1),
    );
    // 3 s: ten RMS time constants.
    driver.feed(24_000);
    let m = rec.meter();
    let half = 20.0 * 0.5f32.log10();
    assert!((m.peak_db - half).abs() < 0.01, "{m:?}");
    assert!((m.rms_db - half).abs() < 0.01, "{m:?}");
    assert!((m.held_peak_db - half).abs() < 0.01, "{m:?}");
    assert!(!m.clipped);
    // Metering wrote nothing.
    assert!(!dir.path().join("meter.wav").exists());
    // +6 dB takes 0.5 to about −0.02 dBFS: past the clip point.
    rec.set_gain(GainDb::from_hundredths(600));
    driver.feed(8_000);
    let m = rec.meter();
    assert!(m.peak_db > -0.1 && m.peak_db < 0.0, "{m:?}");
    assert!(m.clipped);
    rec.reset_clip();
    assert!(!rec.meter().clipped);
    // The level is clamped to ±20 dB.
    rec.set_gain(GainDb::from_hundredths(-6_000));
    // 2 s: the peak has fallen 40 dB from where it was.
    driver.feed(16_000);
    let m = rec.meter();
    assert!((m.peak_db - (half - 20.0)).abs() < 0.01, "{m:?}");
    // A new take clears the latch.
    rec.set_gain(GainDb::from_hundredths(600));
    driver.feed(800);
    assert!(rec.meter().clipped);
    rec.start().unwrap();
    assert!(!rec.meter().clipped);
}

#[test]
fn monitoring_carries_the_gained_input_only_when_on() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = RecordConfig {
        gain: GainDb::from_hundredths(-602),
        ..config(dir.path(), "mon.wav")
    };
    let (mut rec, driver) = open_driven(cfg, SyntheticInput::new(Signal::Dc(0.5), RATE, 1));
    let mut feed = rec.monitor_feed().unwrap();
    assert!(rec.monitor_feed().is_none());
    assert_eq!(feed.rate(), RATE);
    driver.feed(10);
    assert_eq!(feed.next_frame(), None);
    rec.set_monitor(true);
    driver.feed(10);
    let amp = 0.5 * gain_amplitude(GainDb::from_hundredths(-602));
    for _ in 0..10 {
        let (l, r) = feed.next_frame().unwrap();
        assert!((l - amp).abs() < 1e-6 && l == r, "{l} {r}");
    }
    assert_eq!(feed.next_frame(), None);
}

#[test]
fn takes_record_latency_and_the_first_frame_time() {
    let dir = tempfile::tempdir().unwrap();
    let source = ramp_source(1).with_latency(Duration::from_millis(20));
    let (mut rec, driver) = open_driven(config(dir.path(), "lat.wav"), source);
    assert_eq!(rec.input_latency(), Duration::from_millis(20));
    let before = Instant::now();
    rec.start().unwrap();
    driver.feed(2_400);
    let take = rec.stop().unwrap();
    assert!(take.input_latency >= Duration::from_millis(20));
    let at = take.started_at.instant();
    assert!(at <= Instant::now());
    assert!(
        at + Duration::from_millis(25) >= before,
        "{at:?} vs {before:?}"
    );
    assert!(!take.is_too_short());
    let short = TakeInfo {
        frames: 2_399 + 100,
        preroll_frames: 100,
        ..take
    };
    assert!(short.is_too_short());
}

#[test]
fn a_real_time_source_records_on_its_own_thread() {
    let dir = tempfile::tempdir().unwrap();
    let source = SyntheticInput::new(
        Signal::Sine {
            hz: 440.0,
            amp: 0.5,
        },
        RATE,
        1,
    );
    let rec_cfg = RecordConfig {
        dither: true,
        ..config(dir.path(), "rt.wav")
    };
    let mut rec = Recorder::open(rec_cfg, Box::new(source)).unwrap();
    rec.start().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let take = rec.stop().unwrap();
    // 10 ms blocks: roughly 2 400 frames, a whole number of blocks.
    assert!(
        take.frames >= 800 && take.frames <= 8_000,
        "{}",
        take.frames
    );
    assert_eq!(take.frames % 80, 0);
    let (_, samples) = wav::read_samples(&take.path).unwrap();
    assert_eq!(samples.len() as u64, take.frames);
    let peak = samples.iter().map(|s| s.abs()).max().unwrap();
    assert!((4_100_000..=4_195_000).contains(&peak), "{peak}");
}

#[test]
fn an_unsupported_rate_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let source = SyntheticInput::new(Signal::Silence, 37_800, 1);
    assert!(matches!(
        Recorder::open(config(dir.path(), "r.wav"), Box::new(source)),
        Err(RecordError::UnsupportedRate(37_800))
    ));
}
