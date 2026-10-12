//! FileOpener + Mixer on a real file decoded by freecut-media.
#![cfg(feature = "media")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use freecut_audio::{Fade, FileOpener, MixInput, Mixer, Schedule, SourceKey, TrackStage};
use freecut_timeline::time::{SampleRate, TICKS_PER_SECOND, Tick, TimeRange};

struct One(MixInput);

impl Schedule for One {
    fn inputs(&self, window: TimeRange) -> Vec<MixInput> {
        if self.0.range.overlaps(window) {
            vec![self.0.clone()]
        } else {
            Vec::new()
        }
    }
}

/// Writes a 1 s, 440 Hz stereo sine (amplitude 1/8) at 48 kHz with the
/// `ffmpeg` CLI. `false` when ffmpeg isn't installed.
fn make_sine(path: &Path) -> bool {
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("sine=frequency=440:sample_rate=48000:duration=1")
        .args(["-af", "pan=stereo|c0=c0|c1=c0", "-c:a", "pcm_s16le"])
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

fn rms(v: &[f32]) -> f64 {
    (v.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>() / v.len() as f64).sqrt()
}

/// Upward zero crossings per second of the left channel.
fn frequency(left: &[f32], rate: u32) -> f64 {
    let ups = left
        .windows(2)
        .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
        .count();
    ups as f64 * f64::from(rate) / left.len() as f64
}

#[test]
fn file_plays_at_its_timeline_offset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sine.wav");
    if !make_sine(&path) {
        return;
    }
    let opener = FileOpener::new(BTreeMap::from([(3, path)]));
    let half = Tick::from_raw(TICKS_PER_SECOND / 2);
    // Placed at 0.5 s on the timeline, playing the file from its start.
    let clip = MixInput {
        key: 1,
        clip: 1,
        source: SourceKey {
            asset: 3,
            stream: 0,
        },
        range: TimeRange::new(half, Tick::from_secs(1).unwrap()).unwrap(),
        source_start: Tick::ZERO,
        gain: 1.0.into(),
        fade_in: Fade::NONE,
        fade_out: Fade::NONE,
        pan: 0.0,
        stream_mix: 1.0,
        track: TrackStage::default(),
    };

    for hz in [48_000u32, 44_100] {
        let rate = SampleRate::new(hz).unwrap();
        let r = hz as usize;
        let mut mixer = Mixer::new(rate);
        // Two seconds in blocks of 10 ms, as playback would pull them.
        let mut out = Vec::new();
        let block = r / 100;
        for k in 0..200 {
            let mut buf = vec![0.0; block * 2];
            mixer.render(&One(clip.clone()), &opener, (k * block) as i64, &mut buf);
            out.extend(buf);
        }
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let right: Vec<f32> = out.iter().skip(1).step_by(2).copied().collect();

        assert!(
            left[..r / 2].iter().all(|&v| v == 0.0),
            "{hz} Hz: sound before the clip"
        );
        assert!(
            left[3 * r / 2..].iter().all(|&v| v == 0.0),
            "{hz} Hz: sound after the clip"
        );
        // The first audible sample is the clip's first sample (the sine
        // starts at 0, so look at the following ones).
        assert!(left[r / 2 + 1] > 0.0 && left[r / 2 + 3] > left[r / 2 + 1]);

        let body = &left[r / 2 + r / 20..3 * r / 2 - r / 20];
        let expected = 0.125 / 2f64.sqrt();
        let level = rms(body);
        assert!(
            (level - expected).abs() < 0.02 * expected,
            "{hz} Hz: RMS {level}"
        );
        let f = frequency(body, hz);
        assert!((f - 440.0).abs() < 3.0, "{hz} Hz: frequency {f}");
        assert_eq!(left, right, "{hz} Hz: channels differ");
    }
}

#[test]
fn missing_files_play_silence() {
    let opener = FileOpener::new(BTreeMap::from([(
        3,
        "/nonexistent/freecut/sine.wav".into(),
    )]));
    let clip = MixInput {
        key: 1,
        clip: 1,
        source: SourceKey {
            asset: 3,
            stream: 0,
        },
        range: TimeRange::new(Tick::ZERO, Tick::from_secs(1).unwrap()).unwrap(),
        source_start: Tick::ZERO,
        gain: 1.0.into(),
        fade_in: Fade::NONE,
        fade_out: Fade::NONE,
        pan: 0.0,
        stream_mix: 1.0,
        track: TrackStage::default(),
    };
    let mut mixer = Mixer::new(SampleRate::new(48_000).unwrap());
    let mut out = vec![1.0; 9600];
    mixer.render(&One(clip), &opener, 0, &mut out);
    assert!(out.iter().all(|&v| v == 0.0));
}
