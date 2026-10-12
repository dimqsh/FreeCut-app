//! Takes written by the recorder, read back through FFmpeg (freecut-media):
//! plain RIFF, RF64 and a file whose writer never finished.
#![cfg(feature = "media")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::path::Path;
use std::time::Duration;

use freecut_audio::record::wav::{self, WavForm, WavSpec};
use freecut_audio::record::{Channels, RecordConfig, Recorder, Signal, SpaceProbe, SyntheticInput};
use freecut_media::AudioReader;
use freecut_timeline::time::{SampleRate, Tick};

const RATE: u32 = 48_000;

/// Decodes `frames` stereo frames of `path` at its own rate.
fn decode(path: &Path, frames: usize) -> Vec<f32> {
    let mut reader = AudioReader::open(path, 0, SampleRate::new(RATE).unwrap(), 2).unwrap();
    assert_eq!(reader.native_rate(), RATE);
    reader.read(Tick::ZERO, frames).unwrap()
}

fn record(path: &Path, rf64_threshold: u64, frames: u64) -> u64 {
    let cfg = RecordConfig {
        channels: Channels::Stereo,
        countdown: Duration::ZERO,
        dither: false,
        rf64_threshold,
        free_space: SpaceProbe::new(|_| Some(u64::MAX)),
        ..RecordConfig::new(path)
    };
    let (input, driver) = SyntheticInput::new(Signal::Ramp, RATE, 2).driven();
    let mut rec = Recorder::open(cfg, Box::new(input)).unwrap();
    rec.start().unwrap();
    let mut fed = 0;
    while fed < frames {
        fed += driver.feed((frames - fed).min(4_800));
        std::thread::sleep(Duration::from_millis(1));
    }
    let take = rec.stop().unwrap();
    take.frames
}

fn check_ramp(decoded: &[f32], frames: usize) {
    assert_eq!(decoded.len(), frames * 2);
    for n in 0..frames {
        let want = Signal::Ramp.value(n as u64, 0, RATE);
        assert_eq!(decoded[2 * n], want, "frame {n}");
        assert_eq!(decoded[2 * n + 1], -want, "frame {n}");
    }
}

#[test]
fn ffmpeg_reads_riff_and_rf64_takes_sample_exact() {
    let dir = tempfile::tempdir().unwrap();
    let riff = dir.path().join("riff.wav");
    assert_eq!(record(&riff, wav::RIFF_LIMIT, 24_000), 24_000);
    assert_eq!(wav::read_info(&riff).unwrap().form, WavForm::Riff);
    check_ramp(&decode(&riff, 24_000), 24_000);

    let rf64 = dir.path().join("rf64.wav");
    assert_eq!(record(&rf64, 50_000, 24_000), 24_000);
    assert_eq!(wav::read_info(&rf64).unwrap().form, WavForm::Rf64);
    let probed = freecut_media::probe(&rf64).unwrap();
    assert_eq!(probed.info.audio.len(), 1, "{:?}", probed.info);
    check_ramp(&decode(&rf64, 24_000), 24_000);
}

#[test]
fn ffmpeg_reads_an_unfinished_file_up_to_its_last_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crash.wav");
    let spec = WavSpec {
        rate: RATE,
        channels: 2,
    };
    let mut w = wav::Writer::create(&path, spec).unwrap();
    let samples: Vec<i32> = (0..4_800u64)
        .flat_map(|n| {
            let v = (Signal::Ramp.value(n, 0, RATE) * 8_388_608.0) as i32;
            [v, -v]
        })
        .collect();
    w.write_samples(&samples).unwrap();
    w.refresh_header().unwrap();
    w.write_samples(&samples[..960]).unwrap();
    drop(w); // never finished, as after a crash
    assert!(wav::marker_path(&path).exists());
    let mut reader = AudioReader::open(&path, 0, SampleRate::new(RATE).unwrap(), 2).unwrap();
    let decoded = reader.read(Tick::ZERO, 4_800).unwrap();
    check_ramp(&decoded, 4_800);
    // After repair the extra frames are part of the file.
    assert_eq!(wav::repair(&path).unwrap().frames(), 5_280);
    assert!(!wav::marker_path(&path).exists());
    let decoded = decode(&path, 5_280);
    check_ramp(&decoded[..9_600], 4_800);
    check_ramp(&decoded[9_600..], 480);
}
